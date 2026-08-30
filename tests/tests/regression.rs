//! Regression tests for security edge cases.
//!
//! These tests verify that DocBunker correctly handles various adversarial
//! inputs without panicking, consuming excessive resources, or allowing
//! security bypasses. Each test is self-contained and builds its test data
//! programmatically (no external fixture files).

use std::io::Write;

use docbunker_renderer_api::DocumentRenderer;

// ── ZIP Bomb / Aggregate Size Tests ──────────────────────────────────────────

/// Build a ZIP archive with a single entry whose central directory declares
/// a tiny uncompressed size but the actual data is much larger.
fn zip_with_patched_central_directory(declared_uncompressed: u32, actual_data: &[u8]) -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("payload.bin", options).unwrap();
        writer.write_all(actual_data).unwrap();
        writer.finish().unwrap();
    }

    // Patch the central directory to declare a smaller uncompressed size.
    const EOCD: &[u8; 4] = b"PK\x05\x06";
    const CENTRAL: &[u8; 4] = b"PK\x01\x02";

    let eocd_pos = buffer
        .windows(EOCD.len())
        .rposition(|w| w == EOCD)
        .expect("end of central directory");
    let central_offset =
        u32::from_le_bytes(buffer[eocd_pos + 16..eocd_pos + 20].try_into().unwrap()) as usize;
    assert_eq!(&buffer[central_offset..central_offset + 4], CENTRAL);
    buffer[central_offset + 24..central_offset + 28]
        .copy_from_slice(&declared_uncompressed.to_le_bytes());

    buffer
}

#[test]
fn zip_bomb_with_huge_actual_data_is_rejected() {
    let data = vec![b'x'; 2 * 1024 * 1024]; // 2 MiB actual
    let archive_bytes = zip_with_patched_central_directory(1, &data);

    // The OOXML parser should reject this because the actual output exceeds
    // the per-entry limit, even though the central directory claims 1 byte.
    let result = docbunker_renderer_ooxml::OoxmlRenderer::open(&archive_bytes);
    assert!(result.is_err(), "ZIP bomb should be rejected");
}

#[test]
fn zip_with_many_entries_exceeding_count_limit_is_rejected() {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        // Create 1025 entries (limit is 1024).
        for i in 0..1025 {
            let name = format!("entry_{i:04}.xml");
            writer.start_file(&name, options).unwrap();
            writer.write_all(b"<data/>").unwrap();
        }
        writer.finish().unwrap();
    }

    let result = docbunker_renderer_ooxml::OoxmlRenderer::open(&buffer);
    assert!(result.is_err(), "too many entries should be rejected");
}

// ── PDF Edge Case Tests ──────────────────────────────────────────────────────

/// Build a minimal valid PDF with a single page.
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
fn minimal_pdf_is_detected() {
    let pdf = minimal_pdf();
    assert!(docbunker_native_broker::has_supported_signature_bytes(&pdf));
}

#[test]
fn truncated_pdf_with_valid_header_is_detected() {
    let mut pdf = minimal_pdf();
    pdf.truncate(32); // Keep only "%PDF-1.4\n..." prefix
    assert!(docbunker_native_broker::has_supported_signature_bytes(&pdf));
}

#[test]
fn pdf_with_corrupted_version_still_has_valid_signature() {
    let mut pdf = minimal_pdf();
    // Keep %PDF- intact, corrupt bytes after the signature prefix
    // The signature check only looks at the first 5 bytes: %PDF-
    assert!(pdf.starts_with(b"%PDF-"));
    // Modify a byte after the signature prefix (e.g., version minor digit)
    pdf[6] = b'X'; // Change '4' to 'X' in "1.4"
                   // Still has valid %PDF- prefix for signature detection
    assert!(docbunker_native_broker::has_supported_signature_bytes(&pdf));
}

// ── OOXML Edge Case Tests ────────────────────────────────────────────────────

/// Build a minimal OOXML document with the specified body XML.
fn ooxml_with_body(body_xml: &str) -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        archive.start_file("[Content_Types].xml", options).unwrap();
        archive
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#,
            )
            .unwrap();
        archive.start_file("word/document.xml", options).unwrap();
        archive.write_all(body_xml.as_bytes()).unwrap();
        archive.finish().unwrap();
    }
    buffer
}

