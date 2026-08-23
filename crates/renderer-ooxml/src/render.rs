//! Text â†’ RGBA page rendering (ADR-007).
//!
//! Layout happens at the requested target size with a base font size scaled
//! from an A4 canvas at 150 dpi; words wrap at the target width and text is
//! truncated to the page. The output is a validated [`RenderedPage`] like any
//! other renderer's.

use std::sync::OnceLock;

use docbunker_renderer_api::limits;
use docbunker_renderer_api::{PixelFormat, RenderError, RenderOptions, RenderedPage};
use fontdue::{Font, FontSettings, LineMetrics};

/// A4 at 150 dpi â€” the preview's intrinsic page geometry.
pub const PAGE_WIDTH: u32 = 1240;
pub const PAGE_HEIGHT: u32 = 1754;

/// Base font size in pixels on the intrinsic page.
const BASE_FONT_PX: f32 = 16.0;
const LINE_HEIGHT_RATIO: f32 = 1.35;
const MARGIN_RATIO: f32 = 0.04;

/// Roboto (variable font; default instance â‰ˆ Regular), SIL OFL 1.1 â€” see
/// `fonts/OFL.txt`. Embedded so the isolated worker never touches host
/// filesystem paths (ADR-007).
const FONT_BYTES: &[u8] = include_bytes!("fonts/Roboto-Regular.ttf");

fn font() -> Result<&'static Font, RenderError> {
    static FONT: OnceLock<Result<Font, String>> = OnceLock::new();
    FONT.get_or_init(|| {
        Font::from_bytes(FONT_BYTES, FontSettings::default()).map_err(|error| error.to_string())
    })
    .as_ref()
    .map_err(|error| RenderError::Internal(error.clone()))
}

/// Draw a black-on-white text page with optional embedded images below the
/// text (ADR-008). Media is decoded one image at a time, scaled to the
/// content width and clamped to the remaining page height.
pub fn render_text_page(
    lines: &[String],
    media: &[Vec<u8>],
    options: &RenderOptions,
) -> Result<RenderedPage, RenderError> {
    limits::validate_render_target(options.target_width, options.target_height)?;
    let font = font()?;

    let width = options.target_width;
    let height = options.target_height;
    let mut buffer = vec![255u8; limits::pixel_buffer_size(width, height, 4).unwrap()];

    let scale = (width as f32 / PAGE_WIDTH as f32).min(height as f32 / PAGE_HEIGHT as f32);
    let font_px = (BASE_FONT_PX * scale).max(1.0);
    let line_height = (font_px * LINE_HEIGHT_RATIO).round().max(1.0) as u32;
    let margin = ((width as f32 * MARGIN_RATIO).round() as u32).max(1);
    let max_x = width.saturating_sub(margin);

    let line_metrics = font
        .horizontal_line_metrics(font_px)
        .unwrap_or(LineMetrics {
            ascent: font_px,
            descent: 0.0,
            line_gap: 0.0,
            new_line_size: font_px,
        });

    let mut x = margin;
    let mut y = margin;
    let mut in_word = false;

    for line in lines {
        if line.is_empty() {
            y += line_height;
            x = margin;
            in_word = false;
            continue;
        }
        for ch in line.chars() {
            if ch.is_whitespace() {
                x += advance_of(font, ch, font_px);
                in_word = false;
                continue;
            }
            if x >= max_x || (x + advance_of(font, ch, font_px) > max_x && !in_word) {
                x = margin;
                y += line_height;
                if y + line_height > height {
                    break;
                }
            }
            draw_glyph(
                font,
                ch,
                font_px,
                x,
                y,
                &line_metrics,
                &mut buffer,
                width,
                height,
            );
            x += advance_of(font, ch, font_px);
            in_word = true;
        }
        y += line_height;
        x = margin;
        in_word = false;
        if y + line_height > height {
            break;
        }
    }

    draw_embedded_media(media, width, height, margin, max_x, &mut y, &mut buffer)?;
    let stride = width
        .checked_mul(PixelFormat::Rgba8888.bytes_per_pixel())
        .ok_or(RenderError::ResourceLimitExceeded)?;
    let rendered = RenderedPage {
        width,
        height,
        stride,
        pixel_format: PixelFormat::Rgba8888,
        bytes: buffer,
    };
    rendered.validate()?;
    Ok(rendered)
}

