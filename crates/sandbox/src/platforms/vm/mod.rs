//! Cross-platform VM backend: QEMU + a native host accelerator.
//!
//! A minimal Linux guest (Alpine initramfs: kernel + `renderer-worker` +
//! `runsc` + OCI bundle, built by `sandbox/scripts/build-vm-image.sh`) runs
//! under QEMU accelerated with WHPX (Windows), HVF (macOS), or KVM (Linux). The outer
//! isolation boundary is the hardware VM; the inner boundary is gVisor
//! (`runsc`) inside the guest — see ADR-003 and ADR-006.
//!
//! The worker protocol travels over `virtio-serial`. A loopback TCP chardev
//! gives QEMU a portable duplex host transport, while the guest `/init`
//! redirects `runsc`'s stdio to `/dev/vport0p1`. The guest has no network
//! device; the socket only backs the virtual serial port. Protocol validation
//! is identical to the subprocess/runsc backends.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};

/// Build a `Command` that never opens a visible console window on Windows.
fn silent_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

use crate::backend::SandboxBackend;
use crate::config::SandboxConfig;
use crate::error::SandboxError;
use crate::process::{ProcessTransport, WorkerSession};
use crate::session::{DocumentId, DocumentInput, SandboxKind, SandboxSession};
use docbunker_renderer_api::{DocumentInfo, RenderOptions, RenderedPage};
use sha2::{Digest, Sha256};

mod command;
mod config;

pub use config::{HostProfile, QemuConfig};

use command::build_qemu_command;

struct QemuSession {
    config: SandboxConfig,
    _runtime_dir: tempfile::TempDir,
    worker: WorkerSession,
}

/// QEMU VM backend using the host platform's native accelerator.
///
/// Each session is one disposable VM. Documents enter the guest only as
/// protocol bytes over the pipe — never via shared filesystem paths — and
/// every response is validated by [`ProcessTransport`] before it reaches the
/// caller.
pub struct QemuVmBackend {
    config: QemuConfig,
    next_session_id: u64,
    next_document_id: u64,
    sessions: HashMap<u64, QemuSession>,
    assets_verified: bool,
}

impl QemuVmBackend {
    pub fn new(config: QemuConfig) -> Self {
        Self {
            config,
            next_session_id: 0,
            next_document_id: 0,
            sessions: HashMap::new(),
            assets_verified: false,
        }
    }
}

fn accelerator_available(output: &[u8], accelerator: &str) -> bool {
    String::from_utf8_lossy(output)
        .lines()
        .any(|line| line.split_whitespace().next() == Some(accelerator))
}

fn verify_sha256(path: &Path, expected: [u8; 32], asset: &str) -> Result<(), SandboxError> {
    let mut file = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let actual: [u8; 32] = hasher.finalize().into();
    if actual != expected {
        return Err(SandboxError::Internal(format!(
            "VM {asset} SHA-256 mismatch"
        )));
    }
    Ok(())
}

