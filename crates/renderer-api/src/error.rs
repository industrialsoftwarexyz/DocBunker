//! Errors produced by renderers and by domain-level validation.
//!
//! These errors are renderer-facing. The trusted host maps them (and the wire
//! error codes of `docbunker-protocol`) onto `DocBunkerError` before anything is
//! shown to the user; internal parser details never reach the UI.

use thiserror::Error;

/// Renderer-side error. [`RenderError::Internal`] may carry a short diagnostic
/// string; it must never contain document content.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RenderError {
    #[error("unsupported document format")]
    UnsupportedFormat,
    #[error("document exceeds the maximum allowed size")]
    DocumentTooLarge,
    #[error("invalid or corrupt document")]
    InvalidDocument,
    #[error("document is password protected")]
    PasswordProtected,
    #[error("rendering failed")]
    RenderingFailed,
    #[error("operation timed out")]
    Timeout,
    #[error("resource limit exceeded")]
    ResourceLimitExceeded,
    #[error("internal renderer error: {0}")]
    Internal(String),
}
