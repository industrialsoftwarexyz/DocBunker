//! Text-document renderers (ADR-007/010).
//!
//! `docx`/`pptx`/`xlsx` are ZIP containers; this crate also previews the
//! other text formats (EPUB, RTF, HTML). Every container is read with hard
//! caps (entry count, per-entry and total uncompressed size, compression
//! ratio) and every markup parse never expands entities (billion-laughs
//! safe). Output is always a validated A4 RGBA page.

pub mod container;
mod epub;
mod html;
mod render;
mod rtf;
mod text;

use docbunker_renderer_api::limits;
use docbunker_renderer_api::{
    DocumentFormat, DocumentInfo, DocumentRenderer, RenderError, RenderOptions, RenderedPage,
};

pub use epub::EpubRenderer;
pub use html::HtmlRenderer;
pub use render::PAGE_WIDTH;
pub use rtf::RtfRenderer;
pub use text::{OoxmlText, MAX_TEXT_CHARS};

/// Maximum number of ZIP entries (A5 mitigation, ADR-007).
pub const MAX_ZIP_ENTRIES: usize = 1_024;
/// Maximum uncompressed size of a single archive entry.
pub const MAX_ENTRY_UNCOMPRESSED: u64 = 16 * 1024 * 1024;
/// Maximum total uncompressed size across all entries.
pub const MAX_TOTAL_UNCOMPRESSED: u64 = limits::MAX_DOCUMENT_SIZE as u64;
/// Maximum declared compression ratio (uncompressed / compressed), plus slack.
pub const MAX_COMPRESSION_RATIO: u64 = 1_000;
pub const RATIO_SLACK: u64 = 64;

/// Maximum number of embedded images collected from a container (ADR-008).
pub const MAX_EMBEDDED_IMAGES: usize = 64;
/// Maximum total bytes of embedded image media per document (ADR-008).
pub const MAX_EMBEDDED_MEDIA_BYTES: u64 = 16 * 1024 * 1024;

/// Text-preview renderer for Office (OOXML) containers.
pub struct OoxmlRenderer {
    text: OoxmlText,
    info: DocumentInfo,
}

impl DocumentRenderer for OoxmlRenderer {
    fn open(data: &[u8]) -> Result<Self, RenderError> {
        limits::validate_document_size(data)?;
        let mut text = text::extract(data)?;
        // xlsx previews render the table on a single page with no text.
        if text.pages.is_empty() && text.table.is_some() {
            text.pages.push(Vec::new());
        }
        if text.pages.is_empty() {
            return Err(RenderError::InvalidDocument);
        }
        let info = DocumentInfo {
            page_count: u32::try_from(text.pages.len())
                .map_err(|_| RenderError::ResourceLimitExceeded)?,
            width: render::PAGE_WIDTH,
            height: render::PAGE_HEIGHT,
            format: DocumentFormat::Ooxml,
        };
        info.validate()?;
        Ok(Self { text, info })
    }

    fn info(&self) -> Result<DocumentInfo, RenderError> {
        Ok(self.info.clone())
    }

    fn render_page(
        &mut self,
        page: u32,
        options: &RenderOptions,
    ) -> Result<RenderedPage, RenderError> {
        let page = usize::try_from(page).map_err(|_| RenderError::InvalidDocument)?;
        if page >= self.text.pages.len() {
            return Err(RenderError::InvalidDocument);
        }
        if let Some(table) = &self.text.table {
            return render::render_table_page(table, options);
        }
        let media: &[Vec<u8>] = if page == 0 { &self.text.media } else { &[] };
        render::render_text_page(&self.text.pages[page], media, options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a minimal in-memory docx container.
    fn docx(body_xml: &str) -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut archive = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            archive
                .start_file("[Content_Types].xml", options)
                .expect("content types");
            archive
                .write_all(
                    br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#,
                )
                .expect("content types body");
            archive
                .start_file("word/document.xml", options)
                .expect("doc");
            archive.write_all(body_xml.as_bytes()).expect("doc body");
            archive.finish().expect("archive");
        }
        buffer
    }

