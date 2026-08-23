//! Frame-level protocol limits.
//!
//! Domain limits (document size, page dimensions, pixel buffers) live in
//! `docbunker-renderer-api::limits`; this module adds the wire-level caps.

use crate::error::ProtocolError;
use docbunker_renderer_api::limits as domain_limits;

/// Protocol version. Bump on any incompatible wire change.
pub const PROTOCOL_VERSION: u8 = 2;

/// Size of the length prefix in bytes (u32 LE).
pub const FRAME_HEADER_SIZE: usize = 4;

/// Hard cap for any single frame. Must leave room for a full
/// `MAX_PIXEL_BUFFER` raster plus overhead.
pub const MAX_IPC_MESSAGE: usize = 96 * 1024 * 1024;

/// Cap for any string that crosses the wire (error messages etc.).
pub const MAX_STRING_LENGTH: usize = 256;

/// Minimum sensible `max_ipc_message` a peer may advertise.
pub const MIN_IPC_MESSAGE: usize = 4096;

/// Validate that `s` fits the wire string cap.
pub fn validate_message_string(s: &str) -> Result<(), ProtocolError> {
    if s.len() > MAX_STRING_LENGTH {
        Err(ProtocolError::StringTooLong)
    } else {
        Ok(())
    }
}

/// The absolute pixel-buffer cap that can ever be negotiated. Kept in one place
/// so the protocol and the domain limits cannot drift apart.
pub fn absolute_max_pixel_buffer() -> usize {
    domain_limits::MAX_PIXEL_BUFFER
}

// Compile-time invariants: a single frame must be able to carry any rendered
// page and any document.
const _: () = assert!(MAX_IPC_MESSAGE > domain_limits::MAX_PIXEL_BUFFER);
const _: () = assert!(MAX_IPC_MESSAGE > domain_limits::MAX_DOCUMENT_SIZE);
const _: () = assert!(MAX_IPC_MESSAGE < u32::MAX as usize);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_cap_enforced() {
        assert!(validate_message_string("ok").is_ok());
        let long = "x".repeat(MAX_STRING_LENGTH + 1);
        assert!(validate_message_string(&long).is_err());
    }
}
