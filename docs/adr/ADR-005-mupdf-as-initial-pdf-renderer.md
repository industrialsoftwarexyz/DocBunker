# ADR-005: PDF rendering engine - Hayro by default, MuPDF opt-in

> Status: **accepted**. This ADR records the original MuPDF decision, the
> license conflict, and the permissive default that replaced it.

## Context

For PDF rasterization we need an engine that renders without executing
document scripts, embeds in a small Rust worker, and is compatible with
DocBunker's licensing policy (MIT, `copyleft = "deny"` in `deny.toml`).

## Decision (original)

Use **MuPDF**, behind the `DocumentRenderer` trait, rasterize-only, with
JavaScript/multimedia/URL/attachment/forms disabled. MuPDF was chosen for
maturity, hostile-input track record and predictable rasterization.

## Discovery: license conflict

The Rust `mupdf` crate (Artifex's official binding) and `mupdf-sys` are
**AGPL-3.0**. Linking them into the worker binary makes the distributed
binary AGPL-3.0 — incompatible with the MIT project policy and with
`copyleft = "deny"`. Building `mupdf-sys` also requires a full C toolchain
(`libclang` for bindgen plus `make` and a C compiler), which the default
developer experience does not include.

## Revised decision

1. The **default build does not include MuPDF**. PDF support exists behind the
   opt-in `pdf-mupdf` cargo feature (`default-features = false`, so MuPDF's
   JavaScript engine is **not** compiled), for AGPL-compliant deployments.
   `deny.toml` records a documented exception for the feature.
2. The **default engine is Hayro 0.4** (Apache-2.0), a pure-Rust renderer that
   keeps the distributed worker compatible with the project's license policy
   and static musl build. It is rasterize-only behind `DocumentRenderer`.
3. PDFium remains a possible future alternative if Hayro's compatibility is
   insufficient. The renderer seam means changing engines requires no
   protocol changes.
4. Poppler remains rejected (GPL, same copyleft problem).

## Alternatives considered

- MuPDF with commercial license: viable but adds a licensing process.
- Poppler: GPL; rejected.
- Custom parser: rejected (scope).
- PDF out of MVP: the goal explicitly includes PDF; deferred only until the
  engine toolchain/license is settled.

## Security implications

Whichever engine is used runs only inside the sandboxed worker (A1/A3 in the
threat model): unprivileged, no network, read-only rootfs, resource limits,
timeouts. Engine version pinning + advisory tracking is mandatory. Fuzzing the
engine happens inside the sandbox toolchain, never from the host.

## Consequences

- Default builds render PDF through Hayro.
- The `pdf-mupdf` feature exists and is CI-verified on Linux (Ubuntu provides
  `libclang`, `make`, `gcc`), but its binaries are AGPL.
- The worker remains raster-only and runs in the same gVisor/VM sandbox as the
  image renderers.
