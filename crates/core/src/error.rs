//! User-facing error type.
//!
//! Internal detail (parser messages, renderer internals) is deliberately not
//! exposed: [`DocBunkerError`] only carries a stable code and a short,
//! user-safe message. It serializes to `{ "code": "...", "message": "..." }`
//! for the Tauri IPC layer.

use serde::ser::{Serialize, SerializeMap, Serializer};
use thiserror::Error;

/// Stable, user-safe error codes.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DocBunkerError {
    #[error("unsupported document format")]
    UnsupportedFormat,
    #[error("the document is too large")]
    DocumentTooLarge,
    #[error("the document is invalid or corrupt")]
    InvalidDocument,
    #[error("the document is password protected")]
    PasswordProtected,
    #[error("page rendering failed")]
    RenderingFailed,
    #[error("the sandbox could not be started")]
    SandboxStartupFailed,
    #[error("the sandbox crashed")]
    SandboxCrashed,
    #[error("the operation timed out")]
    Timeout,
    #[error("a resource limit was exceeded")]
    ResourceLimitExceeded,
    #[error("the sandbox protocol was violated")]
    ProtocolViolation,
    #[error("an internal error occurred")]
    InternalError,
    #[error("invalid page number")]
    InvalidPage,
    #[error("no document is open")]
    NoDocumentOpen,
    #[error("the operation was cancelled")]
    Cancelled,
}

impl DocBunkerError {
    /// Stable machine-readable code.
    pub const fn code(&self) -> &'static str {
        match self {
            DocBunkerError::UnsupportedFormat => "unsupported_format",
            DocBunkerError::DocumentTooLarge => "document_too_large",
            DocBunkerError::InvalidDocument => "invalid_document",
            DocBunkerError::PasswordProtected => "password_protected",
            DocBunkerError::RenderingFailed => "rendering_failed",
            DocBunkerError::SandboxStartupFailed => "sandbox_startup_failed",
            DocBunkerError::SandboxCrashed => "sandbox_crashed",
            DocBunkerError::Timeout => "timeout",
            DocBunkerError::ResourceLimitExceeded => "resource_limit_exceeded",
            DocBunkerError::ProtocolViolation => "protocol_violation",
            DocBunkerError::InternalError => "internal_error",
            DocBunkerError::InvalidPage => "invalid_page",
            DocBunkerError::NoDocumentOpen => "no_document_open",
            DocBunkerError::Cancelled => "cancelled",
        }
    }

    /// Map a worker-reported wire code to a host error.
    pub fn from_worker_code(code: docbunker_protocol::message::ErrorCode) -> Self {
        use docbunker_protocol::message::ErrorCode as W;
        match code {
            W::UnsupportedFormat => DocBunkerError::UnsupportedFormat,
            W::DocumentTooLarge => DocBunkerError::DocumentTooLarge,
            W::InvalidDocument => DocBunkerError::InvalidDocument,
            W::PasswordProtected => DocBunkerError::PasswordProtected,
            W::RenderingFailed => DocBunkerError::RenderingFailed,
            W::Timeout => DocBunkerError::Timeout,
            W::ResourceLimitExceeded => DocBunkerError::ResourceLimitExceeded,
            W::ProtocolViolation => DocBunkerError::ProtocolViolation,
            W::UnsupportedOperation => DocBunkerError::InternalError,
            W::Internal => DocBunkerError::InternalError,
        }
    }
}

impl Serialize for DocBunkerError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("code", self.code())?;
        map.serialize_entry("message", &self.to_string())?;
        map.end()
    }
}

impl From<docbunker_renderer_api::RenderError> for DocBunkerError {
    fn from(err: docbunker_renderer_api::RenderError) -> Self {
        match err {
            docbunker_renderer_api::RenderError::UnsupportedFormat => {
                DocBunkerError::UnsupportedFormat
            }
            docbunker_renderer_api::RenderError::DocumentTooLarge => {
                DocBunkerError::DocumentTooLarge
            }
            docbunker_renderer_api::RenderError::InvalidDocument => DocBunkerError::InvalidDocument,
            docbunker_renderer_api::RenderError::PasswordProtected => {
                DocBunkerError::PasswordProtected
            }
            docbunker_renderer_api::RenderError::RenderingFailed => DocBunkerError::RenderingFailed,
            docbunker_renderer_api::RenderError::Timeout => DocBunkerError::Timeout,
            docbunker_renderer_api::RenderError::ResourceLimitExceeded => {
                DocBunkerError::ResourceLimitExceeded
            }
            docbunker_renderer_api::RenderError::Internal(_) => DocBunkerError::InternalError,
        }
    }
}

impl From<docbunker_sandbox::SandboxError> for DocBunkerError {
    fn from(err: docbunker_sandbox::SandboxError) -> Self {
        match err {
            docbunker_sandbox::SandboxError::BackendUnsupported(_) => {
                DocBunkerError::SandboxStartupFailed
            }
            docbunker_sandbox::SandboxError::Timeout => DocBunkerError::Timeout,
            docbunker_sandbox::SandboxError::WorkerCrashed => DocBunkerError::SandboxCrashed,
            docbunker_sandbox::SandboxError::WorkerError(code, _) => {
                DocBunkerError::from_worker_code(code)
            }
            docbunker_sandbox::SandboxError::Protocol(_) => DocBunkerError::ProtocolViolation,
            docbunker_sandbox::SandboxError::Render(re) => re.into(),
            docbunker_sandbox::SandboxError::InvalidSession => DocBunkerError::InternalError,
            docbunker_sandbox::SandboxError::UnknownDocument => DocBunkerError::NoDocumentOpen,
            docbunker_sandbox::SandboxError::Io(_) => DocBunkerError::InternalError,
            docbunker_sandbox::SandboxError::Internal(_) => DocBunkerError::InternalError,
        }
    }
}

impl From<std::io::Error> for DocBunkerError {
    fn from(_: std::io::Error) -> Self {
        DocBunkerError::InternalError
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_to_expected_shape() {
        let json = serde_json::to_value(DocBunkerError::DocumentTooLarge).unwrap();
        assert_eq!(json["code"], "document_too_large");
        assert!(json["message"].is_string());
    }

    #[test]
    fn codes_are_stable() {
        let errs = [
            DocBunkerError::UnsupportedFormat,
            DocBunkerError::DocumentTooLarge,
            DocBunkerError::InvalidDocument,
            DocBunkerError::PasswordProtected,
            DocBunkerError::RenderingFailed,
            DocBunkerError::SandboxStartupFailed,
            DocBunkerError::SandboxCrashed,
            DocBunkerError::Timeout,
            DocBunkerError::ResourceLimitExceeded,
            DocBunkerError::ProtocolViolation,
            DocBunkerError::InternalError,
            DocBunkerError::InvalidPage,
            DocBunkerError::NoDocumentOpen,
            DocBunkerError::Cancelled,
        ];
        let mut seen = std::collections::HashSet::new();
        for err in errs {
            assert!(seen.insert(err.code()), "duplicate code: {}", err.code());
        }
    }

    #[test]
    fn worker_codes_map() {
        assert_eq!(
            DocBunkerError::from_worker_code(docbunker_protocol::message::ErrorCode::Timeout),
            DocBunkerError::Timeout
        );
        assert_eq!(
            DocBunkerError::from_worker_code(docbunker_protocol::message::ErrorCode::Internal),
            DocBunkerError::InternalError
        );
    }
}
