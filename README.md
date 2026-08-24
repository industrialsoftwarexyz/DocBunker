<div align="center">

<img src="docs/assets/docbunker-logo.png" alt="DocBunker logo" width="128"/>

# DocBunker

**Open documents you don't trust — without risking your machine.**

[![CI](https://github.com/industrialsoftwarexyz/DocBunker/actions/workflows/ci.yml/badge.svg)](https://github.com/industrialsoftwarexyz/DocBunker/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Release](https://img.shields.io/github/v/release/industrialsoftwarexyz/DocBunker)](https://github.com/industrialsoftwarexyz/DocBunker/releases)

[Download](#download) · [How it works](https://industrialsoftwarexyz.github.io/DocBunker/) · [Build from source](#build-from-source)

</div>

---

## What is this?

You get a PDF from someone you don't know. Or an invoice you weren't
expecting. Or a `.docx` attached to a sketchy job offer.

Normally you'd just double-click it and hope your viewer isn't
exploiting a bug to run code on your machine. DocBunker doesn't do that.
It puts the file in a disposable sandbox, renders it there, and only
shows you the output. When you close it, the sandbox is gone.

On Linux it uses gVisor (`runsc`). On Windows and macOS it uses QEMU
with gVisor inside. Even if the parser has a nasty bug and gets fully
exploited, it still has to break out of two isolation layers — and
there's nothing useful inside the sandbox to steal.

## What can I open?

| File | What you get |
| --- | --- |
| PDF | Rendered pages |
| PNG, JPEG, WebP | Rendered image |
| DOCX, PPTX, XLSX | Text preview of the contents |

You can't copy-paste or search text yet — the viewer works with pixels
only, on purpose. That way documents can't smuggle scripts, links, or
text-based tricks into your machine.

## Download

Get the latest build from
[**GitHub Releases**](https://github.com/industrialsoftwarexyz/DocBunker/releases/latest).

| Platform | What it uses | You need to install |
| --- | --- | --- |
| Windows 10/11 x64 | WHPX (built in) | [QEMU](https://qemu.org) |
| macOS (Apple Silicon) | Hypervisor.framework | QEMU (`brew install qemu`) |
| Linux x64 / aarch64 | KVM | QEMU from your package manager |

QEMU isn't bundled yet (license review in progress). Everything else
ships with the app. This is **0.1.0** — early days, expect rough edges.

## Build from source

You need Rust 1.85+, Node.js 20+ and the
[Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/).

```bash
npm --prefix frontend ci && npm --prefix frontend run build
cargo test --workspace

npm --prefix frontend run dev                    # terminal 1
cargo run --manifest-path src-tauri/Cargo.toml   # terminal 2
```

In debug builds the app uses a fake backend, so you can hack on it
without QEMU or gVisor. Set `DOCBUNKER_BACKEND` to pick `mock`,
`subprocess`, `runsc` or `vm`. Details in
[docs/sandbox.md](docs/sandbox.md).

## Documentation

| Document | Contents |
| --- | --- |
| [How it works](https://industrialsoftwarexyz.github.io/DocBunker/overview/) | The life of a document, step by step |
| [Architecture](https://industrialsoftwarexyz.github.io/DocBunker/architecture/) | Layers, trust boundaries, crate map |
| [Sandbox](https://industrialsoftwarexyz.github.io/DocBunker/sandbox/) | Backends, hardening profile, escape tests |
| [Protocol](https://industrialsoftwarexyz.github.io/DocBunker/protocol/) | Wire format, limits, error codes |
| [Threat model](https://industrialsoftwarexyz.github.io/DocBunker/threat-model/) | What we protect against — and what we don't |
| [ADRs](https://industrialsoftwarexyz.github.io/DocBunker/adr/) | Why every major decision was made |

Contributing? Read [CONTRIBUTING.md](CONTRIBUTING.md). Found a security
issue? [SECURITY.md](SECURITY.md) — please don't open a public issue.

## License

MIT ([LICENSE](LICENSE)). The optional MuPDF feature links AGPL code
and stays out of default builds.
