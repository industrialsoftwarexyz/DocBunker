use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use docbunker_native_broker::{allowed_open_root, has_supported_signature, path_is_within_allowed};
use docbunker_sandbox::platforms::{HostProfile, QemuConfig, RunscConfig};

const BUNDLED_VM_HASHES: &str = include_str!("../../sandbox/vm/SHA256SUMS");
const SUPPORTED_EXTENSIONS: [&str; 14] = [
    "pdf", "png", "jpg", "jpeg", "webp", "docx", "pptx", "xlsx",
    "gif", "tif", "tiff", "bmp", "epub", "rtf",
];

#[derive(Debug, PartialEq, Eq)]
pub struct PendingDocument {
    pub path: PathBuf,
    pub ack_path: Option<PathBuf>,
}

pub fn pending_document_from_args(
    args: impl IntoIterator<Item = OsString>,
) -> Option<PendingDocument> {
    let args: Vec<OsString> = args.into_iter().collect();
    let ack_path = args
        .iter()
        .filter_map(|argument| argument.to_str())
        .find_map(|argument| argument.strip_prefix("--docbunker-ack="))
        .map(PathBuf::from)
        .filter(|path| valid_ack_path(path));
    let path = args.iter().map(PathBuf::from).find(|path| {
        path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        SUPPORTED_EXTENSIONS
                            .iter()
                            .any(|supported| extension.eq_ignore_ascii_case(supported))
                    })
                // On the native-handoff path the file was chosen by the extension
                // flow, not by the user directly: re-check the magic signature
                // here to close the TOCTOU between the native host's validation
                // and this read (the file could have been swapped meanwhile).
                && (ack_path.is_none() || {
                    has_supported_signature(path)
                        && path_is_within_allowed(path, &allowed_open_root())
                })
    })?;
    Some(PendingDocument { path, ack_path })
}

fn valid_ack_path(path: &Path) -> bool {
    path.is_absolute()
        && path.file_name().is_some_and(|name| name == "result")
        && path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("docbunker-native-"))
        && path.starts_with(std::env::temp_dir())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Mock,
    Subprocess,
    Runsc,
    Vm,
}

impl BackendKind {
    const fn default_for_build(debug_assertions: bool) -> Self {
        if debug_assertions {
            Self::Mock
        } else {
            Self::Vm
        }
    }

    fn ensure_allowed(self, development_backends: bool) -> Result<Self, String> {
        match self {
            Self::Mock | Self::Subprocess if !development_backends => Err(format!(
                "DOCBUNKER_BACKEND={} is disabled in production builds",
                self.as_str()
            )),
            _ => Ok(self),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Subprocess => "subprocess",
            Self::Runsc => "runsc",
            Self::Vm => "vm",
        }
    }
}

impl FromStr for BackendKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "mock" => Ok(Self::Mock),
            "subprocess" => Ok(Self::Subprocess),
            "runsc" => Ok(Self::Runsc),
            "vm" => Ok(Self::Vm),
            other => Err(format!("unknown DOCBUNKER_BACKEND value: {other}")),
        }
    }
}

pub enum BackendConfig {
    Mock,
    Subprocess { worker_bin: PathBuf },
    Runsc(RunscConfig),
    Vm(QemuConfig),
}

impl BackendConfig {
    pub const fn is_isolated(&self) -> bool {
        matches!(self, Self::Runsc(_) | Self::Vm(_))
    }
}

pub struct AppConfig {
    pub backend: BackendConfig,
}

impl AppConfig {
    pub fn from_env(vm_dir: Option<&Path>) -> Result<Self, String> {
        let kind = std::env::var("DOCBUNKER_BACKEND")
            .map(|value| value.parse())
            .unwrap_or_else(|_| Ok(BackendKind::default_for_build(cfg!(debug_assertions))))?
            .ensure_allowed(development_backends_enabled())?;

        let backend = match kind {
            BackendKind::Mock => BackendConfig::Mock,
            BackendKind::Subprocess => BackendConfig::Subprocess {
                worker_bin: resolve_worker_bin()?,
            },
            BackendKind::Runsc => BackendConfig::Runsc(RunscConfig::new(
                env_path("DOCBUNKER_RUNSC_BIN").unwrap_or_else(|| PathBuf::from("runsc")),
                env_path("DOCBUNKER_ROOTFS").unwrap_or_else(|| PathBuf::from("sandbox/rootfs")),
                env_path("DOCBUNKER_TMP_BASE")
                    .unwrap_or_else(|| std::env::temp_dir().join("docbunker")),
            )),
            BackendKind::Vm => BackendConfig::Vm(vm_config(vm_dir)?),
        };
        Ok(Self { backend })
    }
}

