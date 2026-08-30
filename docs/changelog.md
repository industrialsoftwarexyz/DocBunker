# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Release workflow (`release.yml`): per-platform bundles, SBOM and SHA256SUMS
  attached to a draft release on version tags.
- `doc_consistency` test that keeps documented limits, wire values and
  environment variable names in sync with the code.
- Docs: landing page with download options, browser extension page,
  git-revision dates, normalized ADR status headers.

### Changed

- Upgraded `quick-xml` from 0.37 to 0.41, clearing RUSTSEC-2026-0194/0195
  (unbounded namespace-declaration allocation). Since quick-xml 0.38 reports
  entity references as separate `GeneralRef` events, the text extractors now
  decode them there under the same ADR-007 policy as before: only the five
  predefined entities and numeric references; custom entities stay unexpanded.
- Added `audit.toml`: documented cargo-audit exceptions for unmaintained
  transitive crates only (ttf-parser via fontdue, gtk-rs 0.18 stack via
  Tauri on Linux, `unic-*` via tauri-utils). No vulnerability-class advisory
  is ignored.

### Fixed

- CI: `cargo-fuzz` is installed from a prebuilt binary (compiling it against
  current nightlies fails), the musl worker build installs `musl-tools` and
  points `cc` at `musl-gcc`, and the supply-chain job pins a resolvable
  `actions/upload-artifact` commit.

## [0.1.0] - 2026-08-18

First public release. DocBunker is a cross-platform desktop viewer for
potentially malicious documents, rendered entirely inside disposable isolation
(gVisor `runsc` on Linux, a QEMU + WHPX/HVF/KVM VM with inner gVisor
everywhere else). See `README.md` and `docs/architecture.md` for the trust
model.

### Added

- Tauri 2 desktop shell with a Vue 3 + TypeScript frontend, explicit command
  bridge and CSP (`src-tauri`, `frontend`).
- `crates/protocol`: versioned, framed, validated binary IPC between the
  trusted host and the untrusted `renderer-worker` (protocol v2, ADR-004/009).
- Renderers, all emitting validated RGBA pages:
  - PNG/JPEG/WebP via `renderer-image` (pure Rust, dimension caps and
    decompression-bomb protection before decode, ADR-002).
  - PDF via Hayro 0.4 (Apache-2.0); optional AGPL MuPDF behind the
    `pdf-mupdf` feature (ADR-005).
  - Office text preview (docx/pptx/xlsx), HTML, EPUB, RTF via
    `renderer-ooxml` with ADR-007 container caps (entry count, per-entry and
    total uncompressed sizes, compression ratio, bounded reads, no entity
    expansion) and ADR-008 embedded media preview.
- Sandbox backends behind `SandboxBackend`/`SandboxTransport`:
  - `mock` (debug default) and `subprocess` (dev, `development-backends`
    feature, rejected in production builds) — both loudly non-isolated.
  - `runsc` — direct gVisor backend for Linux: OCI bundle with read-only
    rootfs, no capabilities, unprivileged user, `--network=none`, cgroup
    limits, empty environment, per-operation timeouts, guaranteed cleanup.
  - `vm` — production default: QEMU + WHPX/HVF/KVM booting a minimal Linux
    initramfs with gVisor inside (ADR-003/006).
- Shared-memory page-buffer transport for the dev subprocess backend
  (protocol v2, ADR-009): 0-copy frames in dev; runsc/VM keep framed copies.
- Windows native messaging host for the Gmail extension (per-user
  registration, origin-scoped), `browser-extension/` keyed Chrome extension.
- Crash-safety hardening: `unsafe_code = "forbid"` workspace-wide, `panic =
  "abort"` in release, no recoverable panics on hostile input in the host.
- CI: fmt, clippy `-D warnings`, full test suite on three OSes, frontend
  audit/lint/build, runsc end-to-end (Linux, main), fuzz targets (nightly),
  `cargo audit` + `cargo deny`.
- Docs: architecture, threat model, sandbox, protocol, roadmap, ADRs 001-010.

### Fixed

- All document text with multi-byte UTF-8 now extracts correctly for the
  preview (`decode_text` no longer mangles accented characters byte by byte).
- `String` truncation under the preview character cap can no longer panic on
  hostile multi-byte boundaries (docx/pptx/xlsx/html/epub/rtf).
- EPUB reading enforces the aggregate uncompressed-size cap (ADR-007), not
  only the per-entry cap.
- EPUB `rootfile`/`item`/`itemref` are self-closing elements; parsing now
  handles `Event::Empty` (EPUBs previously failed to open).
- RTF control-word delimiter spaces are dropped per spec (no double spaces
  around runs like `\b`), and the RTF preview cap applies to all control
  words (`\uN`, `\'hh`, `\tab`, ...).
- The host only ever joins worker pipe threads with a bounded wait; a
  descendant holding stdout can no longer hang `close`/`terminate`/`drop`.
- The worker's shared-memory region name is strictly validated
  (`docbunker_pagebuf_<pid>_<n>.bin`), opened with `O_NOFOLLOW` and a regular
  file check; region files are created `0600` and removed by the host when
  the worker dies.
- The untrusted worker's stderr is no longer inherited by the host console.
- Error messages are truncated by bytes (not characters) before crossing the
  wire, so legitimate multi-byte renderer failures surface as real errors.
- The native-handoff path re-verifies the bytes read by the app before they
  reach a parser, and the acknowledgement file is never followed as a symlink.
- The runsc OCI bundle now emits an explicit empty device list.
- The VM backend no longer passes `-qmp none` to QEMU: QEMU 11+ rejects it
  (`'none' is not a valid char driver`), which aborted sandbox startup on
  Windows/macOS. QMP is disabled by omission (`-nodefaults` creates no
  monitor chardev), so the hardening is unchanged.
- PDF rendering quality: uniform scale (no aspect distortion), adaptive
  1.5 device-pixel supersampling, PNG fast compression, and a best-effort
  next-page preload for instant page flips.
- Clippy `-D warnings` and `fmt` are satisfied on the whole workspace.

### Changed

- `PROTOCOL_VERSION` documented as v2 (ADR-009); a stale worker image fails
  the handshake with `InvalidVersion` instead of misbehaving.

### Known limitations

- Prebuilt VM/runsc images must be rebuilt to carry the newest worker and
  protocol v2 (`sandbox/scripts/build-vm-assets.sh`; a stale image fails the
  handshake). See `docs/sandbox.md`.
- QEMU is not redistributed in installers; set `DOCBUNKER_QEMU_BIN` or install
  QEMU (license and dependency review required, see `sandbox/vm/README.md`).
- The MuPDF/AGPL feature is opt-in and NOT part of the binary you ship by
  default (ADR-005).
- macOS/Linux host end-to-end VM verification remains release-gated; only the
  Windows WHPX path is verified end-to-end today.
- GIF/TIFF/BMP/RTF/EPUB/HTML renderers exist and are unit-tested, but the
  UI/worker surface only wires PDF/PNG/JPEG/WebP/OOXML at this release.

<!-- On first release, add: [0.1.0]: <repo-url>/releases/tag/v0.1.0 -->
