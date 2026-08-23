//! Image decoders and the [`ImageRenderer`].

use std::io::Cursor;

use docbunker_renderer_api::format;
use docbunker_renderer_api::limits;
use docbunker_renderer_api::{
    DocumentFormat, DocumentInfo, DocumentRenderer, PixelFormat, RenderError, RenderOptions,
    RenderedPage,
};

use crate::scaling;

/// A decoded RGBA image.
struct RgbaImage {
    width: u32,
    height: u32,
    bytes: Vec<u8>,
}

/// Renderer for raster image documents (PNG, JPEG, WebP).
pub struct ImageRenderer {
    info: DocumentInfo,
    rgba: RgbaImage,
}

impl ImageRenderer {
    /// Convert 8-bit RGB (3 bpp) or RGBA (4 bpp) rows into packed RGBA.
    fn to_rgba(width: u32, height: u32, data: &[u8], bpp: u32) -> Result<RgbaImage, RenderError> {
        let expected =
            limits::pixel_buffer_size(width, height, bpp).ok_or(RenderError::RenderingFailed)?;
        if data.len() != expected {
            return Err(RenderError::RenderingFailed);
        }
        let size =
            limits::pixel_buffer_size(width, height, PixelFormat::Rgba8888.bytes_per_pixel())
                .ok_or(RenderError::ResourceLimitExceeded)?;
        let mut rgba = vec![0u8; size];
        match bpp {
            3 => {
                for (out, px) in rgba.chunks_exact_mut(4).zip(data.chunks_exact(3)) {
                    out[0] = px[0];
                    out[1] = px[1];
                    out[2] = px[2];
                    out[3] = 255;
                }
            }
            4 => rgba.copy_from_slice(data),
            _ => return Err(RenderError::RenderingFailed),
        }
        Ok(RgbaImage {
            width,
            height,
            bytes: rgba,
        })
    }

    /// Validate decoded dimensions against the hard limits.
    fn validate_dims(width: u32, height: u32) -> Result<(), RenderError> {
        if width == 0 || height == 0 {
            return Err(RenderError::InvalidDocument);
        }
        if width > limits::MAX_PAGE_WIDTH || height > limits::MAX_PAGE_HEIGHT {
            return Err(RenderError::ResourceLimitExceeded);
        }
        let size =
            limits::pixel_buffer_size(width, height, PixelFormat::Rgba8888.bytes_per_pixel())
                .ok_or(RenderError::ResourceLimitExceeded)?;
        if size > limits::MAX_PIXEL_BUFFER {
            return Err(RenderError::ResourceLimitExceeded);
        }
        Ok(())
    }
}

fn decode_png(data: &[u8]) -> Result<RgbaImage, RenderError> {
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|_| RenderError::InvalidDocument)?;
    let width = reader.info().width;
    let height = reader.info().height;
    ImageRenderer::validate_dims(width, height)?;

    let mut buf = vec![0u8; reader.output_buffer_size()];
    reader
        .next_frame(&mut buf)
        .map_err(|_| RenderError::RenderingFailed)?;

    // After normalize_to_color8 the output is 8-bit RGB or RGBA.
    let bpp = match reader.info().color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => {
            tracing::warn!(?other, "unexpected transformed png color type");
            return Err(RenderError::RenderingFailed);
        }
    };
    ImageRenderer::to_rgba(width, height, &buf, bpp)
}

fn decode_jpeg(data: &[u8]) -> Result<RgbaImage, RenderError> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(data));
    decoder
        .read_info()
        .map_err(|_| RenderError::InvalidDocument)?;
    let info = decoder.info().ok_or(RenderError::InvalidDocument)?;
    let width = info.width as u32;
    let height = info.height as u32;
    ImageRenderer::validate_dims(width, height)?;

    // jpeg-decoder produces RGB unless configured otherwise.
    let pixels = decoder.decode().map_err(|_| RenderError::RenderingFailed)?;
    ImageRenderer::to_rgba(width, height, &pixels, 3)
}

fn decode_webp(data: &[u8]) -> Result<RgbaImage, RenderError> {
    // Dimensions from the bitstream header, before any large allocation.
    let features = webp::BitstreamFeatures::new(data).ok_or(RenderError::InvalidDocument)?;
    let width = features.width();
    let height = features.height();
    ImageRenderer::validate_dims(width, height)?;

    let image = webp::Decoder::new(data)
        .decode()
        .ok_or(RenderError::RenderingFailed)?;
    let bpp = image.layout().bytes_per_pixel();
    let rgba = ImageRenderer::to_rgba(width, height, &image, u32::from(bpp))?;
    Ok(rgba)
}

