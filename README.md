<div align="center">

<img src="docs/assets/docbunker-logo.svg" alt="DocBunker logo" width="96"/>

# DocBunker

**Open untrusted documents without letting them touch your machine.**

[![CI](https://github.com/industrialsoftwarexyz/DocBunker/actions/workflows/ci.yml/badge.svg)](https://github.com/industrialsoftwarexyz/DocBunker/actions/workflows/ci.yml)
[![Docs](https://github.com/industrialsoftwarexyz/DocBunker/actions/workflows/docs.yml/badge.svg)](https://github.com/industrialsoftwarexyz/DocBunker/actions/workflows/docs.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](rust-toolchain.toml)
[![Release](https://img.shields.io/github/v/release/industrialsoftwarexyz/DocBunker)](https://github.com/industrialsoftwarexyz/DocBunker/releases)

[Download](#download) · [How it works](#how-it-works) · [Build from source](#build-from-source) · [Documentation](https://industrialsoftwarexyz.github.io/DocBunker/)

</div>

---

A PDF arrives from an unknown sender. A `.docx` from a form nobody filled in.
Today your PDF viewer, with its hundreds of thousands of lines of parsing code,
opens it directly on your machine.

DocBunker flips that around: the document is parsed inside a **disposable
sandbox**, and the only thing that ever reaches your screen is a validated
raster image. The host never runs a document parser at all.

```
┌──────────────────────────── Your machine ────────────────────────────┐
│                                                                      │
│   Tauri UI ──► Rust core ──► Sandbox manager                         │
│   (pixels only)  sessions      spawns / kills / timeouts              │
│                                   │                                  │
│ ═══════════════════ ISOLATION BOUNDARY ═══════════════════════════    │
│                                   ▼                                  │
│   Disposable sandbox (fresh per document, destroyed on close)        │
│   Linux: gVisor (runsc) · Windows/macOS: QEMU VM with gVisor inside  │
│                                                                      │
│   renderer-worker: parses the bytes, returns RGBA pixels —           │
│   no network, read-only rootfs, no capabilities, hard timeouts       │
└──────────────────────────────────────────────────────────────────────┘
```

## Why it is safe by construction

- **The host has no parser to exploit.** Document formats are decoded inside the
  sandbox; a compromised renderer can only talk back through a small, strictly
  validated binary protocol that carries pixels and a few numbers — nothing else.
- **Every document gets its own sandbox**, torn down completely on close. No
  state survives between documents.
- **The OCI profile drops everything**: no network namespace, no capabilities,
  read-only rootfs, empty environment, cgroup memory/CPU/PID limits, and a
  wall-clock timeout on every operation. An adversarial escape test attacks this
  profile in CI.
- **Raster-only boundary** ([ADR-002](docs/adr/ADR-002-raster-only-document-boundary.md)):
  no HTML, JavaScript, SVG or raw text can cross back to the UI.

The full analysis of what can still go wrong lives in the
[threat model](docs/threat-model.md) — DocBunker makes no "100% secure" claim.

## Supported formats

| Format | Rendering |
| --- | --- |
| PDF | Hayro (pure Rust); optional AGPL MuPDF behind a feature flag |
| PNG, JPEG, WebP | Full image decoding with decompression-bomb caps |
| DOCX, PPTX, XLSX | Extracted text plus embedded media on one preview page |

GIF/TIFF/BMP images and EPUB/RTF/HTML previews are implemented and tested but
not wired into the worker dispatch yet ([ADR-010](docs/adr/ADR-010-gif-tiff-bmp-epub-rtf-html.md)).

## Download

Grab the latest build from
[**GitHub Releases**](https://github.com/industrialsoftwarexyz/DocBunker/releases/latest).

| Platform | Package | Requirements |
| --- | --- | --- |
| Windows 10/11 (x64) | package from Releases | [WHPX](https://learn.microsoft.com/en-us/virtualization/hyper-v-on-windows/quick-start/enable-hyper-v) enabled; QEMU installed separately |
| macOS | package from Releases | Hypervisor.framework (built-in); QEMU installed separately |
| Linux (x64, aarch64) | package from Releases | KVM available; QEMU installed separately |

> QEMU is not bundled with the installers yet — its license review is still open
> (see `sandbox/vm/README.md`). Install `qemu` with your package manager or the
> official Windows builds and set `DOCBUNKER_QEMU_BIN` if it is not on `PATH`.
>
> Project status: **0.1.0, early development.** Treat it as a working prototype:
> the design docs describe what is actually implemented, and the release notes
> list the known limitations.

## Build from source

Prerequisites: Rust 1.85+ (pinned via `rust-toolchain.toml`), Node.js 20+ and the
[Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform.

```bash
npm --prefix frontend ci
npm --prefix frontend run build

cargo check --workspace --all-targets
cargo test --workspace
```

Run it:

```bash
npm --prefix frontend run dev                    # terminal 1
cargo run --manifest-path src-tauri/Cargo.toml   # terminal 2
```

Debug builds use an in-process mock backend by default so you can develop
without QEMU or gVisor. `DOCBUNKER_BACKEND` selects `mock`, `subprocess`,
`runsc` or `vm`; release builds refuse the two unisolated ones. Details in
[docs/sandbox.md](docs/sandbox.md).

## Documentation

| Document | Contents |
| --- | --- |
| [How it works](docs/overview.md) | The life of a document, step by step |
| [Architecture](docs/architecture.md) | Layers, trust boundaries, crate map |
| [Sandbox](docs/sandbox.md) | Backends, OCI hardening profile, escape tests |
| [Protocol](docs/protocol.md) | Wire format, limits, error codes |
| [Threat model](docs/threat-model.md) | 23 documented threats and their mitigations |
| [ADRs](docs/adr/index.md) | Why every major decision was made |

Contributing? Start with [CONTRIBUTING.md](CONTRIBUTING.md). Security issues go
through [SECURITY.md](SECURITY.md), never public issues.

## License

MIT — see [LICENSE](LICENSE). The optional `pdf-mupdf` feature links AGPL code
and is excluded from default builds ([ADR-005](docs/adr/ADR-005-mupdf-as-initial-pdf-renderer.md)).
