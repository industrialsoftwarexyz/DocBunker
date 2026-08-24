//! Native-messaging host **registration** for Chrome, Edge, and Firefox.
//!
//! The inbound native-message loop lives in the dedicated `docbunker-native-broker`
//! binary (`crates/native-broker`), which is the only component the browser
//! extension can reach. The app writes the host manifest and registry keys —
//! and verifies the broker is actually deployed next to it so we never
//! register a dangling host.

#[cfg(any(target_os = "windows", test))]
use std::path::PathBuf;

#[cfg(target_os = "windows")]
use docbunker_native_broker::{EXTENSION_ORIGIN, HOST_NAME};

/// Locate the broker binary (a sibling of the app executable).
///
/// Release packaging must ship `docbunker-native-broker` next to the app;
/// `DOCBUNKER_NATIVE_BROKER_BIN` overrides it for developers.
#[cfg(any(target_os = "windows", test))]
pub fn broker_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DOCBUNKER_NATIVE_BROKER_BIN") {
        return Some(PathBuf::from(path));
    }
    let name = if cfg!(windows) {
        "docbunker-native-broker.exe"
    } else {
        "docbunker-native-broker"
    };
    std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(|dir| dir.join(name)))
}

/// The broker must exist before we point the browser at it; otherwise the
/// browser would either fail loudly (misconfiguration) or, worse, invoke nothing.
#[cfg(target_os = "windows")]
fn broker_binary_or_error() -> Result<PathBuf, String> {
    broker_binary()
        .filter(|path| path.is_file())
        .ok_or_else(|| {
            "DocBunker native-messaging broker is missing (ship docbunker-native-broker next to the app)"
                .to_string()
        })
}

/// Write a native-messaging host manifest JSON file at `manifest_path`.
fn write_manifest(manifest_path: &std::path::Path, broker: &std::path::Path) -> Result<(), String> {
    let manifest = serde_json::json!({
        "name": HOST_NAME,
        "description": "Open downloaded webmail attachments in DocBunker",
        "path": broker,
        "type": "stdio",
        "allowed_origins": [EXTENSION_ORIGIN],
    });
    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create native host directory: {error}"))?;
    }
    std::fs::write(
        manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("cannot write native host manifest: {error}"))
}

// ---------------------------------------------------------------------------
// Windows — register via registry for Chrome, Edge, and Firefox
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
pub fn register() -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let broker = broker_binary_or_error()?;
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA is unavailable".to_string())?
        .join("DocBunker")
        .join("NativeHosts");
    std::fs::create_dir_all(&base)
        .map_err(|error| format!("cannot create native host directory: {error}"))?;
    let manifest_path = base.join(format!("{HOST_NAME}.json"));
    write_manifest(&manifest_path, &broker)?;

    let manifest_str = manifest_path.to_string_lossy().to_string();
    let chrome = RegKey::predef(HKEY_CURRENT_USER);

    // Chrome
    let (key, _) = chrome
        .create_subkey(format!(
            "Software\\Google\\Chrome\\NativeMessagingHosts\\{HOST_NAME}"
        ))
        .map_err(|error| format!("cannot register Chrome native host: {error}"))?;
    key.set_value("", &manifest_str)
        .map_err(|error| format!("cannot register Chrome native host manifest: {error}"))?;

    // Edge (same manifest format as Chrome)
    let (key, _) = chrome
        .create_subkey(format!(
            "Software\\Microsoft\\Edge\\NativeMessagingHosts\\{HOST_NAME}"
        ))
        .map_err(|error| format!("cannot register Edge native host: {error}"))?;
    key.set_value("", &manifest_str)
        .map_err(|error| format!("cannot register Edge native host manifest: {error}"))?;

    // Firefox (registry path mirrors Chrome's structure)
    let (key, _) = chrome
        .create_subkey(format!(
            "Software\\Mozilla\\NativeMessagingHosts\\{HOST_NAME}"
        ))
        .map_err(|error| format!("cannot register Firefox native host: {error}"))?;
    key.set_value("", &manifest_str)
        .map_err(|error| format!("cannot register Firefox native host manifest: {error}"))
}