    fn docx_text_xml(runs: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>{runs}</w:t></w:r></w:p>
    <w:p><w:r><w:t>segundo parrafo</w:t></w:r></w:p>
  </w:body>
</w:document>"#
        )
    }

    #[test]
    fn opens_and_renders_docx() {
        let document = docx(&docx_text_xml("Hola &amp; adiós"));
        let mut renderer = OoxmlRenderer::open(&document).expect("docx opens");
        let info = renderer.info().unwrap();
        assert_eq!(info.format, DocumentFormat::Ooxml);
        assert_eq!(info.page_count, 1);

        let page = renderer
            .render_page(
                0,
                &RenderOptions {
                    target_width: 620,
                    target_height: 877,
                },
            )
            .expect("renders");
        page.validate().unwrap();
        assert_eq!((page.width, page.height), (620, 877));
        assert_eq!(page.bytes.len(), 620 * 877 * 4);
    }

    #[test]
    fn rejects_non_container_zip() {
        let mut buffer = Vec::new();
        {
            let mut archive = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            archive
                .start_file("README.txt", zip::write::SimpleFileOptions::default())
                .expect("file");
            archive.write_all(b"not an office document").expect("body");
            archive.finish().expect("archive");
        }
        assert!(OoxmlRenderer::open(&buffer).is_err());
    }

    #[test]
    fn rejects_entry_count_over_cap() {
        let mut buffer = Vec::new();
        {
            let mut archive = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
            archive
                .start_file("[Content_Types].xml", options)
                .expect("ct");
            archive.write_all(b"<Types/>").expect("ct body");
            for index in 0..(MAX_ZIP_ENTRIES + 1) {
                archive
                    .start_file(format!("x/entry-{index}.bin"), options)
                    .expect("entry");
                archive.write_all(b"x").expect("entry body");
            }
            archive.finish().expect("archive");
        }
        assert!(matches!(
            OoxmlRenderer::open(&buffer),
            Err(RenderError::ResourceLimitExceeded)
        ));
    }

    #[test]
    fn rejects_compression_ratio_over_cap() {
        assert!(container::ratio_allowed(1_000, 1_000_000));
        assert!(container::ratio_allowed(1_000, 1_000_064));
        assert!(!container::ratio_allowed(1_000, 1_000_065));
        assert!(container::ratio_allowed(64, 64));
    }

    #[test]
    fn billion_laughs_dtd_is_not_expanded() {
        let body = r#"<?xml version="1.0"?>
<!DOCTYPE lolz [
  <!ENTITY lol "lol">
  <!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">
]>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>&lol2;</w:t></w:r></w:p>
    <w:p><w:r><w:t>contenido normal</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
        let renderer = OoxmlRenderer::open(&docx(body)).expect("billion laughs stays bounded");
        assert_eq!(
            renderer.text.pages[0],
            vec!["contenido normal"],
            "entities must not expand"
        );
    }

    #[test]
    fn extracts_numeric_character_references() {
        let renderer =
            OoxmlRenderer::open(&docx(&docx_text_xml("caf&#xE9; &#233;"))).expect("docx opens");
        let text = &renderer.text.pages[0][0];
        assert!(text.contains("café é"), "got: {text:?}");
    }

    #[test]
    fn embedded_image_is_extracted_and_rendered() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            8,
            6,
            image::Rgba([200, 30, 40, 255]),
        ))
        .write_to(&mut cursor, image::ImageFormat::Png)
        .expect("fixture encodes");
        let png = cursor.into_inner();

        let mut buffer = Vec::new();
        {
            let mut archive = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
            archive
                .start_file("[Content_Types].xml", options)
                .expect("ct");
            archive.write_all(br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#).expect("ct body");
            archive.write_all(b"x").expect("ct body");
            archive
                .start_file("word/document.xml", options)
                .expect("doc");
            archive
                .write_all(docx_text_xml("Figura a continuacion").as_bytes())
                .expect("doc body");
            archive
                .start_file("word/media/image1.png", options)
                .expect("media");
            archive.write_all(&png).expect("media body");
            archive.finish().expect("archive");
        }

        let mut renderer = OoxmlRenderer::open(&buffer).expect("docx with media opens");
        assert_eq!(renderer.text.media.len(), 1);
        let page = renderer
            .render_page(
                0,
                &RenderOptions {
                    target_width: 620,
                    target_height: 792,
                },
            )
            .expect("renders");
        page.validate().unwrap();
        let red = [200u8, 30, 40, 255];
        assert!(
            page.bytes
                .chunks_exact(4)
                .any(|pixel| pixel[..3] == red[..3]),
            "embedded image must be visible in the preview"
        );
    }
}
