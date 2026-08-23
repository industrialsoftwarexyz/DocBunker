//! The worker core: dispatch protocol requests to a renderer.
//!
//! Design notes:
//!
//! - `handle` must never panic on hostile input. A panic here kills the
//!   process, which the host treats as `SandboxCrashed` — an acceptable
//!   failure mode, but it should never be reachable from malformed requests.
//! - The worker enforces the negotiated limits on **every** request, even
//!   though the host already validated them (defense in depth).
//! - All output is validated again before being sent.

use std::collections::HashMap;

use docbunker_protocol::limits::MAX_STRING_LENGTH;
use docbunker_protocol::message::{
    CloseDocumentRequest, CloseDocumentResponse, DocumentInfoResponse, DocumentOpenedResponse,
    ErrorCode, ErrorResponse, GetDocumentInfoRequest, HelloRequest, OpenDocumentRequest,
    PingRequest, PongResponse, RenderPageRequest, Request, Response,
};
use docbunker_protocol::validate::{
    document_info_to_wire, rendered_page_to_wire, NegotiatedLimits,
};
use docbunker_renderer_api::{DocumentInfo, DocumentRenderer, RenderError, RenderOptions};

/// Creates a [`DocumentRenderer`] for a document's raw bytes.
///
/// This is the swap point for real renderers. The protocol and worker core do
/// not change when image or PDF engines are replaced.
pub trait RendererFactory {
    type Renderer: DocumentRenderer;

    fn create(&mut self, data: Vec<u8>) -> Result<Self::Renderer, RenderError>;
}

/// Factory producing the Phase 1 mock renderer.
#[derive(Debug, Default)]
pub struct MockRendererFactory;

impl RendererFactory for MockRendererFactory {
    type Renderer = docbunker_renderer_api::mock::MockRenderer;

    fn create(&mut self, data: Vec<u8>) -> Result<Self::Renderer, RenderError> {
        docbunker_renderer_api::mock::MockRenderer::open(&data)
    }
}

/// A renderer dispatched by content-detected format.
///
/// This is what the real worker binary uses: images, Hayro PDF rendering and
/// the OOXML text preview (ADR-007) are always available; `pdf-mupdf`
/// replaces Hayro for opt-in deployments.
pub enum AnyRenderer {
    Image(docbunker_renderer_image::ImageRenderer),
    Ooxml(docbunker_renderer_ooxml::OoxmlRenderer),
    #[cfg(feature = "pdf-mupdf")]
    Pdf(crate::pdf::PdfRenderer),
    #[cfg(not(feature = "pdf-mupdf"))]
    Pdf(crate::pdf_hayro::PdfRenderer),
}

impl AnyRenderer {
    /// Dispatch on content-based format detection (never the filename).
    pub fn open(data: &[u8]) -> Result<Self, RenderError> {
        use docbunker_renderer_api::DocumentFormat;
        match docbunker_renderer_api::format::detect(data) {
            DocumentFormat::Png | DocumentFormat::Jpeg | DocumentFormat::WebP => {
                Ok(AnyRenderer::Image(
                    <docbunker_renderer_image::ImageRenderer as DocumentRenderer>::open(data)?,
                ))
            }
            DocumentFormat::Ooxml => Ok(AnyRenderer::Ooxml(
                <docbunker_renderer_ooxml::OoxmlRenderer as DocumentRenderer>::open(data)?,
            )),
            #[cfg(feature = "pdf-mupdf")]
            DocumentFormat::Pdf => Ok(AnyRenderer::Pdf(crate::pdf::PdfRenderer::open(data)?)),
            #[cfg(not(feature = "pdf-mupdf"))]
            DocumentFormat::Pdf => Ok(AnyRenderer::Pdf(crate::pdf_hayro::PdfRenderer::open(data)?)),
            _ => Err(RenderError::UnsupportedFormat),
        }
    }
}

impl docbunker_renderer_api::DocumentRenderer for AnyRenderer {
    fn open(data: &[u8]) -> Result<Self, RenderError>
    where
        Self: Sized,
    {
        AnyRenderer::open(data)
    }

    fn info(&self) -> Result<DocumentInfo, RenderError> {
        match self {
            AnyRenderer::Image(image) => image.info(),
            AnyRenderer::Ooxml(ooxml) => ooxml.info(),
            #[cfg(feature = "pdf-mupdf")]
            AnyRenderer::Pdf(pdf) => pdf.info(),
            #[cfg(not(feature = "pdf-mupdf"))]
            AnyRenderer::Pdf(pdf) => pdf.info(),
        }
    }

