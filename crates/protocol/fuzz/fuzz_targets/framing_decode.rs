//! Fuzz target: protocol payload decoding.
//!
//! The decoder must never panic and must never accept structurally invalid
//! input (it may legitimately error). Run with `cargo fuzz run framing_decode`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = docbunker_protocol::decode_request(data);
    let _ = docbunker_protocol::decode_response(data);
});
