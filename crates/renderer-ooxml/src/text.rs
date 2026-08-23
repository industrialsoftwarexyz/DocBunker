//! OOXML container reading and text extraction (ADR-007).
//!
//! Security model:
//!
//! - The ZIP is read with hard caps on entry count, per-entry and total
//!   uncompressed size, and declared compression ratio; reads are bounded
//!   with `Read::take` so header-declared sizes can never cause big
//!   allocations.
//! - Only the text-bearing parts of the three subtypes are read
//!   (`word/document.xml`, `ppt/slides/slideN.xml`, `xl/sharedStrings.xml`).
//! - XML is parsed without any entity expansion: DTD declarations and custom
//!   entities are ignored; only the five predefined entities and numeric
//!   character references are decoded. Billion-laughs payloads stay bounded.

use std::io::Cursor;
use std::path::Path;

use docbunker_renderer_api::{limits, RenderError};
use quick_xml::events::Event;
use quick_xml::Reader;
use zip::ZipArchive;

use crate::container::{
    decode_text, entry_within_caps, floor_char_boundary, push_line, read_entry_bounded,
};
use crate::MAX_ZIP_ENTRIES;

/// Maximum number of characters extracted from a document (canonical in
/// `container`, re-exported here for the public API).
pub use crate::container::MAX_TEXT_CHARS;

/// The preview pages and embedded media extracted from a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OoxmlText {
    /// One entry per preview page (docx/xlsx: one; pptx: one per slide).
    pub pages: Vec<Vec<String>>,
    /// Encoded PNG/JPEG/WebP media from `*/media/` parts (ADR-008), already
    /// validated against the container and media caps. Drawn on page 0.
    pub media: Vec<Vec<u8>>,
    /// Structured table for xlsx previews (rows of cells), when present.
    pub table: Option<Vec<Vec<String>>>,
}

/// Maximum rows/columns/cell length rendered from an xlsx sheet.
const MAX_TABLE_ROWS: usize = 500;
const MAX_TABLE_COLS: usize = 64;
const MAX_CELL_CHARS: usize = 256;

/// Slide index of a `ppt/slides/slideN.xml` entry name.
fn slide_number(name: &str) -> Option<u64> {
    let file = Path::new(name).file_name()?.to_str()?;
    let digits = file.strip_prefix("slide")?.strip_suffix(".xml")?;
    digits.parse().ok()
}

/// Whether a local tag name carries text content.
fn is_text_tag(local: &[u8]) -> bool {
    local == b"t"
}

/// Whether a local tag name ends a logical line (paragraph, table row).
fn is_paragraph_end(local: &[u8]) -> bool {
    matches!(local, b"p" | b"tr" | b"row")
}

/// Extract visible text from one OOXML part (docx/pptx/xlsx XML body).
fn extract_part_xml(
    xml: &[u8],
    out: &mut Vec<String>,
    truncated: &mut bool,
) -> Result<(), RenderError> {
    // Non-validating parse: DTDs and custom entities are ignored (they stay
    // raw in Text events and are dropped by `decode_text`), so no entity
    // expansion can ever happen (billion-laughs safe).
    let mut reader = Reader::from_reader(xml);

    let mut in_text_tag = false;
    let mut line = String::new();
    let mut line_truncated = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                let local = start.local_name().as_ref().to_vec();
                in_text_tag = is_text_tag(&local);
            }
            Ok(Event::Empty(empty)) => {
                let local = empty.local_name().into_inner();
                if is_paragraph_end(local) {
                    push_line(out, line.trim_end(), truncated);
                    line.clear();
                }
            }
            Ok(Event::Text(text)) => {
                if !in_text_tag {
                    continue;
                }
                let decoded = decode_text(text.as_ref());
                if !decoded.trim().is_empty() {
                    if line.len() + decoded.len() > MAX_TEXT_CHARS {
                        line_truncated = true;
                        // Cut at a char boundary; slicing mid-character panics.
                        let room = MAX_TEXT_CHARS - line.len();
                        let cut = floor_char_boundary(&decoded, room);
                        line.push_str(&decoded[..cut]);
                    } else {
                        line.push_str(&decoded);
                    }
                }
            }
            Ok(Event::End(end)) => {
                let local = end.local_name().into_inner();
                if is_paragraph_end(local) {
                    push_line(out, line.trim_end(), truncated);
                    line.clear();
                }
                in_text_tag = false;
            }
            Ok(Event::DocType(_)) | Ok(Event::Decl(_)) | Ok(Event::PI(_)) => {}
            Ok(Event::Eof) => break,
            Err(error) => {
                tracing::warn!(%error, "ooxml: malformed xml part");
                return Err(RenderError::InvalidDocument);
            }
            _ => {}
        }
    }
    if !line.is_empty() {
        push_line(out, line.trim_end(), truncated);
    }
    if line_truncated {
        *truncated = true;
    }
    Ok(())
}

