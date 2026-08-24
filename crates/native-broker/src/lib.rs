//! Shared native-host validation.
//!
//! DocBunker is registered with Chrome as a *native messaging host*. The
//! browser speaks to whatever binary the manifest points at; that binary must
//! therefore be as small and restricted as possible. This library powers
//! `docbunker-native-broker` (a dedicated binary with **no WebView, no Tauri,
//! no document parsers**), and the main app uses the same validation on the
//! startup path for defense in depth (TOCTOU re-check).
//!
//! Security properties enforced here:
//!
//! - only a fixed set of document extensions and magic signatures;
//! - the file must exist, be a regular file, and fit `MAX_DOCUMENT_SIZE`;
//! - the file must live inside the user's allowed download directory
//!   (canonicalized, so symlinks/junctions cannot escape it) — threat
//!   model A22;
//! - the ack path is validated separately by the app (`app_config.rs`).

use std::io::Read;
use std::path::Path;

use docbunker_renderer_api::limits::MAX_DOCUMENT_SIZE;

/// Chrome native-messaging host name (registry key + manifest name).
pub const HOST_NAME: &str = "dev.docbunker.viewer";

/// The only extension origin allowed to reach DocBunker (Chrome Manifest V3).
pub const EXTENSION_ORIGIN: &str = "chrome-extension://lmmdckggliegiglepibblfnpaiaeojpf/";

/// Chrome native-messaging message size cap (64 KiB, per browser docs).
pub const MAX_NATIVE_MESSAGE_SIZE: usize = 64 * 1024;

pub const SUPPORTED_EXTENSIONS: [&str; 14] = [
    "pdf", "png", "jpg", "jpeg", "webp", "docx", "pptx", "xlsx",
    "gif", "tif", "tiff", "bmp", "epub", "rtf",
];

