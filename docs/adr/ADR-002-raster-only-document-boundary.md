# ADR-002: Raster-only document boundary

## Context
What may cross the sandbox boundary back to the host? Returning the original document, HTML, SVG,
PDF objects, extracted text, or URLs would each be a new attack surface in the trusted side. We
want the minimum data needed to *display* a document.

## Decision
The boundary carries only:
- **RGBA raster buffers** (`Rgba8888`, capped dimensions/bytes, validated),
- **minimal numeric metadata** (`page_count`, `width`, `height`, `format`).

No HTML, JS, active SVG, PDF objects, URLs, attachments, or arbitrary files are ever returned.
The frontend renders buffers as `<img>` with `data:` URLs of PNGs we encode ourselves; it never
parses document content.

## Alternatives considered
- Return PDF.js-parseable output: rejected (JS parsing in the WebView = huge trusted parser).
- Return extracted text/SVG for overlays: rejected for MVP; adds parsing surface and text
  exfiltration concerns. Revisited only behind a new ADR with strict validation.

## Security implications
Removes whole classes of UI-side injection (document HTML, SVG script, URL auto-open). The only
untrusted data the host decodes is *our own* PNG encoding of validated RGBA — and the WebView only
rasterizes it.

## Consequences
No search/select-text/copy in the MVP. Zoom is raster-only (image scaling, not re-layout).
Metadata extraction is intentionally minimal, which also keeps the worker smaller.
