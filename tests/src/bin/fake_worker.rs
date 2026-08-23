//! A deliberately misbehaving worker, used by the integration tests to verify
//! that the host validates responses and enforces timeouts.
//!
//! Modes:
//!
//! - `silent`: never writes anything (host must time out waiting for HelloOk).
//! - `hang`: reads the first frame, then never responds (host must time out).
//! - `bad-version`: responds to the first frame with an invalid protocol
//!   version byte (host must reject the response).
//! - `stop-reading`: completes the handshake, then never reads again.
//! - `wrong-pong`: returns a Pong with the wrong nonce.
//! - `wrong-render`: returns a rendered page for the wrong document and page.
//! - `extra-pong`: returns two Pong responses for one Ping request.

use std::io::{BufReader, BufWriter, Read, Write};

use docbunker_protocol::message::{PageRenderedResponse, PongResponse, Request, Response};
use docbunker_protocol::validate::NegotiatedLimits;
use docbunker_protocol::{
    decode_request, encode_response, FrameReader, FrameWriter, MAX_IPC_MESSAGE,
};

fn sleep_forever() -> ! {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());

    match mode.as_str() {
        "silent" => {
            // Never write anything; hold the pipes open.
            let mut buf = [0u8; 4096];
            loop {
                if reader.read(&mut buf).is_err() {
                    break;
                }
            }
            std::process::exit(2);
        }
        "hang" => {
            // Read one frame, never respond, keep the pipe open.
            let mut buf = [0u8; 4096];
            let _ = reader.read(&mut buf);
            sleep_forever();
        }
        "bad-version" => {
            // Read one frame, then respond with an invalid version byte.
            let mut header = [0u8; 4];
            if reader.read_exact(&mut header).is_err() {
                std::process::exit(2);
            }
            let len = u32::from_le_bytes(header) as usize;
            let mut payload = vec![0u8; len];
            if reader.read_exact(&mut payload).is_err() {
                std::process::exit(2);
            }
            if !payload.is_empty() {
                payload[0] = 0xFF; // invalid protocol version
            }
            writer.write_all(&(payload.len() as u32).to_le_bytes()).ok();
            writer.write_all(&payload).ok();
            writer.flush().ok();
            sleep_forever();
        }
        "stop-reading" | "wrong-pong" | "wrong-render" | "extra-pong" => {
            drop(reader);
            drop(writer);
            run_protocol_mode(&mode);
        }
        _ => {
            eprintln!(
                "usage: fake_worker <silent|hang|bad-version|stop-reading|wrong-pong|wrong-render|extra-pong>"
            );
            std::process::exit(2);
        }
    }
}

fn run_protocol_mode(mode: &str) {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = FrameReader::new(BufReader::new(stdin.lock()), MAX_IPC_MESSAGE);
    let mut writer = FrameWriter::new(BufWriter::new(stdout.lock()), MAX_IPC_MESSAGE);

    let hello = match read_request(&mut reader) {
        Request::Hello(hello) => hello,
        _ => std::process::exit(2),
    };
    let limits = NegotiatedLimits::server_negotiate(&hello).unwrap();
    write_response(&mut writer, &Response::HelloOk(limits.to_hello_ok()));

    if mode == "stop-reading" {
        sleep_forever();
    }

    if mode == "wrong-render" {
        let render = match read_request(&mut reader) {
            Request::RenderPage(render) => render,
            _ => std::process::exit(2),
        };
        write_response(
            &mut writer,
            &Response::PageRendered(PageRenderedResponse {
                document_id: render.document_id.wrapping_add(1),
                page: render.page.wrapping_add(1),
                width: 1,
                height: 1,
                stride: 4,
                pixel_format: 1,
                bytes: vec![0; 4],
                shm_len: None,
            }),
        );
        sleep_forever();
    }

    let nonce = match read_request(&mut reader) {
        Request::Ping(ping) => ping.nonce,
        _ => std::process::exit(2),
    };
    let response = Response::Pong(PongResponse {
        nonce: if mode == "wrong-pong" {
            nonce.wrapping_add(1)
        } else {
            nonce
        },
    });
    write_response(&mut writer, &response);
    if mode == "extra-pong" {
        write_response(&mut writer, &response);
    }
    sleep_forever();
}

fn read_request<R: Read>(reader: &mut FrameReader<R>) -> Request {
    let frame = reader.next_frame().unwrap().unwrap();
    decode_request(&frame).unwrap()
}

fn write_response<W: Write>(writer: &mut FrameWriter<W>, response: &Response) {
    let payload = encode_response(response).unwrap();
    writer.write_frame(&payload).unwrap();
    writer.flush().unwrap();
}
