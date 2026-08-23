# DocBunker: a bunker for untrusted documents

DocBunker is a cross-platform desktop viewer for **potentially malicious
documents**. PDFs, images and Office files are parsed **only inside a
disposable sandbox** — a Linux VM (QEMU + gVisor) on Windows/macOS, or gVisor
/`runsc` directly on Linux — and the only thing that ever reaches your screen
is a validated raster image.

<div class="hero-grid">

<div class="hero-card">
## 🧱 What you get
Open any suspicious PDF, image or Office file. It renders fast and crisp,
and it **cannot touch your machine**: no network, read-only filesystem,
temporary session, destroyed on close.
</div>

<div class="hero-card">
## 🚀 Where to start
- **[How it works](overview.md)** — what happens when you open a document
- **[Architecture](architecture.md)** — layers, trust boundaries, crates
- **[Sandbox](sandbox.md)** — the four backends and the OCI hardening
- **[Threat model](threat-model.md)** — everything that can go wrong
- **[Protocol](protocol.md)** — the wire protocol after IPC
- **[Roadmap](roadmap.md)** — phase-by-phase status
- **[ADRs](adr/index.md)** — architecture decision records
</div>

</div>

## Highlights

- **Disposable isolation**: each open document gets a fresh sandbox session
  (QEMU + WHPX/HVF/KVM on Windows/macOS, `runsc` on Linux). Close it and
  everything is torn down.
- **Raster-only boundary** (ADR-002): only validated RGBA pixels ever leave
  the sandbox. No HTML, no JS, no raw text.
- **Hardened by default**: no network, no capabilities, read-only rootfs,
  empty environment, resource limits, timeouts (ADR-003/006).
- **Reproducible, audited supply chain**: locked builds, SBOMs, and
  reproducible worker binaries in CI.

> **Security note:** DocBunker does not claim to be "100% secure". See the
> [threat model](threat-model.md) for the residual risks.