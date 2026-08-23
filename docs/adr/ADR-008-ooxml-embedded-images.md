# ADR-008: OOXML embedded images in the text preview (Phase 7)

> Status: accepted and implemented as an extension of the `renderer-ooxml`
> text preview (ADR-007).

## Context

The ADR-007 text preview deliberately ignored embedded media ("Render embedded
images too" was listed as a rejected alternative). Office documents carry
figures in `word/media/`, `ppt/media/` and `xl/media/`. Showing them makes the
preview materially more useful for documents whose content is mostly images.

The existing worker already decodes PNG/JPEG/WebP with hard caps
(`renderer-image`); the medium must stay a **validated RGBA raster** inside
the single preview page (ADR-002 boundary).

## Decision

- `OoxmlText` gains `media: Vec<Vec<u8>>` (encoded bytes only, validated
  against the same per-entry and total container caps already enforced).
- Only PNG/JPEG/WebP members under a `/media/` path are collected, with a
  cap of **64 images** per document; other media types (EMF, WMF, SVG, GIF,
  OLE) are ignored, never decoded.
- Decoding happens at **render time** (never during `open`), one image at a
  time, reusing `docbunker-renderer-image`'s public
  `decode_embedded` + `scaling::scale_rgba` (dimension caps and
  decompression-bomb protection identical to the standalone image renderer).
- Layout: images are drawn below the extracted text, scaled to the content
  width (aspect preserved), on hairline-bordered white fields; the page
  remains one A4 raster and truncates at the page boundary like text.
- Total decoded media memory is bounded: images are decoded, scaled and
  composited sequentially, so peak memory is one decoded image plus the page
  buffer (both already within the hard caps).

## Alternatives considered

- **Rendering full Office layout incl. images**: still out of scope
  (ADR-007).
- **Interleaving images at their in-document positions**: requires parsing
  the drawing/relationship markup (`r:embed`), which is a bigger OOXML
  surface with little preview benefit; images are grouped after the text.

## Security implications

- Same container caps as ADR-007 (per-entry and total uncompressed sizes,
  ratio, entry count) plus the media count/subset caps above.
- Decoders are the same audited ones as the image renderer, running in the
  same sandboxed worker; output is composited into the same validated pages;
  nothing extracted is ever written to disk.

## Consequences

- `renderer-ooxml` now depends on `renderer-image` (pure Rust decoders,
  already used by the worker).
- The wire protocol is unchanged: media stays inside the container and the
  preview is still a single validated `PageRenderedResponse`.