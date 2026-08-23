//! Fuzz target: content-based format detection.
//!
//! Detection must never panic and must never misidentify formats on
//! truncated/junk input in a way that lets a decoder be selected wrongly
//! (detection here is the security gate). Run with
//! `cargo fuzz run format_detect`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use docbunker_renderer_api::format;

fuzz_target!(|data: &[u8]| {
    let _ = format::detect(data);
});
