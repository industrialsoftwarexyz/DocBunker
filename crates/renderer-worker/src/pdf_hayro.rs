//! Pure-Rust PDF renderer backed by Hayro.

use std::sync::Arc;

use docbunker_renderer_api::{
    DocumentFormat, DocumentInfo, DocumentRenderer, PixelFormat, RenderError, RenderOptions,
    RenderedPage,
};
use hayro::{InterpreterSettings, Pdf, RenderSettings};

pub struct PdfRenderer {
    pdf: Pdf,
    info: DocumentInfo,
}

impl PdfRenderer {
    fn page_dimensions(pdf: &Pdf, page: usize) -> Result<(u32, u32), RenderError> {
        let page = pdf.pages().get(page).ok_or(RenderError::InvalidDocument)?;
        let (width, height) = page.render_dimensions();
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(RenderError::InvalidDocument);
        }
        Ok((width.ceil() as u32, height.ceil() as u32))
    }
}

impl DocumentRenderer for PdfRenderer {
    fn open(data: &[u8]) -> Result<Self, RenderError> {
        docbunker_renderer_api::limits::validate_document_size(data)?;
        let pdf = Pdf::new(Arc::new(data.to_vec())).map_err(|_| RenderError::InvalidDocument)?;
        let page_count =
            u32::try_from(pdf.pages().len()).map_err(|_| RenderError::ResourceLimitExceeded)?;
        if page_count == 0 {
            return Err(RenderError::InvalidDocument);
        }
        let (width, height) = Self::page_dimensions(&pdf, 0)?;
        let info = DocumentInfo {
            page_count,
            width,
            height,
            format: DocumentFormat::Pdf,
        };
        info.validate()?;
        Ok(Self { pdf, info })
    }

    fn info(&self) -> Result<DocumentInfo, RenderError> {
        Ok(self.info.clone())
    }

    fn render_page(
        &mut self,
        page: u32,
        options: &RenderOptions,
    ) -> Result<RenderedPage, RenderError> {
        docbunker_renderer_api::limits::validate_render_target(
            options.target_width,
            options.target_height,
        )?;
        let page = self
            .pdf
            .pages()
            .get(page as usize)
            .ok_or(RenderError::InvalidDocument)?;
        let (source_width, source_height) = page.render_dimensions();
        if source_width <= 0.0 || source_height <= 0.0 {
            return Err(RenderError::InvalidDocument);
        }
        let width =
            u16::try_from(options.target_width).map_err(|_| RenderError::ResourceLimitExceeded)?;
        let height =
            u16::try_from(options.target_height).map_err(|_| RenderError::ResourceLimitExceeded)?;
        // Uniform scale: a page whose aspect ratio differs from the target is
        // never stretched — Hayro letterboxes it with a white margin (the
        // pixmap is white-filled and the page is clipped at `scale`).
        let scale = (options.target_width as f32 / source_width)
            .min(options.target_height as f32 / source_height);
        let pixmap = hayro::render(
            page,
            &InterpreterSettings::default(),
            &RenderSettings {
                x_scale: scale,
                y_scale: scale,
                width: Some(width),
                height: Some(height),
            },
        );
        let stride = options
            .target_width
            .checked_mul(PixelFormat::Rgba8888.bytes_per_pixel())
            .ok_or(RenderError::ResourceLimitExceeded)?;
        let rendered = RenderedPage {
            width: options.target_width,
            height: options.target_height,
            stride,
            pixel_format: PixelFormat::Rgba8888,
            bytes: pixmap.data_as_u8_slice().to_vec(),
        };
        rendered.validate()?;
        Ok(rendered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_pdf() -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] >>",
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn opens_and_renders_pdf() {
        let mut renderer = PdfRenderer::open(&minimal_pdf()).expect("valid PDF opens");
        let info = renderer.info().unwrap();
        assert_eq!(info.page_count, 1);
        assert_eq!(info.format, DocumentFormat::Pdf);

        let page = renderer
            .render_page(
                0,
                &RenderOptions {
                    target_width: 200,
                    target_height: 100,
                },
            )
            .unwrap();
        assert_eq!((page.width, page.height), (200, 100));
        assert_eq!(page.bytes.len(), 200 * 100 * 4);
    }

    #[test]
    fn letterboxes_when_target_aspect_mismatches_source() {
        let mut renderer = PdfRenderer::open(&minimal_pdf()).expect("valid PDF opens");
        // Source page is 200x100 (2:1); request a 200x120 (5:3) canvas.
        let page = renderer
            .render_page(
                0,
                &RenderOptions {
                    target_width: 200,
                    target_height: 120,
                },
            )
            .unwrap();
        assert_eq!((page.width, page.height), (200, 120));
        // The scaled page occupies 200x100; the extra 20px are white padding
        // (RGB 255,255,255) at the bottom — the page is not stretched.
        let padding = &page.bytes[200 * 100 * 4..page.bytes.len()];
        assert_eq!(padding.len(), 200 * 20 * 4);
        assert!(padding
            .chunks_exact(4)
            .all(|pixel| pixel[0] == 255 && pixel[1] == 255 && pixel[2] == 255));
    }

    #[test]
    fn rejects_corrupt_pdf_and_invalid_page() {
        assert!(PdfRenderer::open(b"%PDF-1.7 invalid").is_err());

        let mut renderer = PdfRenderer::open(&minimal_pdf()).unwrap();
        assert!(matches!(
            renderer.render_page(
                1,
                &RenderOptions {
                    target_width: 100,
                    target_height: 100,
                }
            ),
            Err(RenderError::InvalidDocument)
        ));
    }
}
