# Threat model

Scope: the DocBunker application as described in `docs/architecture.md`. The attacker model is a
malicious document (PDF, PNG, JPEG, WebP) delivered to the viewer. The attacker may also control
the network-side content only if the host is already compromised — that case is out of scope; we
assume a clean host, but we do not assume the renderer stays clean.

Severity legend: **Critical** = host compromise, **High** = sandbox escape / resource exhaustion
of the host, **Medium** = DoS within the sandbox or protocol abuse, **Low** = annoyance /
information leak with limited impact.

---

## A. Parser exploits

### A1. RCE in the PDF engine
- **Threat**: A crafted PDF triggers an engine defect -> code execution in the renderer.
- **Attack surface**: PDF parser, font parsers, image decoders reachable from PDF, the rasterizer.
- **Mitigation**: Hayro (or opt-in MuPDF) runs only in the untrusted `renderer-worker` inside gVisor; the worker is
  unprivileged, has no capabilities, no network, read-only rootfs, memory/CPU/PID limits and a
  per-operation timeout. The only exfiltration channel is the tiny validated IPC protocol, which
  rejects anything but raster data. PDF features (JS, multimedia, URLs, attachments, forms) are
  disabled at compile/config time.
- **Residual risk**: RCE within the sandbox is expected and contained; only a sandbox escape
  matters (see A10). If MuPDF is statically linked, a bug can trigger parser defects we cannot
  patch without rebuilding — tracked via `cargo audit`/advisory feeds.

### A2. Exploit in a JPEG decoder
- **Threat**: Crafted JPEG exploits a decoder bug (e.g., in `libjpeg-turbo` or a pure-Rust decoder).
- **Attack surface**: JPEG entropy/coefficient decoding paths.
- **Mitigation**: Same sandbox containment as A1. Dimensions are validated before decode, output
  buffers are allocated with checked arithmetic, and the decoder is picked for minimal attack
  surface (pure-Rust, maintained — ADR). Truncated/oversized files are rejected.
- **Residual risk**: A decoder bug inside the sandbox is contained; decoder CVEs require a rebuild.

### A3. Malicious font
- **Threat**: A PDF embeds a crafted font that exploits the font rasterizer.
- **Attack surface**: TrueType/OpenType/CFF parsing in the renderer.
- **Mitigation**: Font parsing happens only in the sandboxed worker. Only minimal system fonts are
  present in the rootfs; document fonts are parsed by the PDF engine under the sandbox limits. Host
  fonts are never mounted.
- **Residual risk**: Contained to the sandbox; a font bug that survives gVisor would be Critical.

### A4. Decompression bomb (images)
- **Threat**: A tiny PNG/JPEG/WebP expands to gigabytes of pixel data (e.g., huge dimensions or
  pathological coefficients).
- **Attack surface**: Decoder allocation paths driven by file-declared dimensions.
- **Mitigation**: Hard caps `MAX_PAGE_WIDTH/HEIGHT`, `MAX_PIXEL_BUFFER` enforced before allocation;
  content-based format detection before decode; checked arithmetic everywhere; worker memory limit.
- **Residual risk**: A decompressor that allocates *before* we can check (e.g., progressive scan
  buffering) could hit the worker's memory cgroup limit and kill it — contained, reported as
  `ResourceLimitExceeded`.

### A5. Zip bomb (Office/document containers)
- **Threat**: A container (ZIP-based) expands to a huge tree.
- **Attack surface**: OOXML container parser (`renderer-ooxml`, ADR-007).
- **Mitigation**: Implemented with hard caps: entry count ≤ 1 024, declared and
  actual per-entry output ≤ 16 MiB, actual total output ≤ 64 MiB, declared and
  actual compression ratio ≤ 1 000×, bounded reads (`Read::take`), and XML
  parsing without entity expansion (billion-laughs safe). Only text parts are
  ever read; nothing is extracted to disk. ADR-007 precedes the feature.
- **Residual risk**: A parser bug in `zip`/`quick-xml` is contained by the
  sandbox (worker memory limits); actual output accounting does not trust ZIP
  size declarations.

