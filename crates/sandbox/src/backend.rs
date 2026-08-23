//! The [`SandboxBackend`] trait — the seam where different isolation
//! strategies plug in (mock, `runsc` on Linux, VM on Windows/macOS).

use crate::config::SandboxConfig;
use crate::error::SandboxError;
use crate::session::{DocumentId, DocumentInput, SandboxSession};
use docbunker_renderer_api::{DocumentInfo, RenderOptions, RenderedPage};

/// A sandbox backend.
///
/// Implementations must guarantee:
///
/// - Each session is independently destroyable ([`SandboxBackend::close_session`]).
/// - Document bytes only enter the sandbox through
///   [`SandboxBackend::send_document`] (never via shared filesystem paths).
/// - Every returned value is validated before it leaves the backend.
///
/// The `Send` bound is required so a backend can live behind an `Arc<Mutex<_>>`
/// shared with Tauri's async runtime.
///
/// This trait is deliberately synchronous: async orchestration lives in
/// `docbunker-core`, which runs backend calls on blocking threads.
pub trait SandboxBackend: Send {
    /// One-time initialization (e.g. locating `runsc`, verifying versions).
    fn initialize(&mut self) -> Result<(), SandboxError>;

    /// Start a new, disposable sandbox session.
    fn start_session(&mut self, config: SandboxConfig) -> Result<SandboxSession, SandboxError>;

    /// Send a document into the session; returns a host-assigned id.
    fn send_document(
        &mut self,
        session: &mut SandboxSession,
        document: DocumentInput,
    ) -> Result<DocumentId, SandboxError>;

    /// Query minimal metadata for an open document.
    fn get_document_info(
        &mut self,
        session: &mut SandboxSession,
        document_id: DocumentId,
    ) -> Result<DocumentInfo, SandboxError>;

    /// Render one page as validated RGBA.
    fn render_page(
        &mut self,
        session: &mut SandboxSession,
        document_id: DocumentId,
        page: u32,
        options: RenderOptions,
    ) -> Result<RenderedPage, SandboxError>;

    /// Destroy the session and release all its resources.
    fn close_session(&mut self, session: SandboxSession) -> Result<(), SandboxError>;
}
