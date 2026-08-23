//! Shared container/markup helpers for the text-document renderers
//! (OOXML, EPUB, HTML — ADR-007/010).
//!
//! The same A5-style caps apply to every ZIP container: entry count,
//! per-entry and total uncompressed size, declared compression ratio, and
//! bounded reads (`Read::take`). Markup extraction never expands entities.

use std::io::Read;

use docbunker_renderer_api::{limits, RenderError};
use quick_xml::events::Event;
use quick_xml::Reader;
use zip::ZipArchive;

use crate::{MAX_ENTRY_UNCOMPRESSED, MAX_TOTAL_UNCOMPRESSED, MAX_ZIP_ENTRIES};

/// Maximum number of characters extracted from a document.
pub const MAX_TEXT_CHARS: usize = 200_000;

/// Allow a declared compression ratio of `uncompressed / compressed`,
/// with a fixed slack so tiny entries are not rejected.
pub fn ratio_allowed(compressed: u64, uncompressed: u64) -> bool {
    uncompressed <= compressed.saturating_mul(crate::MAX_COMPRESSION_RATIO) + crate::RATIO_SLACK
}

/// Validate one entry's declared sizes before any allocation (A5 caps).
pub fn entry_within_caps(
    compressed: u64,
    uncompressed: u64,
    total: u64,
) -> Result<(), RenderError> {
    if uncompressed > MAX_ENTRY_UNCOMPRESSED {
        return Err(RenderError::ResourceLimitExceeded);
    }
    if !ratio_allowed(compressed, uncompressed) {
        return Err(RenderError::ResourceLimitExceeded);
    }
    let next_total = total
        .checked_add(uncompressed)
        .ok_or(RenderError::ResourceLimitExceeded)?;
    if next_total > MAX_TOTAL_UNCOMPRESSED {
        return Err(RenderError::ResourceLimitExceeded);
    }
    Ok(())
}

/// Open a ZIP archive with the container caps applied.
pub fn open_archive(data: &[u8]) -> Result<ZipArchive<std::io::Cursor<Vec<u8>>>, RenderError> {
    limits::validate_document_size(data)?;
    let archive = ZipArchive::new(std::io::Cursor::new(data.to_vec()))
        .map_err(|_| RenderError::InvalidDocument)?;
    if archive.is_empty() || archive.len() > MAX_ZIP_ENTRIES {
        return Err(RenderError::ResourceLimitExceeded);
    }
    Ok(archive)
}

/// Read a ZIP entry into a `Vec<u8>`, enforcing the per-entry caps
/// (entry count, per-entry size, ratio, bounded read).
pub fn read_entry_bounded(
    archive: &mut ZipArchive<std::io::Cursor<Vec<u8>>>,
    index: usize,
) -> Result<Vec<u8>, RenderError> {
    let mut total = 0u64;
    read_entry_bounded_total(archive, index, &mut total)
}

/// Read a ZIP entry into a `Vec<u8>`, enforcing every container cap and
/// charging its uncompressed size against the running `total` (so a multi-
/// entry document cannot bypass `MAX_TOTAL_UNCOMPRESSED` by staying under
/// the per-entry cap — ADR-007).
pub fn read_entry_bounded_total(
    archive: &mut ZipArchive<std::io::Cursor<Vec<u8>>>,
    index: usize,
    total: &mut u64,
) -> Result<Vec<u8>, RenderError> {
    let entry = archive
        .by_index(index)
        .map_err(|_| RenderError::InvalidDocument)?;
    let compressed = entry.compressed_size();
    let uncompressed = entry.size();
    entry_within_caps(compressed, uncompressed, *total)?;
    *total = total
        .checked_add(uncompressed)
        .ok_or(RenderError::ResourceLimitExceeded)?;

    let mut bytes = Vec::new();
    let read = entry
        .take(MAX_ENTRY_UNCOMPRESSED + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| RenderError::InvalidDocument)?;
    if read as u64 > MAX_ENTRY_UNCOMPRESSED {
        return Err(RenderError::ResourceLimitExceeded);
    }
    Ok(bytes)
}