#[test]
fn ooxml_with_empty_body_is_handled() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body/>
</w:document>"#;
    let docx = ooxml_with_body(xml);
    // Should not panic; may return empty pages or an error for empty body.
    let _ = docbunker_renderer_ooxml::OoxmlRenderer::open(&docx);
}

#[test]
fn ooxml_with_deeply_nested_xml_is_handled() {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>"#,
    );
    // Create 1000 levels of nesting.
    for _ in 0..1000 {
        xml.push_str("<w:p>");
    }
    xml.push_str("<w:r><w:t>deep</w:t></w:r>");
    for _ in 0..1000 {
        xml.push_str("</w:p>");
    }
    xml.push_str("</w:body></w:document>");

    let docx = ooxml_with_body(&xml);
    // Should not panic even with deep nesting.
    let _ = docbunker_renderer_ooxml::OoxmlRenderer::open(&docx);
}

#[test]
fn ooxml_with_binary_content_in_xml_is_handled() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>&#x00;&#x01;&#x02;&#xFF;</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
    let docx = ooxml_with_body(xml);
    // Should not panic on binary content in XML.
    let _ = docbunker_renderer_ooxml::OoxmlRenderer::open(&docx);
}

// ── Signature Detection Tests ────────────────────────────────────────────────

#[test]
fn png_signature_is_detected() {
    let mut data = vec![0u8; 64];
    data[0..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    assert!(docbunker_native_broker::has_supported_signature_bytes(
        &data
    ));
}

#[test]
fn jpeg_signature_is_detected() {
    let mut data = vec![0u8; 64];
    data[0..3].copy_from_slice(b"\xff\xd8\xff");
    assert!(docbunker_native_broker::has_supported_signature_bytes(
        &data
    ));
}

#[test]
fn docx_signature_is_detected() {
    let mut data = vec![0u8; 64];
    data[0..4].copy_from_slice(b"PK\x03\x04");
    assert!(docbunker_native_broker::has_supported_signature_bytes(
        &data
    ));
}

#[test]
fn rtf_signature_is_detected() {
    let mut data = vec![0u8; 64];
    data[0..5].copy_from_slice(b"{\\rtf");
    assert!(docbunker_native_broker::has_supported_signature_bytes(
        &data
    ));
}

#[test]
fn epub_signature_is_detected() {
    let mut data = vec![0u8; 64];
    data[0..4].copy_from_slice(b"PK\x03\x04"); // EPUB is a ZIP
    assert!(docbunker_native_broker::has_supported_signature_bytes(
        &data
    ));
}

#[test]
fn html_is_not_detected_as_document() {
    let data = b"<html><head><title>Not a document</title></head></html>";
    assert!(!docbunker_native_broker::has_supported_signature_bytes(
        data
    ));
}

#[test]
fn random_bytes_are_not_detected() {
    let data: Vec<u8> = (0..64).map(|i| i as u8).collect();
    assert!(!docbunker_native_broker::has_supported_signature_bytes(
        &data
    ));
}

// ── Path Safety Tests ────────────────────────────────────────────────────────

#[test]
fn relative_paths_are_rejected() {
    let path = std::path::Path::new("relative/path/file.pdf");
    assert!(!docbunker_native_broker::is_safe_local_path(path));
}

#[test]
fn empty_path_is_rejected() {
    let path = std::path::Path::new("");
    assert!(!docbunker_native_broker::is_safe_local_path(path));
}

#[cfg(windows)]
mod windows_path_tests {
    #[test]
    fn unc_path_is_rejected() {
        let path = std::path::Path::new(r"\\server\share\file.pdf");
        assert!(!docbunker_native_broker::is_safe_local_path(path));
    }

    #[test]
    fn device_path_is_rejected() {
        let path = std::path::Path::new(r"\\.\PhysicalDrive0");
        assert!(!docbunker_native_broker::is_safe_local_path(path));
    }

    #[test]
    fn verbatim_unc_path_is_rejected() {
        let path = std::path::Path::new(r"\\?\UNC\server\share\file.pdf");
        assert!(!docbunker_native_broker::is_safe_local_path(path));
    }

    #[test]
    fn verbatim_disk_path_is_rejected() {
        let path = std::path::Path::new(r"\\?\C:\Users\file.pdf");
        assert!(!docbunker_native_broker::is_safe_local_path(path));
    }

    #[test]
    fn normal_drive_path_is_accepted() {
        let path = std::path::Path::new(r"C:\Users\alice\file.pdf");
        assert!(docbunker_native_broker::is_safe_local_path(path));
    }
}