---

## B. Resource exhaustion

### A6. Integer overflow
- **Threat**: `width * height * bytes_per_pixel` or length arithmetic overflows, causing
  undersized allocation and a buffer overflow.
- **Attack surface**: Any size computation derived from wire data.
- **Mitigation**: All such math uses `usize::checked_mul/add` (`renderer-api::limits::pixel_buffer_size`),
  and every wire message is validated against negotiated caps before use. `panic = "abort"` in
  release builds turns a hypothetical overflow panic into an aborted worker, not undefined behavior.
- **Residual risk**: Low; a missed site is caught by fuzzing (protocol targets) and review.

### A7. Excessive memory consumption
- **Threat**: The renderer allocates until the host OOM-kills processes outside the sandbox.
- **Attack surface**: Renderer allocations, protocol buffers.
- **Mitigation**: Worker memory cgroup limit; `MAX_PIXEL_BUFFER`/`MAX_IPC_MESSAGE` caps; the frame
  cap bounds every wire allocation; the host LRU cache is bounded to 3 pages.
- **Residual risk**: On non-Linux backends (VM in Phase 6) the guest memory cap must be tuned; the
  host-side allocations are bounded by the caps above.

### A8. Fork bomb
- **Threat**: Worker spawns processes until the host is exhausted.
- **Attack surface**: `fork`/`exec` from the renderer.
- **Mitigation**: The worker never executes commands by design (no shell, no subprocess API used);
  the sandbox sets `pids.max`; process creation inside gVisor is interposed.
- **Residual risk**: Low; enforced by both the OCI pid limit and gVisor process control.

### A9. CPU exhaustion
- **Threat**: A pathological document spins the renderer for minutes (bzip loops, huge page count).
- **Attack surface**: Decoder/Rasterizer hot loops; "render page 0 of 100 000" requests.
- **Mitigation**: Per-operation host-side timeouts (open/info/render/close); `MAX_PAGE_COUNT` cap;
  CPU quota in the sandbox; cancellable close (kill + delete on timeout).
- **Residual risk**: Timeouts are wall-clock; a workload that stays under the timeout but at high
  CPU can still consume quota until it is finished — bounded by the CPU cgroup.

---

## C. Filesystem attacks

### A10. Sandbox escape (kernel/gVisor)
- **Threat**: A renderer exploit escapes gVisor and executes on the host kernel.
- **Attack surface**: gVisor syscall translation, host kernel bugs reachable via gVisor.
- **Mitigation**: Defense in depth — escape alone does not grant privileges (unprivileged user,
  no capabilities, no network, read-only rootfs, no host mounts, no device nodes). A host kernel
  bug that bypasses gVisor and is combined with a privilege vector would be Critical. On
  Windows/macOS the outer VM boundary adds another layer.
- **Residual risk**: gVisor is the main boundary; a gVisor-escape vulnerability in the wild is
  treated as Critical and drives an immediate update.

### A11. Path traversal
- **Threat**: A crafted document references `../../etc/passwd` (e.g., PDF file-spec links) to read
  or write host files.
- **Attack surface**: Any path handling inside the renderer or future code that mirrors document paths.
- **Mitigation**: The worker never receives paths and has no filesystem access to host data (read-only
  rootfs, no host mounts). File selection on the host uses a dialog + generated internal IDs, never
  attacker-derived paths. No document value is ever used as a path.
- **Residual risk**: Low; remains relevant if a future feature ever writes extracted content.

### A12. Symlink attack
- **Threat**: A symlink planted in a writable location (or a future extracted document) redirects
  writes to a host file.
- **Attack surface**: Writable paths in the sandbox (tmpfs `/tmp` only).
- **Mitigation**: `/tmp` is a private, size-capped tmpfs owned by the worker uid; no host directory
  is writable or mounted; cleanup uses generated names. Future extraction features must open files
  with `O_NOFOLLOW` and within the capped tmpfs.
- **Residual risk**: Low until extraction features exist.