/// Find the index of an exact archive member name.
pub fn find_entry(archive: &mut ZipArchive<std::io::Cursor<Vec<u8>>>, name: &str) -> Option<usize> {
    (0..archive.len()).find(|index| {
        archive
            .by_index(*index)
            .map(|entry| entry.name() == name)
            .unwrap_or(false)
    })
}

/// Decode a text run: the five predefined entities and numeric character
/// references are decoded; anything else (`&foo;`) is dropped. Never expands.
///
/// Non-entity bytes are copied as UTF-8 (quick_xml delivers raw UTF-8); the
/// old byte-per-char mapping mangled every accented character.
pub fn decode_text(raw: &[u8]) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        if raw[index] != b'&' {
            // Copy the run up to the next `&` as UTF-8; invalid bytes are
            // replaced (they cannot carry entity syntax).
            let end = raw[index + 1..]
                .iter()
                .position(|b| *b == b'&')
                .map(|offset| index + 1 + offset)
                .unwrap_or(raw.len());
            out.push_str(&String::from_utf8_lossy(&raw[index..end]));
            index = end;
            continue;
        }
        let Some(end) = raw[index + 1..].iter().position(|b| *b == b';') else {
            break;
        };
        let end = index + 1 + end;
        let entity = &raw[index + 1..end];
        let decoded = match entity {
            b"amp" => Some('&'),
            b"lt" => Some('<'),
            b"gt" => Some('>'),
            b"quot" => Some('"'),
            b"apos" => Some('\''),
            _ => decode_numeric(entity),
        };
        if let Some(ch) = decoded {
            out.push(ch);
        }
        index = end + 1;
    }
    out
}

fn decode_numeric(entity: &[u8]) -> Option<char> {
    let digits = entity.strip_prefix(b"#")?;
    let (radix, text) = if digits.starts_with(b"x") || digits.starts_with(b"X") {
        (16, &digits[1..])
    } else {
        (10, digits)
    };
    let value = std::str::from_utf8(text).ok()?;
    let codepoint = u32::from_str_radix(value, radix).ok()?;
    if !(0x1..=0xD7FF).contains(&codepoint) && !(0xE000..=0x10FFFF).contains(&codepoint) {
        return None;
    }
    char::from_u32(codepoint)
}

/// Largest byte index `<= max` that is a UTF-8 char boundary. Portable
/// equivalent of the not-yet-stable `str::floor_char_boundary`; only ever
/// walks back over the tail of one multi-byte character.
pub(crate) fn floor_char_boundary(text: &str, max: usize) -> usize {
    let max = max.min(text.len());
    let mut cut = max;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

/// Append text to `out`, enforcing the character cap. Blank lines are dropped.
pub fn push_line(out: &mut Vec<String>, text: &str, truncated: &mut bool) {
    if text.trim().is_empty() {
        return;
    }
    if out.len() >= 4_096 || *truncated {
        return;
    }
    let remaining = MAX_TEXT_CHARS.saturating_sub(out.iter().map(String::len).sum::<usize>());
    if remaining == 0 {
        *truncated = true;
        return;
    }
    let mut line = text.to_string();
    if line.len() > remaining {
        // Never truncate mid-character: `String::truncate` panics on a
        // non-char-boundary index, and the byte cut can land anywhere in a
        // hostile multi-byte document.
        line.truncate(floor_char_boundary(&line, remaining));
        *truncated = true;
    }
    out.push(line);
}

/// Extract visible text from one markup part (HTML/XHTML).
///
/// Lenient parsing (unmatched end tags allowed, entities never expanded);
/// `<script>`, `<style>`, `<head>` and everything inside is skipped.
pub fn extract_markup_text(
    data: &[u8],
    out: &mut Vec<String>,
    truncated: &mut bool,
) -> Result<(), RenderError> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().allow_unmatched_ends = true;
    reader.config_mut().check_end_names = false;
    reader.config_mut().expand_empty_elements = false;

    let mut line = String::new();
    let mut skip_depth: Option<usize> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                let local = start.local_name().into_inner().to_vec();
                if skip_depth.is_none()
                    && matches!(local.as_slice(), b"script" | b"style" | b"head")
                {
                    skip_depth = Some(0);
                } else if let Some(depth) = skip_depth.as_mut() {
                    *depth += 1;
                } else if is_block_tag_start(&local) {
                    flush_line(out, &mut line, truncated);
                }
            }
            Ok(Event::Empty(empty)) => {
                let local = empty.local_name().into_inner();
                if skip_depth.is_none() && is_markup_break(local) {
                    flush_line(out, &mut line, truncated);
                }
            }
            Ok(Event::End(end)) => {
                let local = end.local_name().into_inner();
                if let Some(depth) = skip_depth.as_mut() {
                    if *depth == 0 {
                        skip_depth = None;
                    } else {
                        *depth -= 1;
                    }
                } else if is_block_tag_start(local) {
                    flush_line(out, &mut line, truncated);
                }
            }
            Ok(Event::Text(text)) => {
                if skip_depth.is_some() {
                    continue;
                }
                let decoded = decode_text(text.as_ref());
                if !decoded.trim().is_empty() {
                    if line.len() + decoded.len() > MAX_TEXT_CHARS {
                        // Cut at a char boundary; slicing mid-character panics.
                        let room = MAX_TEXT_CHARS - line.len();
                        let cut = floor_char_boundary(&decoded, room);
                        line.push_str(&decoded[..cut]);
                    } else {
                        line.push_str(&decoded);
                    }
                }
            }
            Ok(Event::DocType(_))
            | Ok(Event::Decl(_))
            | Ok(Event::PI(_))
            | Ok(Event::Comment(_)) => {}
            Ok(Event::Eof) => break,
            Err(_) => return Err(RenderError::InvalidDocument),
            _ => {}
        }
    }
    flush_line(out, &mut line, truncated);
    Ok(())
}

