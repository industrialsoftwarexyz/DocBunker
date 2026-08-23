//! Mock renderer for development and tests.
//!
//! `MockRenderer` implements [`DocumentRenderer`] but does **not** parse the
//! document. It validates only the size limits, sniffs the format from magic
//! bytes, and draws a deterministic test page. This is the Phase 1 renderer:
//! it proves the whole pipeline without decoding anything.
//!
//! It must never be used as a production renderer.

use crate::error::RenderError;
use crate::format;
use crate::limits;
use crate::types::{
    DocumentFormat, DocumentInfo, DocumentRenderer, PixelFormat, RenderOptions, RenderedPage,
};

const FONT_WIDTH: u32 = 5;
const FONT_HEIGHT: u32 = 7;

/// 5x7 bitmap glyphs (one row per byte, MSB = leftmost pixel).
fn glyph(ch: char) -> Option<[u8; FONT_HEIGHT as usize]> {
    match ch {
        '0' => Some([30, 17, 19, 21, 25, 17, 30]),
        '1' => Some([4, 12, 4, 4, 4, 4, 14]),
        '2' => Some([30, 17, 1, 2, 4, 8, 31]),
        '3' => Some([30, 17, 1, 6, 1, 17, 30]),
        '4' => Some([2, 6, 10, 18, 31, 2, 2]),
        '5' => Some([31, 16, 30, 1, 1, 17, 30]),
        '6' => Some([6, 8, 16, 30, 17, 17, 14]),
        '7' => Some([31, 1, 2, 4, 8, 8, 8]),
        '8' => Some([14, 17, 17, 14, 17, 17, 14]),
        '9' => Some([14, 17, 17, 15, 1, 2, 12]),
        'A' => Some([14, 17, 17, 31, 17, 17, 17]),
        'E' => Some([31, 16, 16, 30, 16, 16, 31]),
        'G' => Some([15, 16, 16, 16, 19, 17, 14]),
        'P' => Some([30, 17, 17, 30, 16, 16, 16]),
        '/' => Some([2, 4, 8, 16, 8, 4, 2]),
        _ => None,
    }
}

fn set_pixel(bytes: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 3]) {
    let idx = ((y as usize) * (width as usize) + (x as usize)) * 4;
    let Some(slot) = bytes.get_mut(idx..idx + 4) else {
        return;
    };
    slot[0] = color[0];
    slot[1] = color[1];
    slot[2] = color[2];
    slot[3] = 255;
}

/// Draw `text` centered at `(cx, cy)` (top-left of the text block).
fn draw_text(
    bytes: &mut [u8],
    width: u32,
    height: u32,
    text: &str,
    cx: i64,
    cy: i64,
    color: [u8; 3],
) {
    let total_w = text.len() as i64 * (FONT_WIDTH as i64 + 1);
    let x0 = cx - total_w / 2;
    let y0 = cy;
    let mut pen_x = x0;
    for ch in text.chars() {
        let Some(g) = glyph(ch) else { continue };
        for (row, bits) in g.iter().enumerate() {
            for col in 0..FONT_WIDTH {
                if (bits >> (FONT_WIDTH - 1 - col)) & 1 == 1 {
                    let x = pen_x + col as i64;
                    let y = y0 + row as i64;
                    if (0..width as i64).contains(&x) && (0..height as i64).contains(&y) {
                        set_pixel(bytes, width, x as u32, y as u32, color);
                    }
                }
            }
        }
        pen_x += FONT_WIDTH as i64 + 1;
    }
}

fn draw_border(bytes: &mut [u8], width: u32, height: u32, thickness: u32, color: [u8; 3]) {
    for t in 0..thickness {
        for x in t..(width - t) {
            set_pixel(bytes, width, x, t, color);
            set_pixel(bytes, width, x, height - 1 - t, color);
        }
        for y in t..(height - t) {
            set_pixel(bytes, width, t, y, color);
            set_pixel(bytes, width, width - 1 - t, y, color);
        }
    }
}

/// Deterministic test-pattern renderer (Phase 1). See module docs.
#[derive(Debug)]
pub struct MockRenderer {
    info: DocumentInfo,
}

impl MockRenderer {
    const PAGE_COUNT: u32 = 3;
    const PAGE_WIDTH: u32 = 612;
    const PAGE_HEIGHT: u32 = 792;
}

impl DocumentRenderer for MockRenderer {
    fn open(data: &[u8]) -> Result<Self, RenderError> {
        limits::validate_document_size(data)?;
        let format = format::detect(data);
        if format == DocumentFormat::Unknown {
            return Err(RenderError::UnsupportedFormat);
        }
        Ok(Self {
            info: DocumentInfo {
                page_count: Self::PAGE_COUNT,
                width: Self::PAGE_WIDTH,
                height: Self::PAGE_HEIGHT,
                format,
            },
        })
    }

    fn info(&self) -> Result<DocumentInfo, RenderError> {
        Ok(self.info.clone())
    }