impl SandboxBackend for QemuVmBackend {
    fn initialize(&mut self) -> Result<(), SandboxError> {
        if !self.config.kernel.is_file() {
            return Err(SandboxError::BackendUnsupported(
                "vm kernel not found (set DOCBUNKER_VM_KERNEL)",
            ));
        }
        if !self.config.initrd.is_file() {
            return Err(SandboxError::BackendUnsupported(
                "vm initrd not found (set DOCBUNKER_VM_INITRD)",
            ));
        }

        let output = silent_command(&self.config.qemu_bin)
            .arg("--version")
            .output();
        match output {
            Ok(out) if out.status.success() => {}
            Ok(out) => {
                tracing::error!(
                    "qemu --version failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
                return Err(SandboxError::BackendUnsupported(
                    "qemu not usable (check DOCBUNKER_QEMU_BIN)",
                ));
            }
            Err(e) => {
                tracing::error!(%e, "qemu binary not found");
                return Err(SandboxError::BackendUnsupported(
                    "qemu system binary not found",
                ));
            }
        }

        let accel_output = silent_command(&self.config.qemu_bin)
            .args(["-accel", "help"])
            .output()
            .map_err(|_| SandboxError::BackendUnsupported("cannot query QEMU accelerators"))?;
        let mut available = accel_output.stdout;
        available.extend_from_slice(&accel_output.stderr);
        if !accel_output.status.success() || !accelerator_available(&available, &self.config.accel)
        {
            tracing::error!(accelerator = %self.config.accel, "QEMU accelerator unavailable");
            return Err(SandboxError::BackendUnsupported(
                "configured QEMU accelerator is unavailable",
            ));
        }

        std::fs::create_dir_all(&self.config.tmp_base)?;
        tracing::info!(accelerator = %self.config.accel, "QemuVmBackend initialized");
        Ok(())
    }

    fn start_session(&mut self, config: SandboxConfig) -> Result<SandboxSession, SandboxError> {
        if !self.assets_verified {
            if let Some(expected) = self.config.kernel_sha256 {
                verify_sha256(&self.config.kernel, expected, "kernel")?;
            }
            if let Some(expected) = self.config.initrd_sha256 {
                verify_sha256(&self.config.initrd, expected, "initramfs")?;
            }
            self.assets_verified = true;
        }

        config
            .validate()
            .map_err(|msg| SandboxError::Internal(msg.into()))?;

        self.next_session_id += 1;
        let id = self.next_session_id;
        let runtime_dir = tempfile::Builder::new()
            .prefix("docbunker-vm-")
            .rand_bytes(16)
            .tempdir_in(&self.config.tmp_base)?;
        let log_path = runtime_dir.path().join("guest.log");
        let qemu_log_path = runtime_dir.path().join("qemu.log");
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();

        let mut command = build_qemu_command(&self.config, &config, &log_path, port);
        command.stderr(Stdio::from(std::fs::File::create(qemu_log_path)?));

        let transport = ProcessTransport::spawn_command_over_tcp(
            &mut command,
            listener,
            config.operation_timeout,
        )?;

        self.sessions.insert(
            id,
            QemuSession {
                config,
                _runtime_dir: runtime_dir,
                worker: WorkerSession::new(transport),
            },
        );
        Ok(SandboxSession {
            id,
            kind: SandboxKind::QemuVm,
        })
    }

    fn send_document(
        &mut self,
        session: &mut SandboxSession,
        document: DocumentInput,
    ) -> Result<DocumentId, SandboxError> {
        let session_state = self
            .sessions
            .get_mut(&session.id)
            .ok_or(SandboxError::InvalidSession)?;

        if document.data.len() > session_state.config.max_document_size {
            return Err(docbunker_renderer_api::RenderError::DocumentTooLarge.into());
        }

        self.next_document_id += 1;
        let id = DocumentId(self.next_document_id);
        session_state.worker.send_document(document.data, id.0)?;
        Ok(id)
    }

    fn get_document_info(
        &mut self,
        session: &mut SandboxSession,
        document_id: DocumentId,
    ) -> Result<DocumentInfo, SandboxError> {
        let session_state = self
            .sessions
            .get_mut(&session.id)
            .ok_or(SandboxError::InvalidSession)?;
        session_state.worker.get_document_info(document_id.0)
    }

    fn render_page(
        &mut self,
        session: &mut SandboxSession,
        document_id: DocumentId,
        page: u32,
        options: RenderOptions,
    ) -> Result<RenderedPage, SandboxError> {
        let session_state = self
            .sessions
            .get_mut(&session.id)
            .ok_or(SandboxError::InvalidSession)?;

        if options.target_width > session_state.config.max_page_width
            || options.target_height > session_state.config.max_page_height
        {
            return Err(docbunker_renderer_api::RenderError::ResourceLimitExceeded.into());
        }
        session_state
            .worker
            .render_page(document_id.0, page, options)
    }

    fn close_session(&mut self, session: SandboxSession) -> Result<(), SandboxError> {
        let Some(mut session_state) = self.sessions.remove(&session.id) else {
            return Err(SandboxError::InvalidSession);
        };

        session_state.worker.close_all_documents();
        session_state.worker.shutdown();
        Ok(())
    }
}

impl Drop for QemuVmBackend {
    fn drop(&mut self) {
        for (_, mut session) in self.sessions.drain() {
            session.worker.terminate();
        }
    }
}

#[cfg(test)]
mod tests;