/// Extract the text preview from a docx/pptx/xlsx container.
pub fn extract(data: &[u8]) -> Result<OoxmlText, RenderError> {
    limits::validate_document_size(data)?;

    let mut archive =
        ZipArchive::new(Cursor::new(data.to_vec())).map_err(|_| RenderError::InvalidDocument)?;
    if archive.is_empty() || archive.len() > MAX_ZIP_ENTRIES {
        return Err(RenderError::ResourceLimitExceeded);
    }

    // Collect the text-bearing entries and their declared sizes first, with
    // the container caps applied, so no part is ever decoded twice.
    let mut document_part = None;
    let mut slides: Vec<(u64, usize)> = Vec::new();
    let mut shared_strings = None;
    let mut media_indices: Vec<usize> = Vec::new();
    let mut has_content_types = false;
    let mut total: u64 = 0;
    let mut media_bytes: u64 = 0;

    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| RenderError::InvalidDocument)?;
        let (compressed, uncompressed) = (entry.compressed_size(), entry.size());
        entry_within_caps(compressed, uncompressed, total)?;
        total = total.saturating_add(uncompressed);

        let name = entry.name().to_string();
        if name == "[Content_Types].xml" {
            has_content_types = true;
        } else if name == "word/document.xml" {
            document_part = Some(index);
        } else if let Some(slide) = slide_number(&name) {
            slides.push((slide, index));
        } else if name == "xl/sharedStrings.xml" {
            shared_strings = Some(index);
        } else if is_media_member(&name) {
            // Embedded image caps (ADR-008): bounded count and bytes.
            if media_indices.len() >= crate::MAX_EMBEDDED_IMAGES {
                return Err(RenderError::ResourceLimitExceeded);
            }
            media_bytes = media_bytes.saturating_add(uncompressed);
            if media_bytes > crate::MAX_EMBEDDED_MEDIA_BYTES {
                return Err(RenderError::ResourceLimitExceeded);
            }
            media_indices.push(index);
        }
    }

    // Must be a real Office container (ADR-007); a plain ZIP fails closed.
    if !has_content_types
        || (document_part.is_none() && slides.is_empty() && shared_strings.is_none())
    {
        return Err(RenderError::InvalidDocument);
    }

    let mut truncated = false;
    let mut pages: Vec<Vec<String>> = Vec::new();

    if let Some(index) = document_part {
        let bytes = read_entry_bounded(&mut archive, index)?;
        let mut lines = Vec::new();
        extract_part_xml(&bytes, &mut lines, &mut truncated)?;
        pages.push(lines);
    }

    if !slides.is_empty() {
        // One preview page per slide (ADR-010).
        slides.sort_by_key(|(number, _)| *number);
        for (_, index) in slides {
            if truncated {
                break;
            }
            let bytes = read_entry_bounded(&mut archive, index)?;
            let mut lines = Vec::new();
            extract_part_xml(&bytes, &mut lines, &mut truncated)?;
            pages.push(lines);
        }
    }

    let mut shared_strings_text: Vec<String> = Vec::new();
    if let Some(index) = shared_strings {
        let bytes = read_entry_bounded(&mut archive, index)?;
        extract_part_xml(&bytes, &mut shared_strings_text, &mut truncated)?;
    }

    let table = extract_table(&mut archive, &shared_strings_text)?;

    let mut media = Vec::with_capacity(media_indices.len());
    for index in media_indices {
        media.push(read_entry_bounded(&mut archive, index)?);
    }

    let has_content = pages
        .iter()
        .any(|page| page.iter().any(|line| !line.trim().is_empty()))
        || table.is_some()
        || !media.is_empty();
    if !has_content {
        return Err(RenderError::InvalidDocument);
    }

    Ok(OoxmlText {
        pages,
        media,
        table,
    })
}