/// Draw the embedded media list below the text (ADR-008). Decoding is
/// sequential, so peak memory is one decoded image plus the page buffer.
fn draw_embedded_media(
    media: &[Vec<u8>],
    width: u32,
    height: u32,
    margin: u32,
    max_x: u32,
    y: &mut u32,
    buffer: &mut [u8],
) -> Result<(), RenderError> {
    const MEDIA_GAP: u32 = 14;
    let content_width = max_x.saturating_sub(margin).max(1);
    for encoded in media {
        if *y + MEDIA_GAP >= height {
            return Ok(());
        }
        let (image_width, image_height, rgba) =
            match docbunker_renderer_image::decode_embedded(encoded) {
                Ok(decoded) => decoded,
                // An undecodable member must not break the preview.
                Err(_) => continue,
            };
        let remaining = height.saturating_sub(*y + MEDIA_GAP).max(1);
        let scale =
            (content_width as f32 / image_width as f32).min(remaining as f32 / image_height as f32);
        if !scale.is_finite() || scale <= 0.0 {
            return Ok(());
        }
        let draw_width = (image_width as f32 * scale).round().max(1.0) as u32;
        let draw_height = (image_height as f32 * scale).round().max(1.0) as u32;
        let scaled = docbunker_renderer_image::scaling::scale_rgba(
            &rgba,
            image_width,
            image_height,
            draw_width,
            draw_height,
        )?;

        *y = y.saturating_add(MEDIA_GAP);
        blit_rgba(
            &scaled,
            draw_width,
            draw_height,
            margin,
            *y,
            width,
            height,
            buffer,
        );
        draw_rect_outline(
            margin.saturating_sub(1),
            y.saturating_sub(1),
            draw_width.saturating_add(2),
            draw_height.saturating_add(2),
            width,
            height,
            buffer,
            [206, 210, 214, 255],
        );
        *y = y.saturating_add(draw_height);
        if *y >= height {
            return Ok(());
        }
    }
    Ok(())
}

fn advance_of(font: &Font, ch: char, px: f32) -> u32 {
    font.metrics(ch, px).advance_width.round().max(1.0) as u32
}

/// Blit one glyph into the page buffer. A pixel-blit primitive; the many
/// parameters are the geometry it needs.
#[allow(clippy::too_many_arguments)]
fn draw_glyph(
    font: &Font,
    ch: char,
    px: f32,
    x: u32,
    baseline: u32,
    line_metrics: &LineMetrics,
    buffer: &mut [u8],
    width: u32,
    height: u32,
) {
    let (metrics, coverage) = font.rasterize(ch, px);
    if metrics.width == 0 || metrics.height == 0 {
        return;
    }
    // Baseline for this row is `line_start + ascent`; glyph pixels live at
    // `baseline + metrics.ymin`.
    let row_top = baseline as i64 + line_metrics.ascent as i64 + metrics.ymin as i64;
    let col_left = x as i64 + metrics.xmin as i64;
    for row in 0..metrics.height {
        let py = row_top + row as i64;
        if py < 0 || py >= height as i64 {
            continue;
        }
        for col in 0..metrics.width {
            let px_ = col_left + col as i64;
            if px_ < 0 || px_ >= width as i64 {
                continue;
            }
            let alpha = coverage[row * metrics.width + col];
            if alpha == 0 {
                continue;
            }
            // White background blended to black ink by coverage:
            // out = white*(1-a) + black*a = 255 - a.
            let index = ((py as usize) * width as usize + px_ as usize) * 4;
            let shade = 255u8 - alpha;
            buffer[index] = shade;
            buffer[index + 1] = shade;
            buffer[index + 2] = shade;
        }
    }
}

