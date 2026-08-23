//! Semantic validation and wire <-> domain conversion.
//!
//! Every message is validated on **both** sides against the negotiated limits.
//! Wire values never become domain values (`renderer-api` types) without
//! passing through these functions.

use crate::error::ProtocolError;
use crate::limits::{MAX_IPC_MESSAGE, MAX_STRING_LENGTH};
use crate::message::{
    DocumentInfoWire, ErrorResponse, HelloOk, HelloRequest, OpenDocumentRequest,
    PageRenderedResponse, RenderPageRequest,
};
use docbunker_renderer_api::limits as domain;
use docbunker_renderer_api::{DocumentFormat, DocumentInfo, PixelFormat, RenderedPage};

/// Effective limits negotiated during the handshake (minimum of both sides).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedLimits {
    pub max_document_size: usize,
    pub max_ipc_message: usize,
    pub max_page_width: u32,
    pub max_page_height: u32,
    pub max_pixel_buffer: usize,
}

impl NegotiatedLimits {
    /// Worker side: compute effective limits from the host's `Hello`.
    pub fn server_negotiate(client: &HelloRequest) -> Result<Self, ProtocolError> {
        validate_hello_request(client)?;
        Ok(Self {
            max_document_size: usize::try_from(client.max_document_size)
                .map_err(|_| ProtocolError::LimitViolation("document size out of range"))?
                .min(domain::MAX_DOCUMENT_SIZE),
            max_ipc_message: usize::try_from(client.max_ipc_message)
                .map_err(|_| ProtocolError::LimitViolation("ipc message size out of range"))?
                .min(MAX_IPC_MESSAGE),
            max_page_width: domain::MAX_PAGE_WIDTH,
            max_page_height: domain::MAX_PAGE_HEIGHT,
            max_pixel_buffer: domain::MAX_PIXEL_BUFFER,
        })
    }

    /// Host side: compute effective limits from the worker's `HelloOk`.
    pub fn client_negotiate(hello: &HelloOk) -> Result<Self, ProtocolError> {
        validate_hello_ok(hello)?;
        Ok(Self {
            max_document_size: usize::try_from(hello.max_document_size)
                .map_err(|_| ProtocolError::LimitViolation("document size out of range"))?
                .min(domain::MAX_DOCUMENT_SIZE),
            max_ipc_message: usize::try_from(hello.max_ipc_message)
                .map_err(|_| ProtocolError::LimitViolation("ipc message size out of range"))?
                .min(MAX_IPC_MESSAGE),
            max_page_width: hello.max_page_width.min(domain::MAX_PAGE_WIDTH),
            max_page_height: hello.max_page_height.min(domain::MAX_PAGE_HEIGHT),
            max_pixel_buffer: usize::try_from(hello.max_pixel_buffer)
                .map_err(|_| ProtocolError::LimitViolation("pixel buffer size out of range"))?
                .min(domain::MAX_PIXEL_BUFFER),
        })
    }

    /// Build the `HelloOk` this worker advertises.
    pub fn to_hello_ok(&self) -> HelloOk {
        HelloOk {
            protocol_version: crate::limits::PROTOCOL_VERSION,
            max_document_size: self.max_document_size as u64,
            max_ipc_message: self.max_ipc_message as u32,
            max_page_width: self.max_page_width,
            max_page_height: self.max_page_height,
            max_pixel_buffer: self.max_pixel_buffer as u64,
            shm_name: None,
            shm_capacity: None,
        }
    }

    /// Validate an `OpenDocument` request against the negotiated limits.
    pub fn validate_open_document(&self, req: &OpenDocumentRequest) -> Result<(), ProtocolError> {
        if req.document_id == 0 {
            return Err(ProtocolError::LimitViolation("invalid document id"));
        }
        if req.data.len() > self.max_document_size {
            return Err(ProtocolError::LimitViolation("document too large"));
        }
        Ok(())
    }

    /// Validate a `RenderPage` request against the negotiated limits.
    pub fn validate_render_request(&self, req: &RenderPageRequest) -> Result<(), ProtocolError> {
        if req.document_id == 0 {
            return Err(ProtocolError::LimitViolation("invalid document id"));
        }
        self.validate_target(req.target_width, req.target_height)
    }

