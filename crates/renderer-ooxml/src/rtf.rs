//! RTF text-preview renderer (ADR-010).
//!
//! A minimal, non-expanding RTF parser: control words and groups are
//! understood just enough to produce the visible text (`\par`, `\line`,
//! `\tab`, `\'hh` Latin-1 escapes, `\uN` unicode escapes). Font tables,
//! headers and every other destination group are skipped by group depth.
//! Nothing is ever executed or interpreted as markup.

use docbunker_renderer_api::{
    DocumentFormat, DocumentInfo, DocumentRenderer, RenderError, RenderOptions, RenderedPage,
};

use crate::container;
use crate::render;

const MAX_RTF_TEXT: usize = container::MAX_TEXT_CHARS;

pub struct RtfRenderer {
    lines: Vec<String>,
    info: DocumentInfo,
}

/// Append `ch` to the current line, honouring the preview character cap.
/// Every control-word escape (`\uN`, `\'hh`, `\tab`, literal `\\` ...) emits
/// through here; without it a hostile RTF could grow `line` far past
/// `MAX_RTF_TEXT` before `push_line` ever sees it.
fn push_capped(line: &mut String, ch: char, truncated: &mut bool) -> bool {
    if *truncated {
        return false;
    }
    if line.len().saturating_add(ch.len_utf8()) > MAX_RTF_TEXT {
        *truncated = true;
        return false;
    }
    line.push(ch);
    true
}

/// Extract plain text from an RTF document body.
fn extract_rtf_text(data: &[u8]) -> Result<Vec<String>, RenderError> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut truncated = false;
    let mut index = 0;
    let bytes = data;

    // One entry per open group: `true` means its content is a destination
    // (font table, header, picture, ...) and must be skipped.
    let mut group_skip: Vec<bool> = Vec::new();
    let mut unicode_remaining = 0;

    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b'{' => {
                let skip = group_skip.last().copied().unwrap_or(false);
                group_skip.push(skip);
                index += 1;
            }
            b'}' => {
                if group_skip.is_empty() {
                    return Err(RenderError::InvalidDocument);
                }
                group_skip.pop();
                index += 1;
            }
            b'\\' if index + 1 < bytes.len() => {
                let control = bytes[index + 1];
                match control {
                    b'\\' | b'{' | b'}' => {
                        if !deep_skip(&group_skip) {
                            push_capped(&mut line, control as char, &mut truncated);
                        }
                        index += 2;
                    }
                    b'\'' => {
                        // \'hh — Latin-1 byte escape.
                        if index + 3 < bytes.len() {
                            let hex = std::str::from_utf8(&bytes[index + 2..index + 4])
                                .ok()
                                .and_then(|s| u8::from_str_radix(s, 16).ok());
                            if let Some(value) = hex {
                                if !deep_skip(&group_skip) {
                                    push_capped(&mut line, value as char, &mut truncated);
                                }
                            }
                            index += 4;
                        } else {
                            index = bytes.len();
                        }
                    }
                    b'*' => {
                        // \* marks an ignorable destination: skip this group.
                        *group_skip.last_mut().unwrap_or(&mut true) = true;
                        index += 2;
                    }
                    _ if control.is_ascii_alphabetic() => {
                        let mut end = index + 1;
                        while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
                            end += 1;
                        }
                        let mut param_end = end;
                        while param_end < bytes.len()
                            && (bytes[param_end].is_ascii_digit() || bytes[param_end] == b'-')
                        {
                            param_end += 1;
                        }
                        let word = &bytes[index + 1..end];
                        let param = std::str::from_utf8(&bytes[end..param_end])
                            .ok()
                            .and_then(|s| s.parse::<i32>().ok())
                            .unwrap_or(0);

                        if is_destination(word) {
                            *group_skip.last_mut().unwrap_or(&mut true) = true;
                        } else if !deep_skip(&group_skip) {
                            match word {
                                b"par" | b"line" => {
                                    container::push_line(&mut lines, line.trim(), &mut truncated);
                                    line.clear();
                                }
                                b"tab" => {
                                    push_capped(&mut line, ' ', &mut truncated);
                                }
                                b"u" => {
                                    // \uN — UTF-16 code unit (signed).
                                    if let Some(ch) = char::from_u32(param as u32) {
                                        push_capped(&mut line, ch, &mut truncated);
                                    }
                                    // The replacement character after \uN is
                                    // ignored (it is for non-Unicode readers).
                                    unicode_remaining = 1;
                                }
                                b"rquote" => {
                                    push_capped(&mut line, '\u{2019}', &mut truncated);
                                }
                                b"lquote" => {
                                    push_capped(&mut line, '\u{2018}', &mut truncated);
                                }
                                b"endash" => {
                                    push_capped(&mut line, '\u{2013}', &mut truncated);
                                }
                                b"emdash" => {
                                    push_capped(&mut line, '\u{2014}', &mut truncated);
                                }
                                _ => {}
                            }
                        }
                        index = param_end;
                        // A space directly after a control word (and its
                        // optional numeric parameter) is a delimiter, not text.
                        // Skipping it avoids double spaces around runs like `\b`.
                        if index < bytes.len() && bytes[index] == b' ' {
                            index += 1;
                        }
                    }
                    _ => {
                        // Unknown escape: skip it and any digits.
                        index += 2;
                        while index < bytes.len() && bytes[index].is_ascii_digit() {
                            index += 1;
                        }
                    }
                }
            }
            _ => {
                if !deep_skip(&group_skip) && unicode_remaining == 0 {
                    let ch = byte as char;
                    if !ch.is_ascii_control() {
                        push_capped(&mut line, ch, &mut truncated);
                    }
                }
                if unicode_remaining > 0 {
                    unicode_remaining -= 1;
                }
                index += 1;
            }
        }
        if truncated {
            break;
        }
    }

    container::push_line(&mut lines, line.trim(), &mut truncated);
    if lines.is_empty() {
        return Err(RenderError::InvalidDocument);
    }
    Ok(lines)
}

