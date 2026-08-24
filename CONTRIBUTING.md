# Contributing to DocBunker

Thanks for helping. DocBunker parses **potentially malicious documents**, so
two rules matter more than everything else:

1. **Never let the trusted host parse document content.** All decoding happens
   in `renderer-worker` inside an isolated subprocess/VM; code that touches
   document bytes on the host side is a security bug.
2. **Hostile input must never panic.** The worker runs untrusted parsing; an
   uncontrolled panic is a denial-of-service and a correctness failure.

## Repository layout

See `README.md` → Repository layout and `docs/architecture.md` for the full
trust model. In short:

| Path | Role |
| --- | --- |
| `crates/protocol` | Versioned binary IPC (host ⇄ worker), framing + validation |
| `crates/renderer-api` | Domain types, `DocumentRenderer` trait, format detection, limits |
| `crates/sandbox` | Backends (`mock`, `subprocess`, `runsc`, `vm`), transports, OCI bundle |
| `crates/core` | Document manager, LRU cache, PNG encoding |
| `crates/renderer-*` | The untrusted renderers (image, ooxml, worker) |
| `src-tauri` | Tauri 2 shell: commands, native host, app config |
| `frontend` | Vue 3 + TypeScript UI (trusted) |
| `sandbox/` | Rootfs / VM image build scripts |
| `tests/` | End-to-end integration tests |

## Building and testing

Prerequisites: Rust (1.85+, the repo pins the toolchain), Node.js ≥ 20, and the
[Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your
platform.

```bash
npm --prefix frontend ci
npm --prefix frontend run build

cargo check --workspace --all-targets
cargo test --workspace

cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

All four must pass before a change is reviewable; CI enforces the same.

Run the app in dev mode:

```bash
npm --prefix frontend run dev          # terminal 1
cargo run --manifest-path src-tauri/Cargo.toml   # terminal 2
```

`DOCBUNKER_BACKEND=mock` is the debug default. For real decoding without
isolation use `DOCBUNKER_BACKEND=subprocess` (dev only). Production backends
(`runsc`, `vm`) need their runtimes and images; see `docs/sandbox.md`.

## Code conventions

- `unsafe` is banned workspace-wide (`unsafe_code = "forbid"` in
  `Cargo.toml`). A patch needing `unsafe` needs a documented, reviewed reason.
- Follow `cargo fmt` and keep `cargo clippy -- -D warnings` clean.
- No recoverable `panic!`/`expect`/`unwrap` on data that can be influenced by
  a document (that includes anything the worker sends over the wire).
- Define any new hard limit as a named constant next to the existing caps
  (`renderer-api/src/limits.rs`, `renderer-ooxml/src/lib.rs`); hostile sizes
  are validated **before** allocation, with checked arithmetic.
- Wire messages must be validated on **both** sides via
  `crates/protocol/src/validate.rs`; a wire value never becomes a domain
  value without passing through there.
- Binary/byte lengths are capped in bytes, not characters (error strings,
  text previews). Truncation must stay on UTF-8 char boundaries.
- Formatting of `.sh` scripts: LF endings, `sh` not `bash`, `set -eu`.
  `.gitattributes` enforces this.
- Behavior that affects the security profile needs an ADR update or a new ADR
  in `docs/adr/`; follow the format of the existing numbered records there.

## Testing expectations

- Unit tests: wire framing, validation, limits, format detection, renderer
  behavior with real fixtures plus malformed fixtures.
- Zip-bomb / entity-expansion tests must exist for every new container
  parser (see `renderer-ooxml`).
- Protocol fuzz targets live in `crates/protocol/fuzz` and
  `crates/renderer-api/fuzz`; run them before landing parser changes:
  `cargo +nightly fuzz run --manifest-path crates/protocol/fuzz/Cargo.toml message_validation -- ...`
- Backend end-to-end tests are opt-in (`-- --ignored`): `runsc_end_to_end`
  (needs `runsc` + root) and `qemu_vm_end_to_end` (needs QEMU + images).

## Security

- Report vulnerabilities privately: see `SECURITY.md`. Do **not** open public
  issues for exploitable findings.
- Randomize and bound everything a hostile file can influence; prefer failing
  closed (`DocBunkerError` etc. with a stable error code) over lenient paths.
- Never log document content or paths derived from attacker input into the
  app log loop (log `errors` sanitized).

## Pull request checklist

1. `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings` pass.
2. `cargo test --workspace` passes (including new tests for the change).
3. Frontend checks pass if you touched `frontend/`: `npm run lint`, `npm run build`.
4. Documentation updated: `docs/`, `docs/CHANGELOG.md` unreleased section, ADR if the security model changes.
5. No secrets, no host-side document parsing, no panics on hostile input.

Thanks for contributing.