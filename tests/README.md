# Test strategy

The `tests/` directory is a real Cargo crate (`docbunker-integration-tests`) so
the integration suite builds and runs with `cargo test --workspace`.

| Layer | Location | Examples |
| --- | --- | --- |
| Unit (protocol) | `crates/protocol/src` | framing round trips, caps, overflow, strict decode |
| Unit (renderer-api) | `crates/renderer-api/src` | format detection, limits, mock renderer |
| Unit (renderer-image) | `crates/renderer-image/src` | real PNG/JPEG/WebP decode, scaling, bomb limits |
| Unit (sandbox) | `crates/sandbox/src` | session lifecycle, transports, OCI bundle generation |
| Unit (core) | `crates/core/src` | manager flow, LRU cache, PNG encoding, error codes |
| Worker subprocess | `crates/renderer-worker/tests` | full protocol round trip over pipes with **real decoding**, truncated image, oversized frame, kill/EOF |
| Cross-crate | `tests/tests/scenarios.rs` | manager + backend + protocol together |
| Host vs misbehaving worker | `tests/tests/fake_worker_contract.rs` | invalid version response, timeout contract, silent worker |
| runsc (opt-in) | `crates/sandbox/src/platforms/linux.rs` | `runsc_end_to_end`, `#[ignore]`; requires runsc + rootfs (CI job) |
| runsc escape (opt-in) | `crates/sandbox/src/platforms/linux.rs` + `crates/sandbox/src/bin/escape-worker.rs` | `runsc_escape_test`, `#[ignore]`; adversarial worker attacks the unchanged OCI profile — see `docs/sandbox.md` |

## Fuzz

`crates/protocol/fuzz/` and `crates/renderer-api/fuzz/` contain cargo-fuzz
targets (framing/message decode/validation, format detection). They are **not**
workspace members; run them explicitly with nightly:

```bash
cargo +nightly install cargo-fuzz
cargo +nightly fuzz run --manifest-path crates/protocol/fuzz/Cargo.toml framing_decode
```

CI compiles the fuzz targets (`cargo fuzz build`) to keep them building.
Image/PDF codecs are **never** fuzzed from the host; any renderer fuzzing
happens inside the sandbox toolchain (Phase 4+).

## Mapping to the requested integration cases

- Valid PDF / valid image → decoded by the worker (`pdf-mupdf` feature for PDF,
  always-on image decoders), verified over the subprocess protocol.
- Corrupt PDF / truncated image → rejected with a safe error.
- Absurd dimensions → rejected by limits (`ResourceLimitExceeded`) at every
  layer, including a patched-IHDR PNG fixture.
- Dead renderer process → `killed_worker_surfaces_eof_and_nonzero_exit` (the
  contract the `runsc` backend maps to `SandboxCrashed`).
- Timeout → `worker_that_never_responds_triggers_host_timeout` (the mechanism
  `ProcessTransport` uses for wall-clock timeouts).
- Invalid IPC response → `invalid_response_version_is_rejected`.
