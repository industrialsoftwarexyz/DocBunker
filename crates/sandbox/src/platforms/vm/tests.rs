use std::path::{Path, PathBuf};

use docbunker_renderer_api::{DocumentFormat, RenderOptions};

use crate::error::SandboxError;

use super::command::{qemu_path, vm_memory_mb, VM_MEMORY_HEADROOM_MB, VM_MEMORY_MIN_MB};
use super::*;

fn minimal_pdf() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] >>",
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

#[test]
fn memory_sizing_leaves_headroom() {
    let config = SandboxConfig {
        memory_limit_bytes: Some(512 * 1024 * 1024),
        ..SandboxConfig::default()
    };
    assert_eq!(vm_memory_mb(&config), 512 + VM_MEMORY_HEADROOM_MB);

    let config = SandboxConfig {
        memory_limit_bytes: None,
        ..SandboxConfig::default()
    };
    assert_eq!(vm_memory_mb(&config), VM_MEMORY_MIN_MB);

    let config = SandboxConfig {
        memory_limit_bytes: Some(16 * 1024 * 1024),
        ..SandboxConfig::default()
    };
    assert_eq!(vm_memory_mb(&config), VM_MEMORY_MIN_MB);
}

#[test]
fn qemu_path_uses_forward_slashes() {
    assert_eq!(
        qemu_path(Path::new(r"C:\Users\a\vm.log")),
        "C:/Users/a/vm.log"
    );
    assert_eq!(qemu_path(Path::new("/tmp/vm.log")), "/tmp/vm.log");
}

#[test]
fn platform_defaults_are_native() {
    let profile = HostProfile::current();
    let expected_accelerator = if cfg!(target_os = "windows") {
        "whpx"
    } else if cfg!(target_os = "macos") {
        "hvf"
    } else {
        "kvm"
    };
    assert_eq!(profile.accelerator, expected_accelerator);
    assert_eq!(
        profile.machine,
        if cfg!(target_arch = "aarch64") {
            "virt"
        } else {
            "q35"
        }
    );
    assert_eq!(
        profile.cpu_model,
        if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            "qemu64,svm=off"
        } else {
            "host"
        }
    );
}

#[test]
fn accelerator_list_is_parsed_by_exact_name() {
    let output = b"Accelerators supported in QEMU binary:\nwhpx\ntcg\n";
    assert!(accelerator_available(output, "whpx"));
    assert!(!accelerator_available(output, "hvf"));
    assert!(!accelerator_available(output, "whp"));
}

#[test]
fn expected_sha256_rejects_malformed_digest() {
    let config = QemuConfig::new("qemu", "kernel", "initrd", ".", "tcg", "host", "q35");
    assert!(config
        .with_expected_sha256("not-a-digest", &"0".repeat(64))
        .is_err());
}

#[test]
fn start_session_rejects_vm_asset_hash_mismatch() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let kernel = directory.path().join("kernel");
    let initrd = directory.path().join("initramfs.cpio.gz");
    std::fs::write(&kernel, b"kernel").expect("write kernel");
    std::fs::write(&initrd, b"initrd").expect("write initrd");

    let profile = HostProfile::current();
    let config = QemuConfig::new(
        profile.qemu_bin,
        kernel,
        initrd,
        directory.path().join("tmp"),
        "tcg",
        "host",
        "q35",
    )
    .with_expected_sha256(&"0".repeat(64), &"0".repeat(64))
    .expect("valid digest syntax");

    let mut backend = QemuVmBackend::new(config);
    // Hosted runners may not ship QEMU; the hash gate itself is what this
    // test exercises, so skip (rather than fail) when the binary is absent.
    match backend.initialize() {
        Ok(()) => {}
        Err(SandboxError::BackendUnsupported(message)) if message.contains("qemu") => return,
        Err(other) => panic!("qemu is available: {other}"),
    }

    let error = backend
        .start_session(SandboxConfig::default())
        .expect_err("mismatched kernel must fail before QEMU launch");
    assert!(error.to_string().contains("SHA-256 mismatch"));
}

/// End-to-end test against a real QEMU VM.
///
/// Run with `DOCBUNKER_VM_KERNEL` and `DOCBUNKER_VM_INITRD` set:
/// `cargo test -p docbunker-sandbox qemu_vm_end_to_end -- --ignored`.
///
/// The bundled initramfs embeds a worker binary: it must be rebuilt
/// (`sandbox/scripts/build-vm-image.sh`) to carry the current protocol
/// version — a stale image fails the handshake with `InvalidVersion`.
#[test]
#[ignore = "requires QEMU, a native accelerator, and a prepared VM image"]
fn qemu_vm_end_to_end() {
    let profile = HostProfile::current();
    let config = QemuConfig::new(
        std::env::var("DOCBUNKER_QEMU_BIN")
            .map(PathBuf::from)
            .unwrap_or(profile.qemu_bin),
        std::env::var("DOCBUNKER_VM_KERNEL").expect("DOCBUNKER_VM_KERNEL is required"),
        std::env::var("DOCBUNKER_VM_INITRD").expect("DOCBUNKER_VM_INITRD is required"),
        std::env::var("DOCBUNKER_TMP_BASE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("docbunker-vm")),
        std::env::var("DOCBUNKER_QEMU_ACCEL").unwrap_or_else(|_| profile.accelerator.to_string()),
        std::env::var("DOCBUNKER_QEMU_CPU").unwrap_or_else(|_| profile.cpu_model.to_string()),
        std::env::var("DOCBUNKER_QEMU_MACHINE").unwrap_or_else(|_| profile.machine.to_string()),
    )
    .with_expected_sha256(
        &std::env::var("DOCBUNKER_VM_KERNEL_SHA256")
            .expect("DOCBUNKER_VM_KERNEL_SHA256 is required"),
        &std::env::var("DOCBUNKER_VM_INITRD_SHA256")
            .expect("DOCBUNKER_VM_INITRD_SHA256 is required"),
    )
    .expect("valid VM asset hashes");
    let mut backend = QemuVmBackend::new(config);
    backend
        .initialize()
        .expect("qemu + vm image must be available");

    let session_config = SandboxConfig::default();
    let mut session = backend
        .start_session(session_config)
        .expect("vm session starts");

    let id = backend
        .send_document(&mut session, DocumentInput::new(minimal_pdf()))
        .expect("document enters the vm");
    let info = backend.get_document_info(&mut session, id).expect("info");
    assert_eq!(info.format, DocumentFormat::Pdf);
    assert_eq!(info.page_count, 1);

    let page = backend
        .render_page(
            &mut session,
            id,
            0,
            RenderOptions {
                target_width: 200,
                target_height: 100,
            },
        )
        .expect("page renders inside the vm");
    page.validate().expect("validated page");
    assert_eq!(page.bytes.len(), 200 * 100 * 4);

    backend.close_session(session).expect("vm session closes");
}
