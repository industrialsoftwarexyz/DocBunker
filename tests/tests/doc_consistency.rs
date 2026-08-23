//! Keeps the documentation in sync with the code it describes.
//!
//! Every documented hard limit, wire value and environment variable is checked
//! on both sides: the constant must still exist in the named source file with
//! the expected value, and the docs must still mention it. If someone changes
//! one without the other, this test fails instead of the docs rotting quietly.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative)
}

fn read(relative: &str) -> String {
    let path = repo_file(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// Collapse whitespace so multi-line markdown tables can be matched reliably.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn expect_contains(haystack: &str, context: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "{context} does not contain {needle:?}; update the file or doc_consistency test together"
    );
}

#[test]
fn protocol_limits_match_code_and_docs() {
    let protocol_limits = read("crates/protocol/src/limits.rs");
    expect_contains(
        &protocol_limits,
        "protocol limits.rs",
        "PROTOCOL_VERSION: u8 = 2",
    );
    expect_contains(
        &protocol_limits,
        "protocol limits.rs",
        "MAX_IPC_MESSAGE: usize = 96 * 1024 * 1024",
    );
    expect_contains(
        &protocol_limits,
        "protocol limits.rs",
        "MAX_STRING_LENGTH: usize = 256",
    );

    let renderer_limits = read("crates/renderer-api/src/limits.rs");
    expect_contains(
        &renderer_limits,
        "renderer-api limits.rs",
        "MAX_DOCUMENT_SIZE: usize = 64 * 1024 * 1024",
    );
    expect_contains(
        &renderer_limits,
        "renderer-api limits.rs",
        "MAX_PAGE_WIDTH: u32 = 4096",
    );
    expect_contains(
        &renderer_limits,
        "renderer-api limits.rs",
        "MAX_PAGE_HEIGHT: u32 = 4096",
    );
    expect_contains(
        &renderer_limits,
        "renderer-api limits.rs",
        "MAX_PAGE_COUNT: u32 = 100_000",
    );

    let docs = squash(&read("docs/protocol.md"));
    expect_contains(&docs, "docs/protocol.md", "`PROTOCOL_VERSION` | 2 |");
    expect_contains(&docs, "docs/protocol.md", "`MAX_DOCUMENT_SIZE` | 64 MiB");
    expect_contains(
        &docs,
        "docs/protocol.md",
        "`MAX_PAGE_WIDTH` / `MAX_PAGE_HEIGHT` | 4096 / 4096 px",
    );
    expect_contains(&docs, "docs/protocol.md", "`MAX_PIXEL_BUFFER` | 64 MiB");
    expect_contains(&docs, "docs/protocol.md", "`MAX_PAGE_COUNT` | 100 000");
    expect_contains(&docs, "docs/protocol.md", "`MAX_IPC_MESSAGE` | 96 MiB");
    expect_contains(&docs, "docs/protocol.md", "`MAX_STRING_LENGTH` | 256");
}

#[test]
fn wire_discriminators_match_docs() {
    let framing = read("crates/protocol/src/framing.rs");
    for (name, value) in [
        ("DISCRIMINATOR_REQUEST_HELLO", "0x00"),
        ("DISCRIMINATOR_REQUEST_OPEN_DOCUMENT", "0x01"),
        ("DISCRIMINATOR_REQUEST_GET_DOCUMENT_INFO", "0x02"),
        ("DISCRIMINATOR_REQUEST_RENDER_PAGE", "0x03"),
        ("DISCRIMINATOR_REQUEST_CLOSE_DOCUMENT", "0x04"),
        ("DISCRIMINATOR_REQUEST_PING", "0x05"),
        ("DISCRIMINATOR_RESPONSE_HELLO_OK", "0x80"),
        ("DISCRIMINATOR_RESPONSE_DOCUMENT_OPENED", "0x81"),
        ("DISCRIMINATOR_RESPONSE_DOCUMENT_INFO", "0x82"),
        ("DISCRIMINATOR_RESPONSE_PAGE_RENDERED", "0x83"),
        ("DISCRIMINATOR_RESPONSE_CLOSED", "0x84"),
        ("DISCRIMINATOR_RESPONSE_PONG", "0x85"),
        ("DISCRIMINATOR_RESPONSE_ERROR", "0x86"),
    ] {
        expect_contains(
            &framing,
            "framing.rs",
            &format!("pub const {name}: u8 = {value};"),
        );
    }

    let docs = squash(&read("docs/protocol.md"));
    for row in [
        "| 0x00 | `Hello` |",
        "| 0x03 | `RenderPage` |",
        "| 0x05 | `Ping` |",
        "| 0x80 | `HelloOk` |",
        "| 0x83 | `PageRendered` |",
        "| 0x86 | `Error` |",
    ] {
        expect_contains(&docs, "docs/protocol.md", row);
    }
}

