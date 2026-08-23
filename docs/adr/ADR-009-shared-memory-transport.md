# ADR-009: Shared-memory transport for page buffers (Phase 8)

> Status: accepted and implemented as an additive, negotiated optimization of
> the binary IPC (protocol v2) for **local subprocess backends only**.

## Context

Until now every rendered RGBA buffer crosses the isolation boundary as a
copied frame (`bytes` inside `PageRendered`). The roadmap notes the MVP
"accepted for the MVP, replaced by shared memory / ring buffers in a later
phase" (architecture.md). Page buffers can be up to 64 MiB
(4096×4096×4), so each render copies up to 64 MiB through pipes plus a second
copy inside postcard.

Shared memory cannot apply everywhere: the VM backend (Phase 6) is a separate
machine communicating over `virtio-serial`; `runsc` sits behind its own
mount/PID namespaces where a host-named POSIX shm region is not reachable.
Only the **bare `SubprocessBackend`** (development) can use it.

## Decision

A protocol **v2** addition that stays backward-compatible in behavior (any
side may decline):

1. `HelloRequest` gains `shm_capable: bool` (host asks for shared-memory
   pages).
2. `HelloOk` gains `shm_name: Option<String>` and `shm_capacity: Option<u64>`:
   the worker creates the region itself (best-effort; if creation fails it
   falls back to frames) and advertises it; the host maps it read-only and
   binds `shm_capacity ≤ MAX_PIXEL_BUFFER`.
3. `PageRendered` gains `shm_len: Option<u32>`: when set, `bytes` must be
   empty and the payload lives at the start of the shared region.

Flow: worker renders → stores RGBA at region offset 0 → `fence(SeqCst)` →
sends the (small) frame. Host receives → `fence(SeqCst)` → copies `shm_len`
bytes out of the region → normalizes the response (same bytes as the legacy
path) → runs the **unchanged** validation (`rendered_page_from_wire`).

Security properties:

- The worker (untrusted) decides `shm_len`; the host copies at most
  `min(negotiated max_pixel_buffer, bound capacity)`, never allocating from
  worker-controlled sizes beyond the negotiated cap, and the page is
  validated identically afterwards.
- The region is a fixed, host-bounded allocation (`MAX_PIXEL_BUFFER`)
  re-created by the worker per handshake; it is closed on transport drop.
- `shm_capable` is only asserted by `SubprocessBackend` (dev, documented
  un-isolated); `QemuVmBackend`/`RunscBackend` never advertise it. The
  mock/in-process path never uses shared memory.
- Without `shm_capable` (or on worker fallback), the flow byte-for-byte the
  legacy frame protocol (v1 behavior).

The cross-process memory ordering uses `std::sync::atomic::fence(SeqCst)` on
both sides around the mapped region, with the pipe's kernel ordering
providing the handshake; documented residual is "reads/writes are
sequentially consistent within one page-sized slot".

## Alternatives

- **ring buffer with fence/sequence words**: more machinery, same security
  envelope; not needed when host and worker strictly alternate one page.

## Security implications

- Shared memory is a **performance optimization only**: the untrusted worker
  already *owns the bytes* it sends; the risk envelope (host must bound and
  validate before use) is unchanged, and re-uses the negotiated `max_pixel_buffer`.
- The region is created/owned by the worker side; the host only maps the
  advertised name via the OS (0600 shm, per-session random name); region
  names are never derived from documents.

## Consequences

- `PROTOCOL_VERSION` → 2 (additive fields; v1 peers reject the handshake).
- Frames for large pages stop carrying the raster; `MAX_IPC_MESSAGE` no
  longer needs the 64 MiB headroom for local subprocess use (kept for the
  VM path).
- Implementation: a per-user temp file (`docbunker_pagebuf_*`) created by the
  worker with `create_new`, sized to `MAX_PIXEL_BUFFER`, written/read at
  offset 0 through the **safe** `std::os::*::FileExt` APIs (`seek_write` /
  `seek_read`). This keeps the workspace `unsafe_code = "forbid"` lint
  (mmap-crate variants mark `map`/`as_slice` unsafe) and works identically
  on Windows and POSIX. The worker removes the file on drop; the host opens
  it read-only and validates the advertised capacity before use.