//! Bilinear RGBA scaling with checked arithmetic.
//!
//! Used to honor the requested render target when the document's native
//! resolution differs. Pure integer math (u64 fixed point); all sizes are
//! pre-validated against the shared limits by the caller.

use docbunker_renderer_api::limits;
use docbunker_renderer_api::{PixelFormat, RenderError};

/// Scale a packed RGBA image from `(src_w, src_h)` to `(dst_w, dst_h)`.
pub fn scale_rgba(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Result<Vec<u8>, RenderError> {
    if src_w == 0 || src_h == 0 {
        return Err(RenderError::InvalidDocument);
    }
    let bpp = PixelFormat::Rgba8888.bytes_per_pixel();
    let src_size =
        limits::pixel_buffer_size(src_w, src_h, bpp).ok_or(RenderError::ResourceLimitExceeded)?;
    if src.len() != src_size {
        return Err(RenderError::RenderingFailed);
    }

    let dst_size =
        limits::pixel_buffer_size(dst_w, dst_h, bpp).ok_or(RenderError::ResourceLimitExceeded)?;
    if dst_size > limits::MAX_PIXEL_BUFFER {
        return Err(RenderError::ResourceLimitExceeded);
    }

    if src_w == dst_w && src_h == dst_h {
        return Ok(src.to_vec());
    }

    let mut dst = vec![0u8; dst_size];

    // Fixed-point sampling coordinates: src coordinate = (dst * src) / dst.
    // 16.16 fixed point keeps products within u64 for our dimensions.
    // A target dimension of 1 has no step (the loop runs once).
    let step_x = if dst_w > 1 {
        (u64::from(src_w - 1) << 16) / u64::from(dst_w - 1)
    } else {
        0
    };
    let step_y = if dst_h > 1 {
        (u64::from(src_h - 1) << 16) / u64::from(dst_h - 1)
    } else {
        0
    };

    let mut fy: u64 = 0;
    for dy in 0..dst_h {
        let y0 = (fy >> 16) as usize;
        let y1 = (y0 + 1).min(src_h as usize - 1);
        let fy_frac = (fy & 0xFFFF) as u32;
        let fy_inv = 0x10000 - fy_frac;

        let mut fx: u64 = 0;
        for dx in 0..dst_w {
            let x0 = (fx >> 16) as usize;
            let x1 = (x0 + 1).min(src_w as usize - 1);
            let fx_frac = (fx & 0xFFFF) as u32;
            let fx_inv = 0x10000 - fx_frac;

            let i00 = (y0 * src_w as usize + x0) * 4;
            let i01 = (y0 * src_w as usize + x1) * 4;
            let i10 = (y1 * src_w as usize + x0) * 4;
            let i11 = (y1 * src_w as usize + x1) * 4;

            let o = (dy as usize * dst_w as usize + dx as usize) * 4;
            for c in 0..4 {
                let top = u64::from(src[i00 + c]) * u64::from(fx_inv)
                    + u64::from(src[i01 + c]) * u64::from(fx_frac);
                let bottom = u64::from(src[i10 + c]) * u64::from(fx_inv)
                    + u64::from(src[i11 + c]) * u64::from(fx_frac);
                let value = (top * u64::from(fy_inv) + bottom * u64::from(fy_frac)) >> 32;
                dst[o + c] = value as u8;
            }

            fx += step_x;
        }
        fy += step_y;
    }

    Ok(dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut b = vec![0u8; (w * h * 4) as usize];
        for px in b.chunks_exact_mut(4) {
            px.copy_from_slice(&rgba);
        }
        b
    }

    #[test]
    fn same_size_returns_copy() {
        let src = solid(4, 3, [10, 20, 30, 255]);
        let out = scale_rgba(&src, 4, 3, 4, 3).unwrap();
        assert_eq!(out, src);
    }

    #[test]
    fn upscales_without_changing_color() {
        let src = solid(2, 2, [100, 150, 200, 255]);
        let out = scale_rgba(&src, 2, 2, 4, 4).unwrap();
        assert_eq!(out.len(), 4 * 4 * 4);
        for px in out.chunks_exact(4) {
            assert_eq!(px, [100, 150, 200, 255]);
        }
    }

    #[test]
    fn downscales_to_target_size() {
        let src = solid(10, 10, [1, 2, 3, 255]);
        let out = scale_rgba(&src, 10, 10, 5, 5).unwrap();
        assert_eq!(out.len(), 5 * 5 * 4);
    }

    #[test]
    fn rejects_bad_input_size() {
        let src = vec![0u8; 7];
        assert!(matches!(
            scale_rgba(&src, 4, 3, 4, 3),
            Err(RenderError::RenderingFailed)
        ));
    }
}
