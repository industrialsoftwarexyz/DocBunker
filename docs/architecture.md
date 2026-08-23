# Architecture

This document describes the concrete architecture implemented in this repository and the trust
boundaries it relies on. It is the source of truth for how components relate; ADRs record *why*
decisions were made.

## Layers

```
┌──────────────────────────────────────────────────────────────────┐
│  Tauri UI (trusted)        frontend/  Vue 3 + TS + Vite          │
│  - displays raster images only                                   │
│  - speaks 4 explicit commands (open / info / render / close)     │
│  - strict CSP, no fs/shell/http capabilities                     │
├──────────────────────────────────────────────────────────────────┤
│  Tauri shell (trusted)     src-tauri/                            │
│  - file dialog (rfd) + reads bytes (core::io)                    │
│  - spawn_blocking so the UI thread never blocks                  │
├──────────────────────────────────────────────────────────────────┤
│  Rust Core (trusted)       crates/core/                          │
│  - DocumentManager: session lifecycle, page cache, errors        │
│  - PNG-encodes *our own* RGBA before returning to the UI         │
├──────────────────────────────────────────────────────────────────┤
│  Sandbox (trusted)         crates/sandbox/                       │
│  - SandboxBackend trait, SandboxTransport trait                  │
│  - MockBackend (Phase 1) / runsc backend (Phase 4+)              │
├════════════════════════════ ISOLATION BOUNDARY ══════════════════┤
│  renderer-worker (untrusted)  crates/renderer-worker/            │
│  - protocol loop over the transport (stdin/stdout today)         │
│  - renderer dispatch: PNG/JPEG/WebP, OOXML text preview, Hayro (or MuPDF) for PDF │
│  - returns RGBA + minimal metadata only                          │
└──────────────────────────────────────────────────────────────────┘
```

## Trust boundaries

| Boundary | From → To | Guarantee |
| --- | --- | --- |
| UI → Core | Tauri commands (typed DTOs) | No raw paths back to the UI; the UI never reads the filesystem |
| Core → Sandbox | `SandboxBackend` trait | Backends are swappable; core never parses documents |
| Host → Worker | binary protocol (`crates/protocol`) | Versioned, framed, size-capped, strictly validated |
| Worker → Host | same protocol | Only RGBA buffers + small numeric metadata |

Rules that must never be relaxed:

1. The document bytes and everything derived from them are untrusted in the host.
2. The host never parses document formats. MockBackend in Phase 1 does not parse either — it
   *detects* the format from magic bytes and renders a generated test page.
3. The worker never returns HTML, JS, SVG, PDF objects, URLs, or arbitrary files.
4. Every message is validated against hard limits with checked arithmetic (`usize::checked_mul` etc.)
   before any allocation that depends on attacker-controlled sizes.

## Crate responsibilities

### `crates/renderer-api`
Pure domain types with no I/O and no serde:

- `PixelFormat`, `DocumentFormat`, `DocumentInfo`, `RenderOptions`, `RenderedPage`
- `DocumentRenderer` trait — the seam where MuPDF / image decoders / future engines plug in
- `limits` — the numeric hard limits (document size, page dimensions, pixel buffer, page count)
- `format::detect` — content-based format sniffing (magic bytes, no parsing)
- `mock::MockRenderer` — deterministic test-pattern renderer used by Phase 1; clearly not real

### `crates/protocol`
The versioned binary wire protocol, shared by host and worker:

- `limits` — frame-level caps (`MAX_IPC_MESSAGE`, string caps, protocol version)
- `message` — `Request` / `Response` wire structs (postcard-serialized, serde-derived)
- `framing` — length-prefixed frames `[u32 len][payload]`, `FrameReader`/`FrameWriter`
- `validate` — semantic validation of every message against negotiated limits

The protocol only depends on `renderer-api` types (as wire representations). No JSON is ever used
to transport RGBA.

### `crates/sandbox`
Host-side sandbox management:

- `SandboxBackend` — the trait from the architecture spec (`initialize`, `start_session`,
  `send_document`, `get_document_info`, `render_page`, `close_session`)
- `SandboxTransport` — the replaceable IPC seam (`send(Request) -> Response`)
- `transport::InProcessTransport` — in-process transport over the real `Worker` code path (used by
  the mock backend and tests; proves the protocol layer without spawning processes)
- `MockBackend` — full session lifecycle using `InProcessTransport`; **no OS-level isolation** (mock)
- `platforms/linux.rs` / `windows.rs` / `macos.rs` — stubs that return `BackendUnsupported` and
  document the planned implementation

### `crates/renderer-worker`
The untrusted binary. In Phase 1 it runs `MockRenderer`; Phase 2/3 replace the factory with real
decoders. It has no UI, no network, no updater, no shell, no link opening.

### `crates/core`
Trusted orchestration:

- `DocBunkerError` — user-safe, serialized error enum (internal detail is kept out of the UI)
- `DocumentManager` — owns the backend, sessions, the LRU page cache, and document handles
- `PageCache` — LRU with capacity 3 (pages N-1, N, N+1), never unbounded
- `imaging` — PNG encoding of trusted RGBA buffers
- `io` — bounded file reading (size-checked) for the file-dialog flow

## Session lifecycle

```
open(bytes)
  ├─ backend.start_session(config)          → SandboxSession
  ├─ backend.send_document(session, bytes)  → DocumentId   (validates MAX_DOCUMENT_SIZE)
  └─ backend.get_document_info(...)         → DocumentInfo

render_page(handle, page, options)
  ├─ cache lookup (doc, page, target dims)
  ├─ miss → backend.render_page(...)        → RenderedPage (re-validated in core)
  └─ cache put (LRU, capacity 3)

close(handle)
  ├─ backend.close_session(session)
  └─ cache entries for that document evicted
```

If any backend call fails the document is left open but the error is surfaced; `close` always
destroys the session. Timeout/crash handling is enforced by the backend (Phase 4) and reported as
`SandboxCrashed` / `Timeout`.

## Concurrency

- All backend calls are synchronous and run inside `tauri::async_runtime::spawn_blocking`, so the UI
  thread never blocks on parsing/rendering.
- `DocumentManager` is `Arc<Mutex<...>>`: operations are serialized. This is a deliberate Phase 1
  simplification — Phase 4 will add per-document task queues and cooperative cancellation.
- Tokio is used implicitly through Tauri's async runtime; no direct Tokio dependency in Phase 1.

## Memory strategy

- Page buffers are capped by `MAX_PIXEL_BUFFER` (checked arithmetic on `width × height × 4`).
- The host keeps an LRU cache of 3 rendered pages; copies of cached buffers are made on read
  (accepted for the MVP, replaced by shared memory / ring buffers in a later phase).
- The worker is bounded by `MAX_IPC_MESSAGE`; the host queues at most one response and terminates
  sessions that flood, send unsolicited frames, or miss the end-to-end operation deadline.
