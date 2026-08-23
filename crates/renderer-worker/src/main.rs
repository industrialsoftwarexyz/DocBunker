//! `renderer-worker` binary entry point.
//!
//! Runs the worker protocol loop over stdin/stdout (length-prefixed frames).
//! This is the process that will run inside the sandbox (Phase 4).
//!
//! Exit codes: `0` = clean EOF at a frame boundary, `1` = protocol or I/O
//! failure. The host treats any unclean exit as `SandboxCrashed`.

use std::io::{BufReader, BufWriter, Read, Write};

use docbunker_protocol::message::{ErrorCode, ErrorResponse, Response};
use docbunker_protocol::{
    decode_request, encode_response, FrameReader, FrameWriter, MAX_IPC_MESSAGE,
};
use docbunker_renderer_worker::{
    SharedMemStore, SharedStoreFactory, StandardRendererFactory, Worker,
};

fn shm_factory() -> Option<SharedStoreFactory> {
    Some(Box::new(|| {
        SharedMemStore::try_create()
            .map(|store| Box::new(store) as Box<dyn docbunker_renderer_worker::PageBufferStore>)
    }))
}

fn send_error<W: Write>(
    writer: &mut FrameWriter<W>,
    code: ErrorCode,
    message: &str,
) -> Result<(), docbunker_protocol::ProtocolError> {
    let resp = Response::Error(ErrorResponse {
        code: code.as_u8(),
        message: message
            .chars()
            .take(docbunker_protocol::MAX_STRING_LENGTH)
            .collect(),
    });
    let payload = encode_response(&resp)?;
    writer.write_frame(&payload)?;
    writer.flush()
}

fn run<R: Read, W: Write>(
    worker: &mut Worker<StandardRendererFactory>,
    reader: &mut FrameReader<R>,
    writer: &mut FrameWriter<W>,
) -> Result<(), docbunker_protocol::ProtocolError> {
    loop {
        let payload = match reader.next_frame() {
            Ok(Some(payload)) => payload,
            Ok(None) => return Ok(()),
            Err(e) => return Err(e),
        };

        let req = match decode_request(&payload) {
            Ok(req) => req,
            Err(e) => {
                tracing::warn!(%e, "cannot decode request");
                send_error(
                    writer,
                    ErrorCode::ProtocolViolation,
                    "cannot decode request",
                )?;
                continue;
            }
        };

        let resp = worker.handle(req);
        let payload = encode_response(&resp)?;
        writer.write_frame(&payload)?;
        writer.flush()?;
    }
}

fn main() {
    // Logs go to stderr; stdout carries ONLY protocol frames.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();

    let mut worker = Worker::with_shm_factory(StandardRendererFactory, shm_factory());
    let mut reader = FrameReader::new(BufReader::new(stdin.lock()), MAX_IPC_MESSAGE);
    let mut writer = FrameWriter::new(BufWriter::new(stdout.lock()), MAX_IPC_MESSAGE);

    match run(&mut worker, &mut reader, &mut writer) {
        Ok(()) => {
            tracing::info!("clean EOF, exiting");
            std::process::exit(0);
        }
        Err(e) => {
            tracing::error!(%e, "renderer-worker failed");
            std::process::exit(1);
        }
    }
}