/// Whether any open group is a skipped destination.
fn deep_skip(groups: &[bool]) -> bool {
    groups.last().copied().unwrap_or(false)
}

/// Control words that start an ignorable destination group.
fn is_destination(word: &[u8]) -> bool {
    matches!(
        word,
        b"fonttbl"
            | b"colortbl"
            | b"stylesheet"
            | b"info"
            | b"header"
            | b"headerl"
            | b"headerr"
            | b"headerf"
            | b"footer"
            | b"footerl"
            | b"footerr"
            | b"footerf"
            | b"footnote"
            | b"annotation"
            | b"pict"
            | b"object"
            | b"math"
            | b"revtbl"
            | b"datastore"
            | b"listtable"
            | b"listoverridetable"
            | b"generator"
            | b"themedata"
            | b"colorschememapping"
    )
}

impl DocumentRenderer for RtfRenderer {
    fn open(data: &[u8]) -> Result<Self, RenderError> {
        docbunker_renderer_api::limits::validate_document_size(data)?;
        if !data.starts_with(b"{\\rtf") {
            return Err(RenderError::UnsupportedFormat);
        }
        let lines = extract_rtf_text(data)?;
        let info = DocumentInfo {
            page_count: 1,
            width: render::PAGE_WIDTH,
            height: render::PAGE_HEIGHT,
            format: DocumentFormat::Rtf,
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
    fn extracts_visible_rtf_text() {
        let rtf = br#"{\rtf1\ansi\deff0 {\fonttbl {\f0 Times New Roman;}}
{\colortbl ;\red0\green0\blue0;}
Hola \b mundo\par Segunda \'e9\'f3l\'ednea\par Fin\line Dentro de la misma linea}"#;
        let mut renderer = RtfRenderer::open(rtf).expect("rtf opens");
        assert_eq!(
            renderer.lines,
            vec![
                "Hola mundo",
                "Segunda éólínea",
                "Fin",
                "Dentro de la misma linea"
            ]
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
    fn unicode_escapes_are_decoded() {
        let rtf = br#"{\rtf1 Caf\u233? \u1080?}"#;
        let renderer = RtfRenderer::open(rtf).expect("rtf opens");
        assert_eq!(renderer.lines, vec!["Café и"]);
    }

    #[test]
    fn ignores_ignorable_destinations() {
        let rtf = br#"{\rtf1 Hola {\*\nonsense secreto} mundo}"#;
        let renderer = RtfRenderer::open(rtf).expect("rtf opens");
        assert_eq!(renderer.lines, vec!["Hola  mundo"]);
    }

    #[test]
    fn rejects_corrupt_rtf() {
        let rtf = br#"{\rtf1 bad }}}}}"#;
        assert!(RtfRenderer::open(rtf).is_err());
        assert!(RtfRenderer::open(b"not rtf").is_err());
    }

    #[test]
    fn long_escape_stream_is_capped_without_panic() {
        // Control-word output (`\uN`) used to bypass the preview cap and the
        // final `String::truncate` landed mid-character (panic) on multi-byte
        // text.
        let mut body = String::from("{\\rtf1 x\\par ");
        body.push_str(&"\\u233? ".repeat(90_000));
        body.push('}');
        let renderer = RtfRenderer::open(body.as_bytes()).expect("rtf opens bounded");
        let total: usize = renderer.lines.iter().map(String::len).sum();
        assert!(total <= MAX_RTF_TEXT, "got {total} bytes");
        assert!(renderer
            .lines
            .iter()
            .all(|line| { std::str::from_utf8(line.as_bytes()).is_ok() }));
    }
}
