//! Phase 1 mock backend.
//!
//! `MockBackend` implements the full [`SandboxBackend`] session lifecycle but
//! drives the renderer **in-process** (no OS-level isolation). It exists so
//! the application, the protocol and the host core are fully wired before the
//! real sandbox lands in Phase 4.
//!
//! **Security note**: `MockBackend` must never be used for real untrusted
//! documents outside development and tests. The production path is the
//! `runsc` backend (see `platforms::linux::RunscBackend`).

use std::collections::HashMap;

use crate::backend::SandboxBackend;
use crate::config::SandboxConfig;
use crate::error::SandboxError;
use crate::session::{DocumentId, DocumentInput, SandboxKind, SandboxSession};
use crate::transport::{InProcessTransport, SandboxTransport};
use docbunker_protocol::message::{
    CloseDocumentRequest, ErrorCode, GetDocumentInfoRequest, OpenDocumentRequest,
    RenderPageRequest, Request, Response,
};
use docbunker_protocol::validate::{document_info_from_wire, rendered_page_from_wire};
use docbunker_renderer_api::{DocumentInfo, RenderOptions, RenderedPage};

struct MockSession {
    config: SandboxConfig,
    transport: InProcessTransport<docbunker_renderer_worker::MockRendererFactory>,
    docs: HashMap<u64, DocumentInfo>,
}

/// In-process mock sandbox backend (development/tests only).
pub struct MockBackend {
    next_session_id: u64,
    next_document_id: u64,
    sessions: HashMap<u64, MockSession>,
}

impl MockBackend {
    pub fn new() -> Self {
        Self {
            next_session_id: 0,
            next_document_id: 0,
            sessions: HashMap::new(),
        }
    }
}

impl Default for MockBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SandboxBackend for MockBackend {
    fn initialize(&mut self) -> Result<(), SandboxError> {
        tracing::warn!(
            "MockBackend: no isolation; do not use for untrusted input outside dev/tests"
        );
        Ok(())
    }

