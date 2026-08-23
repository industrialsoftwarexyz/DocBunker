//! Transport abstraction between host and worker.
//!
//! The protocol crate is transport-agnostic (frames over any `Read`/`Write`).
//! This trait is the higher-level host-side seam: it will be implemented over
//! Unix domain sockets / pipes for the `runsc` backend and over
//! `virtio-vsock`/`virtio-serial` for the VM backends.

use crate::error::SandboxError;
use docbunker_protocol::message::{ErrorCode, HelloRequest, Request, Response};
use docbunker_protocol::validate::{validate_error_response, NegotiatedLimits};
use docbunker_protocol::{MAX_IPC_MESSAGE, PROTOCOL_VERSION};
use docbunker_renderer_api::limits as domain_limits;
use docbunker_renderer_worker::{MockRendererFactory, RendererFactory, Worker};

/// A transport to a single worker.
pub trait SandboxTransport {
    /// Send a request and receive the worker's response.
    ///
    /// Implementations must validate every response (size caps, semantic
    /// limits, error codes) before returning it.
    fn send(&mut self, message: Request) -> Result<Response, SandboxError>;
}

/// A transport that talks to the worker **in-process**.
///
/// This exercises the real protocol dispatch path (worker validation,
/// response validation) without spawning a process. It is used by
/// [`crate::mock::MockBackend`] and by tests. It provides **no isolation**;
/// never use it for real documents.
pub struct InProcessTransport<F: RendererFactory> {
    worker: Worker<F>,
    limits: NegotiatedLimits,
}

impl<F: RendererFactory> InProcessTransport<F> {
    /// Perform the `Hello` handshake and derive the negotiated limits.
    pub fn connect(mut worker: Worker<F>) -> Result<Self, SandboxError> {
        let hello = HelloRequest {
            protocol_version: PROTOCOL_VERSION,
            max_document_size: domain_limits::MAX_DOCUMENT_SIZE as u64,
            max_ipc_message: MAX_IPC_MESSAGE as u32,
            shm_capable: false,
        };
        let response = worker.handle(Request::Hello(hello));
        match response {
            Response::HelloOk(ok) => {
                let limits = NegotiatedLimits::client_negotiate(&ok)?;
                Ok(Self { worker, limits })
            }
            Response::Error(e) => Err(SandboxError::WorkerError(
                ErrorCode::try_from(e.code).unwrap_or(ErrorCode::Internal),
                e.message,
            )),
            _ => Err(SandboxError::Internal(
                "unexpected response to hello".into(),
            )),
        }
    }

    /// The negotiated limits.
    pub fn negotiated_limits(&self) -> &NegotiatedLimits {
        &self.limits
    }
}

impl InProcessTransport<MockRendererFactory> {
    /// Connect an in-process transport backed by the mock renderer factory.
    pub fn connect_mock() -> Result<Self, SandboxError> {
        Self::connect(Worker::new(MockRendererFactory))
    }
}

impl<F: RendererFactory> SandboxTransport for InProcessTransport<F> {
    fn send(&mut self, message: Request) -> Result<Response, SandboxError> {
        // Host-side pre-validation before the message reaches the worker.
        match &message {
            Request::OpenDocument(open) => self.limits.validate_open_document(open)?,
            Request::RenderPage(render) => self.limits.validate_render_request(render)?,
            _ => {}
        }

        let response = self.worker.handle(message);

        // Validate the response before returning it to the caller.
        match &response {
            Response::Error(e) => {
                validate_error_response(e)?;
            }
            Response::PageRendered(page) => {
                self.limits.validate_rendered_page(page)?;
            }
            Response::DocumentOpened(opened) => {
                self.limits.validate_document_info(&opened.info)?;
            }
            Response::DocumentInfo(info) => {
                self.limits.validate_document_info(&info.info)?;
            }
            _ => {}
        }
        Ok(response)
    }
}

/// A transport that always fails: placeholder for not-yet-implemented
/// backends. Fails loudly instead of pretending to work.
#[derive(Debug, Default)]
pub struct NullTransport;

impl SandboxTransport for NullTransport {
    fn send(&mut self, _message: Request) -> Result<Response, SandboxError> {
        Err(SandboxError::BackendUnsupported(
            "no transport implementation (Phase 4+)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use docbunker_protocol::message::{OpenDocumentRequest, PingRequest, RenderPageRequest};

    #[test]
    fn in_process_transport_full_round_trip() {
        let mut transport = InProcessTransport::connect_mock().unwrap();
        assert_eq!(
            transport.negotiated_limits().max_document_size,
            domain_limits::MAX_DOCUMENT_SIZE
        );

        let opened = transport
            .send(Request::OpenDocument(OpenDocumentRequest {
                document_id: 1,
                data: b"%PDF-1.7\n%%EOF".to_vec(),
            }))
            .unwrap();
        assert!(matches!(opened, Response::DocumentOpened(_)));

        let rendered = transport
            .send(Request::RenderPage(RenderPageRequest {
                document_id: 1,
                page: 0,
                target_width: 64,
                target_height: 64,
            }))
            .unwrap();
        assert!(matches!(rendered, Response::PageRendered(_)));

        let pong = transport
            .send(Request::Ping(PingRequest { nonce: 9 }))
            .unwrap();
        assert!(matches!(pong, Response::Pong(_)));
    }

    #[test]
    fn in_process_transport_rejects_oversized_document() {
        let mut transport = InProcessTransport::connect_mock().unwrap();
        let result = transport.send(Request::OpenDocument(OpenDocumentRequest {
            document_id: 1,
            data: vec![0; domain_limits::MAX_DOCUMENT_SIZE + 1],
        }));
        assert!(matches!(
            result,
            Err(SandboxError::Protocol(
                docbunker_protocol::ProtocolError::LimitViolation(_)
            ))
        ));
    }

    #[test]
    fn null_transport_fails_loudly() {
        let mut transport = NullTransport;
        let result = transport.send(Request::Ping(PingRequest { nonce: 1 }));
        assert!(matches!(result, Err(SandboxError::BackendUnsupported(_))));
    }
}