const fn development_backends_enabled() -> bool {
    cfg!(any(debug_assertions, feature = "development-backends"))
}

fn vm_config(vm_dir: Option<&Path>) -> Result<QemuConfig, String> {
    let profile = HostProfile::current();
    let kernel_override = env_path("DOCBUNKER_VM_KERNEL");
    let initrd_override = env_path("DOCBUNKER_VM_INITRD");
    let kernel = kernel_override
        .clone()
        .or_else(|| vm_dir.map(|dir| dir.join("kernel")))
        .ok_or_else(|| {
            "VM kernel not configured (set DOCBUNKER_VM_KERNEL or bundle vm/kernel)".to_string()
        })?;
    let initrd = initrd_override
        .clone()
        .or_else(|| vm_dir.map(|dir| dir.join("initramfs.cpio.gz")))
        .ok_or_else(|| {
            "VM initramfs not configured (set DOCBUNKER_VM_INITRD or bundle vm/initramfs.cpio.gz)"
                .to_string()
        })?;

    let bundled_hashes = if kernel_override.is_none() || initrd_override.is_none() {
        Some(parse_vm_hash_manifest(BUNDLED_VM_HASHES)?)
    } else {
        None
    };
    let development = development_backends_enabled();
    let kernel_sha256 = if kernel_override.is_some() {
        override_hash("DOCBUNKER_VM_KERNEL_SHA256", development)?
    } else {
        bundled_hashes.as_ref().map(|hashes| hashes.kernel.clone())
    };
    let initrd_sha256 = if initrd_override.is_some() {
        override_hash("DOCBUNKER_VM_INITRD_SHA256", development)?
    } else {
        bundled_hashes.as_ref().map(|hashes| hashes.initrd.clone())
    };

    let mut config = QemuConfig::new(
        env_path("DOCBUNKER_QEMU_BIN").unwrap_or(profile.qemu_bin),
        kernel,
        initrd,
        env_path("DOCBUNKER_TMP_BASE").unwrap_or_else(|| std::env::temp_dir().join("docbunker-vm")),
        std::env::var("DOCBUNKER_QEMU_ACCEL").unwrap_or_else(|_| profile.accelerator.to_string()),
        std::env::var("DOCBUNKER_QEMU_CPU").unwrap_or_else(|_| profile.cpu_model.to_string()),
        std::env::var("DOCBUNKER_QEMU_MACHINE").unwrap_or_else(|_| profile.machine.to_string()),
    );
    if let Some(hash) = kernel_sha256 {
        config = config.with_kernel_sha256(&hash)?;
    }
    if let Some(hash) = initrd_sha256 {
        config = config.with_initrd_sha256(&hash)?;
    }
    Ok(config)
}

#[derive(Debug, PartialEq, Eq)]
struct VmAssetHashes {
    kernel: String,
    initrd: String,
}

fn parse_vm_hash_manifest(contents: &str) -> Result<VmAssetHashes, String> {
    let mut kernel = None;
    let mut initrd = None;

    for (index, line) in contents.lines().enumerate() {
        let mut fields = line.split_whitespace();
        let hash = fields
            .next()
            .ok_or_else(|| format!("malformed VM SHA256SUMS line {}", index + 1))?;
        let filename_field = fields
            .next()
            .ok_or_else(|| format!("malformed VM SHA256SUMS line {}", index + 1))?;
        let filename = filename_field.strip_prefix('*').unwrap_or(filename_field);
        if fields.next().is_some()
            || hash.len() != 64
            || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!("malformed VM SHA256SUMS line {}", index + 1));
        }

        let destination = match filename {
            "kernel" => &mut kernel,
            "initramfs.cpio.gz" => &mut initrd,
            _ => return Err(format!("unexpected VM asset in SHA256SUMS: {filename}")),
        };
        if destination.replace(hash.to_string()).is_some() {
            return Err(format!("duplicate VM asset in SHA256SUMS: {filename}"));
        }
    }

    Ok(VmAssetHashes {
        kernel: kernel.ok_or_else(|| "VM SHA256SUMS is missing kernel".to_string())?,
        initrd: initrd.ok_or_else(|| "VM SHA256SUMS is missing initramfs.cpio.gz".to_string())?,
    })
}