    fn start_session(&mut self, config: SandboxConfig) -> Result<SandboxSession, SandboxError> {
        config
            .validate()
            .map_err(|msg| SandboxError::Internal(msg.into()))?;
        self.next_session_id += 1;
        let id = self.next_session_id;
        let transport =
            InProcessTransport::<docbunker_renderer_worker::MockRendererFactory>::connect_mock()?;
        self.sessions.insert(
            id,
            MockSession {
                config,
                transport,
                docs: HashMap::new(),
            },
        );
        Ok(SandboxSession {
            id,
            kind: SandboxKind::Mock,
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

        let response =
            session_state
                .transport
                .send(Request::OpenDocument(OpenDocumentRequest {
                    document_id: id.0,
                    data: document.data,
                }))?;

        match response {
            Response::DocumentOpened(opened) => {
                let info = document_info_from_wire(&opened.info)?;
                session_state.docs.insert(id.0, info.clone());
                Ok(id)
            }
            Response::Error(e) => Err(SandboxError::WorkerError(
                ErrorCode::try_from(e.code).unwrap_or(ErrorCode::Internal),
                e.message,
            )),
            _ => Err(SandboxError::Internal(
                "unexpected response to open document".into(),
            )),
        }
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

        if !session_state.docs.contains_key(&document_id.0) {
            return Err(SandboxError::UnknownDocument);
        }

        let response =
            session_state
                .transport
                .send(Request::GetDocumentInfo(GetDocumentInfoRequest {
                    document_id: document_id.0,
                }))?;

        match response {
            Response::DocumentInfo(info) => {
                let validated = document_info_from_wire(&info.info)?;
                session_state.docs.insert(document_id.0, validated.clone());
                Ok(validated)
            }
            Response::Error(e) => Err(SandboxError::WorkerError(
                ErrorCode::try_from(e.code).unwrap_or(ErrorCode::Internal),
                e.message,
            )),
            _ => Err(SandboxError::Internal(
                "unexpected response to get document info".into(),
            )),
        }
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

        let response = session_state
            .transport
            .send(Request::RenderPage(RenderPageRequest {
                document_id: document_id.0,
                page,
                target_width: options.target_width,
                target_height: options.target_height,
            }))?;

        match response {
            Response::PageRendered(rendered) => {
                let page = rendered_page_from_wire(&rendered)?;
                page.validate()?;
                Ok(page)
            }
            Response::Error(e) => Err(SandboxError::WorkerError(
                ErrorCode::try_from(e.code).unwrap_or(ErrorCode::Internal),
                e.message,
            )),
            _ => Err(SandboxError::Internal(
                "unexpected response to render page".into(),
            )),
        }
    }

    fn close_session(&mut self, session: SandboxSession) -> Result<(), SandboxError> {
        let Some(session_state) = self.sessions.remove(&session.id) else {
            return Err(SandboxError::InvalidSession);
        };

        // Tell the worker to close every open document (mirrors what the real
        // backend must do), then drop the transport to release all resources.
        let document_ids: Vec<u64> = session_state.docs.keys().copied().collect();
        let mut transport = session_state.transport;
        for id in document_ids {
            let response = transport.send(Request::CloseDocument(CloseDocumentRequest {
                document_id: id,
            }))?;
            match response {
                Response::Closed(_) => {}
                Response::Error(e) => {
                    tracing::warn!(code = e.code, "worker refused to close document {id}");
                }
                other => {
                    tracing::warn!("unexpected close response: {other:?}");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (MockBackend, SandboxSession) {
        let mut backend = MockBackend::new();
        backend.initialize().unwrap();
        let session = backend
            .start_session(SandboxConfig::default())
            .expect("start session");
        (backend, session)
    }

    fn sample_doc() -> Vec<u8> {
        b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec()
    }

    #[test]
    fn full_lifecycle() {
        let (mut backend, mut session) = setup();
        let id = backend
            .send_document(&mut session, DocumentInput::new(sample_doc()))
            .unwrap();
        assert_eq!(id.0, 1);

        let info = backend.get_document_info(&mut session, id).unwrap();
        assert_eq!(info.format, docbunker_renderer_api::DocumentFormat::Png);
        assert_eq!(info.page_count, 3);

        let page = backend
            .render_page(
                &mut session,
                id,
                0,
                RenderOptions {
                    target_width: 100,
                    target_height: 100,
                },
            )
            .unwrap();
        page.validate().unwrap();
        assert_eq!(page.bytes.len(), 100 * 100 * 4);

        backend.close_session(session).unwrap();
        assert!(backend.sessions.is_empty());
    }

    #[test]
    fn unknown_document_rejected() {
        let (mut backend, mut session) = setup();
        let result = backend.get_document_info(&mut session, DocumentId(42));
        assert!(matches!(result, Err(SandboxError::UnknownDocument)));
    }

    #[test]
    fn oversized_document_rejected() {
        let (mut backend, mut session) = setup();
        let result = backend.send_document(
            &mut session,
            DocumentInput::new(vec![
                0;
                docbunker_renderer_api::limits::MAX_DOCUMENT_SIZE + 1
            ]),
        );
        assert!(matches!(
            result,
            Err(SandboxError::Render(
                docbunker_renderer_api::RenderError::DocumentTooLarge
            ))
        ));
    }

    #[test]
    fn session_config_limits_enforced() {
        let mut backend = MockBackend::new();
        backend.initialize().unwrap();
        let config = SandboxConfig {
            max_page_width: 64,
            max_page_height: 64,
            ..SandboxConfig::default()
        };
        let mut session = backend.start_session(config).unwrap();

        let id = backend
            .send_document(&mut session, DocumentInput::new(sample_doc()))
            .unwrap();
        let result = backend.render_page(
            &mut session,
            id,
            0,
            RenderOptions {
                target_width: 128,
                target_height: 64,
            },
        );
        assert!(matches!(
            result,
            Err(SandboxError::Render(
                docbunker_renderer_api::RenderError::ResourceLimitExceeded
            ))
        ));
    }

    #[test]
    fn closing_twice_fails() {
        let (mut backend, session) = setup();
        backend.close_session(session.clone()).unwrap();
        let result = backend.close_session(session);
        assert!(matches!(result, Err(SandboxError::InvalidSession)));
    }
}
