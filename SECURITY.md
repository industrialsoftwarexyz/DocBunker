# Security Policy

## Isolation architecture

DocBunker is a defense-in-depth document viewer. The security property we target is:

> Parsing and rendering a malicious document must not compromise the host, even if the renderer
> stack (MuPDF, image decoders, font parsers, libc) is fully exploited.

This is achieved by keeping every parser in a disposable sandbox that is destroyed after use, and by
reducing what leaves the sandbox to a small, strictly validated binary protocol containing only
raster pixel data and a few numeric metadata fields.

## Trusted components

These components are trusted and must therefore stay small, audited and free of parsers:

- **Tauri UI / WebView** — displays only raster images produced by the sandbox; never document HTML.
- **`docbunker-native-broker`** — the only binary the Chrome extension can reach. A minimal
  native-messaging host with **no WebView, no Tauri and no document parsers**: it validates the
  hand-off (supported signature, size, path inside the allowed download directory), spawns the
  app, and waits for the app's acknowledgment. Release packages must ship it next to the app;
  registration refuses to point the browser at a missing broker.
- **Rust Core** — session lifecycle, page cache, PNG encoding of *our own* RGBA buffers. One
  sandbox session per open document; sessions and cached pages are destroyed on close and on
  manager shutdown.
- **Sandbox Manager** — creates/destroys sandboxes, enforces limits and timeouts, validates IPC.
- **QEMU + native hypervisor** — disposable outer VM boundary using WHPX,
  HVF or KVM, launched with `-nodefaults -monitor none -nic none -display none
  -no-reboot` (QMP never enabled) and (on Unix) QEMU's seccomp `-sandbox on`.
- **`runsc` / gVisor** — inner isolation boundary inside the minimal Linux
  guest.

## Untrusted components

- The **document bytes** themselves.
- The **`renderer-worker`** process and every library it links (image decoders
  `png`/`jpeg-decoder`/libwebp, MuPDF when the AGPL `pdf-mupdf` feature is
  enabled, musl libc, ...). The worker is treated as potentially compromised:
  it must run unprivileged, without network, with a read-only rootfs, resource
  limits, and a strict timeout on every operation.
- The **Chrome extension and Gmail content** — the extension can only ask the
  broker to open a supported, signature-checked file inside the user's
  Downloads folder (`DOCBUNKER_ALLOWED_OPEN_DIR` to override); it cannot read
  arbitrary paths.
- The **`SubprocessBackend`** mode (`DOCBUNKER_BACKEND=subprocess`) runs the
  worker with **no isolation** — development only; never with untrusted
  documents. Production builds default to `DOCBUNKER_BACKEND=vm` on every
  supported desktop platform; Linux can explicitly select `runsc` directly.

## Trust boundary rules

- No parser runs in the UI process or the main Rust process. Ever.
- The sandbox ↔ host protocol is versioned, size-limited, and validated on every message.
- The worker never receives paths, environment secrets, or network access.
- Host files are never mounted into the sandbox; document bytes travel over IPC only.
- All sandbox cleanup is guaranteed by the Sandbox Manager on crash, timeout and shutdown.
- Every open document owns a fresh, disposable sandbox session; sessions and page caches are
  destroyed on close and on manager shutdown (no cross-document sharing, no retained page data).
- The browser can only ever reach `docbunker-native-broker`, and only for the single validated
  "open this attachment" flow.
- Release artifacts are built with `--locked`, checked for reproducibility, and the VM assets are
  pinned by reviewed SHA-256 hashes verified before use and in CI.

## Supported versions

| Version | Status |
| --- | --- |
| 0.1.x (current) | Under development. No stable security guarantee yet. |

DocBunker does not claim to be 100% secure. See `docs/threat-model.md` for residual risks.

## Reporting a vulnerability

Do **not** open a public issue for security problems.

1. Send a report to the maintainers via a private channel (email address and PGP key will be published
   here once the project has an official distribution point).
2. Include: affected version, platform, a minimal reproducer (no more than needed), expected vs actual
   behavior, and your assessment of impact.
3. We aim to acknowledge within 5 business days and to coordinate disclosure.
4. If you report a bug found through fuzzing or manual auditing, please also state whether it affects
   the trusted or untrusted side of the boundary.

## Security-relevant changes

Any change touching the sandbox boundary, the protocol, the worker, or the trust model requires an ADR
(`docs/adr/`) and a threat-model review before merge.