### A13. TOCTOU on file/directory handling
- **Threat**: A race between checking a sandbox path and using it lets an attacker swap in a
  different target.
- **Attack surface**: Sandbox bundle preparation and cleanup.
- **Mitigation**: Bundle directories are created with `0700` under a per-session temp dir and are
  only touched by the manager process; cleanup is done via delete with generated names; no user
  input is used in paths. Phase 4 will use `openat`/`O_NOFOLLOW` semantics where the host OS allows.
- **Residual risk**: Low; requires a concurrently malicious process on the host, which is out of
  scope (host is trusted).

### A14. Leaking the host filesystem
- **Threat**: The renderer reads host files (via a missing read-only rootfs or a bind mount).
- **Attack surface**: Mount configuration; IPC content.
- **Mitigation**: Read-only rootfs, zero host mounts, empty device list; the only data into the
  sandbox is the document byte stream over IPC; the only data out is validated RGBA + minimal
  numeric metadata. Enforced by OCI config and asserted in tests.
- **Residual risk**: Low; depends on correct Phase 4 mount config (reviewed + integration-tested).

---

## D. IPC / protocol attacks

### A15. Malformed IPC / protocol confusion
- **Threat**: A compromised or buggy renderer sends oversized, misordered or type-confused messages
  to the host, hoping to crash the host or bypass validation.
- **Attack surface**: Host-side decode path; message dispatch.
- **Mitigation**: Strict framing (`u32` length cap), version byte, discriminator whitelist, strict
  postcard decoding (trailing bytes rejected), semantic validation of every field against
  negotiated caps, checked arithmetic before allocation, and exact request/response correlation
  for nonces, document ids and page numbers. Any violation terminates the worker session.
- **Residual risk**: Low; a host-side bug in validation would be High — mitigated by fuzzing the
  decode path.

### A16. Compromised renderer (protocol-layer abuse)
- **Threat**: After gaining code execution in the worker, the attacker uses the IPC channel as the
  only channel to the host and abuses it (e.g., floods, corrupt responses, lies about metadata).
- **Attack surface**: Everything the host does with worker responses.
- **Mitigation**: The host validates every response structurally and semantically; a capacity-one
  response channel rejects floods and unsolicited frames; one deadline covers request write,
  flush and response read. Page buffers are copied out of the worker's authority and the host
  never uses worker-provided lengths without validation.
- **Residual risk**: Data integrity of the preview is not protected (a compromised renderer can show
  wrong images) — acceptable and documented; it cannot escalate.

### A17. IPC buffer over-allocation on the host
- **Threat**: A response announces a huge buffer size, causing a host-side allocation before validation.
- **Attack surface**: Host `PageRendered` handling.
- **Mitigation**: The frame cap and capacity-one host queue bound wire allocations; semantic validation
  (`stride ≥ width×4`, `len == stride×height`, caps) happens before any page buffer copy; all math
  is checked.
- **Residual risk**: Low.

---

## E. Host / UI attacks

### A18. Environment variable leakage
- **Threat**: The renderer reads host environment variables (API keys, tokens).
- **Attack surface**: Process environment inheritance.
- **Mitigation**: The worker is spawned with an **empty** environment (Phase 4); Phase 1's
  in-process mock never reads env. Asserted by a test.
- **Residual risk**: Low; must be re-verified when the Linux backend lands.

### A19. Accidental networking
- **Threat**: A compromised renderer phones home with host data.
- **Attack surface**: Any network syscall from the worker.
- **Mitigation**: `--network none` (no netns connectivity) at the sandbox; the worker links no
  networking crates; it cannot open sockets (no socket APIs used, gVisor interposes network
  syscalls anyway). No HTTP client exists in the trusted code outside the Tauri CSP (which blocks
  all external origins).
- **Residual risk**: Very low on Linux; the VM backends (Phase 6) must configure the guest with no
  network (virtserial/vsock only).

### A20. Clipboard interaction
- **Threat**: The renderer writes attacker-controlled text to the host clipboard (paste attacks).
- **Attack surface**: Clipboard APIs if the worker had access to them.
- **Mitigation**: The worker has no clipboard access (no display, no Wayland/X, no windowing);
  clipboard is only reachable from the trusted UI, which never inserts document-derived text.
