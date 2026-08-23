//! Wire message types (postcard-serialized via serde).
//!
//! These structs are the *wire representation*. Semantic validation happens in
//! [`crate::validate`]; conversions to/from `renderer-api` domain types live
//! there too, so no unvalidated wire value can become a domain value.

use serde::{Deserialize, Serialize};

/// Opaque document identifier assigned by the trusted host.
pub type DocumentId = u64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloRequest {
    pub protocol_version: u8,
    pub max_document_size: u64,
    pub max_ipc_message: u32,
    /// Host asks the worker to publish a shared-memory region for page
    /// buffers (ADR-009). The worker may decline by omitting `shm_name`.
    pub shm_capable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloOk {
    pub protocol_version: u8,
    pub max_document_size: u64,
    pub max_ipc_message: u32,
    pub max_page_width: u32,
    pub max_page_height: u32,
    pub max_pixel_buffer: u64,
    /// Named OS shared-memory region for page buffers (ADR-009), created by
    /// the worker when the host asked and creation succeeded. `None` falls
    /// back to the legacy in-frame bytes.
    pub shm_name: Option<String>,
    pub shm_capacity: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenDocumentRequest {
    pub document_id: DocumentId,
    pub data: Vec<u8>,
}

/// Minimal metadata returned by the worker (ADR-002).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentInfoWire {
    pub page_count: u32,
    pub width: u32,
    pub height: u32,
    pub format: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentOpenedResponse {
    pub document_id: DocumentId,
    pub info: DocumentInfoWire,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetDocumentInfoRequest {
    pub document_id: DocumentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentInfoResponse {
    pub document_id: DocumentId,
    pub info: DocumentInfoWire,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderPageRequest {
    pub document_id: DocumentId,
    pub page: u32,
    pub target_width: u32,
    pub target_height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRenderedResponse {
    pub document_id: DocumentId,
    pub page: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: u8,
    pub bytes: Vec<u8>,
    /// When set (ADR-009), `bytes` must be empty and the raster starts at
    /// offset 0 of the negotiated shared-memory region.
    pub shm_len: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseDocumentRequest {
    pub document_id: DocumentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseDocumentResponse {
    pub document_id: DocumentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PingRequest {
    pub nonce: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PongResponse {
    pub nonce: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: u8,
    pub message: String,
}

/// Requests the host may send to the worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Hello(HelloRequest),
    OpenDocument(OpenDocumentRequest),
    GetDocumentInfo(GetDocumentInfoRequest),
    RenderPage(RenderPageRequest),
    CloseDocument(CloseDocumentRequest),
    Ping(PingRequest),
}

/// Responses the worker may send to the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    HelloOk(HelloOk),
    DocumentOpened(DocumentOpenedResponse),
    DocumentInfo(DocumentInfoResponse),
    PageRendered(PageRenderedResponse),
    Closed(CloseDocumentResponse),
    Pong(PongResponse),
    Error(ErrorResponse),
}

/// Stable wire error codes (mirrors `DocBunkerError` on the host).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ErrorCode {
    UnsupportedFormat = 1,
    DocumentTooLarge = 2,
    InvalidDocument = 3,
    PasswordProtected = 4,
    RenderingFailed = 5,
    Timeout = 6,
    ResourceLimitExceeded = 7,
    ProtocolViolation = 8,
    UnsupportedOperation = 9,
    Internal = 10,
}

impl ErrorCode {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Short, non-sensitive label used for logging and error messages.
    pub const fn label(self) -> &'static str {
        match self {
            ErrorCode::UnsupportedFormat => "unsupported format",
            ErrorCode::DocumentTooLarge => "document too large",
            ErrorCode::InvalidDocument => "invalid document",
            ErrorCode::PasswordProtected => "password protected",
            ErrorCode::RenderingFailed => "rendering failed",
            ErrorCode::Timeout => "timeout",
            ErrorCode::ResourceLimitExceeded => "resource limit exceeded",
            ErrorCode::ProtocolViolation => "protocol violation",
            ErrorCode::UnsupportedOperation => "unsupported operation",
            ErrorCode::Internal => "internal error",
        }
    }
}

impl TryFrom<u8> for ErrorCode {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(ErrorCode::UnsupportedFormat),
            2 => Ok(ErrorCode::DocumentTooLarge),
            3 => Ok(ErrorCode::InvalidDocument),
            4 => Ok(ErrorCode::PasswordProtected),
            5 => Ok(ErrorCode::RenderingFailed),
            6 => Ok(ErrorCode::Timeout),
            7 => Ok(ErrorCode::ResourceLimitExceeded),
            8 => Ok(ErrorCode::ProtocolViolation),
            9 => Ok(ErrorCode::UnsupportedOperation),
            10 => Ok(ErrorCode::Internal),
            _ => Err(()),
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_round_trip() {
        for code in [
            ErrorCode::UnsupportedFormat,
            ErrorCode::DocumentTooLarge,
            ErrorCode::InvalidDocument,
            ErrorCode::PasswordProtected,
            ErrorCode::RenderingFailed,
            ErrorCode::Timeout,
            ErrorCode::ResourceLimitExceeded,
            ErrorCode::ProtocolViolation,
            ErrorCode::UnsupportedOperation,
            ErrorCode::Internal,
        ] {
            assert_eq!(ErrorCode::try_from(code.as_u8()), Ok(code));
        }
        assert!(ErrorCode::try_from(0).is_err());
        assert!(ErrorCode::try_from(255).is_err());
    }
}