/// Decode GIF/TIFF/BMP through the `image` crate (ADR-010). Dimensions are
/// validated from the header **before** decoding; the decoded buffer is
/// re-validated against the header dimensions afterwards.
fn decode_image_format(data: &[u8], format: DocumentFormat) -> Result<RgbaImage, RenderError> {
    let image_format = match format {
        DocumentFormat::Gif => image::ImageFormat::Gif,
        DocumentFormat::Tiff => image::ImageFormat::Tiff,
        DocumentFormat::Bmp => image::ImageFormat::Bmp,
        _ => return Err(RenderError::UnsupportedFormat),
    };

    let mut probe = image::ImageReader::new(Cursor::new(data));
    probe.set_format(image_format);
    let (width, height) = probe
        .into_dimensions()
        .map_err(|_| RenderError::InvalidDocument)?;
    ImageRenderer::validate_dims(width, height)?;

    let mut reader = image::ImageReader::new(Cursor::new(data));
    reader.set_format(image_format);
    let decoded = reader.decode().map_err(|_| RenderError::RenderingFailed)?;
    let rgba = decoded.to_rgba8();
    if rgba.width() != width || rgba.height() != height {
        return Err(RenderError::RenderingFailed);
    }
    let expected =
        limits::pixel_buffer_size(width, height, PixelFormat::Rgba8888.bytes_per_pixel())
            .ok_or(RenderError::ResourceLimitExceeded)?;
    if rgba.as_raw().len() != expected {
        return Err(RenderError::RenderingFailed);
    }
    Ok(RgbaImage {
        width,
        height,
        bytes: rgba.into_raw(),
    })
}

/// Decode one embedded PNG/JPEG/WebP image (used by the OOXML text preview,
/// ADR-008) into validated, full-resolution RGBA. Dimension caps and
/// decompression-bomb protection are the same as the standalone renderer.
pub fn decode_embedded(data: &[u8]) -> Result<(u32, u32, Vec<u8>), RenderError> {
    limits::validate_document_size(data)?;
    let rgba = match format::detect(data) {
        DocumentFormat::Png => decode_png(data)?,
        DocumentFormat::Jpeg => decode_jpeg(data)?,
        DocumentFormat::WebP => decode_webp(data)?,
        _ => return Err(RenderError::UnsupportedFormat),
    };
    Ok((rgba.width, rgba.height, rgba.bytes))
}

impl DocumentRenderer for ImageRenderer {
    fn open(data: &[u8]) -> Result<Self, RenderError> {
        limits::validate_document_size(data)?;
        let detected = format::detect(data);
        let rgba = match detected {
            DocumentFormat::Png => decode_png(data)?,
            DocumentFormat::Jpeg => decode_jpeg(data)?,
            DocumentFormat::WebP => decode_webp(data)?,
            DocumentFormat::Gif | DocumentFormat::Tiff | DocumentFormat::Bmp => {
                decode_image_format(data, detected)?
            }
            _ => return Err(RenderError::UnsupportedFormat),
        };
        Ok(Self {
            info: DocumentInfo {
                page_count: 1,
                width: rgba.width,
                height: rgba.height,
                format: detected,
            },
            rgba,
        })
    }

    fn info(&self) -> Result<DocumentInfo, RenderError> {
        Ok(self.info.clone())
    }

