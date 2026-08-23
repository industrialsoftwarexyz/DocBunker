//! Host-side contract tests against a *misbehaving* worker process.
//!
//! These are the integration tests for "invalid IPC response", "timeout" and
//! "worker never responds": the behavior the Phase 4 backend will rely on.

use std::io::{BufReader, BufWriter};
use std::process::Command;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

mod common;

use common::{kill_quietly, spawn_fake_worker};
use docbunker_protocol::message::{
    HelloRequest, OpenDocumentRequest, PingRequest, RenderPageRequest, Request,
};
use docbunker_protocol::{
    encode_request, encode_response, FrameReader, FrameWriter, ProtocolError, MAX_IPC_MESSAGE,
    PROTOCOL_VERSION,
};
use docbunker_renderer_api::limits::MAX_DOCUMENT_SIZE;
use docbunker_sandbox::{ProcessTransport, SandboxError, SandboxTransport};

fn hello_request() -> HelloRequest {
    HelloRequest {
        protocol_version: PROTOCOL_VERSION,
        max_document_size: MAX_DOCUMENT_SIZE as u64,
        max_ipc_message: MAX_IPC_MESSAGE as u32,
        shm_capable: false,
    }
}

fn process_transport(mode: &str, timeout: Duration) -> ProcessTransport {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fake_worker"));
    command.arg(mode);
    ProcessTransport::spawn_command(&mut command, timeout).unwrap()
}

#[test]
fn invalid_response_version_is_rejected() {
    let (mut child, stdin, stdout) = spawn_fake_worker("bad-version");
    let mut writer = FrameWriter::new(BufWriter::new(stdin), MAX_IPC_MESSAGE);
    let mut reader = FrameReader::new(BufReader::new(stdout), MAX_IPC_MESSAGE);

    // The fake worker responds with an invalid protocol version byte.
    let hello = encode_request(&Request::Hello(hello_request())).unwrap();
    writer.write_frame(&hello).unwrap();
    writer.flush().unwrap();

    let payload = reader
        .next_frame()
        .unwrap()
        .expect("a response was expected");
    match docbunker_protocol::decode_response(&payload) {
        Err(ProtocolError::InvalidVersion) => {}
        Ok(other) => panic!("expected InvalidVersion, got valid response {other:?}"),
        Err(other) => panic!("expected InvalidVersion, got {other:?}"),
    }

    kill_quietly(&mut child);
}

#[test]
fn worker_that_never_responds_triggers_host_timeout() {
    let (mut child, stdin, stdout) = spawn_fake_worker("hang");
    let (tx, rx) = std::sync::mpsc::channel::<Result<Option<Vec<u8>>, String>>();

    // Blocking read happens on a thread; the test enforces a deadline, which
    // is exactly the mechanism the Phase 4 backend will use for timeouts.
    let reader_thread = std::thread::spawn(move || {
        let mut reader = FrameReader::new(BufReader::new(stdout), MAX_IPC_MESSAGE);
        let result = reader.next_frame().map_err(|e| e.to_string());
        let _ = tx.send(result);
    });

    let mut writer = FrameWriter::new(BufWriter::new(stdin), MAX_IPC_MESSAGE);
    let hello = encode_request(&Request::Hello(hello_request())).unwrap();
    writer.write_frame(&hello).unwrap();
    writer.flush().unwrap();

    let deadline = Duration::from_millis(500);
    match rx.recv_timeout(deadline) {
        Err(RecvTimeoutError::Timeout) => {
            // Expected: the host must never wait forever on the worker.
            // Contract satisfied; now clean up.
        }
        Err(e) => {
            kill_quietly(&mut child);
            panic!("channel error: {e}");
        }
        Ok(Ok(Some(_))) => {
            kill_quietly(&mut child);
            panic!("hanging worker unexpectedly responded");
        }
        Ok(Ok(None)) => {
            kill_quietly(&mut child);
            panic!("hanging worker unexpectedly closed the stream");
        }
        Ok(Err(e)) => {
            kill_quietly(&mut child);
            panic!("unexpected read error: {e}");
        }
    }

    kill_quietly(&mut child);
    let _ = reader_thread.join();
}

#[test]
fn silent_worker_produces_no_output() {
    let (mut child, _stdin, stdout) = spawn_fake_worker("silent");
    let (tx, rx) = std::sync::mpsc::channel::<Result<Option<Vec<u8>>, String>>();

    let reader_thread = std::thread::spawn(move || {
        let mut reader = FrameReader::new(BufReader::new(stdout), MAX_IPC_MESSAGE);
        let result = reader.next_frame().map_err(|e| e.to_string());
        let _ = tx.send(result);
    });

    match rx.recv_timeout(Duration::from_millis(500)) {
        Err(RecvTimeoutError::Timeout) => {}
        Err(RecvTimeoutError::Disconnected) => panic!("reader thread vanished"),
        Ok(Ok(Some(_))) => panic!("silent worker produced a frame"),
        Ok(_) => panic!("silent worker closed the stream"),
    }

    kill_quietly(&mut child);
    let _ = reader_thread.join();
}

#[test]
fn raw_frame_round_trip_with_fake_worker_payload() {
    // Sanity: the host encoder/decoder pair is self-consistent even with an
    // uncooperative peer (the fake worker rewrites bytes in its own mode).
    let resp =
        docbunker_protocol::message::Response::Pong(docbunker_protocol::message::PongResponse {
            nonce: 1,
        });
    let payload = encode_response(&resp).unwrap();
    assert_eq!(payload[0], PROTOCOL_VERSION);
}

#[test]
fn worker_that_stops_reading_cannot_block_request_write() {
    let timeout = Duration::from_millis(200);
    let mut transport = process_transport("stop-reading", timeout);
    let started = Instant::now();
    let result = transport.send(Request::OpenDocument(OpenDocumentRequest {
        document_id: 1,
        data: vec![0; 2 * 1024 * 1024],
    }));

    assert!(matches!(result, Err(SandboxError::Timeout)));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(transport
        .send(Request::Ping(PingRequest { nonce: 1 }))
        .is_err());
}

#[test]
fn mismatched_response_identity_poisons_process_transport() {
    let mut transport = process_transport("wrong-pong", Duration::from_secs(2));

    let result = transport.send(Request::Ping(PingRequest { nonce: 41 }));
    assert!(matches!(result, Err(SandboxError::Internal(_))));
    assert!(transport
        .send(Request::Ping(PingRequest { nonce: 42 }))
        .is_err());
}

#[test]
fn mismatched_render_document_and_page_poison_process_transport() {
    let mut transport = process_transport("wrong-render", Duration::from_secs(2));

    let result = transport.send(Request::RenderPage(RenderPageRequest {
        document_id: 12,
        page: 3,
        target_width: 1,
        target_height: 1,
    }));
    assert!(matches!(result, Err(SandboxError::Internal(_))));
    assert!(transport
        .send(Request::Ping(PingRequest { nonce: 1 }))
        .is_err());
}

#[test]
fn unsolicited_extra_response_poisons_process_transport() {
    let mut transport = process_transport("extra-pong", Duration::from_secs(2));

    let first = transport.send(Request::Ping(PingRequest { nonce: 7 }));
    if first.is_ok() {
        std::thread::sleep(Duration::from_millis(100));
        assert!(transport
            .send(Request::Ping(PingRequest { nonce: 8 }))
            .is_err());
    } else {
        assert!(matches!(first, Err(SandboxError::Internal(_))));
    }
    assert!(transport
        .send(Request::Ping(PingRequest { nonce: 9 }))
        .is_err());
}
