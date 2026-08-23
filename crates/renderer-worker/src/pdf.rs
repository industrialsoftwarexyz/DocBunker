//! PDF rendering via MuPDF (cargo feature `pdf-mupdf`).
//!
//! **License notice (AGPL-3.0)**: linking MuPDF into the worker binary makes
//! the resulting binary subject to AGPL-3.0. The default build does **not**
//! enable this feature; DocBunker itself is MIT. See ADR-005 for the analysis;
//! Hayro is the permissive default renderer.
//!
//! **Feature restrictions**: the `mupdf` crate is used with
//! `default-features = false`, so MuPDF's JavaScript engine (mujs), and the
//! optional HTML/EPUB/SVG/etc. input filters are **not** compiled in. Only
//! rasterization is performed; nothing extracted from the PDF ever leaves the
//! worker except validated RGBA (ADR-002).

use docbunker_renderer_api::limits;
use docbunker_renderer_api::{
    DocumentFormat, DocumentInfo, DocumentRenderer, PixelFormat, RenderError, RenderOptions,
    RenderedPage,
};

/// PDF renderer backed by MuPDF.
pub struct PdfRenderer {
    document: mupdf::Document,
    info: DocumentInfo,
}

impl PdfRenderer {
    /// Map a MuPDF error to a user-safe renderer error (details stay in logs).
    fn map_err(err: mupdf::Error) -> RenderError {
        tracing::warn!(%err, "mupdf error");
        match err {
            mupdf::Error::InvalidPdfDocument => RenderError::InvalidDocument,
            _ => RenderError::InvalidDocument,
        }
    }
}

impl DocumentRenderer for PdfRenderer {
    fn open(data: &[u8]) -> Result<Self, RenderError> {
        limits::validate_document_size(data)?;
        let document =
            mupdf::Document::from_bytes(data, "application/pdf").map_err(Self::map_err)?;
        let page_count = document.page_count().map_err(Self::map_err)?;
        let page_count = u32::try_from(page_count).map_err(|_| RenderError::InvalidDocument)?;
        if page_count == 0 || page_count > limits::MAX_PAGE_COUNT {
            return Err(RenderError::InvalidDocument);
        }

        let first = document.load_page(0).map_err(Self::map_err)?;
        let bounds = first.bounds().map_err(Self::map_err)?;
        let (width, height) = (bounds.width(), bounds.height());
        if width <= 0.0 || height <= 0.0 {
            return Err(RenderError::InvalidDocument);
        }

        let info = DocumentInfo {
            page_count,
            width: width as u32,
            height: height as u32,
            format: DocumentFormat::Pdf,
        };
        info.validate()?;
        Ok(Self { document, info })
    }

    fn info(&self) -> Result<DocumentInfo, RenderError> {
        Ok(self.info.clone())
    }

    fn render_page(
        &mut self,
        page: u32,
        options: &RenderOptions,
    ) -> Result<RenderedPage, RenderError> {
        limits::validate_render_target(options.target_width, options.target_height)?;
        if page >= self.info.page_count {
            return Err(RenderError::InvalidDocument);
        }

        let page = self
            .document
            .load_page(page as i32)
            .map_err(Self::map_err)?;
        let bounds = page.bounds().map_err(Self::map_err)?;
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return Err(RenderError::InvalidDocument);
        }

        let scale = (options.target_width as f32 / bounds.width())
            .min(options.target_height as f32 / bounds.height());
        if !scale.is_finite() || scale <= 0.0 {
            return Err(RenderError::InvalidDocument);
        }
        let predicted_width = (bounds.width() * scale).ceil() as u32;
        let predicted_height = (bounds.height() * scale).ceil() as u32;
        limits::validate_render_target(predicted_width, predicted_height)?;
        limits::pixel_buffer_size(
            predicted_width,
            predicted_height,
            PixelFormat::Rgba8888.bytes_per_pixel(),
        )
        .ok_or(RenderError::ResourceLimitExceeded)?;
        let pixmap = page
            .to_pixmap(
                &mupdf::Matrix::new_scale(scale, scale),
                &mupdf::Colorspace::device_rgb(),
                true, // alpha: RGBA output
                true, // show_extras: render page annotations/content normally
            )
            .map_err(Self::map_err)?;

        let width = pixmap.width();
        let height = pixmap.height();
        let samples = pixmap.samples();
        let expected =
            limits::pixel_buffer_size(width, height, PixelFormat::Rgba8888.bytes_per_pixel())
                .ok_or(RenderError::ResourceLimitExceeded)?;
        if samples.len() != expected {
            return Err(RenderError::RenderingFailed);
        }

        let rendered = RenderedPage {
            width,
            height,
            stride: width
                .checked_mul(PixelFormat::Rgba8888.bytes_per_pixel())
                .ok_or(RenderError::ResourceLimitExceeded)?,
            pixel_format: PixelFormat::Rgba8888,
            bytes: samples.to_vec(),
        };
        rendered.validate()?;
        Ok(rendered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal, valid one-page PDF (no xref; MuPDF rebuilds it).
    fn minimal_pdf() -> Vec<u8> {
        br#"%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R >> endobj
4 0 obj << /Length 44 >> stream
BT /F1 24 Tf 20 50 Td (Hello DocBunker) Tj ET
endstream endobj
5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj
trailer << /Root 1 0 R >>
%%EOF
"#
        .to_vec()
    }

    #[test]
    fn opens_valid_pdf() {
        let renderer = PdfRenderer::open(&minimal_pdf()).expect("valid pdf opens");
        let info = renderer.info().unwrap();
        assert_eq!(info.page_count, 1);
        assert_eq!(info.format, DocumentFormat::Pdf);
        info.validate().unwrap();
    }

    #[test]
    fn renders_page() {
        let mut renderer = PdfRenderer::open(&minimal_pdf()).unwrap();
        let page = renderer
            .render_page(
                0,
                &RenderOptions {
                    target_width: 200,
                    target_height: 100,
                },
            )
            .unwrap();
        page.validate().unwrap();
        assert_eq!(page.width, 200);
        assert_eq!(page.height, 100);
        assert_eq!(page.bytes.len(), 200 * 100 * 4);
    }

    #[test]
    fn constrains_rendering_by_both_target_dimensions() {
        let mut renderer = PdfRenderer::open(&minimal_pdf()).unwrap();
        let page = renderer
            .render_page(
                0,
                &RenderOptions {
                    target_width: 4096,
                    target_height: 10,
                },
            )
            .unwrap();
        assert!(page.width <= 4096);
        assert!(page.height <= 10);
    }

    #[test]
    fn rejects_corrupt_pdf() {
        assert!(PdfRenderer::open(b"%PDF-1.7 this is not a real pdf").is_err());
    }

    #[test]
    fn rejects_page_out_of_range() {
        let mut renderer = PdfRenderer::open(&minimal_pdf()).unwrap();
        assert!(matches!(
            renderer.render_page(
                5,
                &RenderOptions {
                    target_width: 100,
                    target_height: 100
                }
            ),
            Err(RenderError::InvalidDocument)
        ));
    }
}
