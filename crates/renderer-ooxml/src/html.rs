//! HTML text-preview renderer (ADR-010).
//!
//! **Sanitization by construction**: HTML is never rendered, executed or
//! interpreted — only visible text is extracted (lenient parse, entities
//! never expanded, `<script>`/`<style>`/`<head>` skipped) and drawn as plain
//! text on one page.

use docbunker_renderer_api::{
    DocumentFormat, DocumentInfo, DocumentRenderer, RenderError, RenderOptions, RenderedPage,
};

use crate::container;
use crate::render;

pub struct HtmlRenderer {
    lines: Vec<String>,
    info: DocumentInfo,
}

/// Content-based HTML gate (defense in depth; detection runs first).
fn looks_like_html(data: &[u8]) -> bool {
    let head: Vec<u8> = data
        .iter()
        .take(64)
        .map(|b| b.to_ascii_lowercase())
        .collect();
    head.starts_with(b"<html") || head.starts_with(b"<!doctype html")
}

impl DocumentRenderer for HtmlRenderer {
    fn open(data: &[u8]) -> Result<Self, RenderError> {
        docbunker_renderer_api::limits::validate_document_size(data)?;
        if !looks_like_html(data) {
            return Err(RenderError::UnsupportedFormat);
        }
        let mut lines = Vec::new();
        let mut truncated = false;
        container::extract_markup_text(data, &mut lines, &mut truncated)?;
        if lines.is_empty() {
            return Err(RenderError::InvalidDocument);
        }
        let info = DocumentInfo {
            page_count: 1,
            width: render::PAGE_WIDTH,
            height: render::PAGE_HEIGHT,
            format: DocumentFormat::Html,
        };
        info.validate()?;
        Ok(Self { lines, info })
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
        render::render_text_page(&self.lines, &[], options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_visible_text_only() {
        let html = br#"<!doctype html>
<html><head><title>secreto</title><style>.x{}</style></head>
<body>
<script>alert("malware")</script>
<h1>Titulo</h1>
<p>Contenido <b>normal</b> &amp; seguro</p>
<ul><li>Uno</li><li>Dos</li></ul>
</body></html>"#;
        let mut renderer = HtmlRenderer::open(html).expect("html opens");
        assert_eq!(
            renderer.lines,
            vec!["Titulo", "Contenido normal & seguro", "Uno", "Dos"]
        );
        let page = renderer
            .render_page(
                0,
                &RenderOptions {
                    target_width: 620,
                    target_height: 877,
                },
            )
            .unwrap();
        page.validate().unwrap();
    }

    #[test]
    fn rejects_non_html() {
        assert!(HtmlRenderer::open(b"not html at all").is_err());
    }

    #[test]
    fn script_and_style_content_is_never_extracted() {
        let html = br#"<html><body><script>var x = "pwned";</script><p>ok</p></body></html>"#;
        let renderer = HtmlRenderer::open(html).expect("opens");
        assert_eq!(renderer.lines, vec!["ok"]);
    }
}