/// Whether the extension reveals a supported document type by name.
pub fn is_supported_document(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

/// Whether the file content starts with a supported document signature.
pub fn has_supported_signature(path: &Path) -> bool {
    let mut header = [0; 12];
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(read) = file.read(&mut header) else {
        return false;
    };
    let header = &header[..read];
    header.starts_with(b"%PDF-")
        || header.starts_with(b"\x89PNG\r\n\x1a\n")
        || header.starts_with(b"\xff\xd8\xff")
        || (header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP")
        // ZIP local-file header: Office containers + EPUB (ADR-007, ADR-010).
        || header.starts_with(b"PK\x03\x04")
        // GIF
        || header.starts_with(b"GIF87a")
        || header.starts_with(b"GIF89a")
        // TIFF (little-endian or big-endian)
        || header.starts_with(b"II\x2a\x00")
        || header.starts_with(b"MM\x00\x2a")
        // BMP
        || header.starts_with(b"BM")
        // RTF
        || header.starts_with(b"{\\rtf")
}

/// The directory the browser hand-off may read files from.
///
/// Defaults to the user's `Downloads` folder (the Chrome default download
/// location). `DOCBUNKER_ALLOWED_OPEN_DIR` overrides it for redirected
/// download folders. Anything outside is rejected: a compromised or abused
/// extension must not be able to read arbitrary host files.
pub fn allowed_open_root() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("DOCBUNKER_ALLOWED_OPEN_DIR") {
        return std::path::PathBuf::from(dir);
    }
    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(home_var)
        .map(std::path::PathBuf::from)
        .map(|home| home.join("Downloads"))
        .unwrap_or_else(std::env::temp_dir)
}

/// Whether `path` is a regular file strictly inside `root`, **after**
/// canonicalization (symlinks/junctions resolving outside are rejected).
///
/// The returned error string is intentionally generic: it never leaks
/// filesystem layout to the browser.
pub fn validate_document_path(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute() || !is_supported_document(path) {
        return Err("unsupported request");
    }
    if !has_supported_signature(path) {
        return Err("unsupported request");
    }
    let metadata = path.metadata().map_err(|_| "attachment unavailable")?;
    if !metadata.is_file() {
        return Err("unsupported request");
    }
    if metadata.len() > MAX_DOCUMENT_SIZE as u64 {
        return Err("attachment exceeds DocBunker limits");
    }
    let root = allowed_open_root();
    if !path_is_within_allowed(path, &root) {
        return Err("attachment outside the allowed download directory");
    }
    Ok(())
}

/// Canonicalized containment check; both paths must exist.
pub fn path_is_within_allowed(path: &Path, root: &Path) -> bool {
    let Ok(canonical_path) = path.canonicalize() else {
        return false;
    };
    let Ok(canonical_root) = root.canonicalize() else {
        return false;
    };
    if !canonical_root.is_dir() {
        return false;
    }
    canonical_path
        .strip_prefix(&canonical_root)
        .map(|relative| !relative.as_os_str().is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Serializes tests that mutate `DOCBUNKER_ALLOWED_OPEN_DIR` (a process
    /// global — parallel tests would race each other).
    static ALLOWED_DIR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn fixture_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temporary directory")
    }

    fn supported_pdf(path: &Path) {
        let mut file = std::fs::File::create(path).expect("create fixture");
        file.write_all(b"%PDF-1.7\n").expect("write fixture");
    }

    #[test]
    fn only_supported_document_extensions_are_accepted() {
        assert!(is_supported_document(Path::new("attachment.PDF")));
        assert!(is_supported_document(Path::new("attachment.webp")));
        assert!(!is_supported_document(Path::new("attachment.html")));
        assert!(!is_supported_document(Path::new("attachment")));
    }

    #[test]
    fn recognizes_only_supported_document_signatures() {
        let directory = fixture_dir();
        let pdf = directory.path().join("attachment.pdf");
        supported_pdf(&pdf);
        assert!(has_supported_signature(&pdf));
        std::fs::write(&pdf, b"<html>").expect("overwrite");
        assert!(!has_supported_signature(&pdf));
        assert!(!has_supported_signature(
            &directory.path().join("missing.pdf")
        ));
    }

    #[test]
    fn accepted_paths_must_be_absolute_and_supported() {
        let _guard = ALLOWED_DIR_ENV_LOCK.lock().unwrap();
        let root = fixture_dir();
        std::env::set_var("DOCBUNKER_ALLOWED_OPEN_DIR", root.path());
        let document = root.path().join("attachment.pdf");
        supported_pdf(&document);
        assert!(validate_document_path(&document).is_ok());
        assert!(validate_document_path(Path::new("attachment.pdf")).is_err());
        assert!(validate_document_path(&root.path().join("attachment.html")).is_err());
        assert!(validate_document_path(&root.path().join("missing.pdf")).is_err());
        std::env::remove_var("DOCBUNKER_ALLOWED_OPEN_DIR");
    }

    #[test]
    fn paths_outside_the_allowed_root_are_rejected() {
        let root = fixture_dir();
        let outside = fixture_dir();
        let document = outside.path().join("attachment.pdf");
        supported_pdf(&document);
        assert!(!path_is_within_allowed(&document, root.path()));
        assert!(validate_document_path(&document).is_err());
    }

    #[test]
    fn symlink_escaping_the_root_is_rejected() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let root = fixture_dir();
            let outside = fixture_dir();
            let secret = outside.path().join("attachment.pdf");
            supported_pdf(&secret);
            let link = root.path().join("escape.pdf");
            symlink(&secret, &link).expect("symlink");
            assert!(!path_is_within_allowed(&link, root.path()));
        }
    }

    #[test]
    fn allowed_root_defaults_to_downloads_and_honors_override() {
        let _guard = ALLOWED_DIR_ENV_LOCK.lock().unwrap();
        let previous = std::env::var_os("DOCBUNKER_ALLOWED_OPEN_DIR");
        std::env::remove_var("DOCBUNKER_ALLOWED_OPEN_DIR");
        let root = allowed_open_root();
        let home = if cfg!(windows) {
            std::env::var_os("USERPROFILE")
        } else {
            std::env::var_os("HOME")
        };
        if let Some(home) = home {
            assert!(root.ends_with("Downloads"));
            let _ = home;
        }
        match previous {
            Some(value) => std::env::set_var("DOCBUNKER_ALLOWED_OPEN_DIR", value),
            None => std::env::remove_var("DOCBUNKER_ALLOWED_OPEN_DIR"),
        }
    }

    #[test]
    fn override_dir_is_used() {
        let _guard = ALLOWED_DIR_ENV_LOCK.lock().unwrap();
        let dir = fixture_dir();
        std::env::set_var("DOCBUNKER_ALLOWED_OPEN_DIR", dir.path());
        assert_eq!(allowed_open_root(), dir.path());
        std::env::remove_var("DOCBUNKER_ALLOWED_OPEN_DIR");
    }
}