    fn render_page(
        &mut self,
        page: u32,
        options: &RenderOptions,
    ) -> Result<RenderedPage, RenderError> {
        if page != 0 {
            return Err(RenderError::InvalidDocument);
        }
        limits::validate_render_target(options.target_width, options.target_height)?;

        let bytes = scaling::scale_rgba(
            &self.rgba.bytes,
            self.rgba.width,
            self.rgba.height,
            options.target_width,
            options.target_height,
        )?;

        Ok(RenderedPage {
            width: options.target_width,
            height: options.target_height,
            stride: options
                .target_width
                .checked_mul(PixelFormat::Rgba8888.bytes_per_pixel())
                .ok_or(RenderError::ResourceLimitExceeded)?,
            pixel_format: PixelFormat::Rgba8888,
            bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encode_fixture(format: image::ImageFormat, px: image::Rgba<u8>) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(4, 3, px);
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, format)
            .expect("fixture encodes");
        out.into_inner()
    }

    #[test]
    fn opens_and_renders_png() {
        let data = encode_fixture(image::ImageFormat::Png, image::Rgba([200, 30, 40, 255]));
        let mut r = ImageRenderer::open(&data).unwrap();
        let info = r.info().unwrap();
        assert_eq!((info.width, info.height), (4, 3));
        assert_eq!(info.format, DocumentFormat::Png);
        assert_eq!(info.page_count, 1);

        let page = r
            .render_page(
                0,
                &RenderOptions {
                    target_width: 4,
                    target_height: 3,
                },
            )
            .unwrap();
        page.validate().unwrap();
        assert_eq!(page.bytes, [200, 30, 40, 255].repeat(4 * 3));
    }

    #[test]
    fn opens_and_renders_jpeg() {
        let data = encode_fixture(image::ImageFormat::Jpeg, image::Rgba([10, 200, 30, 255]));
        let mut r = ImageRenderer::open(&data).unwrap();
        let info = r.info().unwrap();
        assert_eq!((info.width, info.height), (4, 3));
        assert_eq!(info.format, DocumentFormat::Jpeg);

        let page = r
            .render_page(
                0,
                &RenderOptions {
                    target_width: 4,
                    target_height: 3,
                },
            )
            .unwrap();
        page.validate().unwrap();
        assert_eq!(page.bytes.len(), 4 * 3 * 4);
    }

    #[test]
    fn opens_and_renders_webp() {
        let data = encode_fixture(image::ImageFormat::WebP, image::Rgba([30, 40, 200, 255]));
        let mut r = ImageRenderer::open(&data).unwrap();
        let info = r.info().unwrap();
        assert_eq!((info.width, info.height), (4, 3));
        assert_eq!(info.format, DocumentFormat::WebP);

        let page = r
            .render_page(
                0,
                &RenderOptions {
                    target_width: 4,
                    target_height: 3,
                },
            )
            .unwrap();
        page.validate().unwrap();
        assert_eq!(page.bytes.len(), 4 * 3 * 4);
    }

    #[test]
    fn opens_and_renders_gif_tiff_bmp() {
        for format in [
            image::ImageFormat::Gif,
            image::ImageFormat::Tiff,
            image::ImageFormat::Bmp,
        ] {
            let data = encode_fixture(format, image::Rgba([120, 60, 30, 255]));
            let mut r = ImageRenderer::open(&data)
                .unwrap_or_else(|error| panic!("{format:?} fixture must open: {error:?}"));
            let info = r.info().unwrap();
            assert_eq!((info.width, info.height), (4, 3), "{format:?}");
            let page = r
                .render_page(
                    0,
                    &RenderOptions {
                        target_width: 4,
                        target_height: 3,
                    },
                )
                .unwrap();
            page.validate().unwrap();
            assert_eq!(page.bytes.len(), 4 * 3 * 4, "{format:?}");
        }
    }

    #[test]
    fn renders_at_scaled_target() {
        let data = encode_fixture(image::ImageFormat::Png, image::Rgba([255, 0, 0, 255]));
        let mut r = ImageRenderer::open(&data).unwrap();
        let page = r
            .render_page(
                0,
                &RenderOptions {
                    target_width: 8,
                    target_height: 6,
                },
            )
            .unwrap();
        assert_eq!((page.width, page.height), (8, 6));
        assert_eq!(page.bytes.len(), 8 * 6 * 4);
    }

    #[test]
    fn rejects_non_image() {
        assert!(matches!(
            ImageRenderer::open(b"not an image at all"),
            Err(RenderError::UnsupportedFormat)
        ));
    }

    #[test]
    fn rejects_truncated_image() {
        let mut data = encode_fixture(image::ImageFormat::Png, image::Rgba([1, 2, 3, 255]));
        data.truncate(data.len() / 2);
        assert!(ImageRenderer::open(&data).is_err());
    }

    #[test]
    fn rejects_absurd_dimensions() {
        // Take a valid PNG and patch its IHDR dimensions to absurd values
        // (recomputing the chunk CRC) — the decoder reads the header, then
        // our dimension validation must reject before any large allocation.
        let mut png = encode_fixture(image::ImageFormat::Png, image::Rgba([1, 2, 3, 255]));
        // PNG layout: sig(8) | len(4) type(4) data(13) crc(4)
        png[16..20].copy_from_slice(&200_000u32.to_be_bytes());
        png[20..24].copy_from_slice(&200_000u32.to_be_bytes());
        let crc = crc32fast::hash(&png[12..29]);
        png[29..33].copy_from_slice(&crc.to_be_bytes());

        match ImageRenderer::open(&png) {
            Err(e) => assert_eq!(e, RenderError::ResourceLimitExceeded),
            Ok(r) => panic!("absurd-dimension png unexpectedly opened: {:?}", r.info()),
        }
    }

    #[test]
    fn rejects_second_page() {
        let data = encode_fixture(image::ImageFormat::Png, image::Rgba([0, 0, 0, 255]));
        let mut r = ImageRenderer::open(&data).unwrap();
        assert!(matches!(
            r.render_page(
                1,
                &RenderOptions {
                    target_width: 2,
                    target_height: 2
                }
            ),
            Err(RenderError::InvalidDocument)
        ));
    }
}