/// Extract an xlsx worksheet as a table of cell strings (ADR-010).
///
/// Bounds: 500 rows, 64 columns, 256 chars per cell. Only the first
/// worksheet is previewed.
fn extract_table(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    shared: &[String],
) -> Result<Option<Vec<Vec<String>>>, RenderError> {
    let sheet_index = (0..archive.len()).find(|index| {
        archive
            .by_index(*index)
            .map(|entry| {
                let name = entry.name();
                name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml")
            })
            .unwrap_or(false)
    });
    let Some(sheet_index) = sheet_index else {
        return Ok(None);
    };
    let bytes = read_entry_bounded(archive, sheet_index)?;

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell_ref: Option<(usize, usize)> = None;
    let mut cell_shared = false;
    let mut cell_text = String::new();
    let mut reader = Reader::from_reader(bytes.as_slice());
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                if start.local_name().as_ref() == b"c" {
                    cell_ref = None;
                    cell_shared = false;
                    cell_text.clear();
                    for attribute in start.attributes().flatten() {
                        match attribute.key.as_ref() {
                            b"r" => cell_ref = parse_cell_ref(&attribute.value),
                            b"t" if attribute.value.as_ref() == b"s" => cell_shared = true,
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::Text(text)) => {
                if cell_ref.is_some() {
                    cell_text.push_str(&crate::container::decode_text(text.as_ref()));
                }
            }
            Ok(Event::End(end)) => match end.local_name().as_ref() {
                b"c" => {
                    let value = if cell_shared {
                        cell_text
                            .trim()
                            .parse::<usize>()
                            .ok()
                            .and_then(|index| shared.get(index).cloned())
                            .unwrap_or_default()
                    } else {
                        cell_text.trim().to_string()
                    };
                    let value: String = value
                        .chars()
                        .take(MAX_CELL_CHARS)
                        .filter(|ch| *ch != '\r')
                        .collect();
                    if let Some((col, row_index)) = cell_ref {
                        if row_index < MAX_TABLE_ROWS && col < MAX_TABLE_COLS {
                            if row_index >= row.len() {
                                row.resize(row_index + 1, String::new());
                            }
                            row[row_index] = value;
                        }
                    }
                    cell_ref = None;
                    cell_shared = false;
                    cell_text.clear();
                }
                b"row" => {
                    if !row.is_empty() {
                        rows.push(std::mem::take(&mut row));
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => return Err(RenderError::InvalidDocument),
            _ => {}
        }
        if rows.len() >= MAX_TABLE_ROWS {
            break;
        }
    }
    Ok((!rows.is_empty()).then_some(rows))
}

/// Parse an Excel cell reference like `C4` into `(col, row)` (0-based).
fn parse_cell_ref(reference: &[u8]) -> Option<(usize, usize)> {
    let reference = std::str::from_utf8(reference).ok()?;
    let letters: String = reference
        .chars()
        .take_while(|ch| ch.is_ascii_alphabetic())
        .collect();
    let digits: String = reference
        .chars()
        .skip(letters.len())
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    if letters.is_empty() || digits.is_empty() {
        return None;
    }
    let mut col = 0usize;
    for byte in letters.bytes() {
        col = col
            .checked_mul(26)?
            .checked_add((byte.to_ascii_uppercase() as usize) - ('A' as usize) + 1)?;
    }
    let col = col.checked_sub(1)?;
    let row = digits.parse::<usize>().ok()?.checked_sub(1)?;
    Some((col, row))
}

/// Whether a ZIP member is an embedded raster image under a media folder
/// (ADR-008: only PNG/JPEG/WebP are previewed).
fn is_media_member(name: &str) -> bool {
    let Some((directory, file)) = name.rsplit_once('/') else {
        return false;
    };
    if !directory.ends_with("/media") && !directory.contains("/media/") {
        return false;
    }
    let Some((_, extension)) = file.rsplit_once('.') else {
        return false;
    };
    matches!(extension, "png" | "jpg" | "jpeg" | "webp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::ratio_allowed;

    #[test]
    fn numeric_and_predefined_entities_decode() {
        assert_eq!(decode_text(b"a&amp;b"), "a&b");
        assert_eq!(decode_text(b"caf&#xE9;"), "café");
        assert_eq!(decode_text(b"caf&#233;"), "café");
        assert_eq!(decode_text(b"&lt;&gt;&quot;&apos;"), "<>\"'");
    }

    #[test]
    fn unknown_entities_are_dropped_not_expanded() {
        assert_eq!(decode_text(b"&lol2;&amp;"), "&");
        assert_eq!(decode_text(b"&#x110000;x"), "x");
        assert_eq!(decode_text(b"&#xD800;x"), "x");
        assert_eq!(decode_text(b"&notanentity"), "");
    }

    #[test]
    fn utf8_text_is_preserved() {
        let xml = r#"<w:document xmlns:w="urn:w"><w:body>
            <w:p><w:r><w:t>adiós café — 中文</w:t></w:r></w:p>
          </w:body></w:document>"#;
        let mut lines = Vec::new();
        let mut truncated = false;
        extract_part_xml(xml.as_bytes(), &mut lines, &mut truncated).unwrap();
        assert_eq!(lines, vec!["adiós café — 中文"]);
    }

    #[test]
    fn long_multibyte_text_is_capped_without_panic() {
        let xml = format!(
            r#"<w:document xmlns:w="urn:w"><w:body>
              <w:p><w:r><w:t>a</w:t></w:r></w:p>
              <w:p><w:r><w:t>{}</w:t></w:r></w:p>
            </w:body></w:document>"#,
            "é".repeat(100_001)
        );
        let mut lines = Vec::new();
        let mut truncated = false;
        extract_part_xml(xml.as_bytes(), &mut lines, &mut truncated).unwrap();
        assert!(truncated);
        let total: usize = lines.iter().map(String::len).sum();
        assert!(total <= MAX_TEXT_CHARS);
        assert!(lines
            .iter()
            .all(|line| std::str::from_utf8(line.as_bytes()).is_ok()));
    }

    #[test]
    fn extract_from_minimal_docx_body() {
        let xml = br#"<w:document xmlns:w="urn:w">
            <w:body>
              <w:p><w:r><w:t>Hola mundo</w:t></w:r></w:p>
              <w:p><w:r><w:t>Segunda linea</w:t></w:r></w:p>
            </w:body>
          </w:document>"#;
        let mut lines = Vec::new();
        let mut truncated = false;
        extract_part_xml(xml, &mut lines, &mut truncated).unwrap();
        assert_eq!(lines, vec!["Hola mundo", "Segunda linea"]);
        assert!(!truncated);
    }

    #[test]
    fn extract_from_shared_strings_and_slides() {
        let strings = br#"<sst xmlns="urn:x">
            <si><t>uno</t></si><si><t>dos</t></si>
          </sst>"#;
        let mut lines = Vec::new();
        let mut truncated = false;
        extract_part_xml(strings, &mut lines, &mut truncated).unwrap();
        assert_eq!(lines, vec!["unodos"]);

        let slide = br#"<p:sp xmlns:a="urn:a"><a:p><a:r><a:t>Titulo</a:t></a:r></a:p></p:sp>"#;
        let mut lines = Vec::new();
        extract_part_xml(slide, &mut lines, &mut truncated).unwrap();
        assert_eq!(lines, vec!["Titulo"]);
    }

    #[test]
    fn slide_number_parsing() {
        assert_eq!(slide_number("ppt/slides/slide7.xml"), Some(7));
        assert_eq!(slide_number("ppt/slides/slide1.xml"), Some(1));
        assert_eq!(slide_number("ppt/slides/notesSlide1.xml"), None);
        assert_eq!(slide_number("ppt/slides/slide1.xml/extra"), None);
    }

    #[test]
    fn ratio_caps_reject_extreme_claims() {
        assert!(ratio_allowed(10, 10_000));
        assert!(ratio_allowed(10, 10_064));
        assert!(!ratio_allowed(10, 10_065));
    }
}
