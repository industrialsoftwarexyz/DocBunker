# DocBunker

<img src="assets/docbunker-logo.png" alt="DocBunker logo" class="landing-logo">

Open untrusted documents in a sandbox. PDFs, images, Office files — only
pixels reach your machine.

<div class="landing-cta" markdown="1">
<a href="https://github.com/industrialsoftwarexyz/DocBunker/releases/latest" class="btn btn-primary">Download</a>
<a href="overview.md" class="btn">How it works</a>
</div>

## Download

**0.1.0** — [GitHub Releases](https://github.com/industrialsoftwarexyz/DocBunker/releases/latest)

<div class="hero-grid dl-grid" markdown="1">

<div class="hero-card" markdown="1">
### Windows
Windows 10/11 x64. Install [QEMU](https://qemu.org).
</div>

<div class="hero-card" markdown="1">
### macOS
Apple Silicon. `brew install qemu`.
</div>

<div class="hero-card" markdown="1">
### Linux
x86_64 / aarch64. QEMU from your repos.
</div>

</div>

## How it works

1. Boot a sandbox — no network, no file access, memory/CPU limits.
2. Document bytes go in through a size-capped channel.
3. Untrusted worker renders it, sends pixels back.
4. Pixels are re-checked before reaching your screen.
5. On close, the sandbox is killed.

Even a full parser exploit lands in a box with nothing to steal.
[Honest about what we can't protect against](threat-model.md).

## FAQ

**Why not my browser/Office viewer?**
Their parsers run with full machine access. One bug away from code execution.

**Can I copy text?**
Not yet. Pixels only — documents can't smuggle scripts or links.

## Start here

- [How it works](overview.md)
- [Architecture](architecture.md)
- [Sandbox](sandbox.md)
- [Roadmap](roadmap.md)
- [ADRs](adr/index.md)

[CONTRIBUTING.md](https://github.com/industrialsoftwarexyz/DocBunker/blob/main/CONTRIBUTING.md) ·
[SECURITY.md](https://github.com/industrialsoftwarexyz/DocBunker/blob/main/SECURITY.md)
