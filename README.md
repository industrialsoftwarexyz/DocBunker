<div align="center">

<img src="docs/assets/docbunker-logo.svg" alt="DocBunker logo" width="96"/>

# DocBunker

**Open sketchy documents without betting your computer on it.**

[![CI](https://github.com/industrialsoftwarexyz/DocBunker/actions/workflows/ci.yml/badge.svg)](https://github.com/industrialsoftwarexyz/DocBunker/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Release](https://img.shields.io/github/v/release/industrialsoftwarexyz/DocBunker)](https://github.com/industrialsoftwarexyz/DocBunker/releases)

[Download](#download) · [How it works](https://industrialsoftwarexyz.github.io/DocBunker/) · [Build from source](#build-from-source)

</div>

---

That PDF from an unknown sender. The invoice you never expected. The `.docx`
attached to a job offer.

Your usual viewer opens these files directly on your machine and trusts its
parser not to be exploited that day. DocBunker makes no such bet. It copies
the file into a throwaway sandbox, renders it there, and shows you the result.
The file never touches your real system, and when you close it, the sandbox is
destroyed.

```
 your machine                          disposable sandbox
┌──────────────────┐                 ┌─────────────────────────┐
│ DocBunker window │    document     │ parser runs HERE        │
│                  │ ──────────────► │                         │
│  sees pixels ◄── │    pixels       │ no network              │
│                  │ ◄────────────── │ no access to your files │
└──────────────────┘                 │ destroyed on close      │
                                     └─────────────────────────┘
```

Under the hood this is gVisor (`runsc`) on Linux, and QEMU with gVisor inside
on Windows and macOS. Even a full exploit of the parser still has to get
through two isolation boundaries — and there is nothing in the sandbox worth
taking.

## What can I open?

| File | What you get |
| --- | --- |
| PDF | Rendered pages |
| PNG, JPEG, WebP | Rendered image |
| DOCX, PPTX, XLSX | Text preview of the contents |

No copy-paste or search yet — the viewer deliberately works with pixels only,
so documents cannot smuggle scripts, links or text tricks into your machine.

## Download

Get the latest build from
[**GitHub Releases**](https://github.com/industrialsoftwarexyz/DocBunker/releases/latest).

| Platform | Works on | Extra step needed |
| --- | --- | --- |
| Windows 10/11 x64 | WHPX (built into Windows) | install [QEMU](https://qemu.org) separately |
| macOS (Apple Silicon) | Hypervisor.framework | install QEMU (`brew install qemu`) |
| Linux x64 / aarch64 | KVM | install QEMU from your repos |

QEMU is not bundled yet (license review pending). Everything else ships with
the app. This is version **0.1.0** — early days, expect rough edges.

## Build from source

You need Rust 1.85+, Node.js 20+ and the
[Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/).

```bash
npm --prefix frontend ci && npm --prefix frontend run build
cargo test --workspace

npm --prefix frontend run dev                    # terminal 1
cargo run --manifest-path src-tauri/Cargo.toml   # terminal 2
```

In debug builds the app uses a fake backend, so you can hack on it without
QEMU or gVisor. Set `DOCBUNKER_BACKEND` to pick `mock`, `subprocess`, `runsc`
or `vm`. Details in [docs/sandbox.md](docs/sandbox.md).

## Documentation

| Document | Contents |
| --- | --- |
| [How it works](https://industrialsoftwarexyz.github.io/DocBunker/overview/) | The life of a document, step by step |
| [Architecture](https://industrialsoftwarexyz.github.io/DocBunker/architecture/) | Layers, trust boundaries, crate map |
| [Sandbox](https://industrialsoftwarexyz.github.io/DocBunker/sandbox/) | Backends, hardening profile, escape tests |
| [Protocol](https://industrialsoftwarexyz.github.io/DocBunker/protocol/) | Wire format, limits, error codes |
| [Threat model](https://industrialsoftwarexyz.github.io/DocBunker/threat-model/) | What we protect against — and what we don't |
| [ADRs](https://industrialsoftwarexyz.github.io/DocBunker/adr/) | Why every major decision was made |

Contributing? Read [CONTRIBUTING.md](CONTRIBUTING.md). Found a security issue?
[SECURITY.md](SECURITY.md) — please don't open a public issue.

## License

MIT ([LICENSE](LICENSE)). The optional MuPDF feature links AGPL code and stays
out of default builds.