/// Copy a packed RGBA rectangle into the canvas, clipping at the edges.
#[allow(clippy::too_many_arguments)]
fn blit_rgba(
    src: &[u8],
    src_width: u32,
    src_height: u32,
    x: u32,
    y: u32,
    canvas_width: u32,
    canvas_height: u32,
    canvas: &mut [u8],
) {
    let copy_width = src_width.min(canvas_width.saturating_sub(x));
    let copy_height = src_height.min(canvas_height.saturating_sub(y));
    for row in 0..copy_height {
        let src_row = (row as usize) * src_width as usize * 4;
        let dst_row = ((y + row) as usize) * canvas_width as usize * 4 + x as usize * 4;
        let bytes = copy_width as usize * 4;
        canvas[dst_row..dst_row + bytes].copy_from_slice(&src[src_row..src_row + bytes]);
    }
}

/// Render an xlsx table page: shaded header, wrapped cells and a grid
/// (ADR-010). Bounded by the page size like every other preview.
pub fn render_table_page(
    rows: &[Vec<String>],
    options: &RenderOptions,
) -> Result<RenderedPage, RenderError> {
    limits::validate_render_target(options.target_width, options.target_height)?;
    let font = font()?;

    let width = options.target_width;
    let height = options.target_height;
    let mut buffer = vec![255u8; limits::pixel_buffer_size(width, height, 4).unwrap()];

    let scale = (width as f32 / PAGE_WIDTH as f32).min(height as f32 / PAGE_HEIGHT as f32);
    let font_px = (BASE_FONT_PX * scale).max(1.0);
    let margin = ((width as f32 * MARGIN_RATIO).round() as u32).max(1);
    let content_width = width.saturating_sub(margin * 2).max(1);
    let avg_char = font_px * 0.62;
    let line_height = (font_px * LINE_HEIGHT_RATIO).round().max(1.0) as u32;

    // Column widths from the widest cell (wrapped), clamped to the page.
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0).clamp(1, 64);
    let max_col_share = content_width / columns as u32;
    let mut col_widths = vec![8u32; columns];
    for row in rows {
        for (col, cell) in row.iter().enumerate().take(columns) {
            let needed = (avg_char * cell.chars().count() as f32).ceil() as u32 + 8;
            col_widths[col] = col_widths[col].max(needed);
        }
    }
    let total: u64 = col_widths.iter().map(|w| *w as u64).sum();
    let shrink = if total > content_width as u64 {
        content_width as f32 / total as f32
    } else {
        1.0
    };
    for col_width in &mut col_widths {
        *col_width = (((*col_width as f32) * shrink).round() as u32).clamp(8, max_col_share.max(8));
    }
    // Redistribute leftover to the last column.
    let used: u32 = col_widths.iter().sum();
    if used < content_width {
        *col_widths.last_mut().unwrap() += content_width - used;
    }

    let line_metrics = font
        .horizontal_line_metrics(font_px)
        .unwrap_or(LineMetrics {
            ascent: font_px,
            descent: 0.0,
            line_gap: 0.0,
            new_line_size: font_px,
        });

    let mut y = margin;
    for (row_index, row) in rows.iter().enumerate().take(100) {
        let mut row_height = line_height;
        for (col, cell) in row.iter().enumerate().take(columns) {
            let lines = wrapped_line_count(cell, col_widths[col], avg_char);
            row_height = row_height.max(lines.saturating_mul(line_height).max(line_height));
        }
        if y + row_height > height {
            break;
        }

        // Header band shading.
        if row_index == 0 {
            fill_rect(
                margin,
                y,
                content_width,
                row_height,
                &mut buffer,
                width,
                height,
                [238, 241, 244, 255],
            );
        }

        let mut x = margin;
        for (col, cell) in row.iter().enumerate().take(columns) {
            let mut baseline = y.saturating_add(line_height);
            let col_width = col_widths[col];
            for line in wrap_cell(cell, col_width, avg_char) {
                if baseline >= y + row_height {
                    break;
                }
                if !line.trim().is_empty() {
                    draw_text_run(
                        font,
                        &line,
                        x.saturating_add(4),
                        baseline,
                        font_px,
                        row_index == 0,
                        &line_metrics,
                        &mut buffer,
                        width,
                        height,
                    );
                }
                baseline = baseline.saturating_add(line_height);
            }
            x = x.saturating_add(col_width);
        }
        y = y.saturating_add(row_height);
        draw_hline(
            margin,
            y,
            margin + content_width,
            &mut buffer,
            width,
            [214, 218, 222, 255],
        );
    }
    // Column separators and outer frame.
    draw_rect_outline(
        margin,
        margin,
        content_width,
        y.saturating_sub(margin).max(1),
        width,
        height,
        &mut buffer,
        [150, 156, 162, 255],
    );
    let mut x = margin;
    let column_count = columns.min(rows.iter().map(Vec::len).max().unwrap_or(0));
    for col_width in col_widths.iter().take(column_count) {
        x = x.saturating_add(*col_width);
        draw_vline(
            &mut buffer,
            width,
            height,
            x,
            margin,
            y,
            [214, 218, 222, 255],
        );
    }

    let stride = width
        .checked_mul(PixelFormat::Rgba8888.bytes_per_pixel())
        .ok_or(RenderError::ResourceLimitExceeded)?;
    let rendered = RenderedPage {
        width,
        height,
        stride,
        pixel_format: PixelFormat::Rgba8888,
        bytes: buffer,
    };
    rendered.validate()?;
    Ok(rendered)
}

