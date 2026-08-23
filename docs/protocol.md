# IPC protocol specification

The protocol lives in `crates/protocol` and is shared verbatim by the trusted host and the
untrusted `renderer-worker`. This document specifies it; the code is the normative reference.

## Principles

- **Small surface.** The worker can do exactly six things: handshake, open, info, render, close, ping.
- **Binary only.** No JSON for payloads. `postcard` is used for struct serialization (fixed, compact,
  strict on trailing bytes).
- **Hard caps everywhere.** Every length that arrives over the wire is validated before use; every
  size arithmetic uses checked Rust math.
- **Versioned.** A protocol version byte is the first byte of every payload.
- **Semantic validation** happens after structural decoding and before the worker acts on a message.

## Limits

| Constant | Value | Meaning |
| --- | --- | --- |
| `PROTOCOL_VERSION` | 2 | Bumped on any incompatible wire change (v2 adds the shared-memory page-buffer fields, ADR-009) |
| `MAX_DOCUMENT_SIZE` | 64 MiB | Bytes accepted for `OpenDocument` (must fit in one frame) |
| `MAX_PAGE_WIDTH` / `MAX_PAGE_HEIGHT` | 4096 / 4096 px | Output raster dimensions |
| `MAX_PIXEL_BUFFER` | 64 MiB | `4096 × 4096 × 4` |
| `MAX_PAGE_COUNT` | 100 000 | Metadata sanity bound |
| `MAX_IPC_MESSAGE` | 96 MiB | Hard cap for any single frame |
| `MAX_STRING_LENGTH` | 256 | Error/misc strings |

## Framing

```
frame := [payload_len: u32 LE] [payload]
payload := [version: u8] [discriminator: u8] [body: postcard]
```

- `payload_len` must be `≥ 2` and `≤ MAX_IPC_MESSAGE`.
- `version` must equal `PROTOCOL_VERSION`.
- `discriminator` selects the message type; unknown discriminators are rejected.
- `body` is postcard-encoded and must decode **without trailing bytes** (postcard is strict).

`FrameReader` / `FrameWriter` wrap any `Read`/`Write` pair; the transport is replaceable
(`SandboxTransport`), so Unix sockets, pipes, `virtio-vsock` or `virtio-serial` can be swapped in
without touching message handling.

## Handshake

1. Host → `Hello { protocol_version, max_document_size, max_ipc_message }`
2. Worker → `HelloOk { protocol_version, max_page_width, max_page_height, max_pixel_buffer,
   max_document_size, max_ipc_message }`

Both sides compute negotiated limits as the **minimum** of their own caps and the peer's
advertised caps. Every later message is validated against the negotiated limits by **both** sides
(defense in depth). Messages other than `Hello` before the handshake are a protocol violation.

## Messages

Requests (`Request`):

| Discriminator | Message | Body |
| --- | --- | --- |
| 0x00 | `Hello` | `HelloRequest` |
| 0x01 | `OpenDocument` | `{ document_id: u64, data: Vec<u8> }` |
| 0x02 | `GetDocumentInfo` | `{ document_id: u64 }` |
| 0x03 | `RenderPage` | `{ document_id: u64, page: u32, target_width: u32, target_height: u32 }` |
| 0x04 | `CloseDocument` | `{ document_id: u64 }` |
| 0x05 | `Ping` | `{ nonce: u64 }` |

Responses (`Response`):

| Discriminator | Message | Body |
| --- | --- | --- |
| 0x80 | `HelloOk` | `HelloOk` |
| 0x81 | `DocumentOpened` | `{ document_id, info: DocumentInfoWire }` |
| 0x82 | `DocumentInfo` | `{ document_id, info: DocumentInfoWire }` |
| 0x83 | `PageRendered` | `{ document_id, page, width, height, stride, pixel_format, bytes }` |
| 0x84 | `Closed` | `{ document_id }` |
| 0x85 | `Pong` | `{ nonce }` |
| 0x86 | `Error` | `{ code: u8, message: String(≤ MAX_STRING_LENGTH) }` |

`DocumentInfoWire` is deliberately minimal: `{ page_count: u32, width: u32, height: u32,
format: u8 }`. No title/text extraction in the MVP — less metadata means less attack surface.
`format` values: `0` unknown, `1` PDF, `2` PNG, `3` JPEG, `4` WebP, `5` OOXML (Office
text preview, ADR-007); unknown values are rejected by validation on both sides.

`pixel_format` is `u8`; currently only `Rgba8888 = 1` is defined. `stride` must be `≥ width × 4`.

## Error codes

`code` in an `Error` response:

| Code | Name | Meaning |
| --- | --- | --- |
| 1 | `UnsupportedFormat` | Content could not be identified/opened |
| 2 | `DocumentTooLarge` | Exceeds size limits |
| 3 | `InvalidDocument` | Corrupt or rejected by the renderer |
| 4 | `PasswordProtected` | Encrypted document (not supported) |
| 5 | `RenderingFailed` | Page could not be rasterized |
| 6 | `Timeout` | Operation exceeded its deadline |
| 7 | `ResourceLimitExceeded` | Dimensions/buffer limits hit |
| 8 | `ProtocolViolation` | Malformed or out-of-order message |
| 9 | `UnsupportedOperation` | Feature not implemented yet |
| 10 | `Internal` | Renderer-internal failure (details logged, not sent) |

The host maps these to `DocBunkerError`; the `message` string is treated as untrusted text, logged at
debug level, and never shown verbatim in the UI.

## Validation rules (both sides, always)

- `OpenDocument.data.len()` ≤ negotiated `max_document_size`.
- `RenderPage.target_width/height` in `[1, MAX_PAGE_WIDTH/HEIGHT]`; `width × height × 4` must fit in
  `max_pixel_buffer` (checked arithmetic).
- `PageRendered`: `width ≤ MAX_PAGE_WIDTH`, `height ≤ MAX_PAGE_HEIGHT`, `stride ≥ width × 4`,
  `bytes.len() == stride × height` (checked), total ≤ `max_pixel_buffer`, `pixel_format` known.
- `page < page_count` and `page_count ≤ MAX_PAGE_COUNT`.
- `document_id` must have been returned by `DocumentOpened` and not yet closed.

## Fuzz targets

`crates/protocol` carries fuzz targets for framing, message decoding and validation
(`cargo fuzz`); see the `fuzz/` manifest in that crate. PDF engine internals are **not** fuzzed from the
host — any fuzzing of the renderer happens inside the sandbox toolchain, never in a privileged
process.
