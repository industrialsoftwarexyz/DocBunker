# ADR-007: OOXML container formats — text-only preview (Phase 7)

> Status: accepted and implemented as a text-preview renderer behind the
> existing `DocumentRenderer` seam.

## Context

DocBunker opens PDF, PNG, JPEG and WebP. Threat model entry A5 ("zip bomb")
explicitly requires an ADR *before* any container format is enabled. Modern
Office files (`docx`, `pptx`, `xlsx`) are ZIP containers of OOXML markup, so
they are the natural first container formats. Full WYSIWYG rendering of OOXML
(paragraph layout, tables, charts, images) is a large, long-term project and
is **out of scope** here; the MVP is a safe text preview.

The renderer must satisfy the existing invariants (ADR-002): only validated
RGBA crosses the isolation boundary, nothing is ever extracted to host paths,
and all parsing stays inside the sandboxed worker.

## Decision

Add `DocumentFormat::Ooxml` (wire value 5) and a `renderer-ooxml` crate that:

1. **Detects** ZIP containers by magic (`PK\x03\x04`, content-based, per
   `renderer-api` rules) and requires `[Content_Types].xml` during open.
2. **Extracts text only** from the three subtypes:
   - `docx` → `word/document.xml` (`w:t` runs, `w:p` paragraph breaks),
   - `pptx` → `ppt/slides/slideN.xml` sorted numerically (`a:t`/`a:p`,
     slide separators),
   - `xlsx` → `xl/sharedStrings.xml` (`si/t` entries).
   Images, charts, macros, embedded OLE, links and every other container part
   are ignored — never decoded, never executed, never written.
3. **Enforces hard container caps** (the A5 mitigations) during extraction:

   | Cap | Value |
   | --- | --- |
   | ZIP entry count | ≤ 1 024 |
   | Uncompressed size per entry | ≤ 16 MiB |
   | Total uncompressed size | ≤ 64 MiB (= `MAX_DOCUMENT_SIZE`) |
   | Compression ratio (declared sizes) | ≤ 1 000× (+ 64 B slack) |
   | Extracted text | ≤ 200 000 chars |

   Reads are bounded with `Read::take`; the worker never allocates beyond a
   cap from header-declared sizes.
4. **Parses XML without entity expansion**: `quick-xml` in a non-validating
   mode that never expands DTD-defined or custom entities (`check_entities =
   false`, `GeneralRef` events dropped). Only the five predefined entities
   and numeric character references are decoded into text. This neutralizes
   billion-laughs-style documents: input is capped per entry, and nothing is
   ever recursively expanded. DTD declarations are ignored.
5. **Renders text as one A4-page RGBA raster** (1240×1754 @ 150 dpi) using a
   bundled variable-font instance of Roboto (SIL OFL 1.1, embedded in the
   worker binary), scaled to the requested target exactly like the PDF
   renderer. Long text is truncated to the page; the preview intentionally
   loses fidelity beyond that.

The worker dispatch (`AnyRenderer`) gains the `Ooxml` variant; the protocol,
host core and UI need no changes beyond the new format value and file-type
lists (dialog, file association, native host, extension).

## Alternatives considered

- **Full OOXML layout rendering**: multi-year library work; also creates a
  large attack surface in the untrusted worker. Deferred; the text preview
  keeps the raster-only boundary and the A5 mitigations in one small crate.
- **Render embedded images too**: more decoders in the worker for marginal
  preview value; rejected for MVP (ADR-002 keeps the boundary narrow).
- **System fonts**: the worker is isolated and must not touch host paths;
  the font is embedded in the binary instead.
- **Bitmap font**: smaller binary, but poor Latin-1 coverage and ugly previews;
  rejected in favor of an embedded TTF rasterized with `fontdue`.

## Security implications

- The ZIP and XML parsers run only inside the sandbox (worker), under the
  same transport validation as every other renderer.
- No archive member is ever written to disk; extraction is pure in-memory
  reads bounded by the caps above.
- Entities are never expanded, so zip-bomb and billion-laughs payloads fail
  closed with `InvalidDocument` / `ResourceLimitExceeded` (or a trivially
  small preview) — see the A5 row update in `docs/threat-model.md`.
- The bundled font is trusted static data inside the untrusted binary; it is
  never parsed from document bytes.

## Consequences

- `renderer-worker` grows by the font and two parser crates (`zip`, `quick-xml`,
  both pure Rust, MIT/Apache-2.0) plus `fontdue` (MIT/Apache-2.0).
- Prebuilt sandbox images (VM initramfs, runsc rootfs) embed the worker binary
  and must be rebuilt with the new worker before OOXML works in the isolated
  backends (`subprocess` dev backend works immediately).
- The wire protocol gains format value 5; existing encodings are unchanged.
- UI labels show "OOXML"; the preview shows extracted text only, one page.