fn is_block_tag_start(local: &[u8]) -> bool {
    is_block_tag_end(local) || matches!(local, b"br" | b"td" | b"th")
}

fn is_block_tag_end(local: &[u8]) -> bool {
    matches!(
        local,
        b"p" | b"li"
            | b"tr"
            | b"div"
            | b"h1"
            | b"h2"
            | b"h3"
            | b"h4"
            | b"h5"
            | b"h6"
            | b"blockquote"
            | b"pre"
    )
}

fn is_markup_break(local: &[u8]) -> bool {
    is_block_tag_end(local) || matches!(local, b"br" | b"td" | b"th")
}

fn flush_line(out: &mut Vec<String>, line: &mut String, truncated: &mut bool) {
    if !line.is_empty() {
        push_line(out, line.trim_end(), truncated);
        line.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_text_preserves_utf8() {
        assert_eq!(
            decode_text("adiós café — 中文".as_bytes()),
            "adiós café — 中文"
        );
        assert_eq!(decode_text(b"a&amp;b"), "a&b");
        assert_eq!(decode_text(b"caf&#xE9;"), "café");
        assert_eq!(decode_text(b"&lol2;&amp;"), "&");
    }

    #[test]
    fn push_line_never_truncates_mid_character() {
        let mut out = Vec::new();
        let mut truncated = false;
        push_line(&mut out, "a", &mut truncated);
        // Previous code truncated at `remaining` bytes directly, which
        // panicked when that index landed mid-character (199_999 here).
        let long = "é".repeat(100_001);
        push_line(&mut out, &long, &mut truncated);
        assert!(truncated);
        let total: usize = out.iter().map(String::len).sum();
        assert!(total <= MAX_TEXT_CHARS);
        assert!(out
            .iter()
            .all(|line| std::str::from_utf8(line.as_bytes()).is_ok()));
    }

    #[test]
    fn markup_extraction_never_panics_on_long_multibyte() {
        let body = format!(
            "<html><body><p>a</p><p>{}</p></body></html>",
            "é".repeat(100_001)
        );
        let mut out = Vec::new();
        let mut truncated = false;
        extract_markup_text(body.as_bytes(), &mut out, &mut truncated).unwrap();
        let total: usize = out.iter().map(String::len).sum();
        assert!(total <= MAX_TEXT_CHARS);
        assert!(out
            .iter()
            .all(|line| std::str::from_utf8(line.as_bytes()).is_ok()));
    }
}