- **Residual risk**: Low. Future "copy text" features must go through the sandbox as validated,
  length-capped plain text with an ADR.

### A21. Shell execution
- **Threat**: Renderer or host executes attacker-controlled strings via a shell.
- **Attack surface**: Any `Command` invocation with string interpolation.
- **Mitigation**: No shell is used anywhere. `runsc` is invoked with `std::process::Command` and
  separated arguments; no user/document data ever appears in an argument or environment. The
  worker links no process-spawning code.
- **Residual risk**: Low; enforced by code review and `forbid(unsafe_code)` + dependency policy.

### A22. Malicious filename
- **Threat**: A filename like `-rf` or one containing control characters is used unsafely, or is
  shown to the user in a way that misleads (spoofing) or triggers UI injection.
- **Attack surface**: Command arguments, UI rendering, logging.
- **Mitigation**: Filenames are never used as arguments or paths inside the sandbox; internal IDs
  are program-generated. The UI renders the filename as plain text (no HTML injection). Logs avoid
  full user paths by default. The backend sanitizes the displayed name for UI display.
- **Residual risk**: Low; a misleading display name (homoglyphs) remains a social-engineering
  concern.

### A23. WebView / UI compromise
- **Threat**: A bug in the WebView engine (Chromium/WebKit), a compromised frontend dependency, or
  a confused-deputy chain turns malicious display content into code execution in the UI process.
  Because DocBunker runs the WebView in the same process as file access, the sandbox manager and the
  native-messaging bridge, such an RCE would grant access to *everything trusted*.
- **Attack surface**: The web frontend, WebView engine, Tauri IPC, and any native code reachable
  from `tauri::command`s.
- **Mitigation**: The frontend only receives raster PNG data URLs and status metadata — it never
  parses document content. CSP blocks all external origins; navigation is pinned to the app origin;
  capabilities are explicit. The extension-facing surface is split out into the dedicated
  `docbunker-native-broker` binary (no WebView, no document parsers). The **architecture goal** is a
  hard process split: a minimal UI process that holds no file/sandbox privileges next to a core
  process that owns the sandbox manager. Until then this is the largest residual trusted surface.
- **Residual risk**: **High** while the WebView shares a process with the trusted core; a WebView
  RCE is treated as critical host compromise.

---

## Summary table

| ID | Threat | Mitigation layer | Severity if mitigated |
| --- | --- | --- | --- |
| A1 | PDF engine RCE | sandbox | contained |
| A2 | JPEG decoder exploit | sandbox | contained |
| A3 | malicious font | sandbox | contained |
| A4 | decompression bomb | limits + worker caps | contained |
| A5 | zip bomb (OOXML) | container caps + no entity expansion | contained |
| A6 | integer overflow | checked arithmetic + fuzz | low |
| A7 | excessive memory | caps + cgroup + LRU | low |
| A8 | fork bomb | pid cap + no exec | low |
| A9 | CPU exhaustion | timeouts + cpu quota | low |
| A10 | sandbox escape | gVisor + defense in depth | critical (residual) |
| A11 | path traversal | no paths in worker | low |
| A12 | symlink attack | private tmpfs + O_NOFOLLOW | low |
| A13 | TOCTOU | 0700 dirs + generated names | low |
| A14 | host fs leak | read-only rootfs, no mounts | low |
| A15 | malformed IPC | strict framing + validation | low |
| A16 | compromised renderer | response validation | low |
| A17 | IPC over-allocation | frame cap + pre-validate | low |
| A18 | env leakage | empty env in worker | low |
| A19 | accidental networking | no netns + no socket API | very low |
| A20 | clipboard | no access in worker | low |
| A21 | shell execution | no shell, separated args | low |
| A22 | malicious filename | generated IDs, plain-text UI | low |
| A23 | WebView / UI compromise | smallest possible UI surface + process split (goal) | critical (residual) |
