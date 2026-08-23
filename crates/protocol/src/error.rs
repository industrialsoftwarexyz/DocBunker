//! Protocol-level errors.
//!
//! These are host/worker wire errors, distinct from the user-facing
//! [`docbunker_core::DocBunkerError`] and from renderer errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("frame exceeds the maximum allowed size")]
    FrameTooLarge,
    #[error("frame is smaller than the minimum size")]
    FrameTooSmall,
    #[error("frame was truncated")]
    TruncatedFrame,
    #[error("unsupported protocol version")]
    InvalidVersion,
    #[error("invalid message discriminator")]
    InvalidDiscriminator,
    #[error("message failed to (de)serialize")]
    Serialization(#[from] postcard::Error),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("string exceeds the maximum allowed length")]
    StringTooLong,
    #[error("message violates protocol limits: {0}")]
    LimitViolation(&'static str),
    #[error("message received out of order")]
    OutOfOrder,
    #[error("peer closed the connection")]
    ConnectionClosed,
}
