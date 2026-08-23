//! Integration tests that spawn the real `renderer-worker` binary and talk to
//! it over pipes, exercising the actual frame protocol.
//!
//! These tests define the host<->worker contract that the Phase 4 `runsc`
//! backend will rely on:
//!
//! - clean EOF after `CloseDocument` + stdin close -> exit 0
//! - undecodable frame -> `Error(ProtocolViolation)` response, then continue
//! - oversized frame header -> worker exits non-zero (stream unusable)
//! - killed worker -> read side EOF, non-zero exit (host maps to SandboxCrashed)

use std::io::{BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use docbunker_protocol::message::{
    CloseDocumentRequest, ErrorCode, ErrorResponse, GetDocumentInfoRequest, HelloRequest,
    OpenDocumentRequest, PingRequest, RenderPageRequest, Request, Response,
};
use docbunker_protocol::validate::NegotiatedLimits;
use docbunker_protocol::{
    decode_request, decode_response, encode_request, encode_response, FrameReader, FrameWriter,
    MAX_IPC_MESSAGE, PROTOCOL_VERSION,
};
use docbunker_renderer_api::limits::MAX_DOCUMENT_SIZE;

struct WorkerIo {
    reader: FrameReader<BufReader<ChildStdout>>,
    writer: FrameWriter<BufWriter<ChildStdin>>,
}

impl WorkerIo {
    fn send(&mut self, request: &Request) -> Response {
        let payload = encode_request(request).unwrap();
        self.writer.write_frame(&payload).unwrap();
        self.writer.flush().unwrap();
        let frame = self.reader.next_frame().unwrap().expect("frame expected");
        decode_response(&frame).unwrap()
    }
}

fn spawn_worker() -> (Child, WorkerIo) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_renderer-worker"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn renderer-worker");

    let stdin = child.stdin.take().expect("worker stdin");
    let stdout = child.stdout.take().expect("worker stdout");
    let io = WorkerIo {
        reader: FrameReader::new(BufReader::new(stdout), MAX_IPC_MESSAGE),
        writer: FrameWriter::new(BufWriter::new(stdin), MAX_IPC_MESSAGE),
    };
    (child, io)
}

fn handshake(io: &mut WorkerIo) -> NegotiatedLimits {
    let hello = HelloRequest {
        protocol_version: PROTOCOL_VERSION,
        max_document_size: MAX_DOCUMENT_SIZE as u64,
        max_ipc_message: MAX_IPC_MESSAGE as u32,
        shm_capable: false,
    };
    match io.send(&Request::Hello(hello)) {
        Response::HelloOk(ok) => NegotiatedLimits::client_negotiate(&ok).unwrap(),
        other => panic!("expected HelloOk, got {other:?}"),
    }
}

