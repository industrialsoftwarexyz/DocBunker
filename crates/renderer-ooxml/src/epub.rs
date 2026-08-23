//! EPUB (ZIP of XHTML) text-preview renderer (ADR-010).
//!
//! Only the spine documents (per the OPF manifest) are read, one page each;
//! every container cap from ADR-007 applies (entry count, sizes, ratio,
//! bounded reads, no entity expansion).

use std::io::Cursor;

use docbunker_renderer_api::{
    DocumentFormat, DocumentInfo, DocumentRenderer, RenderError, RenderOptions, RenderedPage,
};
use quick_xml::events::Event;
use quick_xml::Reader;
use zip::ZipArchive;

use crate::container;
use crate::render;

/// Maximum number of spine documents rendered as pages.
const MAX_SPINE_DOCS: usize = 1_024;

pub struct EpubRenderer {
    pages: Vec<Vec<String>>,
    info: DocumentInfo,
}

/// Read `META-INF/container.xml` and resolve the first `rootfile` OPF path.
fn opf_path(archive: &mut ZipArchive<Cursor<Vec<u8>>>, total: &mut u64) -> Option<String> {
    let index = container::find_entry(archive, "META-INF/container.xml")?;
    let xml = container::read_entry_bounded_total(archive, index, total).ok()?;
    let mut reader = Reader::from_reader(xml.as_slice());
    loop {
        // The `<rootfile .../>` element is self-closing (`Event::Empty`).
        match reader.read_event() {
            Ok(Event::Start(start)) | Ok(Event::Empty(start)) => {
                if start.local_name().as_ref() == b"rootfile" {
                    for attribute in start.attributes().flatten() {
                        if attribute.key.as_ref() == b"full-path" {
                            let path = String::from_utf8_lossy(&attribute.value).into_owned();
                            if !path.is_empty() && !path.contains('\\') {
                                return Some(path);
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) => return None,
            Err(_) => return None,
            _ => {}
        }
    }
}

/// Parse the OPF and return spine hrefs (resolved relative to the OPF dir).
fn spine_hrefs(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    opf: &str,
    total: &mut u64,
) -> Result<Vec<String>, RenderError> {
    let Some(index) = container::find_entry(archive, opf) else {
        return Err(RenderError::InvalidDocument);
    };
    let xml = container::read_entry_bounded_total(archive, index, total)?;
    let base_dir = opf
        .rsplit_once('/')
        .map(|(dir, _)| format!("{dir}/"))
        .unwrap_or_default();

    let mut manifest: Vec<(String, String)> = Vec::new();
    let mut spine_order: Vec<String> = Vec::new();
    let mut reader = Reader::from_reader(xml.as_slice());

    loop {
        match reader.read_event() {
            // OPF elements are usually self-closing (`<item .../>`), which
            // quick-xml reports as `Empty`, so both variants are handled here.
            Ok(Event::Start(start)) | Ok(Event::Empty(start)) => {
                match start.local_name().as_ref() {
                    b"item" => {
                        let mut id = None;
                        let mut href = None;
                        for attribute in start.attributes().flatten() {
                            match attribute.key.as_ref() {
                                b"id" => {
                                    id =
                                        Some(String::from_utf8_lossy(&attribute.value).into_owned())
                                }
                                b"href" => {
                                    href =
                                        Some(String::from_utf8_lossy(&attribute.value).into_owned())
                                }
                                _ => {}
                            }
                        }
                        if let (Some(id), Some(href)) = (id, href) {
                            if href.ends_with(".xhtml")
                                || href.ends_with(".html")
                                || href.ends_with(".htm")
                            {
                                manifest.push((id, href));
                            }
                        }
                    }
                    b"itemref" => {
                        for attribute in start.attributes().flatten() {
                            if attribute.key.as_ref() == b"idref" {
                                spine_order
                                    .push(String::from_utf8_lossy(&attribute.value).into_owned());
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return Err(RenderError::InvalidDocument),
            _ => {}
        }
    }

    let by_id: std::collections::HashMap<String, String> = manifest.into_iter().collect();
    let mut hrefs = Vec::new();
    for idref in spine_order {
        if let Some(href) = by_id.get(&idref) {
            if !hrefs.contains(href) && hrefs.len() < MAX_SPINE_DOCS {
                hrefs.push(format!("{base_dir}{href}"));
            }
        }
    }
    Ok(hrefs)
}

impl DocumentRenderer for EpubRenderer {
    fn open(data: &[u8]) -> Result<Self, RenderError> {
        let mut archive = container::open_archive(data)?;
        let mut total = 0u64;
        let opf = opf_path(&mut archive, &mut total).ok_or(RenderError::InvalidDocument)?;
        let hrefs = spine_hrefs(&mut archive, &opf, &mut total)?;
        if hrefs.is_empty() {
            return Err(RenderError::InvalidDocument);
        }

        let mut pages = Vec::with_capacity(hrefs.len());
        let mut truncated = false;
        for href in hrefs {
            let Some(index) = container::find_entry(&mut archive, &href) else {
                continue;
            };
            // The uncompressed size of every spine payload is charged against
            // the same running total, so an EPUB cannot bypass the aggregate
            // zip-bomb cap (ADR-007) by splitting itself into many small
            // entries.
            let bytes = container::read_entry_bounded_total(&mut archive, index, &mut total)?;
            let mut lines = Vec::new();
            container::extract_markup_text(&bytes, &mut lines, &mut truncated)?;
            if !lines.is_empty() {
                pages.push(lines);
            }
            if truncated {
                break;
            }
        }
        if pages.is_empty() {
            return Err(RenderError::InvalidDocument);
        }

        let info = DocumentInfo {
            page_count: u32::try_from(pages.len())
                .map_err(|_| RenderError::ResourceLimitExceeded)?,
            width: render::PAGE_WIDTH,
            height: render::PAGE_HEIGHT,
            format: DocumentFormat::Epub,
        };
        info.validate()?;
        Ok(Self { pages, info })
    }

    fn info(&self) -> Result<DocumentInfo, RenderError> {
        Ok(self.info.clone())
    }

    fn render_page(
        &mut self,
        page: u32,
        options: &RenderOptions,
    ) -> Result<RenderedPage, RenderError> {
        let lines = self
            .pages
            .get(page as usize)
            .ok_or(RenderError::InvalidDocument)?;
        render::render_text_page(lines, &[], options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn epub(opf_dir: &str) -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut archive = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
            archive.start_file("mimetype", options).unwrap();
            archive.write_all(b"application/epub+zip").unwrap();
            archive
                .start_file("META-INF/container.xml", options)
                .unwrap();
            archive
                .write_all(
                    format!(
                        r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="{opf_dir}content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#
                    )
                    .as_bytes(),
                )
                .unwrap();
            let opf_path = format!("{opf_dir}content.opf");
            archive.start_file(opf_path, options).unwrap();
            archive
                .write_all(
                    br#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <manifest>
    <item id="c1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="chapter2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="c1"/>
    <itemref idref="c2"/>
  </spine>
</package>"#,
                )
                .unwrap();
            archive
                .start_file(format!("{opf_dir}chapter1.xhtml"), options)
                .unwrap();
            archive
                .write_all(br#"<html><body><h1>Capitulo 1</h1><p>Primer parrafo</p></body></html>"#)
                .unwrap();
            archive
                .start_file(format!("{opf_dir}chapter2.xhtml"), options)
                .unwrap();
            archive
                .write_all(
                    br#"<html><body><h1>Capitulo 2</h1><p>Segundo parrafo</p></body></html>"#,
                )
                .unwrap();
            archive.finish().unwrap();
        }
        buffer
    }

    #[test]
    fn opens_and_paginates_epub() {
        let mut renderer = EpubRenderer::open(&epub("OEBPS/")).expect("epub opens");
        let info = renderer.info().unwrap();
        assert_eq!(info.format, DocumentFormat::Epub);
        assert_eq!(info.page_count, 2);

        let page = renderer
            .render_page(
                1,
                &RenderOptions {
                    target_width: 620,
                    target_height: 877,
                },
            )
            .unwrap();
        page.validate().unwrap();
        assert_eq!((page.width, page.height), (620, 877));
    }

    #[test]
    fn rejects_non_epub_zip() {
        let mut buffer = Vec::new();
        {
            let mut archive = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            archive
                .start_file("readme.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(b"hi").unwrap();
            archive.finish().unwrap();
        }
        assert!(EpubRenderer::open(&buffer).is_err());
    }

    #[test]
    fn total_uncompressed_cap_is_enforced() {
        // 65 deflated 1 MiB spine documents: every entry is within the
        // per-entry cap and has a sane compression ratio, but together they
        // exceed MAX_TOTAL_UNCOMPRESSED. The EPUB path must charge each
        // payload against a running total (ADR-007); previously only the
        // per-entry cap was applied and this container opened fine.
        // The content is valid XHTML with randomish letters so it cannot
        // shrink below the 1000:1 ratio cap.
        let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut payload = String::from("<html><body><p>");
        for _ in 0..1_040_000 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            payload.push((b'a' + (rng as u8 % 26)) as char);
        }
        payload.push_str("</p></body></html>");

        let mut buffer = Vec::new();
        {
            let mut archive = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            archive
                .start_file("META-INF/container.xml", options)
                .unwrap();
            archive
                .write_all(
                    br#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#,
                )
                .unwrap();
            archive.start_file("OEBPS/content.opf", options).unwrap();
            let mut opf = String::from(
                r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <manifest>"#,
            );
            for index in 0..65 {
                opf.push_str(&format!(
                    r#"<item id="c{index}" href="c{index}.xhtml" media-type="application/xhtml+xml"/>"#
                ));
            }
            opf.push_str("</manifest><spine>");
            for index in 0..65 {
                opf.push_str(&format!(r#"<itemref idref="c{index}"/>"#));
            }
            opf.push_str("</spine></package>");
            archive.write_all(opf.as_bytes()).unwrap();

            for index in 0..65 {
                archive
                    .start_file(format!("OEBPS/c{index}.xhtml"), options)
                    .unwrap();
                archive.write_all(payload.as_bytes()).unwrap();
            }
            archive.finish().unwrap();
        }
        assert!(matches!(
            EpubRenderer::open(&buffer),
            Err(RenderError::ResourceLimitExceeded)
        ));
    }
}
