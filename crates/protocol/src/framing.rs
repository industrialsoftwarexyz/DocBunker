//! Frame-level encoding and decoding over any `Read`/`Write` pair.
//!
//! Wire format:
//!
//! ```text
//! frame  := [payload_len: u32 LE] [payload]
//! payload := [version: u8] [discriminator: u8] [postcard body]
//! ```
//!
//! `encode_request`/`encode_response` produce the *payload* only; the length
//! prefix is added by [`FrameWriter`]. `decode_request`/`decode_response`
//! consume the *payload* produced by [`FrameReader`].
//!
//! The transport is replaceable (`SandboxTransport`); `FrameReader` /
//! `FrameWriter` work over Unix sockets, pipes or future virtio channels alike.

use std::io::{Read, Write};

use crate::error::ProtocolError;
use crate::limits::{FRAME_HEADER_SIZE, PROTOCOL_VERSION};
use crate::message::{Request, Response};

pub const DISCRIMINATOR_REQUEST_HELLO: u8 = 0x00;
pub const DISCRIMINATOR_REQUEST_OPEN_DOCUMENT: u8 = 0x01;
pub const DISCRIMINATOR_REQUEST_GET_DOCUMENT_INFO: u8 = 0x02;
pub const DISCRIMINATOR_REQUEST_RENDER_PAGE: u8 = 0x03;
pub const DISCRIMINATOR_REQUEST_CLOSE_DOCUMENT: u8 = 0x04;
pub const DISCRIMINATOR_REQUEST_PING: u8 = 0x05;

pub const DISCRIMINATOR_RESPONSE_HELLO_OK: u8 = 0x80;
pub const DISCRIMINATOR_RESPONSE_DOCUMENT_OPENED: u8 = 0x81;
pub const DISCRIMINATOR_RESPONSE_DOCUMENT_INFO: u8 = 0x82;
pub const DISCRIMINATOR_RESPONSE_PAGE_RENDERED: u8 = 0x83;
pub const DISCRIMINATOR_RESPONSE_CLOSED: u8 = 0x84;
pub const DISCRIMINATOR_RESPONSE_PONG: u8 = 0x85;
pub const DISCRIMINATOR_RESPONSE_ERROR: u8 = 0x86;

fn request_discriminator(req: &Request) -> u8 {
    match req {
        Request::Hello(_) => DISCRIMINATOR_REQUEST_HELLO,
        Request::OpenDocument(_) => DISCRIMINATOR_REQUEST_OPEN_DOCUMENT,
        Request::GetDocumentInfo(_) => DISCRIMINATOR_REQUEST_GET_DOCUMENT_INFO,
        Request::RenderPage(_) => DISCRIMINATOR_REQUEST_RENDER_PAGE,
        Request::CloseDocument(_) => DISCRIMINATOR_REQUEST_CLOSE_DOCUMENT,
        Request::Ping(_) => DISCRIMINATOR_REQUEST_PING,
    }
}

fn response_discriminator(resp: &Response) -> u8 {
    match resp {
        Response::HelloOk(_) => DISCRIMINATOR_RESPONSE_HELLO_OK,
        Response::DocumentOpened(_) => DISCRIMINATOR_RESPONSE_DOCUMENT_OPENED,
        Response::DocumentInfo(_) => DISCRIMINATOR_RESPONSE_DOCUMENT_INFO,
        Response::PageRendered(_) => DISCRIMINATOR_RESPONSE_PAGE_RENDERED,
        Response::Closed(_) => DISCRIMINATOR_RESPONSE_CLOSED,
        Response::Pong(_) => DISCRIMINATOR_RESPONSE_PONG,
        Response::Error(_) => DISCRIMINATOR_RESPONSE_ERROR,
    }
}

fn split_payload(payload: &[u8]) -> Result<(u8, &[u8]), ProtocolError> {
    if payload.len() < 2 {
        return Err(ProtocolError::FrameTooSmall);
    }
    if payload[0] != PROTOCOL_VERSION {
        return Err(ProtocolError::InvalidVersion);
    }
    Ok((payload[1], &payload[2..]))
}

fn encode_payload(
    discriminator: u8,
    body: &impl serde::Serialize,
) -> Result<Vec<u8>, ProtocolError> {
    let body = postcard::to_allocvec(body)?;
    let mut payload = Vec::with_capacity(2 + body.len());
    payload.push(PROTOCOL_VERSION);
    payload.push(discriminator);
    payload.extend_from_slice(&body);
    Ok(payload)
}

/// Encode a `Request` payload (version + discriminator + body).
pub fn encode_request(req: &Request) -> Result<Vec<u8>, ProtocolError> {
    encode_payload(request_discriminator(req), req)
}

/// Encode a `Response` payload (version + discriminator + body).
pub fn encode_response(resp: &Response) -> Result<Vec<u8>, ProtocolError> {
    encode_payload(response_discriminator(resp), resp)
}

