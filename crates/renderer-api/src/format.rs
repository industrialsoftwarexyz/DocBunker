//! Content-based format detection.
//!
//! Detection only inspects magic bytes plus, for ZIP containers, a bounded
//! central-directory entry-name scan (it never decodes any entry). It is the
//! first gate before any decoder is invoked. Filenames, extensions and MIME
//! types are never trusted for this decision.

use std::io::Cursor;

use crate::limits;
use crate::types::DocumentFormat;

/// Sniff the document format from its leading bytes.
///
/// Returns [`DocumentFormat::Unknown`] for empty, too-short or unrecognized
/// input. Failing safe (Unknown) is intentional: an unknown document must
/// never reach a decoder.
pub fn detect(data: &[u8]) -> DocumentFormat {
    if data.len() < 8 {
        return DocumentFormat::Unknown;
    }

    if data.starts_with(b"%PDF-") {
        return DocumentFormat::Pdf;
    }

    const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if data.starts_with(&PNG_SIGNATURE) {
        return DocumentFormat::Png;
    }

    if data.starts_with(&[0xFF, 0xD8]) {
        return DocumentFormat::Jpeg;
    }

    // RIFF container: "RIFF" + u32 size + "WEBP".
    if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        return DocumentFormat::WebP;
    }

    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return DocumentFormat::Gif;
    }

    if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
        return DocumentFormat::Tiff;
    }

    if data.starts_with(b"BM") {
        return DocumentFormat::Bmp;
    }

    if data.starts_with(b"{\\rtf") {
        return DocumentFormat::Rtf;
    }

    let lower = first_ascii_lower(data, 32);
    if lower.starts_with(b"<html") || lower.starts_with(b"<!doctype html") {
        return DocumentFormat::Html;
    }

    // ZIP local-file-header magic. Both Office containers (ADR-007) and
    // EPUB books (ADR-010) are ZIP files; a bounded entry-name scan tells
    // them apart. Any other ZIP fails closed at detection.
    if data.starts_with(b"PK\x03\x04") {
        return zip_kind(data);
    }

    DocumentFormat::Unknown
}

/// First `n` bytes lowercased (ASCII), for case-insensitive signatures.
fn first_ascii_lower(data: &[u8], n: usize) -> Vec<u8> {
    data.iter()
        .take(n)
        .map(|b| b.to_ascii_lowercase())
        .collect()
}

/// Distinguish the ZIP container subtypes by their required entry names.
/// Only entry names are read (bounded by the container entry cap); no entry
/// is ever decoded here.
fn zip_kind(data: &[u8]) -> DocumentFormat {
    if data.len() > limits::MAX_DOCUMENT_SIZE {
        return DocumentFormat::Unknown;
    }
    let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(data)) else {
        return DocumentFormat::Unknown;
    };
    if archive.is_empty() || archive.len() > 1_024 {
        return DocumentFormat::Unknown;
    }
    let mut has_content_types = false;
    let mut has_container = false;
    for index in 0..archive.len() {
        let Ok(entry) = archive.by_index(index) else {
            return DocumentFormat::Unknown;
        };
        match entry.name() {
            "[Content_Types].xml" => has_content_types = true,
            "META-INF/container.xml" | "mimetype" => has_container = true,
            _ => {}
        }
        if has_content_types && has_container {
            // Both present: OOXML wins (it always carries Content_Types).
            return DocumentFormat::Ooxml;
        }
    }
    if has_content_types {
        DocumentFormat::Ooxml
    } else if has_container {
        DocumentFormat::Epub
    } else {
        DocumentFormat::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut archive = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
            for (name, content) in entries {
                archive.start_file(*name, options).expect("entry");
                archive.write_all(content).expect("body");
            }
            archive.finish().expect("archive");
        }
        buffer
    }

    fn ooxml_zip() -> Vec<u8> {
        zip_with(&[("[Content_Types].xml", b"<Types/>")])
    }

    fn epub_zip() -> Vec<u8> {
        zip_with(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", b"<container/>"),
        ])
    }

    #[test]
    fn detects_new_image_formats() {
        assert_eq!(
            detect(b"GIF89a\x01\x00\x01\x00\x80\x00\x00"),
            DocumentFormat::Gif
        );
        assert_eq!(detect(b"II*\x00\x08\x00\x00\x00"), DocumentFormat::Tiff);
        assert_eq!(detect(b"MM\x00*\x00\x00\x00\x08"), DocumentFormat::Tiff);
        assert_eq!(
            detect(b"BM\x36\x00\x00\x00\x00\x00\x00"),
            DocumentFormat::Bmp
        );
    }

    #[test]
    fn detects_rtf_and_html() {
        assert_eq!(detect(b"{\\rtf1\\ansi hello"), DocumentFormat::Rtf);
        assert_eq!(
            detect(b"<html><body>hi</body></html>"),
            DocumentFormat::Html
        );
        assert_eq!(
            detect(b"<!DOCTYPE html><title>x</title>"),
            DocumentFormat::Html
        );
        assert_eq!(detect(b"<HTML><BODY>x</BODY></HTML>"), DocumentFormat::Html);
    }

    #[test]
    fn distinguishes_zip_containers() {
        assert_eq!(detect(&ooxml_zip()), DocumentFormat::Ooxml);
        assert_eq!(detect(&epub_zip()), DocumentFormat::Epub);
    }

    #[test]
    fn plain_zip_fails_closed() {
        assert_eq!(
            detect(&zip_with(&[("readme.txt", b"hi")])),
            DocumentFormat::Unknown
        );
    }

    #[test]
    fn rejects_short_input() {
        assert_eq!(detect(b"\xFF\xD8"), DocumentFormat::Unknown);
        assert_eq!(detect(b""), DocumentFormat::Unknown);
        assert_eq!(detect(b"\x89PNG\r\n"), DocumentFormat::Unknown);
    }

    #[test]
    fn rejects_junk() {
        assert_eq!(detect(b"not a real file at all"), DocumentFormat::Unknown);
        assert_eq!(detect(b"RIFF\x00\x00\x00\x00XXXP"), DocumentFormat::Unknown);
    }
}
