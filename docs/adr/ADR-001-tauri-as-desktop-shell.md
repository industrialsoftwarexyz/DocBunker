# ADR-001: Tauri as desktop shell

> Status: accepted and implemented (Phase 1).

## Context
We need a cross-platform (Windows/macOS/Linux) desktop shell with a small, controllable trust
surface. Options: Electron, Tauri, Qt, native per-platform. Electron ships a full Chromium+Node
runtime into a process that would have to be trusted; the document boundary would sit inside a huge
trusted blob.

## Decision
Use **Tauri 2** (Rust backend, system WebView frontend). The frontend is a Vue 3/TypeScript app
with a strict CSP and minimal capabilities (no fs, no shell, no HTTP permissions). All document
handling lives in Rust crates behind explicit commands; the WebView receives only raster images
and small DTOs.

## Alternatives considered
- Electron: mature but enormous trusted surface; rejected.
- Qt (native): fine, but a Rust-native stack keeps one language across the whole trusted side and
  simplifies the sandbox manager; rejected for velocity reasons, not security.
- Go-based shells: smaller ecosystem for this kind of desktop work; rejected.

## Security implications
The WebView and its rendering engine become a trusted component. We minimize exposure by:
- CSP that blocks all external origins and inline scripts in production.
- No plugin permissions beyond `core:default`.
- All file access behind backend commands; the UI never receives a usable path.
The WebView still parses our own HTML/CSS/JS and the PNGs *we* generate — both trusted inputs.

## Consequences
The app depends on the platform WebView (WebView2/WebKit/WKWebView); its security updates are
inherited from the OS. Tauri's IPC serialization (JSON for small DTOs) is acceptable; large buffers
already travel as base64 PNG and will move to binary channels/shared memory later (ADR).
