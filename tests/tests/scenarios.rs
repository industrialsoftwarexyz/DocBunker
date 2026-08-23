//! Cross-crate integration scenarios: core + sandbox + protocol + worker.

use docbunker_core::DocumentManager;
use docbunker_protocol::message::{OpenDocumentRequest, Request, Response};
use docbunker_protocol::validate::document_info_from_wire;
use docbunker_renderer_api::{DocumentFormat, RenderOptions};
use docbunker_sandbox::{
    DocumentInput, InProcessTransport, MockBackend, SandboxBackend, SandboxConfig, SandboxTransport,
};

#[test]
fn open_render_close_through_document_manager() {
    let manager = DocumentManager::new(Box::new(MockBackend::new())).unwrap();

    let handle = manager
        .open(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec())
        .expect("open should succeed");
    let info = manager.get_document_info(&handle).unwrap();
    assert_eq!(info.format, DocumentFormat::Png);

    let page = manager
        .render_page(
            &handle,
            1,
            RenderOptions {
                target_width: 128,
                target_height: 160,
            },
        )
        .unwrap();
    page.validate().unwrap();

    manager.close(&handle).unwrap();
}

#[test]
fn in_process_transport_handles_document_lifecycle() {
    let mut transport = InProcessTransport::connect_mock().unwrap();

    let resp = transport
        .send(Request::OpenDocument(OpenDocumentRequest {
            document_id: 1,
            data: b"%PDF-1.7\n%%EOF".to_vec(),
        }))
        .unwrap();
    match resp {
        Response::DocumentOpened(opened) => {
            let info = document_info_from_wire(&opened.info).unwrap();
            assert_eq!(info.format, DocumentFormat::Pdf);
        }
        other => panic!("expected DocumentOpened, got {other:?}"),
    }
}

#[test]
fn backend_sessions_are_independent() {
    let mut backend = MockBackend::new();
    backend.initialize().unwrap();

    let mut session_a = backend.start_session(SandboxConfig::default()).unwrap();
    let mut session_b = backend.start_session(SandboxConfig::default()).unwrap();

    let doc_a = backend
        .send_document(
            &mut session_a,
            DocumentInput::new(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec()),
        )
        .unwrap();
    let doc_b = backend
        .send_document(
            &mut session_b,
            DocumentInput::new(b"%PDF-1.7\n%%EOF".to_vec()),
        )
        .unwrap();

    // Closing session A must not affect session B.
    backend.close_session(session_a).unwrap();
    let info_b = backend.get_document_info(&mut session_b, doc_b).unwrap();
    assert_eq!(info_b.format, DocumentFormat::Pdf);
    assert!(backend.get_document_info(&mut session_b, doc_a).is_err());

    backend.close_session(session_b).unwrap();
}