    fn render_page(
        &mut self,
        page: u32,
        options: &RenderOptions,
    ) -> Result<RenderedPage, RenderError> {
        if page >= self.info.page_count {
            return Err(RenderError::InvalidDocument);
        }
        let (width, height) = (options.target_width, options.target_height);
        limits::validate_render_target(width, height)?;
        let size =
            limits::pixel_buffer_size(width, height, PixelFormat::Rgba8888.bytes_per_pixel())
                .ok_or(RenderError::ResourceLimitExceeded)?;
        let mut bytes = vec![0u8; size];

        // Vertical gradient (deterministic, integer math only).
        for y in 0..height {
            let t = (u64::from(y) * 255) / u64::from(height);
            let t = u32::try_from(t).unwrap_or(255);
            let color = [
                (24 + (t * 231) / 255) as u8,
                (32 + (t * 212) / 255) as u8,
                (96 + (t * 152) / 255) as u8,
            ];
            for x in 0..width {
                let idx = (y as usize * width as usize + x as usize) * 4;
                bytes[idx] = color[0];
                bytes[idx + 1] = color[1];
                bytes[idx + 2] = color[2];
                bytes[idx + 3] = 255;
            }
        }

        draw_border(&mut bytes, width, height, 2, [255, 255, 255]);

        let label = format!("PAGE {} / {}", page + 1, self.info.page_count);
        draw_text(
            &mut bytes,
            width,
            height,
            &label,
            i64::from(width) / 2,
            i64::from(height) / 2 - 4,
            [255, 255, 255],
        );

        Ok(RenderedPage {
            width,
            height,
            stride: width
                .checked_mul(PixelFormat::Rgba8888.bytes_per_pixel())
                .ok_or(RenderError::ResourceLimitExceeded)?,
            pixel_format: PixelFormat::Rgba8888,
            bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_doc(format: DocumentFormat) -> Vec<u8> {
        match format {
            DocumentFormat::Pdf => b"%PDF-1.7\n%%EOF".to_vec(),
            DocumentFormat::Png => b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec(),
            DocumentFormat::Jpeg => b"\xFF\xD8\xFF\xE0\x00\x10JFIF".to_vec(),
            DocumentFormat::WebP => b"RIFF\x10\x00\x00\x00WEBPVP8 ".to_vec(),
            DocumentFormat::Ooxml => b"PK\x03\x04\x14\x00\x00\x00".to_vec(),
            DocumentFormat::Gif => b"GIF89a\x01\x00\x01\x00".to_vec(),
            DocumentFormat::Tiff => b"II*\x00\x08\x00\x00\x00".to_vec(),
            DocumentFormat::Bmp => b"BM\x36\x00\x00\x00".to_vec(),
            DocumentFormat::Epub => b"PK\x03\x04\x14\x00\x00\x00".to_vec(),
            DocumentFormat::Rtf => b"{\\rtf1\\ansi hello".to_vec(),
            DocumentFormat::Html => b"<html><body>hi</body></html>".to_vec(),
            DocumentFormat::Unknown => b"garbage".to_vec(),
        }
    }

    #[test]
    fn opens_recognized_formats() {
        for format in [
            DocumentFormat::Pdf,
            DocumentFormat::Png,
            DocumentFormat::Jpeg,
            DocumentFormat::WebP,
        ] {
            let renderer = MockRenderer::open(&sample_doc(format)).expect("should open");
            let info = renderer.info().unwrap();
            assert_eq!(info.page_count, MockRenderer::PAGE_COUNT);
            assert_eq!(info.format, format);
            info.validate().unwrap();
        }
    }

    #[test]
    fn rejects_unknown_format() {
        assert!(matches!(
            MockRenderer::open(&sample_doc(DocumentFormat::Unknown)),
            Err(RenderError::UnsupportedFormat)
        ));
    }

    #[test]
    fn rejects_oversized_document() {
        let data = vec![0u8; limits::MAX_DOCUMENT_SIZE + 1];
        assert!(matches!(
            MockRenderer::open(&data),
            Err(RenderError::DocumentTooLarge)
        ));
    }

    #[test]
    fn renders_valid_page() {
        let mut renderer = MockRenderer::open(&sample_doc(DocumentFormat::Pdf)).unwrap();
        let page = renderer
            .render_page(
                0,
                &RenderOptions {
                    target_width: 400,
                    target_height: 500,
                },
            )
            .unwrap();
        assert_eq!(page.width, 400);
        assert_eq!(page.height, 500);
        assert_eq!(page.stride, 400 * 4);
        assert_eq!(page.bytes.len(), 400 * 500 * 4);
        page.validate().unwrap();
    }

    #[test]
    fn rejects_page_out_of_range() {
        let mut renderer = MockRenderer::open(&sample_doc(DocumentFormat::Pdf)).unwrap();
        assert!(matches!(
            renderer.render_page(
                3,
                &RenderOptions {
                    target_width: 100,
                    target_height: 100
                }
            ),
            Err(RenderError::InvalidDocument)
        ));
    }

    #[test]
    fn rejects_absurd_targets() {
        let mut renderer = MockRenderer::open(&sample_doc(DocumentFormat::Pdf)).unwrap();
        assert!(matches!(
            renderer.render_page(
                0,
                &RenderOptions {
                    target_width: 0,
                    target_height: 100
                }
            ),
            Err(RenderError::ResourceLimitExceeded)
        ));
        assert!(matches!(
            renderer.render_page(
                0,
                &RenderOptions {
                    target_width: 4097,
                    target_height: 100
                }
            ),
            Err(RenderError::ResourceLimitExceeded)
        ));
        assert!(matches!(
            renderer.render_page(
                0,
                &RenderOptions {
                    target_width: u32::MAX,
                    target_height: u32::MAX
                }
            ),
            Err(RenderError::ResourceLimitExceeded)
        ));
    }
}