fn wrapped_line_count(text: &str, width_px: u32, avg_char: f32) -> u32 {
    let per_line = chars_per_line(width_px, avg_char).max(1);
    if text.is_empty() {
        return 1;
    }
    text.chars().count().div_ceil(per_line) as u32
}

fn chars_per_line(width_px: u32, avg_char: f32) -> usize {
    ((width_px.saturating_sub(8) as f32) / avg_char.max(1.0))
        .floor()
        .max(1.0) as usize
}

fn wrap_cell(text: &str, width_px: u32, avg_char: f32) -> Vec<String> {
    let per_line = chars_per_line(width_px, avg_char);
    if per_line >= text.chars().count() || text.is_empty() {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        if current.chars().count() + word.chars().count() + 1 > per_line && !current.is_empty() {
            lines.push(current.trim_end().to_string());
            current.clear();
        }
        current.push_str(word);
        current.push(' ');
    }
    if !current.trim().is_empty() {
        lines.push(current.trim_end().to_string());
    }
    lines
}

#[allow(clippy::too_many_arguments)]
fn draw_text_run(
    font: &Font,
    text: &str,
    x: u32,
    baseline: u32,
    font_px: f32,
    bold: bool,
    line_metrics: &LineMetrics,
    buffer: &mut [u8],
    width: u32,
    height: u32,
) {
    let mut cursor = x;
    for ch in text.chars() {
        draw_glyph(
            font,
            ch,
            font_px,
            cursor,
            baseline,
            line_metrics,
            buffer,
            width,
            height,
        );
        if bold {
            draw_glyph(
                font,
                ch,
                font_px,
                cursor.saturating_add(1),
                baseline,
                line_metrics,
                buffer,
                width,
                height,
            );
        }
        cursor = cursor.saturating_add(advance_of(font, ch, font_px));
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_rect(
    x: u32,
    y: u32,
    rect_width: u32,
    rect_height: u32,
    buffer: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    color: [u8; 4],
) {
    let x_end = x.saturating_add(rect_width).min(canvas_width);
    let y_end = y.saturating_add(rect_height).min(canvas_height);
    for py in y..y_end {
        if py >= canvas_height {
            break;
        }
        let row = (py as usize) * canvas_width as usize * 4;
        for px in x..x_end {
            let index = row + px as usize * 4;
            if index + 4 <= buffer.len() {
                buffer[index..index + 4].copy_from_slice(&color);
            }
        }
    }
}

fn draw_hline(
    x_start: u32,
    y: u32,
    x_end: u32,
    buffer: &mut [u8],
    canvas_width: u32,
    color: [u8; 4],
) {
    if y as usize * canvas_width as usize * 4 >= buffer.len() {
        return;
    }
    let row = (y as usize) * canvas_width as usize * 4;
    for px in x_start..x_end.min(canvas_width) {
        let index = row + px as usize * 4;
        buffer[index..index + 4].copy_from_slice(&color);
    }
}

fn draw_vline(
    buffer: &mut [u8],
    canvas_width: u32,
    canvas_height: u32,
    x: u32,
    y_start: u32,
    y_end: u32,
    color: [u8; 4],
) {
    if x >= canvas_width {
        return;
    }
    for py in y_start..y_end.min(canvas_height) {
        let index = (py as usize * canvas_width as usize + x as usize) * 4;
        buffer[index..index + 4].copy_from_slice(&color);
    }
}

/// Draw a 1 px rectangle outline in `color` (image frame / table grid).
#[allow(clippy::too_many_arguments)]
fn draw_rect_outline(
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    canvas_width: u32,
    canvas_height: u32,
    canvas: &mut [u8],
    color: [u8; 4],
) {
    let x_end = x.saturating_add(width).min(canvas_width);
    let y_end = y.saturating_add(height).min(canvas_height);
    for py in y..y_end {
        for px in x..x_end {
            let on_border = px == x || px + 1 == x_end || py == y || py + 1 == y_end;
            if !on_border {
                continue;
            }
            let index = (py as usize * canvas_width as usize + px as usize) * 4;
            canvas[index..index + 4].copy_from_slice(&color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_font_loads() {
        font().expect("embedded Roboto must parse (variable font, default instance)");
    }

    #[test]
    fn renders_and_validates_page() {
        let page = render_text_page(
            &["Hola DocBunker".into(), "Segunda línea".into()],
            &[],
            &RenderOptions {
                target_width: 620,
                target_height: 877,
            },
        )
        .expect("renders");
        assert_eq!((page.width, page.height), (620, 877));
        page.validate().unwrap();
    }

    #[test]
    fn renders_with_embedded_media() {
        let mut cursor = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            8,
            6,
            image::Rgba([200, 30, 40, 255]),
        ))
        .write_to(&mut cursor, image::ImageFormat::Png)
        .expect("fixture encodes");
        let media = vec![cursor.into_inner()];

        let page = render_text_page(
            &["Texto con figura".into()],
            &media,
            &RenderOptions {
                target_width: 620,
                target_height: 877,
            },
        )
        .expect("renders");
        page.validate().unwrap();
        let red = [200u8, 30, 40, 255];
        let found = page
            .bytes
            .chunks_exact(4)
            .any(|pixel| pixel[..3] == red[..3]);
        assert!(found, "embedded image must be visible in the preview");
    }

    #[test]
    fn renders_table_page() {
        let page = render_table_page(
            &[
                vec!["Nombre".into(), "Edad".into()],
                vec!["Ana".into(), "30".into()],
                vec!["Luis".into(), "25".into()],
            ],
            &RenderOptions {
                target_width: 620,
                target_height: 877,
            },
        )
        .expect("table renders");
        page.validate().unwrap();
        assert_eq!((page.width, page.height), (620, 877));
    }

    #[test]
    fn rejects_invalid_targets() {
        assert!(render_text_page(
            &[],
            &[],
            &RenderOptions {
                target_width: 0,
                target_height: 1,
            },
        )
        .is_err());
        assert!(render_text_page(
            &[],
            &[],
            &RenderOptions {
                target_width: 9999,
                target_height: 1,
            },
        )
        .is_err());
    }
}
