//! Platform-specific sandbox backends.
//!
//! - `linux::RunscBackend` — gVisor/`runsc` (Phase 4).
//! - `vm::QemuVmBackend` — Linux VM via QEMU + WHPX/HVF/KVM.
//!
//! Stubs fail loudly ([`SandboxError::BackendUnsupported`]) instead of
//! pretending to provide isolation. See `docs/sandbox.md` for the design.

mod linux;
mod vm;

pub use linux::{RunscBackend, RunscConfig};
pub use vm::{HostProfile, QemuConfig, QemuVmBackend};
