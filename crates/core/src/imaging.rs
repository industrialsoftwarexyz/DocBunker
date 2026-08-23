//! PNG encoding of trusted RGBA buffers.
//!
//! Encoding our *own* validated raster data to PNG is safe (no parsing of
//! untrusted input). The frontend displays these PNGs via `data:` URLs; it
//! never decodes the original document (ADR-002).

use crate::error::DocBunkerError;
use docbunker_renderer_api::{PixelFormat, RenderedPage};

/// Encode a validated RGBA page into a PNG byte buffer.
pub fn encode_rgba_to_png(page: &RenderedPage) -> Result<Vec<u8>, DocBunkerError> {
    page.validate()?;
    if page.pixel_format != PixelFormat::Rgba8888 {
        return Err(DocBunkerError::InternalError);
    }

    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, page.width, page.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // Fast compression: document pages are mostly flat (white + text or
        // photos), so the size penalty is minor while the CPU win is large —
        // the PNG crosses IPC as a data URL on every page/zoom change.
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|_| DocBunkerError::InternalError)?;

        let bpp = page.pixel_format.bytes_per_pixel() as usize;
        let row_len = (page.width as usize)
            .checked_mul(bpp)
            .ok_or(DocBunkerError::InternalError)?;
        let stride = page.stride as usize;

        let packed = if stride == row_len {
            page.bytes.clone()
        } else {
            // Row-padded buffers are copied into a packed buffer first.
            let mut packed = vec![0u8; row_len * page.height as usize];
            for y in 0..page.height as usize {
                let src_start = y.checked_mul(stride).ok_or(DocBunkerError::InternalError)?;
                let src_end = src_start
                    .checked_add(row_len)
                    .ok_or(DocBunkerError::InternalError)?;
                let dst_start = y
                    .checked_mul(row_len)
                    .ok_or(DocBunkerError::InternalError)?;
                packed[dst_start..dst_start + row_len].copy_from_slice(
                    page.bytes
                        .get(src_start..src_end)
                        .ok_or(DocBunkerError::InternalError)?,
                );
            }
            packed
        };
        writer
            .write_image_data(&packed)
            .map_err(|_| DocBunkerError::InternalError)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(w: u32, h: u32) -> RenderedPage {
        RenderedPage {
            width: w,
            height: h,
            stride: w * 4,
            pixel_format: PixelFormat::Rgba8888,
            bytes: vec![0x80; (w * h * 4) as usize],
        }
    }

    #[test]
    fn encodes_small_page() {
        let png = encode_rgba_to_png(&page(16, 16)).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]));
    }

    #[test]
    fn rejects_invalid_page() {
        let mut p = page(10, 10);
        p.bytes = vec![0; 1];
        assert!(matches!(
            encode_rgba_to_png(&p),
            Err(DocBunkerError::RenderingFailed)
        ));
    }

    #[test]
    fn handles_padded_stride() {
        let mut p = page(8, 8);
        p.stride = 8 * 4 + 8; // 8 extra bytes per row
        p.bytes = vec![0x80; p.stride as usize * 8];
        assert!(encode_rgba_to_png(&p).is_ok());
    }
}