// ---------------------------------------------------------------------------
// macOS — write manifest files to per-browser directories
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
pub fn register() -> Result<(), String> {
    let broker = std::env::var_os("DOCBUNKER_NATIVE_BROKER_BIN")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("docbunker-native-broker")))
        })
        .filter(|path| path.is_file())
        .ok_or_else(|| {
            "DocBunker native-messaging broker is missing (ship docbunker-native-broker next to the app)"
                .to_string()
        })?;

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is unavailable".to_string())?;
    let filename = format!("{HOST_NAME}.json");

    // Chrome
    let chrome_dir = home.join("Library/Application Support/Google/Chrome/NativeMessagingHosts");
    write_manifest(&chrome_dir.join(&filename), &broker)?;

    // Edge
    let edge_dir = home.join("Library/Application Support/Microsoft Edge/NativeMessagingHosts");
    write_manifest(&edge_dir.join(&filename), &broker)?;

    // Firefox
    let firefox_dir = home.join(".mozilla/native-messaging-hosts");
    write_manifest(&firefox_dir.join(&filename), &broker)
}

// ---------------------------------------------------------------------------
// Linux — write manifest files to per-browser directories
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
pub fn register() -> Result<(), String> {
    let broker = std::env::var_os("DOCBUNKER_NATIVE_BROKER_BIN")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("docbunker-native-broker")))
        })
        .filter(|path| path.is_file())
        .ok_or_else(|| {
            "DocBunker native-messaging broker is missing (ship docbunker-native-broker next to the app)"
                .to_string()
        })?;

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is unavailable".to_string())?;
    let filename = format!("{HOST_NAME}.json");

    // Chrome
    let chrome_dir = home.join(".config/google-chrome/NativeMessagingHosts");
    write_manifest(&chrome_dir.join(&filename), &broker)?;

    // Chromium
    let chromium_dir = home.join(".config/chromium/NativeMessagingHosts");
    write_manifest(&chromium_dir.join(&filename), &broker)?;

    // Edge
    let edge_dir = home.join(".config/microsoft-edge/NativeMessagingHosts");
    write_manifest(&edge_dir.join(&filename), &broker)?;

    // Firefox
    let firefox_dir = home.join(".mozilla/native-messaging-hosts");
    write_manifest(&firefox_dir.join(&filename), &broker)
}

// ---------------------------------------------------------------------------
// Fallback for other platforms (shouldn't happen in practice)
// ---------------------------------------------------------------------------

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub fn register() -> Result<(), String> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes tests that mutate `DOCBUNKER_NATIVE_BROKER_BIN` (a process
    /// global — parallel tests would race each other).
    static BROKER_BIN_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn broker_name_is_platform_consistent() {
        let _guard = BROKER_BIN_ENV_LOCK.lock().unwrap();
        let name = if cfg!(windows) {
            "docbunker-native-broker.exe"
        } else {
            "docbunker-native-broker"
        };
        let executable = std::env::current_exe().unwrap();
        let expected = executable.parent().unwrap().join(name);
        let previous = std::env::var_os("DOCBUNKER_NATIVE_BROKER_BIN");
        std::env::remove_var("DOCBUNKER_NATIVE_BROKER_BIN");
        assert_eq!(broker_binary().as_ref(), Some(&expected));
        match previous {
            Some(value) => std::env::set_var("DOCBUNKER_NATIVE_BROKER_BIN", value),
            None => std::env::remove_var("DOCBUNKER_NATIVE_BROKER_BIN"),
        }
    }

    #[test]
    fn broker_override_is_honored() {
        let _guard = BROKER_BIN_ENV_LOCK.lock().unwrap();
        let path = PathBuf::from("C:\\DocBunker\\broker.exe");
        let previous = std::env::var_os("DOCBUNKER_NATIVE_BROKER_BIN");
        std::env::set_var("DOCBUNKER_NATIVE_BROKER_BIN", &path);
        assert_eq!(broker_binary().as_deref(), Some(path.as_path()));
        match previous {
            Some(value) => std::env::set_var("DOCBUNKER_NATIVE_BROKER_BIN", value),
            None => std::env::remove_var("DOCBUNKER_NATIVE_BROKER_BIN"),
        }
    }
}
