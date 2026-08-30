use std::path::{Path, PathBuf};

use crate::version::SemVer;

/// Native QEMU defaults for the current host OS and CPU architecture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostProfile {
    pub qemu_bin: PathBuf,
    pub accelerator: &'static str,
    pub cpu_model: &'static str,
    pub machine: &'static str,
}

impl HostProfile {
    pub fn current() -> Self {
        let binary = if cfg!(target_arch = "aarch64") {
            "qemu-system-aarch64"
        } else {
            "qemu-system-x86_64"
        };
        let qemu_bin = installed_qemu(binary).unwrap_or_else(|| PathBuf::from(binary));

        Self {
            qemu_bin,
            accelerator: if cfg!(target_os = "windows") {
                "whpx"
            } else if cfg!(target_os = "macos") {
                "hvf"
            } else {
                "kvm"
            },
            cpu_model: if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
                "qemu64,svm=off"
            } else {
                "host"
            },
            machine: if cfg!(target_arch = "aarch64") {
                "virt"
            } else {
                "q35"
            },
        }
    }
}

#[cfg(target_os = "windows")]
fn installed_qemu(binary: &str) -> Option<PathBuf> {
    let candidate = PathBuf::from(std::env::var_os("ProgramFiles")?)
        .join("qemu")
        .join(format!("{binary}.exe"));
    candidate.is_file().then_some(candidate)
}

#[cfg(not(target_os = "windows"))]
fn installed_qemu(_binary: &str) -> Option<PathBuf> {
    None
}

/// Fully resolved runtime configuration for the QEMU VM backend.
#[derive(Debug, Clone)]
pub struct QemuConfig {
    pub qemu_bin: PathBuf,
    pub kernel: PathBuf,
    pub initrd: PathBuf,
    pub tmp_base: PathBuf,
    pub accel: String,
    pub cpu_model: String,
    pub machine: String,
    pub(crate) kernel_sha256: Option<[u8; 32]>,
    pub(crate) initrd_sha256: Option<[u8; 32]>,
    /// Recommended minimum QEMU version. If the detected version is below this,
    /// a warning is logged but the backend is not blocked.
    pub(crate) min_version: Option<SemVer>,
}

impl QemuConfig {
    pub fn new(
        qemu_bin: impl AsRef<Path>,
        kernel: impl AsRef<Path>,
        initrd: impl AsRef<Path>,
        tmp_base: impl AsRef<Path>,
        accel: impl Into<String>,
        cpu_model: impl Into<String>,
        machine: impl Into<String>,
    ) -> Self {
        Self {
            qemu_bin: qemu_bin.as_ref().to_path_buf(),
            kernel: kernel.as_ref().to_path_buf(),
            initrd: initrd.as_ref().to_path_buf(),
            tmp_base: tmp_base.as_ref().to_path_buf(),
            accel: accel.into(),
            cpu_model: cpu_model.into(),
            machine: machine.into(),
            kernel_sha256: None,
            initrd_sha256: None,
            min_version: None,
        }
    }

    /// Require the VM assets to match the supplied hexadecimal SHA-256 digests.
    pub fn with_expected_sha256(
        mut self,
        kernel_sha256: &str,
        initrd_sha256: &str,
    ) -> Result<Self, String> {
        self.kernel_sha256 = Some(parse_sha256(kernel_sha256)?);
        self.initrd_sha256 = Some(parse_sha256(initrd_sha256)?);
        Ok(self)
    }

    pub fn with_kernel_sha256(mut self, sha256: &str) -> Result<Self, String> {
        self.kernel_sha256 = Some(parse_sha256(sha256)?);
        Ok(self)
    }

    pub fn with_initrd_sha256(mut self, sha256: &str) -> Result<Self, String> {
        self.initrd_sha256 = Some(parse_sha256(sha256)?);
        Ok(self)
    }

    /// Set the recommended minimum QEMU version (warning only, not blocking).
    pub fn with_min_version(mut self, version: &str) -> Result<Self, String> {
        self.min_version = Some(
            SemVer::parse(version)
                .ok_or_else(|| format!("invalid QEMU minimum version: {version}"))?,
        );
        Ok(self)
    }
}

fn parse_sha256(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64 {
        return Err("SHA-256 digest must contain exactly 64 hexadecimal characters".into());
    }
    if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("SHA-256 digest contains a non-hexadecimal character".into());
    }

    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        let offset = index * 2;
        *byte = u8::from_str_radix(&value[offset..offset + 2], 16)
            .map_err(|_| "SHA-256 digest contains a non-hexadecimal character")?;
    }
    Ok(digest)
}
