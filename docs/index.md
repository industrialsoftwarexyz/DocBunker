# DocBunker

<img src="assets/docbunker-logo.svg" alt="DocBunker logo" class="landing-logo">

A PDF from a stranger. An invoice you never expected. A `.docx` attached to
a job offer.

Your usual viewer opens these files on your machine and hopes its parser holds
up. DocBunker doesn't hope: it copies the file into a throwaway sandbox,
renders it there, and shows you the pixels. Close the document and the sandbox
is destroyed.

<div class="landing-cta" markdown="1">
<a href="https://github.com/industrialsoftwarexyz/DocBunker/releases/latest" class="btn btn-primary">Download</a>
<a href="overview.md" class="btn">How it works</a>
</div>

## Download

Current release: **0.1.0** — early days, expect rough edges.
[GitHub Releases](https://github.com/industrialsoftwarexyz/DocBunker/releases/latest)
has the builds.

<div class="hero-grid dl-grid" markdown="1">

<div class="hero-card" markdown="1">
### Windows
Windows 10/11 x64. Enable *Windows Hypervisor Platform*, install
[QEMU](https://qemu.org) separately.
</div>

<div class="hero-card" markdown="1">
### macOS
Apple Silicon Macs. Just install QEMU (`brew install qemu`).
</div>

<div class="hero-card" markdown="1">
### Linux
x86_64 and aarch64. Install QEMU from your repos and make sure `/dev/kvm`
is accessible.
</div>

</div>

!!! note "Why is QEMU a manual step?"
    Its license review for bundling is still open. Everything else the sandbox
    needs ships with the app.

## What happens when you open a file

1. The app boots a fresh sandbox: no network, no access to your files, hard
   memory/CPU limits.
2. Your document's bytes go in through a size-capped channel — never written
   to disk there.
3. An untrusted worker renders it and sends pixels back.
4. Every response is re-checked before a single pixel reaches your screen.
5. On close (or any timeout or crash) the sandbox is killed and deleted.

Even if an attacker fully compromises the parser, they land in a box with
nothing to steal and one tiny protocol to talk through. What could still go
wrong is documented honestly in the [threat model](threat-model.md) — we don't
claim to be 100% secure.

## Questions people ask first

**Why not just use my browser or Office viewer?**
They're convenient, but their parsers run with full access to your machine and
session. One engine bug away from code execution on your computer. DocBunker
assumes that bug will be found.

**Can I copy text or search?**
Not yet. The sandbox returns pixels only, so documents can't smuggle scripts
or link tricks to your machine. Validated text extraction may come later,
behind its own design review.

**Does it protect against everything?**
No. A compromised host, a WebView bug, or a hypervisor escape are outside what
any tool can fully promise. The [threat model](threat-model.md) lists them all.

## Where to start

- [How it works](overview.md): what happens when you open a document
- [Architecture](architecture.md): layers, trust boundaries, crates
- [Sandbox](sandbox.md): the four backends and the OCI hardening profile
- [Protocol](protocol.md): the wire format across the IPC channel
- [Roadmap](roadmap.md): phase-by-phase status
- [ADRs](adr/index.md): architecture decision records

Building from source, contributing, or reporting a security issue:
[CONTRIBUTING.md](https://github.com/industrialsoftwarexyz/DocBunker/blob/main/CONTRIBUTING.md)
and [SECURITY.md](https://github.com/industrialsoftwarexyz/DocBunker/blob/main/SECURITY.md).