    fn validate_target(&self, width: u32, height: u32) -> Result<(), ProtocolError> {
        if width == 0 || height == 0 {
            return Err(ProtocolError::LimitViolation(
                "render target must be non-zero",
            ));
        }
        if width > self.max_page_width || height > self.max_page_height {
            return Err(ProtocolError::LimitViolation("render target too large"));
        }
        let size =
            domain::pixel_buffer_size(width, height, PixelFormat::Rgba8888.bytes_per_pixel())
                .ok_or(ProtocolError::LimitViolation("pixel buffer size overflow"))?;
        if size > self.max_pixel_buffer {
            return Err(ProtocolError::LimitViolation("pixel buffer too large"));
        }
        Ok(())
    }

    /// Validate `DocumentInfoWire` and convert it to a domain value.
    pub fn validate_document_info(
        &self,
        info: &DocumentInfoWire,
    ) -> Result<DocumentInfo, ProtocolError> {
        document_info_from_wire(info)
    }

    /// Validate a `PageRenderedResponse` and convert it to a domain value.
    pub fn validate_rendered_page(
        &self,
        resp: &PageRenderedResponse,
    ) -> Result<RenderedPage, ProtocolError> {
        let page = rendered_page_from_wire(resp)?;
        if page.bytes.len() > self.max_pixel_buffer {
            return Err(ProtocolError::LimitViolation("pixel buffer too large"));
        }
        Ok(page)
    }
}

/// Validate the host's `Hello` request.
pub fn validate_hello_request(req: &HelloRequest) -> Result<(), ProtocolError> {
    if req.protocol_version != crate::limits::PROTOCOL_VERSION {
        return Err(ProtocolError::InvalidVersion);
    }
    if req.max_document_size == 0 {
        return Err(ProtocolError::LimitViolation(
            "document size must be non-zero",
        ));
    }
    let ipc = usize::try_from(req.max_ipc_message)
        .map_err(|_| ProtocolError::LimitViolation("ipc message size out of range"))?;
    if !(crate::limits::MIN_IPC_MESSAGE..=MAX_IPC_MESSAGE).contains(&ipc) {
        return Err(ProtocolError::LimitViolation(
            "ipc message size out of range",
        ));
    }
    Ok(())
}

/// Validate the worker's `HelloOk` response.
pub fn validate_hello_ok(ok: &HelloOk) -> Result<(), ProtocolError> {
    if ok.protocol_version != crate::limits::PROTOCOL_VERSION {
        return Err(ProtocolError::InvalidVersion);
    }
    if ok.max_document_size == 0 || ok.max_document_size as usize > domain::MAX_DOCUMENT_SIZE {
        return Err(ProtocolError::LimitViolation("document size out of range"));
    }
    if ok.max_page_width == 0
        || ok.max_page_height == 0
        || ok.max_page_width > domain::MAX_PAGE_WIDTH
        || ok.max_page_height > domain::MAX_PAGE_HEIGHT
    {
        return Err(ProtocolError::LimitViolation("invalid page limits"));
    }
    let ipc = usize::try_from(ok.max_ipc_message)
        .map_err(|_| ProtocolError::LimitViolation("ipc message size out of range"))?;
    if !(crate::limits::MIN_IPC_MESSAGE..=MAX_IPC_MESSAGE).contains(&ipc) {
        return Err(ProtocolError::LimitViolation(
            "ipc message size out of range",
        ));
    }
    let pix = usize::try_from(ok.max_pixel_buffer)
        .map_err(|_| ProtocolError::LimitViolation("pixel buffer size out of range"))?;
    if pix == 0 || pix > domain::MAX_PIXEL_BUFFER {
        return Err(ProtocolError::LimitViolation(
            "pixel buffer size out of range",
        ));
    }
    // Shared-memory region advertisement (ADR-009): either both fields are
    // present and sane, or neither is.
    match (&ok.shm_name, ok.shm_capacity) {
        (Some(name), Some(capacity)) => {
            crate::limits::validate_message_string(name)?;
            if !is_valid_shm_name(name) {
                return Err(ProtocolError::LimitViolation(
                    "shm name must match docbunker_pagebuf_<pid>_<n>.bin",
                ));
            }
            let capacity = usize::try_from(capacity)
                .map_err(|_| ProtocolError::LimitViolation("shm capacity out of range"))?;
            if capacity == 0 || capacity > domain::MAX_PIXEL_BUFFER {
                return Err(ProtocolError::LimitViolation("shm capacity out of range"));
            }
        }
        (None, None) => {}
        _ => {
            return Err(ProtocolError::LimitViolation(
                "shm name and capacity must be both present or both absent",
            ));
        }
    }
    Ok(())
}