fn override_hash(name: &str, development: bool) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) => override_hash_for_mode(name, Some(value), development),
        Err(std::env::VarError::NotPresent) => override_hash_for_mode(name, None, development),
        Err(error) => Err(format!("invalid {name}: {error}")),
    }
}

fn override_hash_for_mode(
    name: &str,
    value: Option<String>,
    development: bool,
) -> Result<Option<String>, String> {
    match value {
        Some(value) => Ok(Some(value)),
        None if development => Ok(None),
        None => Err(format!("{name} is required for a VM asset override")),
    }
}

fn resolve_worker_bin() -> Result<PathBuf, String> {
    if let Some(path) = env_path("DOCBUNKER_WORKER_BIN") {
        return Ok(path);
    }
    let name = if cfg!(windows) {
        "renderer-worker.exe"
    } else {
        "renderer-worker"
    };
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(|dir| dir.join(name)));
    sibling
        .filter(|path| path.is_file())
        .ok_or_else(|| "renderer worker binary not found (set DOCBUNKER_WORKER_BIN)".to_string())
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes tests that mutate `DOCBUNKER_ALLOWED_OPEN_DIR` (a process
    /// global — parallel tests would race each other).
    static ALLOWED_DIR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn backend_kind_parses_supported_values() {
        assert_eq!("mock".parse(), Ok(BackendKind::Mock));
        assert_eq!("subprocess".parse(), Ok(BackendKind::Subprocess));
        assert_eq!("runsc".parse(), Ok(BackendKind::Runsc));
        assert_eq!("vm".parse(), Ok(BackendKind::Vm));
        assert!(BackendKind::from_str("other").is_err());
    }

    #[test]
    fn production_policy_rejects_unisolated_backends() {
        assert!(BackendKind::Mock.ensure_allowed(false).is_err());
        assert!(BackendKind::Subprocess.ensure_allowed(false).is_err());
        assert_eq!(
            BackendKind::Runsc.ensure_allowed(false),
            Ok(BackendKind::Runsc)
        );
        assert_eq!(BackendKind::Vm.ensure_allowed(false), Ok(BackendKind::Vm));
    }

    #[test]
    fn development_policy_allows_explicit_unisolated_backends() {
        assert_eq!(
            BackendKind::Mock.ensure_allowed(true),
            Ok(BackendKind::Mock)
        );
        assert_eq!(
            BackendKind::Subprocess.ensure_allowed(true),
            Ok(BackendKind::Subprocess)
        );
    }

    #[test]
    fn non_debug_default_is_vm() {
        assert_eq!(BackendKind::default_for_build(false), BackendKind::Vm);
        assert_eq!(BackendKind::default_for_build(true), BackendKind::Mock);
    }

    #[test]
    fn associated_file_requires_supported_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let document = directory.path().join("document.PDF");
        std::fs::write(&document, b"%PDF").unwrap();
        assert_eq!(
            pending_document_from_args([OsString::from("--ignored"), document.clone().into()]),
            Some(PendingDocument {
                path: document,
                ack_path: None,
            })
        );
        assert_eq!(
            pending_document_from_args([OsString::from("missing.pdf")]),
            None
        );
    }

    #[test]
    fn native_handoff_ack_path_is_accepted_only_in_private_temp_directory() {
        let _guard = ALLOWED_DIR_ENV_LOCK.lock().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let document = directory.path().join("document.pdf");
        std::fs::write(&document, b"%PDF-1.7").unwrap();
        let ack_directory = tempfile::Builder::new()
            .prefix("docbunker-native-")
            .tempdir()
            .unwrap();
        let ack = ack_directory.path().join("result");
        // The native-handoff path must also be inside the allowed download
        // directory; point the override at the fixture directory.
        std::env::set_var("DOCBUNKER_ALLOWED_OPEN_DIR", directory.path());
        let pending = pending_document_from_args([
            document.clone().into_os_string(),
            OsString::from(format!("--docbunker-ack={}", ack.display())),
        ])
        .unwrap();
        std::env::remove_var("DOCBUNKER_ALLOWED_OPEN_DIR");
        assert_eq!(pending.path, document);
        assert_eq!(pending.ack_path, Some(ack));
    }

    #[test]
    fn native_handoff_rechecks_signature_to_close_toctou() {
        let directory = tempfile::tempdir().unwrap();
        // Looks like a PDF by extension, but the content was swapped after the
        // native host validated it: the startup path must re-reject it.
        let document = directory.path().join("document.pdf");
        std::fs::write(&document, b"<html>not a pdf</html>").unwrap();
        let ack_directory = tempfile::Builder::new()
            .prefix("docbunker-native-")
            .tempdir()
            .unwrap();
        let ack = ack_directory.path().join("result");
        assert_eq!(
            pending_document_from_args([
                document.clone().into_os_string(),
                OsString::from(format!("--docbunker-ack={}", ack.display())),
            ]),
            None
        );
    }

    #[test]
    fn native_handoff_rejects_paths_outside_the_allowed_download_directory() {
        let _guard = ALLOWED_DIR_ENV_LOCK.lock().unwrap();
        let document_dir = tempfile::tempdir().unwrap();
        let document = document_dir.path().join("document.pdf");
        std::fs::write(&document, b"%PDF-1.7").unwrap();
        let ack_directory = tempfile::Builder::new()
            .prefix("docbunker-native-")
            .tempdir()
            .unwrap();
        let ack = ack_directory.path().join("result");
        // Allowed dir is a *different* directory: the hand-off must refuse to
        // open a file the broker was not allowed to hand over.
        let allowed_dir = tempfile::tempdir().unwrap();
        std::env::set_var("DOCBUNKER_ALLOWED_OPEN_DIR", allowed_dir.path());
        assert_eq!(
            pending_document_from_args([
                document.clone().into_os_string(),
                OsString::from(format!("--docbunker-ack={}", ack.display())),
            ]),
            None
        );
        std::env::remove_var("DOCBUNKER_ALLOWED_OPEN_DIR");
    }

    #[test]
    fn vm_hash_manifest_parses_required_assets() {
        let hashes = parse_vm_hash_manifest(
            "76A7F2EF15FCBD2F5C25CD7E7B413F903B2078396063557F1DFFB4A0B089A964  kernel\n\
             B97A391C617E77A8AA7EDEEB509E8CF14F3424090F7639A8449FADEB827EE5E6 *initramfs.cpio.gz\n",
        )
        .expect("valid manifest");
        assert_eq!(
            hashes,
            VmAssetHashes {
                kernel: "76A7F2EF15FCBD2F5C25CD7E7B413F903B2078396063557F1DFFB4A0B089A964".into(),
                initrd: "B97A391C617E77A8AA7EDEEB509E8CF14F3424090F7639A8449FADEB827EE5E6".into(),
            }
        );
    }

    #[test]
    fn vm_hash_manifest_rejects_missing_or_malformed_assets() {
        assert!(parse_vm_hash_manifest(&format!("{}  kernel\n", "0".repeat(64))).is_err());
        assert!(parse_vm_hash_manifest("not-a-hash  kernel\n").is_err());
        assert!(parse_vm_hash_manifest(&format!(
            "{}  kernel\n{}  kernel\n{}  initramfs.cpio.gz\n",
            "0".repeat(64),
            "1".repeat(64),
            "2".repeat(64)
        ))
        .is_err());
    }

    #[test]
    fn production_vm_override_requires_hash() {
        assert!(override_hash_for_mode("DOCBUNKER_VM_KERNEL_SHA256", None, false).is_err());
        assert_eq!(
            override_hash_for_mode("DOCBUNKER_VM_KERNEL_SHA256", None, true),
            Ok(None)
        );
        assert_eq!(
            override_hash_for_mode("DOCBUNKER_VM_KERNEL_SHA256", Some("0".repeat(64)), false),
            Ok(Some("0".repeat(64)))
        );
    }
}