    fn render_page(
        &mut self,
        page: u32,
        options: &RenderOptions,
    ) -> Result<docbunker_renderer_api::RenderedPage, RenderError> {
        match self {
            AnyRenderer::Image(image) => image.render_page(page, options),
            AnyRenderer::Ooxml(ooxml) => ooxml.render_page(page, options),
            #[cfg(feature = "pdf-mupdf")]
            AnyRenderer::Pdf(pdf) => pdf.render_page(page, options),
            #[cfg(not(feature = "pdf-mupdf"))]
            AnyRenderer::Pdf(pdf) => pdf.render_page(page, options),
        }
    }
}

/// The factory used by the real `renderer-worker` binary.
#[derive(Debug, Default)]
pub struct StandardRendererFactory;

impl RendererFactory for StandardRendererFactory {
    type Renderer = AnyRenderer;

    fn create(&mut self, data: Vec<u8>) -> Result<Self::Renderer, RenderError> {
        AnyRenderer::open(&data)
    }
}

struct WorkerDocument<R: DocumentRenderer> {
    renderer: R,
    info: DocumentInfo,
}

/// The protocol dispatch core.
pub struct Worker<F: RendererFactory> {
    factory: F,
    docs: HashMap<u64, WorkerDocument<F::Renderer>>,
    limits: NegotiatedLimits,
    handshake_done: bool,
    /// Optional shared-memory page-buffer store (ADR-009). Created on the
    /// first `Hello` that requests it; `None` falls back to in-frame bytes.
    shm_store: Option<Box<dyn crate::shm::PageBufferStore>>,
    shm_factory: Option<SharedStoreFactory>,
}

/// Lazy factory for the shared-memory store; only the real worker binary
/// wires one in. Return `None` to decline shared memory (fallback to frames).
pub type SharedStoreFactory =
    Box<dyn FnMut() -> Option<Box<dyn crate::shm::PageBufferStore>> + Send>;