/// The worker's shared-region name must be a plain file name that cannot be
/// interpreted as a path: the host joins it onto the temp directory and a
/// separator (`\`, `/`) or `..` would traverse out of it. Digits-only fields
/// make that impossible (ADR-009).
fn is_valid_shm_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("docbunker_pagebuf_") else {
        return false;
    };
    let Some((pid, counter)) = rest.split_once('_') else {
        return false;
    };
    let Some(digits) = counter.strip_suffix(".bin") else {
        return false;
    };
    !pid.is_empty()
        && pid.bytes().all(|b| b.is_ascii_digit())
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Validate an `ErrorResponse` (host side, when the worker reports an error).
pub fn validate_error_response(err: &ErrorResponse) -> Result<(), ProtocolError> {
    crate::message::ErrorCode::try_from(err.code)
        .map_err(|_| ProtocolError::LimitViolation("unknown error code"))?;
    if err.message.len() > MAX_STRING_LENGTH {
        return Err(ProtocolError::StringTooLong);
    }
    crate::limits::validate_message_string(&err.message)
}

/// Convert and validate a wire metadata struct into a domain value.
pub fn document_info_from_wire(wire: &DocumentInfoWire) -> Result<DocumentInfo, ProtocolError> {
    let format = DocumentFormat::try_from(wire.format)
        .map_err(|_| ProtocolError::LimitViolation("unknown document format"))?;
    let info = DocumentInfo {
        page_count: wire.page_count,
        width: wire.width,
        height: wire.height,
        format,
    };
    info.validate()
        .map_err(|_| ProtocolError::LimitViolation("invalid document metadata"))?;
    Ok(info)
}

/// Convert a domain metadata value into its wire representation.
pub fn document_info_to_wire(info: &DocumentInfo) -> DocumentInfoWire {
    DocumentInfoWire {
        page_count: info.page_count,
        width: info.width,
        height: info.height,
        format: info.format.as_u8(),
    }
}

/// Convert and validate a wire raster response into a domain value.
///
/// `shm_len` must already have been normalized by the transport (the host
/// copies the shared region out before validation — ADR-009); an unnormalized
/// shared-memory page is rejected here.
pub fn rendered_page_from_wire(wire: &PageRenderedResponse) -> Result<RenderedPage, ProtocolError> {
    if wire.shm_len.is_some() {
        return Err(ProtocolError::LimitViolation(
            "shared-memory page not normalized",
        ));
    }
    let pixel_format = PixelFormat::try_from(wire.pixel_format)
        .map_err(|_| ProtocolError::LimitViolation("unknown pixel format"))?;
    let page = RenderedPage {
        width: wire.width,
        height: wire.height,
        stride: wire.stride,
        pixel_format,
        bytes: wire.bytes.clone(),
    };
    page.validate()
        .map_err(|_| ProtocolError::LimitViolation("invalid rendered page"))?;
    Ok(page)
}

