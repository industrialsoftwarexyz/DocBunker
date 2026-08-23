//! Sandbox-side errors.

use thiserror::Error;

/// Errors produced by sandbox backends and transports.
#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(#[from] docbunker_protocol::ProtocolError),
    #[error("renderer error: {0}")]
    Render(#[from] docbunker_renderer_api::RenderError),
    #[error("no such sandbox session")]
    InvalidSession,
    #[error("unknown document")]
    UnknownDocument,
    #[error("backend not available: {0}")]
    BackendUnsupported(&'static str),
    #[error("operation timed out")]
    Timeout,
    #[error("worker process terminated unexpectedly")]
    WorkerCrashed,
    #[error("worker reported {0}: {1}")]
    WorkerError(docbunker_protocol::message::ErrorCode, String),
    #[error("internal error: {0}")]
    Internal(String),
}