/// Decode a `Request` from a payload slice (version + discriminator + body).
///
/// The discriminator byte must match the decoded enum variant (defense in
/// depth: two independent encodings of the message type).
pub fn decode_request(payload: &[u8]) -> Result<Request, ProtocolError> {
    let (discriminator, body) = split_payload(payload)?;
    let request: Request = postcard::from_bytes(body)?;
    if discriminator != request_discriminator(&request) {
        return Err(ProtocolError::InvalidDiscriminator);
    }
    Ok(request)
}

/// Decode a `Response` from a payload slice.
pub fn decode_response(payload: &[u8]) -> Result<Response, ProtocolError> {
    let (discriminator, body) = split_payload(payload)?;
    let response: Response = postcard::from_bytes(body)?;
    if discriminator != response_discriminator(&response) {
        return Err(ProtocolError::InvalidDiscriminator);
    }
    Ok(response)
}

/// Reads length-prefixed frames from a `Read` stream, enforcing a size cap.
pub struct FrameReader<R: Read> {
    inner: R,
    max_frame: usize,
}

impl<R: Read> FrameReader<R> {
    pub fn new(inner: R, max_frame: usize) -> Self {
        Self { inner, max_frame }
    }

    pub fn max_frame(&self) -> usize {
        self.max_frame
    }

    /// Read the next frame payload (version + discriminator + body).
    ///
    /// Returns `None` on clean EOF at a frame boundary. Any other EOF or
    /// size violation is an error and the stream should be considered unusable.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, ProtocolError> {
        let mut header = [0u8; FRAME_HEADER_SIZE];
        let mut read = 0;
        while read < FRAME_HEADER_SIZE {
            match self.inner.read(&mut header[read..]) {
                Ok(0) => {
                    if read == 0 {
                        return Ok(None);
                    }
                    return Err(ProtocolError::TruncatedFrame);
                }
                Ok(n) => read += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(ProtocolError::Io(e)),
            }
        }

        let len = u32::from_le_bytes(header) as usize;
        if len < 2 {
            return Err(ProtocolError::FrameTooSmall);
        }
        if len > self.max_frame {
            return Err(ProtocolError::FrameTooLarge);
        }

        let mut payload = vec![0u8; len];
        self.inner.read_exact(&mut payload).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                ProtocolError::TruncatedFrame
            } else {
                ProtocolError::Io(e)
            }
        })?;
        Ok(Some(payload))
    }
}

/// Writes length-prefixed frames to a `Write` stream.
pub struct FrameWriter<W: Write> {
    inner: W,
    max_frame: usize,
}

impl<W: Write> FrameWriter<W> {
    pub fn new(inner: W, max_frame: usize) -> Self {
        Self { inner, max_frame }
    }

