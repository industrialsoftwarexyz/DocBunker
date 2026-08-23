# DocBunker

<img src="assets/docbunker-logo.svg" alt="DocBunker logo" class="landing-logo">

A desktop viewer for documents you do not trust. PDFs, images and Office files
are parsed only inside a disposable sandbox — gVisor (`runsc`) directly on
Linux, a Linux VM (QEMU + gVisor) on Windows and macOS — and the only thing
that ever reaches your screen is a validated raster image.

<div class="landing-cta" markdown="1">
<a href="https://github.com/industrialsoftwarexyz/DocBunker/releases/latest" class="btn btn-primary">Download</a>
<a href="overview.md" class="btn">How it works</a>
</div>

## Download

Current release: **0.1.0** (early development). Pick the asset for your
platform from
[GitHub Releases](https://github.com/industrialsoftwarexyz/DocBunker/releases/latest).

<div class="hero-grid dl-grid" markdown="1">

<div class="hero-card" markdown="1">
### Windows
Package for Windows 10/11 x64, isolated through QEMU with WHPX.

Enable *Windows Hypervisor Platform* in "Turn Windows features on or off",
and install QEMU separately (not bundled yet). Set `DOCBUNKER_QEMU_BIN` if
QEMU is not on your `PATH`.
</div>

<div class="hero-card" markdown="1">
### macOS
Package for Intel and Apple Silicon Macs, isolated through QEMU with
Hypervisor.framework.

Nothing to enable; install QEMU (for example with Homebrew:
`brew install qemu`).
</div>

<div class="hero-card" markdown="1">
### Linux
Package for x86_64 and aarch64, isolated through QEMU with KVM
(gVisor via `runsc` is also available without a VM).

Install QEMU from your distribution's repositories and make sure `/dev/kvm`
is accessible.
</div>

</div>

!!! note "Why QEMU is a manual step"
    QEMU is not redistributed inside the installers yet; its license and
    dependency review is still open (see `sandbox/vm/README.md`). Everything
    else the sandbox needs ships with the app.

## What happens when you open a file

1. You pick a file (or hand one over from the Gmail extension).
2. The sandbox manager boots a fresh sandbox: no network, read-only rootfs,
   no capabilities, empty environment, memory/CPU/PID limits.
3. The document bytes cross into the sandbox over a size-capped binary IPC
   channel — they are never written to disk there.
4. An untrusted worker parses the bytes and returns RGBA pixels only.
5. Every response is re-validated on the host before a single pixel is drawn.
6. On close (or on any timeout, crash or protocol violation) the sandbox is
   killed, deleted and its bundle destroyed.

A compromised renderer therefore has exactly one way to talk to your machine:
a small, strictly validated protocol that cannot express HTML, paths, URLs or
executable content. The [threat model](threat-model.md) documents what remains
risky anyway — DocBunker makes no "100% secure" claim.

## Questions people ask first

**Why not just open the file in the browser or Office viewer I already have?**
Those viewers are convenient, but their parsers run with full access to your
machine and your session. A single engine bug becomes code execution on your
computer. DocBunker assumes the parser *will* be exploited: even then, the
attacker lands in a disposable VM with no network, no files and one tiny
protocol to talk through.

**What does DocBunker not protect against?**
The honest list is in the [threat model](threat-model.md): an already-compromised
host is out of scope; a WebView vulnerability remains the largest trusted
surface (A23); a compromised renderer can draw wrong pixels (the preview has no
integrity guarantee); and a hypervisor escape would defeat the VM boundary.

**Can I copy text or search inside documents?**
Not yet. The sandbox returns pixels only (ADR-002), so there is nothing to
select or search. Features like validated text extraction need a new ADR first.

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
