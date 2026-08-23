//! Shared domain types for DocBunker rendering.
//!
//! None of these types carry document-derived content beyond minimal numeric
//! metadata, per ADR-002 ("Raster-only document boundary").

use crate::error::RenderError;
use crate::limits;

/// Pixel layout of a rendered page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 8 bits per channel, 4 bytes per pixel: R, G, B, A.
    Rgba8888,
}

impl PixelFormat {
    /// Bytes per pixel for this format.
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            PixelFormat::Rgba8888 => limits::RGBA_BYTES_PER_PIXEL,
        }
    }
}

impl TryFrom<u8> for PixelFormat {
    type Error = RenderError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(PixelFormat::Rgba8888),
            other => Err(RenderError::Internal(format!(
                "unknown pixel format: {other}"
            ))),
        }
    }
}

impl From<PixelFormat> for u8 {
    fn from(value: PixelFormat) -> Self {
        match value {
            PixelFormat::Rgba8888 => 1,
        }
    }
}

/// Supported document formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocumentFormat {
    Unknown,
    Pdf,
    Png,
    Jpeg,
    WebP,
    /// ZIP-based Office container (docx/pptx/xlsx), rendered as text preview
    /// only (ADR-007).
    Ooxml,
    Gif,
    Tiff,
    Bmp,
    /// ZIP-based EPUB e-book container, text preview (ADR-010).
    Epub,
    Rtf,
    Html,
}

impl DocumentFormat {
    /// Stable wire representation.
    pub const fn as_u8(self) -> u8 {
        match self {
            DocumentFormat::Unknown => 0,
            DocumentFormat::Pdf => 1,
            DocumentFormat::Png => 2,
            DocumentFormat::Jpeg => 3,
            DocumentFormat::WebP => 4,
            DocumentFormat::Ooxml => 5,
            DocumentFormat::Gif => 6,
            DocumentFormat::Tiff => 7,
            DocumentFormat::Bmp => 8,
            DocumentFormat::Epub => 9,
            DocumentFormat::Rtf => 10,
            DocumentFormat::Html => 11,
        }
    }

    /// Human-readable label, safe for display.
    pub const fn label(self) -> &'static str {
        match self {
            DocumentFormat::Unknown => "Unknown",
            DocumentFormat::Pdf => "PDF",
            DocumentFormat::Png => "PNG",
            DocumentFormat::Jpeg => "JPEG",
            DocumentFormat::WebP => "WebP",
            DocumentFormat::Ooxml => "OOXML",
            DocumentFormat::Gif => "GIF",
            DocumentFormat::Tiff => "TIFF",
            DocumentFormat::Bmp => "BMP",
            DocumentFormat::Epub => "EPUB",
            DocumentFormat::Rtf => "RTF",
            DocumentFormat::Html => "HTML",
        }
    }
}

impl TryFrom<u8> for DocumentFormat {
    type Error = RenderError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(DocumentFormat::Unknown),
            1 => Ok(DocumentFormat::Pdf),
            2 => Ok(DocumentFormat::Png),
            3 => Ok(DocumentFormat::Jpeg),
            4 => Ok(DocumentFormat::WebP),
            5 => Ok(DocumentFormat::Ooxml),
            6 => Ok(DocumentFormat::Gif),
            7 => Ok(DocumentFormat::Tiff),
            8 => Ok(DocumentFormat::Bmp),
            9 => Ok(DocumentFormat::Epub),
            10 => Ok(DocumentFormat::Rtf),
            11 => Ok(DocumentFormat::Html),
            other => Err(RenderError::Internal(format!(
                "unknown document format: {other}"
            ))),
        }
    }
}

/// Minimal metadata about an open document.
///
/// `width`/`height` are points for PDFs and pixels for images; consumers treat
/// them purely as an aspect-ratio hint (ADR-002).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentInfo {
    pub page_count: u32,
    pub width: u32,
    pub height: u32,
    pub format: DocumentFormat,
}

impl DocumentInfo {
    /// Validate against the hard limits. Always call before trusting the data.
    pub fn validate(&self) -> Result<(), RenderError> {
        if self.page_count == 0 || self.page_count > limits::MAX_PAGE_COUNT {
            return Err(RenderError::InvalidDocument);
        }
        if self.width > limits::MAX_PAGE_WIDTH || self.height > limits::MAX_PAGE_HEIGHT {
            return Err(RenderError::ResourceLimitExceeded);
        }
        Ok(())
    }
}

/// Render target: the desired output raster size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOptions {
    pub target_width: u32,
    pub target_height: u32,
}

/// A rendered page: one raster buffer plus the minimal geometry needed to
/// display it. This is the only payload that may cross the isolation boundary
/// (plus `DocumentInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPage {
    pub width: u32,
    pub height: u32,
    /// Row stride in bytes; must be >= `width * bytes_per_pixel`.
    pub stride: u32,
    pub pixel_format: PixelFormat,
    pub bytes: Vec<u8>,
}

impl RenderedPage {
    /// Validate geometry, stride and buffer size against the hard limits.
    ///
    /// All arithmetic is checked; this must be called by the host on every
    /// page received from a renderer (defense in depth).
    pub fn validate(&self) -> Result<(), RenderError> {
        if self.width == 0 || self.height == 0 {
            return Err(RenderError::ResourceLimitExceeded);
        }
        if self.width > limits::MAX_PAGE_WIDTH || self.height > limits::MAX_PAGE_HEIGHT {
            return Err(RenderError::ResourceLimitExceeded);
        }

        let bpp = self.pixel_format.bytes_per_pixel();
        let min_stride = self
            .width
            .checked_mul(bpp)
            .ok_or(RenderError::ResourceLimitExceeded)?;
        if self.stride < min_stride {
            return Err(RenderError::ResourceLimitExceeded);
        }

        let stride =
            usize::try_from(self.stride).map_err(|_| RenderError::ResourceLimitExceeded)?;
        let height =
            usize::try_from(self.height).map_err(|_| RenderError::ResourceLimitExceeded)?;
        let expected = stride
            .checked_mul(height)
            .ok_or(RenderError::ResourceLimitExceeded)?;
        if expected > limits::MAX_PIXEL_BUFFER {
            return Err(RenderError::ResourceLimitExceeded);
        }
        if self.bytes.len() != expected {
            return Err(RenderError::RenderingFailed);
        }
        Ok(())
    }
}

/// The renderer abstraction. Implementations are untrusted and must run inside
/// the sandbox; they must never touch the host directly (ADR-002, ADR-005).
pub trait DocumentRenderer {
    /// Open a document from its raw bytes (untrusted input).
    ///
    /// Implementations must validate the size, sniff the format and reject
    /// anything that does not match an enabled format.
    fn open(data: &[u8]) -> Result<Self, RenderError>
    where
        Self: Sized;

    /// Return minimal metadata about the open document.
    fn info(&self) -> Result<DocumentInfo, RenderError>;

    /// Render one page into a validated [`RenderedPage`].
    ///
    /// `page` is 0-based and must be `< info().page_count`.
    fn render_page(
        &mut self,
        page: u32,
        options: &RenderOptions,
    ) -> Result<RenderedPage, RenderError>;
}
