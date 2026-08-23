# Architecture Decision Records

Each ADR explains a decision and the context that forced it. If you want to
understand why the project is shaped the way it is, read them in order.

| # | Decision | Why it exists |
| --- | --- | --- |
| [ADR-001](ADR-001-tauri-as-desktop-shell.md) | Tauri as the desktop shell | Any UI would need filesystem/process access; Tauri keeps the trusted surface small. |
| [ADR-002](ADR-002-raster-only-document-boundary.md) | Documents leave the sandbox as raster only | The worker must never send back anything interpretable (HTML, SVG, links). |
| [ADR-003](ADR-003-gvisor-as-linux-isolation-layer.md) | gVisor/runsc as the Linux isolation layer | A real kernel boundary for parsing, cheaper than a full VM. |
| [ADR-004](ADR-004-binary-ipc-protocol.md) | Binary, versioned IPC protocol | Small, quantifiable attack surface; strict validation on both sides. |
| [ADR-005](ADR-005-mupdf-as-initial-pdf-renderer.md) | Hayro as default PDF engine, MuPDF opt-in | MuPDF is AGPL; Hayro keeps the default build MIT-compatible. |
| [ADR-006](ADR-006-vm-layer-for-windows-macos.md) | QEMU VM layer for Windows/macOS | Same gVisor stack, now inside a disposable VM. |
| [ADR-007](ADR-007-ooxml-text-preview.md) | OOXML preview with container caps | Zip bombs and entity expansion need hard, pre-decode caps. |
| [ADR-008](ADR-008-ooxml-embedded-images.md) | Embedded office media preview | Bounded counts/bytes, sequential decode, same validation path. |
| [ADR-009](ADR-009-shared-memory-transport.md) | Shared-memory page transport (protocol v2) | 0-copy frames for the local dev backend. |
| [ADR-010](ADR-010-gif-tiff-bmp-epub-rtf-html.md) | GIF/TIFF/BMP images, EPUB/RTF/HTML text preview | Same boundaries absorb the formats users actually open; dispatch stays fail-closed. |