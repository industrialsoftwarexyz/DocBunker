//! Versioned binary IPC protocol between the trusted host and the untrusted
//! `renderer-worker`.
//!
//! Design rules (see `docs/protocol.md`):
//!
//! - Every frame is length-prefixed and capped by [`limits::MAX_IPC_MESSAGE`].
//! - Every payload starts with the protocol version byte.
//! - Payloads are postcard-encoded; decoding is strict (trailing bytes rejected).
//! - Semantic validation (`validate`) runs on both sides against negotiated limits.
//! - No JSON transports RGBA buffers, ever.

pub mod error;
pub mod framing;
pub mod limits;
pub mod message;
pub mod validate;

pub use error::ProtocolError;
pub use framing::{
    decode_request, decode_response, encode_request, encode_response, FrameReader, FrameWriter,
};
pub use limits::{MAX_IPC_MESSAGE, MAX_STRING_LENGTH, PROTOCOL_VERSION};
pub use message::{
    CloseDocumentRequest, CloseDocumentResponse, DocumentId, DocumentInfoResponse,
    DocumentInfoWire, DocumentOpenedResponse, ErrorCode, ErrorResponse, GetDocumentInfoRequest,
    HelloOk, HelloRequest, OpenDocumentRequest, PageRenderedResponse, PingRequest, PongResponse,
    RenderPageRequest, Request, Response,
};
pub use validate::{validate_hello_request, NegotiatedLimits};