fn png_fixture() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 3, image::Rgba([200, 30, 40, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("fixture encodes");
    out.into_inner()
}

/// A minimal docx container with two paragraphs.
fn docx_fixture() -> Vec<u8> {
    use std::io::Write;
    let mut buffer = Vec::new();
    {
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        archive
            .start_file("[Content_Types].xml", options)
            .expect("content types");
        archive
            .write_all(
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#,
            )
            .expect("content types body");
        archive
            .start_file("word/document.xml", options)
            .expect("document");
        archive
            .write_all(
                br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>
  <w:p><w:r><w:t>Hola DocBunker</w:t></w:r></w:p>
  <w:p><w:r><w:t>Segundo parrafo</w:t></w:r></w:p>
</w:body>
</w:document>"#,
            )
            .expect("document body");
        archive.finish().expect("archive");
    }
    buffer
}

#[test]
fn full_round_trip_over_process_pipes() {
    let (mut child, mut io) = spawn_worker();
    let limits = handshake(&mut io);

    // Open a real PNG; the worker decodes it.
    let opened = io.send(&Request::OpenDocument(OpenDocumentRequest {
        document_id: 1,
        data: png_fixture(),
    }));
    match opened {
        Response::DocumentOpened(resp) => {
            assert_eq!(resp.document_id, 1);
            assert_eq!(resp.info.page_count, 1);
            assert_eq!((resp.info.width, resp.info.height), (4, 3));
        }
        other => panic!("expected DocumentOpened, got {other:?}"),
    }

    // Info
    match io.send(&Request::GetDocumentInfo(GetDocumentInfoRequest {
        document_id: 1,
    })) {
        Response::DocumentInfo(resp) => assert_eq!(resp.info.format, 2), // PNG
        other => panic!("expected DocumentInfo, got {other:?}"),
    }

    // Render within negotiated limits
    match io.send(&Request::RenderPage(RenderPageRequest {
        document_id: 1,
        page: 0,
        target_width: 100,
        target_height: 120,
    })) {
        Response::PageRendered(resp) => {
            assert_eq!(resp.width, 100);
            assert_eq!(resp.height, 120);
            assert_eq!(resp.bytes.len(), 100 * 120 * 4);
            let page = limits.validate_rendered_page(&resp).unwrap();
            assert_eq!(page.width, 100);
        }
        other => panic!("expected PageRendered, got {other:?}"),
    }

    // Render target beyond negotiated limits must be rejected by the worker.
    match io.send(&Request::RenderPage(RenderPageRequest {
        document_id: 1,
        page: 0,
        target_width: 5000,
        target_height: 5000,
    })) {
        Response::Error(e) => assert_eq!(e.code, ErrorCode::ResourceLimitExceeded.as_u8()),
        other => panic!("expected Error, got {other:?}"),
    }

    // Ping
    match io.send(&Request::Ping(PingRequest { nonce: 42 })) {
        Response::Pong(p) => assert_eq!(p.nonce, 42),
        other => panic!("expected Pong, got {other:?}"),
    }

    // Close + clean EOF
    match io.send(&Request::CloseDocument(CloseDocumentRequest {
        document_id: 1,
    })) {
        Response::Closed(_) => {}
        other => panic!("expected Closed, got {other:?}"),
    }
    drop(io.writer);
    assert!(io.reader.next_frame().unwrap().is_none());

    let status = child.wait().unwrap();
    assert!(status.success(), "worker must exit 0 on clean EOF");
}

#[test]
fn truncated_image_is_rejected() {
    let (mut child, mut io) = spawn_worker();
    handshake(&mut io);

    let mut data = png_fixture();
    data.truncate(data.len() / 2);
    let resp = io.send(&Request::OpenDocument(OpenDocumentRequest {
        document_id: 1,
        data,
    }));
    match resp {
        Response::Error(e) => {
            assert!(
                e.code == ErrorCode::InvalidDocument.as_u8()
                    || e.code == ErrorCode::RenderingFailed.as_u8(),
                "unexpected code {}",
                e.code
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }

    drop(io.writer);
    assert!(io.reader.next_frame().unwrap().is_none());
    assert!(child.wait().unwrap().success());
}

#[test]
fn ooxml_round_trip_over_process_pipes() {
    let (mut child, mut io) = spawn_worker();
    let limits = handshake(&mut io);

    let opened = io.send(&Request::OpenDocument(OpenDocumentRequest {
        document_id: 1,
        data: docx_fixture(),
    }));
    match opened {
        Response::DocumentOpened(resp) => {
            assert_eq!(resp.document_id, 1);
            assert_eq!(resp.info.page_count, 1);
            assert_eq!(resp.info.format, 5); // Ooxml
        }
        other => panic!("expected DocumentOpened, got {other:?}"),
    }

    match io.send(&Request::RenderPage(RenderPageRequest {
        document_id: 1,
        page: 0,
        target_width: 620,
        target_height: 877,
    })) {
        Response::PageRendered(resp) => {
            assert_eq!(resp.width, 620);
            assert_eq!(resp.height, 877);
            assert_eq!(resp.bytes.len(), 620 * 877 * 4);
            limits.validate_rendered_page(&resp).unwrap();
        }
        other => panic!("expected PageRendered, got {other:?}"),
    }

    match io.send(&Request::CloseDocument(CloseDocumentRequest {
        document_id: 1,
    })) {
        Response::Closed(_) => {}
        other => panic!("expected Closed, got {other:?}"),
    }
    drop(io.writer);
    assert!(io.reader.next_frame().unwrap().is_none());
    assert!(child.wait().unwrap().success());
}

#[test]
fn shared_memory_page_round_trip_over_process_pipes() {
    let (mut child, mut io) = spawn_worker();

    // Ask for shared-memory page buffers.
    let hello = HelloRequest {
        protocol_version: PROTOCOL_VERSION,
        max_document_size: MAX_DOCUMENT_SIZE as u64,
        max_ipc_message: MAX_IPC_MESSAGE as u32,
        shm_capable: true,
    };
    let shm_region = match io.send(&Request::Hello(hello)) {
        Response::HelloOk(ok) => {
            assert!(
                ok.shm_name.is_some() && ok.shm_capacity.is_some(),
                "worker must advertise a shm region"
            );
            let capacity = ok.shm_capacity.unwrap();
            assert!(capacity <= docbunker_renderer_api::limits::MAX_PIXEL_BUFFER as u64);
            ok.shm_name.unwrap()
        }
        other => panic!("expected HelloOk, got {other:?}"),
    };

    let opened = io.send(&Request::OpenDocument(OpenDocumentRequest {
        document_id: 1,
        data: png_fixture(),
    }));
    assert!(matches!(opened, Response::DocumentOpened(_)));

    let rendered = io.send(&Request::RenderPage(RenderPageRequest {
        document_id: 1,
        page: 0,
        target_width: 100,
        target_height: 120,
    }));
    let (shm_len, _bytes) = match rendered {
        Response::PageRendered(resp) => {
            assert!(resp.shm_len.is_some(), "page must ride the shm region");
            assert!(
                resp.bytes.is_empty(),
                "shm pages must not carry in-frame bytes"
            );
            (resp.shm_len.unwrap(), resp.bytes)
        }
        other => panic!("expected PageRendered, got {other:?}"),
    };
    assert_eq!(shm_len as usize, 100 * 120 * 4);

    // The file backing the region must be readable and hold the raster.
    let path = std::env::temp_dir().join(&shm_region);
    let content = std::fs::read(&path).expect("shm region file exists");
    assert!(content.len() >= shm_len as usize);
    assert!(
        content[..shm_len as usize].iter().any(|b| *b != 0),
        "shm region must contain the rendered page"
    );

    match io.send(&Request::CloseDocument(CloseDocumentRequest {
        document_id: 1,
    })) {
        Response::Closed(_) => {}
        other => panic!("expected Closed, got {other:?}"),
    }
    drop(io.writer);
    assert!(io.reader.next_frame().unwrap().is_none());
    assert!(child.wait().unwrap().success());
}

#[test]
fn undecodable_frame_gets_error_then_continues() {
    let (mut child, mut io) = spawn_worker();
    handshake(&mut io);

    // Write a payload with an unknown discriminator directly.
    io.writer
        .write_frame(&[PROTOCOL_VERSION, 0x7F, 0x00])
        .unwrap();
    io.writer.flush().unwrap();
    let frame = io.reader.next_frame().unwrap().expect("error frame");
    match decode_response(&frame).unwrap() {
        Response::Error(e) => assert_eq!(e.code, ErrorCode::ProtocolViolation.as_u8()),
        other => panic!("expected protocol violation error, got {other:?}"),
    }

    // The worker must still be alive and functional.
    match io.send(&Request::Ping(PingRequest { nonce: 7 })) {
        Response::Pong(p) => assert_eq!(p.nonce, 7),
        other => panic!("expected Pong after protocol error, got {other:?}"),
    }

    drop(io.writer);
    assert!(io.reader.next_frame().unwrap().is_none());
    assert!(child.wait().unwrap().success());
}

#[test]
fn oversized_frame_aborts_worker() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_renderer-worker"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn renderer-worker");
    let mut stdin = child.stdin.take().unwrap();

    // A header claiming more than MAX_IPC_MESSAGE bytes must be rejected.
    stdin
        .write_all(&(MAX_IPC_MESSAGE as u32 + 1).to_le_bytes())
        .unwrap();
    stdin.flush().unwrap();
    drop(stdin);

    // Worker must exit non-zero (stream unusable).
    let status = child.wait().unwrap();
    assert!(
        !status.success(),
        "worker must exit non-zero on frame violation"
    );
}

#[test]
fn killed_worker_surfaces_eof_and_nonzero_exit() {
    let (mut child, mut io) = spawn_worker();
    handshake(&mut io);

    // Kill the worker mid-session. The host (Phase 4) will observe EOF or a
    // read error and map that to SandboxCrashed.
    child.kill().unwrap();
    let read_result = io.reader.next_frame();
    match read_result {
        Ok(None) => {}
        Ok(Some(_)) => panic!("no frame expected after kill"),
        Err(e) => {
            let kind = match &e {
                docbunker_protocol::ProtocolError::Io(ioe) => ioe.kind(),
                docbunker_protocol::ProtocolError::TruncatedFrame => {
                    std::io::ErrorKind::UnexpectedEof
                }
                other => panic!("unexpected error kind: {other:?}"),
            };
            assert!(matches!(
                kind,
                std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::BrokenPipe
            ));
        }
    }

    let status = child.wait().unwrap();
    assert!(!status.success(), "killed worker must not exit 0");

    drop(io.writer);
    drop(io.reader);
}

#[test]
fn invalid_document_rejected() {
    let (mut child, mut io) = spawn_worker();
    handshake(&mut io);

    let resp = io.send(&Request::OpenDocument(OpenDocumentRequest {
        document_id: 1,
        data: b"garbage that is not a recognized format".to_vec(),
    }));
    match resp {
        Response::Error(e) => assert_eq!(e.code, ErrorCode::UnsupportedFormat.as_u8()),
        other => panic!("expected UnsupportedFormat error, got {other:?}"),
    }

    drop(io.writer);
    assert!(io.reader.next_frame().unwrap().is_none());
    assert!(child.wait().unwrap().success());
}

#[test]
fn oversized_document_rejected() {
    let (mut child, mut io) = spawn_worker();
    handshake(&mut io);

    let resp = io.send(&Request::OpenDocument(OpenDocumentRequest {
        document_id: 1,
        data: vec![0; MAX_DOCUMENT_SIZE + 1],
    }));
    match resp {
        Response::Error(e) => assert_eq!(e.code, ErrorCode::DocumentTooLarge.as_u8()),
        other => panic!("expected DocumentTooLarge error, got {other:?}"),
    }

    drop(io.writer);
    assert!(io.reader.next_frame().unwrap().is_none());
    assert!(child.wait().unwrap().success());
}

/// Placeholder asserted here so the file documents the intent: JSON is never
/// used to transport RGBA. The check itself is a compile-time guarantee (the
/// protocol crate has no JSON for raster data), this just keeps the intent
/// explicit in tests.
#[test]
fn raw_response_encoding_is_binary_only() {
    let resp = Response::Error(ErrorResponse {
        code: ErrorCode::Internal.as_u8(),
        message: "x".into(),
    });
    let payload = encode_response(&resp).unwrap();
    assert_eq!(payload[0], PROTOCOL_VERSION);
    assert_eq!(payload[1], 0x86); // ERROR discriminator
    let _ = decode_request(&payload);
}
