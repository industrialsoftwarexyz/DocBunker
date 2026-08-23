# Roadmap

Implementation is phase-based. Each phase ends with a compiling, tested repository. Phases 1
through 8 are complete.

## Phase 1 — Foundation, mock sandbox, mock renderer (done)

- Cargo workspace, Tauri 2 shell, Vue 3/TS frontend, `crates/` split.
- Binary IPC protocol (versioned, framed, validated, unit-tested).
- `SandboxBackend` + `SandboxTransport` abstractions; `MockBackend` (in-process, clearly mock).
- `renderer-worker` binary speaking the real protocol over stdin/stdout.
- End-to-end: open a file → view a **mock-generated test page** → navigate/zoom/close.
- Docs: architecture, threat model, protocol, sandbox, ADRs 001–005.
- CI: fmt, clippy, tests, frontend lint/build, cargo audit, license check.

## Phase 2 — Real image rendering (PNG/JPEG/WebP) (done)

- New crate `crates/renderer-image`: `png` (pure Rust), `jpeg-decoder` (pure
  Rust), `webp`/libwebp — dimension caps and decompression-bomb protection
  applied before decode, bilinear scaling to the requested target.
- `StandardRendererFactory` dispatch in `renderer-worker` (content-detected).
- `ProcessTransport` (pipes + per-op timeout) and `SubprocessBackend`
  (**no isolation — development only**); the app selects the backend via
  `DOCBUNKER_BACKEND`.
- Integration tests with real PNG/JPEG/WebP fixtures (valid, truncated,
  absurd dimensions) over the worker subprocess.

## Phase 3 — PDF rendering (done)

- Default `PdfRenderer` over Hayro 0.4 (Apache-2.0), a pure-Rust,
  rasterize-only engine behind the `DocumentRenderer` seam.
- Optional `pdf-mupdf` cargo feature replaces Hayro with MuPDF (`mupdf`
  crate, `default-features = false` -> no JS engine).
- **License conflict discovered**: MuPDF is AGPL-3.0, incompatible with the
  MIT project + `copyleft = "deny"`; the feature remains opt-in for
  AGPL-compliant deployments.
- Tests: valid PDF, corrupt PDF, page out of range, and QEMU/gVisor E2E.

## Phase 4 — Linux `runsc` sandbox (done, Linux, CI-verified)

- `OciBundle` generator (platform-independent, unit-tested): read-only
  rootfs, no capabilities, unprivileged user, empty network namespace,
  cgroup limits, capped `/tmp` tmpfs.
- `RunscBackend` drives `runsc run` directly over piped stdio (worker = init),
  with `runsc delete --force` + bundle removal on close/drop.
- `ProcessTransport` enforces wall-clock timeouts on every operation and
  terminates the sandbox on timeout/crash (`Timeout`, `SandboxCrashed`).
- End-to-end runsc test (`#[ignore]`, opt-in) + `runsc`-installing CI job.
- All `runsc` invocations use `std::process::Command` with separated args.

## Phase 5 — Hardening (done, folded into Phase 4)

- `--network=none`, read-only rootfs, unprivileged user, capability drop,
  cgroup memory/CPU/PID limits, empty worker environment, per-op timeouts,
  robust cleanup on kill/delete/crash.
- Protocol fuzz targets exist (`crates/protocol/fuzz`,
  `crates/renderer-api/fuzz`); CI compiles them with `cargo-fuzz`.

## Phase 6 — Cross-platform VM backend (done)

- `QemuVmBackend` uses QEMU + WHPX (Windows), HVF (macOS), or KVM (Linux) as
  the outer boundary and a prebuilt Linux initramfs with gVisor as the inner
  boundary. Architecture-specific release images and host smoke tests remain
  release artifacts rather than source-controlled files.

## Phase 7 — Office containers (text preview) (done)

- `DocumentFormat::Ooxml` (wire value 5) + new crate `renderer-ooxml`:
  docx/pptx/xlsx containers, text-only extraction with the ADR-007 container
  caps (entry count, per-entry/total uncompressed size, compression ratio,
  bounded reads, no XML entity expansion).
- Text preview rendered as one A4 RGBA page with an embedded Roboto
  (variable-font default instance, SIL OFL) rasterized by `fontdue`; the
  output is a validated `RenderedPage` like any other renderer's.
- File-type lists updated everywhere: dialog filter, file association,
  native host, Gmail extension; CI-checked with unit + subprocess round-trip
  tests (valid docx, zip bomb caps, billion laughs, numeric entities).
- ADR-007 precedes the feature (threat-model A5 requirement).
- Embedded PNG/JPEG/WebP media from `*/media/` folders is also previewed
  below the text (ADR-008), with bounded counts/bytes and sequential decode.

## Phase 8 — Shared-memory page-buffer transport (done)

- Additive protocol v2 (ADR-009): `HelloRequest.shm_capable` /
  `HelloOk.shm_name|shm_capacity` / `PageRendered.shm_len`. The worker
  publishes one bounded temp-file region (safe `FileExt` IO, `unsafe`-free)
  and the host normalizes shared pages back to in-frame bytes before the
  unchanged validation pipeline. Only the dev `SubprocessBackend` advertises
  shared memory; `runsc`/QEMU and the in-process mock keep frame copies.
- Worker falls back to frames automatically when region creation fails.

## Next

- Wire the already-tested GIF/TIFF/BMP and EPUB/RTF/HTML renderers into the
  worker dispatch (ADR-010) and extend the file-type lists.
