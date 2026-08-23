//! Hard limits shared by every renderer, the protocol layer and the host core.
//!
//! Every size derived from wire data or file data must be validated against
//! these constants before use, and every multiplication must go through
//! [`pixel_buffer_size`] (checked arithmetic) — never through plain `*`.

use crate::error::RenderError;

/// Maximum accepted document size in bytes.
///
/// Kept below the IPC frame cap (`MAX_IPC_MESSAGE`) so a document can always
/// be transported in a single frame.
pub const MAX_DOCUMENT_SIZE: usize = 64 * 1024 * 1024;

/// Maximum width/height of a rendered page in pixels.
pub const MAX_PAGE_WIDTH: u32 = 4096;
pub const MAX_PAGE_HEIGHT: u32 = 4096;

/// Maximum number of pages a document may report.
pub const MAX_PAGE_COUNT: u32 = 100_000;

/// Maximum size in bytes of a single raster buffer (4096 x 4096 x 4).
pub const MAX_PIXEL_BUFFER: usize = (MAX_PAGE_WIDTH as usize) * (MAX_PAGE_HEIGHT as usize) * 4;

/// Bytes per pixel for [`crate::types::PixelFormat::Rgba8888`].
pub const RGBA_BYTES_PER_PIXEL: u32 = 4;

/// Compute `width * height * bytes_per_pixel` with checked arithmetic.
///
/// Returns `None` on overflow or when the operands cannot be represented as
/// `usize`. Use this instead of raw multiplication for any attacker-influenced
/// size.
pub fn pixel_buffer_size(width: u32, height: u32, bytes_per_pixel: u32) -> Option<usize> {
    let w = usize::try_from(width).ok()?;
    let h = usize::try_from(height).ok()?;
    let bpp = usize::try_from(bytes_per_pixel).ok()?;
    w.checked_mul(h)?.checked_mul(bpp)
}

/// Validate a requested render target against the hard limits.
pub fn validate_render_target(width: u32, height: u32) -> Result<(), RenderError> {
    if width == 0 || height == 0 {
        return Err(RenderError::ResourceLimitExceeded);
    }
    if width > MAX_PAGE_WIDTH || height > MAX_PAGE_HEIGHT {
        return Err(RenderError::ResourceLimitExceeded);
    }
    match pixel_buffer_size(width, height, RGBA_BYTES_PER_PIXEL) {
        Some(n) if n <= MAX_PIXEL_BUFFER => Ok(()),
        _ => Err(RenderError::ResourceLimitExceeded),
    }
}

/// Validate a byte buffer against the maximum document size.
pub fn validate_document_size(data: &[u8]) -> Result<(), RenderError> {
    if data.len() > MAX_DOCUMENT_SIZE {
        Err(RenderError::DocumentTooLarge)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_math_handles_overflow() {
        assert_eq!(pixel_buffer_size(u32::MAX, u32::MAX, 4), None);
        assert_eq!(pixel_buffer_size(0, 100, 4), Some(0));
        assert_eq!(pixel_buffer_size(10, 10, 4), Some(400));
    }

    #[test]
    fn checked_math_rejects_negative_bpp() {
        // bpp is u32; the interesting case is usize conversion on 32-bit hosts.
        assert!(pixel_buffer_size(1, 1, u32::MAX).is_some());
    }

    #[test]
    fn render_target_validated() {
        assert!(validate_render_target(1, 1).is_ok());
        assert!(validate_render_target(0, 1).is_err());
        assert!(validate_render_target(1, 0).is_err());
        assert!(validate_render_target(MAX_PAGE_WIDTH, MAX_PAGE_HEIGHT).is_ok());
        assert!(validate_render_target(MAX_PAGE_WIDTH + 1, 1).is_err());
    }

    #[test]
    fn document_size_validated() {
        assert!(validate_document_size(&[0u8; 8]).is_ok());
        assert!(validate_document_size(&[0u8; MAX_DOCUMENT_SIZE + 1]).is_err());
    }
}