#[test]
fn document_format_wire_values_match_docs() {
    let types = read("crates/renderer-api/src/types.rs");
    for pair in [
        "DocumentFormat::Unknown => 0",
        "DocumentFormat::Pdf => 1",
        "DocumentFormat::Png => 2",
        "DocumentFormat::Jpeg => 3",
        "DocumentFormat::WebP => 4",
        "DocumentFormat::Ooxml => 5",
        "DocumentFormat::Gif => 6",
        "DocumentFormat::Tiff => 7",
        "DocumentFormat::Bmp => 8",
        "DocumentFormat::Epub => 9",
        "DocumentFormat::Rtf => 10",
        "DocumentFormat::Html => 11",
        "PixelFormat::Rgba8888 => 1",
    ] {
        expect_contains(&types, "types.rs", pair);
    }

    let docs = squash(&read("docs/protocol.md"));
    expect_contains(
        &docs,
        "docs/protocol.md",
        "`5` OOXML (Office text preview, ADR-007), `6` GIF, `7` TIFF, `8` BMP, `9` EPUB (ADR-010), `10` RTF, `11` HTML.",
    );
}

#[test]
fn error_codes_match_docs() {
    let message = read("crates/protocol/src/message.rs");
    for pair in [
        "UnsupportedFormat = 1",
        "DocumentTooLarge = 2",
        "InvalidDocument = 3",
        "PasswordProtected = 4",
        "RenderingFailed = 5",
        "Timeout = 6",
        "ResourceLimitExceeded = 7",
        "ProtocolViolation = 8",
        "UnsupportedOperation = 9",
        "Internal = 10",
    ] {
        expect_contains(&message, "message.rs", pair);
    }

    let docs = squash(&read("docs/protocol.md"));
    expect_contains(&docs, "docs/protocol.md", "| 1 | `UnsupportedFormat` |");
    expect_contains(&docs, "docs/protocol.md", "| 6 | `Timeout` |");
    expect_contains(&docs, "docs/protocol.md", "| 10 | `Internal` |");
}

#[test]
fn shared_memory_handshake_is_documented() {
    let docs = squash(&read("docs/protocol.md"));
    expect_contains(&docs, "docs/protocol.md", "shm_capable");
    expect_contains(&docs, "docs/protocol.md", "shm_name?");
    expect_contains(&docs, "docs/protocol.md", "shm_len?");
}

#[test]
fn sandbox_hardening_values_match_code_and_docs() {
    let bundle = read("crates/sandbox/src/runsc_bundle.rs");
    expect_contains(&bundle, "runsc_bundle.rs", "SANDBOX_UID: u32 = 65534");
    expect_contains(
        &bundle,
        "runsc_bundle.rs",
        "TMPFS_TMP_SIZE: u64 = 256 * 1024 * 1024",
    );

    let ooxml = read("crates/renderer-ooxml/src/lib.rs");
    expect_contains(
        &ooxml,
        "renderer-ooxml lib.rs",
        "MAX_ZIP_ENTRIES: usize = 1_024",
    );
    expect_contains(
        &ooxml,
        "renderer-ooxml lib.rs",
        "MAX_ENTRY_UNCOMPRESSED: u64 = 16 * 1024 * 1024",
    );
    expect_contains(
        &ooxml,
        "renderer-ooxml lib.rs",
        "MAX_COMPRESSION_RATIO: u64 = 1_000",
    );
    expect_contains(
        &ooxml,
        "renderer-ooxml lib.rs",
        "MAX_EMBEDDED_IMAGES: usize = 64",
    );

    let container = read("crates/renderer-ooxml/src/container.rs");
    expect_contains(
        &container,
        "container.rs",
        "MAX_TEXT_CHARS: usize = 200_000",
    );

    let cache = read("crates/core/src/cache.rs");
    expect_contains(&cache, "cache.rs", "DEFAULT_CAPACITY: usize = 3");

    let threat_model = squash(&read("docs/threat-model.md"));
    for needle in ["≤ 1 024", "≤ 16 MiB", "≤ 64 MiB", "≤ 1 000×"] {
        expect_contains(&threat_model, "threat-model.md", needle);
    }
}

#[test]
fn documented_environment_variables_exist_in_code() {
    let cases: &[(&str, &str)] = &[
        ("DOCBUNKER_BACKEND", "src-tauri/src/app_config.rs"),
        (
            "DOCBUNKER_RUNSC_BIN",
            "crates/sandbox/src/platforms/linux.rs",
        ),
        ("DOCBUNKER_ROOTFS", "crates/sandbox/src/platforms/linux.rs"),
        (
            "DOCBUNKER_ESCAPE_WORKER",
            "crates/sandbox/src/platforms/linux.rs",
        ),
        ("DOCBUNKER_QEMU_BIN", "src-tauri/src/app_config.rs"),
        ("DOCBUNKER_VM_KERNEL", "src-tauri/src/app_config.rs"),
        ("DOCBUNKER_VM_INITRD", "src-tauri/src/app_config.rs"),
        ("DOCBUNKER_QEMU_ACCEL", "src-tauri/src/app_config.rs"),
        ("DOCBUNKER_QEMU_CPU", "src-tauri/src/app_config.rs"),
        ("DOCBUNKER_QEMU_MACHINE", "src-tauri/src/app_config.rs"),
        ("DOCBUNKER_ALLOWED_OPEN_DIR", "src-tauri/src/app_config.rs"),
        ("DOCBUNKER_WORKER_BIN", "src-tauri/src/app_config.rs"),
    ];

    let sandbox_docs = read("docs/sandbox.md");
    for (variable, source) in cases {
        expect_contains(&read(source), source, variable);
        if !matches!(
            *variable,
            "DOCBUNKER_ALLOWED_OPEN_DIR" | "DOCBUNKER_WORKER_BIN"
        ) {
            expect_contains(&sandbox_docs, "docs/sandbox.md", variable);
        }
    }
}