impl<F: RendererFactory> Worker<F> {
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            docs: HashMap::new(),
            limits: NegotiatedLimits {
                max_document_size: 0,
                max_ipc_message: 0,
                max_page_width: 0,
                max_page_height: 0,
                max_pixel_buffer: 0,
            },
            handshake_done: false,
            shm_store: None,
            shm_factory: None,
        }
    }

    /// Like [`Worker::new`], but with an optional shared-memory store factory
    /// consulted when the host asks for shared page buffers (ADR-009).
    pub fn with_shm_factory(factory: F, shm_factory: Option<SharedStoreFactory>) -> Self {
        Self {
            shm_factory,
            ..Self::new(factory)
        }
    }

    /// Handle one request and produce the response.
    pub fn handle(&mut self, req: Request) -> Response {
        match req {
            Request::Hello(hello) => self.on_hello(&hello),
            Request::Ping(ping) => self.on_ping(&ping),
            Request::OpenDocument(open) => self.on_open_document(open),
            Request::GetDocumentInfo(info) => self.on_get_document_info(&info),
            Request::RenderPage(render) => self.on_render_page(&render),
            Request::CloseDocument(close) => self.on_close_document(&close),
        }
    }

    fn on_hello(&mut self, hello: &HelloRequest) -> Response {
        match NegotiatedLimits::server_negotiate(hello) {
            Ok(limits) => {
                self.limits = limits.clone();
                self.handshake_done = true;
                let mut hello_ok = limits.to_hello_ok();
                // Shared-memory page buffers (ADR-009): set up a region now
                // when the host asked for it and a factory is available.
                if hello.shm_capable {
                    if let Some(mut factory) = self.shm_factory.take() {
                        if let Some(store) = factory() {
                            hello_ok.shm_name = Some(store.name());
                            hello_ok.shm_capacity = Some(store.capacity());
                            self.shm_store = Some(store);
                        }
                    }
                }
                Response::HelloOk(hello_ok)
            }
            Err(e) => {
                tracing::warn!(%e, "rejected hello");
                self.handshake_done = false;
                Self::error(ErrorCode::ProtocolViolation, "invalid hello")
            }
        }
    }

    fn on_ping(&mut self, ping: &PingRequest) -> Response {
        if !self.handshake_done {
            return Self::error(ErrorCode::ProtocolViolation, "handshake required");
        }
        Response::Pong(PongResponse { nonce: ping.nonce })
    }

    fn on_open_document(&mut self, open: OpenDocumentRequest) -> Response {
        if !self.handshake_done {
            return Self::error(ErrorCode::ProtocolViolation, "handshake required");
        }
        if let Err(e) = self.limits.validate_open_document(&open) {
            tracing::warn!(%e, "open document rejected by limits");
            return Self::error(ErrorCode::DocumentTooLarge, "document rejected by limits");
        }
        if self.docs.contains_key(&open.document_id) {
            return Self::error(ErrorCode::InvalidDocument, "document id already open");
        }

        let renderer = match self.factory.create(open.data) {
            Ok(renderer) => renderer,
            Err(e) => return Self::error_from_render(e),
        };
        let info = match renderer.info() {
            Ok(info) => info,
            Err(e) => return Self::error_from_render(e),
        };
        if info.validate().is_err() {
            return Self::error(ErrorCode::InvalidDocument, "invalid document metadata");
        }

        let document_id = open.document_id;
        self.docs.insert(
            document_id,
            WorkerDocument {
                renderer,
                info: info.clone(),
            },
        );
        Response::DocumentOpened(DocumentOpenedResponse {
            document_id,
            info: document_info_to_wire(&info),
        })
    }

    fn on_get_document_info(&mut self, req: &GetDocumentInfoRequest) -> Response {
        if !self.handshake_done {
            return Self::error(ErrorCode::ProtocolViolation, "handshake required");
        }
        match self.docs.get(&req.document_id) {
            Some(doc) => Response::DocumentInfo(DocumentInfoResponse {
                document_id: req.document_id,
                info: document_info_to_wire(&doc.info),
            }),
            None => Self::error(ErrorCode::InvalidDocument, "unknown document"),
        }
    }

    fn on_render_page(&mut self, req: &RenderPageRequest) -> Response {
        if !self.handshake_done {
            return Self::error(ErrorCode::ProtocolViolation, "handshake required");
        }
        if let Err(e) = self.limits.validate_render_request(req) {
            tracing::warn!(%e, "render request rejected by limits");
            return Self::error(
                ErrorCode::ResourceLimitExceeded,
                "render request rejected by limits",
            );
        }

        let doc = match self.docs.get_mut(&req.document_id) {
            Some(doc) => doc,
            None => return Self::error(ErrorCode::InvalidDocument, "unknown document"),
        };
        if req.page >= doc.info.page_count {
            return Self::error(ErrorCode::InvalidDocument, "page out of range");
        }

        let options = RenderOptions {
            target_width: req.target_width,
            target_height: req.target_height,
        };
        let page = match doc.renderer.render_page(req.page, &options) {
            Ok(page) => page,
            Err(e) => return Self::error_from_render(e),
        };
        if page.validate().is_err() {
            return Self::error(
                ErrorCode::RenderingFailed,
                "renderer produced an invalid page",
            );
        }

        let mut wire = rendered_page_to_wire(&page);
        wire.document_id = req.document_id;
        wire.page = req.page;
        // Shared-memory page buffers (ADR-009): when a region is active, the
        // raster rides there and the frame carries only the reference. If
        // storing fails we fall back to the in-frame bytes.
        if let Some(store) = self.shm_store.as_mut() {
            if let Ok(len) = u32::try_from(wire.bytes.len()) {
                if store.store(&wire.bytes).is_ok() {
                    wire.shm_len = Some(len);
                    wire.bytes.clear();
                }
            }
        }
        Response::PageRendered(wire)
    }

    fn on_close_document(&mut self, req: &CloseDocumentRequest) -> Response {
        if !self.handshake_done {
            return Self::error(ErrorCode::ProtocolViolation, "handshake required");
        }
        match self.docs.remove(&req.document_id) {
            Some(_) => Response::Closed(CloseDocumentResponse {
                document_id: req.document_id,
            }),
            None => Self::error(ErrorCode::InvalidDocument, "unknown document"),
        }
    }

    fn error_from_render(err: RenderError) -> Response {
        let code = match &err {
            RenderError::UnsupportedFormat => ErrorCode::UnsupportedFormat,
            RenderError::DocumentTooLarge => ErrorCode::DocumentTooLarge,
            RenderError::InvalidDocument => ErrorCode::InvalidDocument,
            RenderError::PasswordProtected => ErrorCode::PasswordProtected,
            RenderError::RenderingFailed => ErrorCode::RenderingFailed,
            RenderError::Timeout => ErrorCode::Timeout,
            RenderError::ResourceLimitExceeded => ErrorCode::ResourceLimitExceeded,
            RenderError::Internal(_) => ErrorCode::Internal,
        };
        Self::error(code, &err.to_string())
    }

    fn error(code: ErrorCode, detail: &str) -> Response {
        tracing::debug!(code = code.label(), detail, "worker error");
        // The host validates error messages in bytes (`MAX_STRING_LENGTH`);
        // a char-counted truncation of multi-byte text would be rejected and
        // the legitimate failure would surface as "sandbox crashed".
        let mut message = String::new();
        for ch in detail.chars() {
            if message.len().saturating_add(ch.len_utf8()) > MAX_STRING_LENGTH {
                break;
            }
            message.push(ch);
        }
        Response::Error(ErrorResponse {
            code: code.as_u8(),
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use docbunker_protocol::message::HelloRequest;
    use docbunker_protocol::{MAX_IPC_MESSAGE, PROTOCOL_VERSION};
    use docbunker_renderer_api::limits;

    fn hello() -> HelloRequest {
        HelloRequest {
            protocol_version: PROTOCOL_VERSION,
            max_document_size: limits::MAX_DOCUMENT_SIZE as u64,
            max_ipc_message: MAX_IPC_MESSAGE as u32,
            shm_capable: false,
        }
    }

    fn sample_doc() -> Vec<u8> {
        b"%PDF-1.7\n%%EOF".to_vec()
    }

    fn worker() -> Worker<MockRendererFactory> {
        Worker::new(MockRendererFactory)
    }

    fn open_doc(worker: &mut Worker<MockRendererFactory>, id: u64) -> Response {
        worker.handle(Request::OpenDocument(OpenDocumentRequest {
            document_id: id,
            data: sample_doc(),
        }))
    }

    #[test]
    fn full_lifecycle() {
        let mut w = worker();
        assert!(matches!(
            w.handle(Request::Hello(hello())),
            Response::HelloOk(_)
        ));

        assert!(matches!(open_doc(&mut w, 1), Response::DocumentOpened(_)));

        match w.handle(Request::GetDocumentInfo(GetDocumentInfoRequest {
            document_id: 1,
        })) {
            Response::DocumentInfo(resp) => {
                assert_eq!(resp.document_id, 1);
                assert_eq!(resp.info.page_count, 3);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        match w.handle(Request::RenderPage(RenderPageRequest {
            document_id: 1,
            page: 0,
            target_width: 100,
            target_height: 120,
        })) {
            Response::PageRendered(resp) => {
                assert_eq!(resp.width, 100);
                assert_eq!(resp.height, 120);
                assert_eq!(resp.bytes.len(), 100 * 120 * 4);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        assert!(matches!(
            w.handle(Request::CloseDocument(CloseDocumentRequest {
                document_id: 1
            })),
            Response::Closed(_)
        ));
        assert!(matches!(
            w.handle(Request::GetDocumentInfo(GetDocumentInfoRequest {
                document_id: 1
            })),
            Response::Error(_)
        ));
    }

    #[test]
    fn rejects_requests_before_handshake() {
        let mut w = worker();
        let resp = w.handle(Request::Ping(PingRequest { nonce: 1 }));
        match resp {
            Response::Error(e) => assert_eq!(e.code, ErrorCode::ProtocolViolation.as_u8()),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_hello() {
        let mut w = worker();
        let mut h = hello();
        h.protocol_version = 99;
        assert!(matches!(w.handle(Request::Hello(h)), Response::Error(_)));
    }

    #[test]
    fn rejects_unknown_document() {
        let mut w = worker();
        w.handle(Request::Hello(hello()));
        assert!(matches!(
            w.handle(Request::RenderPage(RenderPageRequest {
                document_id: 99,
                page: 0,
                target_width: 10,
                target_height: 10,
            })),
            Response::Error(_)
        ));
    }

    #[test]
    fn rejects_page_out_of_range() {
        let mut w = worker();
        w.handle(Request::Hello(hello()));
        open_doc(&mut w, 1);
        assert!(matches!(
            w.handle(Request::RenderPage(RenderPageRequest {
                document_id: 1,
                page: 3,
                target_width: 10,
                target_height: 10,
            })),
            Response::Error(_)
        ));
    }

    #[test]
    fn rejects_oversized_document() {
        let mut w = worker();
        w.handle(Request::Hello(hello()));
        let resp = w.handle(Request::OpenDocument(OpenDocumentRequest {
            document_id: 1,
            data: vec![0; limits::MAX_DOCUMENT_SIZE + 1],
        }));
        match resp {
            Response::Error(e) => assert_eq!(e.code, ErrorCode::DocumentTooLarge.as_u8()),
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[test]
    fn rejects_duplicate_document_id() {
        let mut w = worker();
        w.handle(Request::Hello(hello()));
        open_doc(&mut w, 1);
        assert!(matches!(open_doc(&mut w, 1), Response::Error(_)));
    }

    #[test]
    fn ping_pong() {
        let mut w = worker();
        w.handle(Request::Hello(hello()));
        match w.handle(Request::Ping(PingRequest { nonce: 1234 })) {
            Response::Pong(p) => assert_eq!(p.nonce, 1234),
            other => panic!("unexpected response: {other:?}"),
        }
    }
}