    /// Write one frame (`[len][payload]`), enforcing `payload + header <= max_frame`.
    pub fn write_frame(&mut self, payload: &[u8]) -> Result<(), ProtocolError> {
        if payload.len() > self.max_frame.saturating_sub(FRAME_HEADER_SIZE) {
            return Err(ProtocolError::FrameTooLarge);
        }
        let len = u32::try_from(payload.len()).expect("payload bounded by max_frame");
        let mut header = [0u8; FRAME_HEADER_SIZE];
        header.copy_from_slice(&len.to_le_bytes());
        self.inner.write_all(&header)?;
        self.inner.write_all(payload)?;
        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), ProtocolError> {
        self.inner.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::{MAX_IPC_MESSAGE, MAX_STRING_LENGTH, PROTOCOL_VERSION};
    use crate::message::*;
    use std::io::Cursor;

    fn sample_request() -> Request {
        Request::RenderPage(RenderPageRequest {
            document_id: 7,
            page: 2,
            target_width: 100,
            target_height: 200,
        })
    }

    fn sample_response() -> Response {
        Response::PageRendered(PageRenderedResponse {
            document_id: 7,
            page: 2,
            width: 100,
            height: 200,
            stride: 400,
            pixel_format: 1,
            bytes: vec![0xAB; 100 * 200 * 4],
            shm_len: None,
        })
    }

    /// Write `req` through a `FrameWriter` and read it back through a
    /// `FrameReader`, returning the decoded value.
    fn round_trip_request(req: &Request) -> Request {
        let payload = encode_request(req).unwrap();
        let mut stream = Vec::new();
        {
            let mut writer = FrameWriter::new(&mut stream, MAX_IPC_MESSAGE);
            writer.write_frame(&payload).unwrap();
            writer.flush().unwrap();
        }
        let mut reader = FrameReader::new(Cursor::new(stream), MAX_IPC_MESSAGE);
        let frame = reader.next_frame().unwrap().expect("one frame");
        assert!(reader.next_frame().unwrap().is_none());
        decode_request(&frame).unwrap()
    }

    #[test]
    fn request_round_trips() {
        assert_eq!(round_trip_request(&sample_request()), sample_request());
    }

    #[test]
    fn response_round_trips() {
        let payload = encode_response(&sample_response()).unwrap();
        let mut stream = Vec::new();
        {
            let mut writer = FrameWriter::new(&mut stream, MAX_IPC_MESSAGE);
            writer.write_frame(&payload).unwrap();
        }
        let mut reader = FrameReader::new(Cursor::new(stream), MAX_IPC_MESSAGE);
        let frame = reader.next_frame().unwrap().unwrap();
        assert_eq!(decode_response(&frame).unwrap(), sample_response());
    }

    #[test]
    fn all_request_variants_round_trip() {
        let requests = vec![
            Request::Hello(HelloRequest {
                protocol_version: PROTOCOL_VERSION,
                max_document_size: 1024,
                max_ipc_message: 4096,
                shm_capable: true,
            }),
            Request::OpenDocument(OpenDocumentRequest {
                document_id: 1,
                data: vec![1, 2, 3],
            }),
            Request::GetDocumentInfo(GetDocumentInfoRequest { document_id: 1 }),
            Request::RenderPage(RenderPageRequest {
                document_id: 1,
                page: 0,
                target_width: 10,
                target_height: 10,
            }),
            Request::CloseDocument(CloseDocumentRequest { document_id: 1 }),
            Request::Ping(PingRequest { nonce: 42 }),
        ];
        for req in requests {
            assert_eq!(round_trip_request(&req), req);
        }
    }

    #[test]
    fn rejects_bad_version() {
        let mut payload =
            encode_payload(DISCRIMINATOR_REQUEST_PING, &PingRequest { nonce: 1 }).unwrap();
        payload[0] = PROTOCOL_VERSION + 1;
        assert!(matches!(
            decode_request(&payload),
            Err(ProtocolError::InvalidVersion)
        ));
    }

    #[test]
    fn rejects_unknown_discriminator() {
        // A valid Ping message with a wrong discriminator byte must be rejected.
        let mut payload = encode_request(&Request::Ping(PingRequest { nonce: 1 })).unwrap();
        payload[1] = 0x7F;
        assert!(matches!(
            decode_request(&payload),
            Err(ProtocolError::InvalidDiscriminator)
        ));

        // A body that cannot decode as any known request is also rejected.
        let payload = [PROTOCOL_VERSION, 0x7F, 0x00];
        assert!(decode_request(&payload).is_err());
    }

    #[test]
    fn rejects_trailing_bytes() {
        // postcard is strict: appending junk after a valid message must fail.
        let mut payload =
            encode_payload(DISCRIMINATOR_REQUEST_PING, &PingRequest { nonce: 1 }).unwrap();
        payload.push(0xFF);
        assert!(decode_request(&payload).is_err());
    }

    #[test]
    fn writer_rejects_oversized_payload() {
        let payload = vec![0u8; MAX_IPC_MESSAGE]; // larger than max_frame - header
        let mut writer = FrameWriter::new(std::io::sink(), MAX_IPC_MESSAGE);
        assert!(matches!(
            writer.write_frame(&payload),
            Err(ProtocolError::FrameTooLarge)
        ));
    }

    #[test]
    fn reader_rejects_oversized_frame_header() {
        let mut header = vec![0u8; 4];
        header.copy_from_slice(&1_000_000_000u32.to_le_bytes());
        let mut reader = FrameReader::new(Cursor::new(header), 100);
        assert!(matches!(
            reader.next_frame(),
            Err(ProtocolError::FrameTooLarge)
        ));
    }

    #[test]
    fn reader_rejects_truncated_frame() {
        // Header claims 100 bytes, only 3 arrive.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&100u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3]);
        let mut reader = FrameReader::new(Cursor::new(bytes), 1024);
        assert!(matches!(
            reader.next_frame(),
            Err(ProtocolError::TruncatedFrame)
        ));
    }

    #[test]
    fn reader_handles_multiple_frames() {
        let payload = encode_request(&Request::Ping(PingRequest { nonce: 5 })).unwrap();
        let mut stream = Vec::new();
        {
            let mut writer = FrameWriter::new(&mut stream, MAX_IPC_MESSAGE);
            writer.write_frame(&payload).unwrap();
            writer.write_frame(&payload).unwrap();
        }
        let mut reader = FrameReader::new(Cursor::new(stream), MAX_IPC_MESSAGE);
        assert_eq!(reader.next_frame().unwrap().unwrap(), payload);
        assert_eq!(reader.next_frame().unwrap().unwrap(), payload);
        assert!(reader.next_frame().unwrap().is_none());
    }

    #[test]
    fn error_response_with_long_message_rejected_at_validate_layer() {
        let resp = Response::Error(ErrorResponse {
            code: ErrorCode::Internal.as_u8(),
            message: "x".repeat(MAX_STRING_LENGTH + 1),
        });
        // Encoding is allowed (bounded by frame cap); validation is the gate.
        let payload = encode_response(&resp).unwrap();
        let mut stream = Vec::new();
        {
            let mut writer = FrameWriter::new(&mut stream, MAX_IPC_MESSAGE);
            writer.write_frame(&payload).unwrap();
        }
        let mut reader = FrameReader::new(Cursor::new(stream), MAX_IPC_MESSAGE);
        let frame = reader.next_frame().unwrap().unwrap();
        match decode_response(&frame).unwrap() {
            Response::Error(e) => assert!(crate::validate::validate_error_response(&e).is_err()),
            _ => panic!("expected error response"),
        }
    }
}