/// Convert a validated domain page into its wire representation.
pub fn rendered_page_to_wire(page: &RenderedPage) -> PageRenderedResponse {
    PageRenderedResponse {
        document_id: 0,
        page: 0,
        width: page.width,
        height: page.height,
        stride: page.stride,
        pixel_format: page.pixel_format.into(),
        bytes: page.bytes.clone(),
        shm_len: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::PROTOCOL_VERSION;

    fn client_hello() -> HelloRequest {
        HelloRequest {
            protocol_version: PROTOCOL_VERSION,
            max_document_size: domain::MAX_DOCUMENT_SIZE as u64,
            max_ipc_message: MAX_IPC_MESSAGE as u32,
            shm_capable: false,
        }
    }

    fn client_hello_ok() -> HelloOk {
        HelloOk {
            protocol_version: PROTOCOL_VERSION,
            max_document_size: domain::MAX_DOCUMENT_SIZE as u64,
            max_ipc_message: MAX_IPC_MESSAGE as u32,
            max_page_width: domain::MAX_PAGE_WIDTH,
            max_page_height: domain::MAX_PAGE_HEIGHT,
            max_pixel_buffer: domain::MAX_PIXEL_BUFFER as u64,
            shm_name: None,
            shm_capacity: None,
        }
    }

    #[test]
    fn negotiation_takes_minimum() {
        let mut hello = client_hello();
        hello.max_ipc_message = 8192;
        let limits = NegotiatedLimits::server_negotiate(&hello).unwrap();
        assert_eq!(limits.max_ipc_message, 8192);
        assert_eq!(limits.max_document_size, domain::MAX_DOCUMENT_SIZE);
    }

    #[test]
    fn negotiation_rejects_bad_hello() {
        let mut hello = client_hello();
        hello.protocol_version = 99;
        assert!(NegotiatedLimits::server_negotiate(&hello).is_err());

        let mut hello = client_hello();
        hello.max_ipc_message = 64;
        assert!(NegotiatedLimits::server_negotiate(&hello).is_err());

        let mut hello = client_hello();
        hello.max_document_size = 0;
        assert!(NegotiatedLimits::server_negotiate(&hello).is_err());
    }

    #[test]
    fn open_document_limits_enforced() {
        let limits = NegotiatedLimits::server_negotiate(&client_hello()).unwrap();
        assert!(limits
            .validate_open_document(&OpenDocumentRequest {
                document_id: 1,
                data: vec![0; 1024],
            })
            .is_ok());
        assert!(limits
            .validate_open_document(&OpenDocumentRequest {
                document_id: 0,
                data: vec![0; 1024],
            })
            .is_err());
        let mut req = OpenDocumentRequest {
            document_id: 1,
            data: vec![0; 1024],
        };
        req.data.resize(domain::MAX_DOCUMENT_SIZE + 1, 0);
        assert!(limits.validate_open_document(&req).is_err());
    }

    #[test]
    fn render_request_limits_enforced() {
        let limits = NegotiatedLimits::server_negotiate(&client_hello()).unwrap();
        assert!(limits
            .validate_render_request(&RenderPageRequest {
                document_id: 1,
                page: 0,
                target_width: 1024,
                target_height: 768,
            })
            .is_ok());
        assert!(limits
            .validate_render_request(&RenderPageRequest {
                document_id: 1,
                page: 0,
                target_width: 0,
                target_height: 768,
            })
            .is_err());
        assert!(limits
            .validate_render_request(&RenderPageRequest {
                document_id: 1,
                page: 0,
                target_width: 5000,
                target_height: 768,
            })
            .is_err());
        assert!(limits
            .validate_render_request(&RenderPageRequest {
                document_id: 1,
                page: 0,
                target_width: u32::MAX,
                target_height: u32::MAX,
            })
            .is_err());
    }

    #[test]
    fn document_info_wire_validated() {
        let limits = NegotiatedLimits::server_negotiate(&client_hello()).unwrap();
        let wire = DocumentInfoWire {
            page_count: 3,
            width: 612,
            height: 792,
            format: 1,
        };
        let info = limits.validate_document_info(&wire).unwrap();
        assert_eq!(info.page_count, 3);
        assert_eq!(info.format.as_u8(), 1);

        let bad = DocumentInfoWire {
            page_count: 0,
            width: 612,
            height: 792,
            format: 1,
        };
        assert!(limits.validate_document_info(&bad).is_err());

        let bad_format = DocumentInfoWire {
            page_count: 3,
            width: 612,
            height: 792,
            format: 99,
        };
        assert!(limits.validate_document_info(&bad_format).is_err());
    }

    #[test]
    fn rendered_page_validated() {
        let limits = NegotiatedLimits::server_negotiate(&client_hello()).unwrap();
        let good = PageRenderedResponse {
            document_id: 1,
            page: 0,
            width: 100,
            height: 100,
            stride: 400,
            pixel_format: 1,
            bytes: vec![0; 100 * 100 * 4],
            shm_len: None,
        };
        assert!(limits.validate_rendered_page(&good).is_ok());

        let bad_stride = PageRenderedResponse {
            stride: 399,
            ..good.clone()
        };
        assert!(limits.validate_rendered_page(&bad_stride).is_err());

        let bad_len = PageRenderedResponse {
            bytes: vec![0; 1],
            ..good.clone()
        };
        assert!(limits.validate_rendered_page(&bad_len).is_err());

        let bad_pf = PageRenderedResponse {
            pixel_format: 9,
            ..good.clone()
        };
        assert!(limits.validate_rendered_page(&bad_pf).is_err());

        // Unnormalized shared-memory pages are rejected by validation.
        let shm_page = PageRenderedResponse {
            bytes: Vec::new(),
            shm_len: Some(100 * 100 * 4),
            ..good.clone()
        };
        assert!(limits.validate_rendered_page(&shm_page).is_err());
    }

    #[test]
    fn hello_ok_shm_advertisement_validated() {
        let base = client_hello_ok();
        assert!(validate_hello_ok(&base).is_ok());

        let both = HelloOk {
            shm_name: Some("docbunker_pagebuf_1234_1.bin".into()),
            shm_capacity: Some(domain::MAX_PIXEL_BUFFER as u64),
            ..base.clone()
        };
        assert!(validate_hello_ok(&both).is_ok());

        // The shm name must be a plain digits-only file name: anything that
        // could escape the temp directory is rejected (ADR-009).
        for bad in [
            "../escape.bin",
            "..\\..\\windows\\win.ini",
            "docbunker_pagebuf_.._1.bin",
            "docbunker_pagebuf_1234_1.bin/../../etc/passwd",
            "C:\\Windows\\win.ini",
            "/etc/passwd",
            "docbunker_pagebuf_1234_1",
            "docbunker_pagebuf_1234_.bin",
            "docbunker_pagebuf__1.bin",
            "docbunker_pagebuf_1234_1_5.bin",
            "docbunker_pagebuf_1234_1.bin.exe",
        ] {
            let case = HelloOk {
                shm_name: Some(bad.into()),
                shm_capacity: Some(domain::MAX_PIXEL_BUFFER as u64),
                ..base.clone()
            };
            assert!(
                validate_hello_ok(&case).is_err(),
                "hostile shm name {bad:?} must be rejected"
            );
        }

        let mismatched = HelloOk {
            shm_name: Some("docbunker_pagebuf_0_1.bin".into()),
            shm_capacity: None,
            ..base.clone()
        };
        assert!(validate_hello_ok(&mismatched).is_err());

        let too_big = HelloOk {
            shm_name: Some("docbunker_pagebuf_0_1.bin".into()),
            shm_capacity: Some(domain::MAX_PIXEL_BUFFER as u64 + 1),
            ..base.clone()
        };
        assert!(validate_hello_ok(&too_big).is_err());

        let empty_name = HelloOk {
            shm_name: Some(String::new()),
            shm_capacity: Some(1),
            ..base.clone()
        };
        assert!(validate_hello_ok(&empty_name).is_err());

        let long_name = HelloOk {
            shm_name: Some("x".repeat(MAX_STRING_LENGTH + 1)),
            shm_capacity: Some(1),
            ..base.clone()
        };
        assert!(validate_hello_ok(&long_name).is_err());
    }

    #[test]
    fn hello_ok_document_size_validated() {
        let base = client_hello_ok();

        let zero = HelloOk {
            max_document_size: 0,
            ..base.clone()
        };
        assert!(validate_hello_ok(&zero).is_err());

        let oversized = HelloOk {
            max_document_size: domain::MAX_DOCUMENT_SIZE as u64 + 1,
            ..base.clone()
        };
        assert!(validate_hello_ok(&oversized).is_err());

        assert!(validate_hello_ok(&base).is_ok());
    }

    #[test]
    fn error_response_validated() {
        assert!(validate_error_response(&ErrorResponse {
            code: 1,
            message: "short".into(),
        })
        .is_ok());
        assert!(validate_error_response(&ErrorResponse {
            code: 0,
            message: "short".into(),
        })
        .is_err());
        assert!(validate_error_response(&ErrorResponse {
            code: 1,
            message: "x".repeat(MAX_STRING_LENGTH + 1),
        })
        .is_err());
    }
}
