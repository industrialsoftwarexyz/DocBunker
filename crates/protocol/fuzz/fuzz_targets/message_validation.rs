//! Fuzz target: semantic validation of wire messages.
//!
//! Validation must never panic on arbitrary decoded values. Run with
//! `cargo fuzz run message_validation`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use docbunker_protocol::message::{DocumentInfoWire, ErrorResponse, OpenDocumentRequest, RenderPageRequest};
use docbunker_protocol::validate::{validate_error_response, NegotiatedLimits};
use docbunker_renderer_api::limits;

fuzz_target!(|data: &[u8]| {
    // The harness reads twelve bytes below; shorter inputs are noise.
    if data.len() < 12 {
        return;
    }

    let info = DocumentInfoWire {
        page_count: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
        width: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
        height: 0,
        format: data[0] % 12,
    };
    let _ = docbunker_protocol::validate::document_info_from_wire(&info);

    let open = OpenDocumentRequest {
        document_id: u64::from_le_bytes(data[..8].try_into().unwrap()),
        data: data[8..].to_vec(),
    };
    let hello = docbunker_protocol::message::HelloRequest {
        protocol_version: docbunker_protocol::PROTOCOL_VERSION,
        max_document_size: limits::MAX_DOCUMENT_SIZE as u64,
        max_ipc_message: docbunker_protocol::MAX_IPC_MESSAGE as u32,
        shm_capable: data[0] & 1 == 1,
    };
    let negotiated = NegotiatedLimits::server_negotiate(&hello).unwrap();
    let _ = negotiated.validate_open_document(&open);

    let render = RenderPageRequest {
        document_id: 1,
        page: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
        target_width: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
        target_height: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
    };
    let _ = negotiated.validate_render_request(&render);

    let err = ErrorResponse {
        code: data[0],
        message: String::from_utf8_lossy(&data[1..]).into_owned(),
    };
    let _ = validate_error_response(&err);
});
