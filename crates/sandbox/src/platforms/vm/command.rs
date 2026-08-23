use std::path::Path;
use std::process::Command;

use crate::config::SandboxConfig;

use super::QemuConfig;

pub(super) const VM_MEMORY_HEADROOM_MB: u64 = 768;
pub(super) const VM_MEMORY_MIN_MB: u64 = 1024;

/// QEMU hardening flags: every host-visible surface is disabled.
///
/// Kept as the single source of truth so tests can assert the exact set and
/// reviewers can audit the QEMU attack surface in one place (threat model
/// A10 — the outer VM boundary is the critical TCB on Windows/macOS).
pub(super) fn hardening_flags() -> Vec<&'static str> {
    let mut flags = vec![
        "-display",
        "none", // no graphics / input devices
        "-monitor",
        "none",        // no HMP console
        "-nodefaults", // no default devices at all
        // QMP is not enabled: `-qmp` is never passed, and `-nodefaults`
        // skips the default monitor chardev. (`-qmp none` is rejected by
        // QEMU 11+, so the QMP channel is disabled by omission instead.)
        "-nic",
        "none",       // no network interface
        "-no-reboot", // guest cannot reboot the VM
    ];
    // QEMU's seccomp sandbox is only built on Unix hosts (libseccomp);
    // Windows builds do not support it.
    if cfg!(not(target_os = "windows")) {
        flags.push("-sandbox");
        flags.push("on");
    }
    flags
}

pub(super) fn build_qemu_command(
    qemu: &QemuConfig,
    sandbox: &SandboxConfig,
    log_path: &Path,
    port: u16,
) -> Command {
    let mut command = Command::new(&qemu.qemu_bin);
    command
        .arg("-accel")
        .arg(&qemu.accel)
        .arg("-machine")
        .arg(&qemu.machine)
        .arg("-cpu")
        .arg(&qemu.cpu_model)
        .arg("-m")
        .arg(format!("{}M", vm_memory_mb(sandbox)))
        .args(["-smp", "2"])
        .args(hardening_flags())
        .arg("-kernel")
        .arg(&qemu.kernel)
        .arg("-initrd")
        .arg(&qemu.initrd)
        .arg("-append")
        // The guest has no AppArmor policy; Ubuntu's built-in restriction
        // would otherwise block the user namespace required by runsc.
        .arg("console=ttyS0 rdinit=/init panic=-1 oops=panic apparmor=0")
        .arg("-serial")
        .arg(format!("file:{}", qemu_path(log_path)))
        .arg("-chardev")
        .arg(format!(
            "socket,id=char0,host=127.0.0.1,port={port},nodelay=on"
        ))
        .args(["-device", "virtio-serial-pci,id=vser0"])
        .args([
            "-device",
            "virtserialport,chardev=char0,id=port0,name=docbunker",
        ]);
    command
}

pub(super) fn qemu_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(super) fn vm_memory_mb(config: &SandboxConfig) -> u64 {
    config
        .memory_limit_bytes
        .map(|memory| memory / (1024 * 1024) + VM_MEMORY_HEADROOM_MB)
        .unwrap_or(VM_MEMORY_MIN_MB)
        .max(VM_MEMORY_MIN_MB)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> QemuConfig {
        QemuConfig::new(
            "qemu-system-x86_64",
            "kernel",
            "initrd",
            ".",
            "kvm",
            "host",
            "q35",
        )
    }

    #[test]
    fn qemu_launch_includes_every_hardening_flag() {
        let log = Path::new("/tmp/guest.log");
        let command = build_qemu_command(&config(), &SandboxConfig::default(), log, 4444);
        let arguments: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();

        // Every host-visible surface must be disabled.
        for flag in ["-display", "-monitor", "-nodefaults", "-nic", "-no-reboot"] {
            assert!(
                arguments.iter().any(|arg| arg == flag),
                "missing hardening flag {flag}"
            );
        }
        for [flag, value] in [["-display", "none"], ["-monitor", "none"], ["-nic", "none"]] {
            let index = arguments
                .iter()
                .position(|arg| arg == flag)
                .unwrap_or_else(|| {
                    panic!("missing hardening flag {flag}");
                });
            assert_eq!(
                arguments.get(index + 1).map(String::as_str),
                Some(value),
                "hardening flag {flag} must be followed by {value}"
            );
        }

        // Only the chardev-backed virtio-serial device may exist.
        assert!(arguments.iter().any(|arg| arg == "-device"));
        assert!(arguments
            .iter()
            .any(|arg| arg == "virtserialport,chardev=char0,id=port0,name=docbunker"));
    }

    #[test]
    fn seccomp_sandbox_flag_is_platform_consistent() {
        let flags = hardening_flags();
        if cfg!(not(target_os = "windows")) {
            let index = flags
                .iter()
                .position(|flag| *flag == "-sandbox")
                .unwrap_or_else(|| panic!("missing -sandbox on unix"));
            assert_eq!(flags.get(index + 1).copied(), Some("on"));
        } else {
            assert!(
                flags.iter().all(|flag| *flag != "-sandbox"),
                "-sandbox must not be passed on Windows builds"
            );
        }
    }
}
