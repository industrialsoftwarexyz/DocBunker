# ADR-010: Additional document formats — GIF/TIFF/BMP images, EPUB/RTF/HTML text preview

> Status: accepted. Detection and renderers are implemented and tested; the worker
> dispatch does not route them yet (they fail closed with `UnsupportedFormat`).

## Context

The viewer opened PDF, PNG/JPEG/WebP and Office containers. Users tried to open
other formats they consider ordinary: screenshots in GIF/BMP, scanned TIFF files,
e-books, RTF exports and saved HTML pages. Each of these is either a raster
format (GIF/TIFF/BMP) or a text container (EPUB is a ZIP; RTF/HTML are markup),
so both existing renderer families can absorb them without a new boundary type.

Threat-model A5 requires hard container caps before any ZIP-based format is
enabled, and ADR-002 still governs what may cross back to the host.

## Decision

1. **Detection first** (`renderer-api::format::detect`): GIF by `GIF87a`/`GIF89a`,
   TIFF by `II*\0`/`MM\0*`, BMP by `BM`, RTF by `{\rtf`, HTML by a
   case-insensitive `<html` / `<!doctype html` prefix. EPUB is told apart from
   OOXML by a bounded central-directory scan: `META-INF/container.xml` or
   `mimetype` without `[Content_Types].xml` means EPUB; anything else fails
   closed as unknown.
2. **GIF/TIFF/BMP decode** via the `image` crate (0.25) compiled with exactly the
   `gif`, `tiff`, `bmp` features — one audited pure-Rust decoder family,
   feature-gated so no other codec is linked. Same dimension caps and
   decompression-bomb protection as PNG/JPEG/WebP (`decode_embedded`,
   `scale_rgba`).
3. **EPUB/RTF/HTML** join `renderer-ooxml` as text-preview renderers behind the
   same ADR-007 container caps (entry count, per-entry and total uncompressed
   sizes, compression ratio, bounded reads) and the same 200 000-character text
   cap. HTML is parsed without executing anything and without loading external
   resources; output remains one validated A4 RGBA page.
4. Nothing about the wire protocol changes beyond new `format` values
   (6–11, see `docs/protocol.md`).

## Alternatives considered

- Shipping them wired end-to-end in the same step: rejected; the decoders land
  reviewed and tested first, the dispatch switch is a separate decision.
- A separate crate per family: unnecessary; both fit the existing crates'
  responsibility without new boundaries.
- libgif/libtiff system libraries: more C surface in the untrusted worker;
  rejected for the same reason ADR-005 prefers pure-Rust engines.

## Security implications

All parsing stays inside the sandboxed worker under the usual profile
(unprivileged user, no network, read-only rootfs, cgroup limits, per-operation
timeouts). The container caps apply to EPUB identically; RTF/HTML parsing never
expands entities or fetches referenced resources, so billion-laughs-style and
SSRF-style documents have nothing to act on. Until the dispatch routes these
formats, opening such a file returns `UnsupportedFormat` — no new code path is
reachable from a document today.

## Consequences

- The worker binary grows by the three `image` codec features and the two
  additional parsers once the formats are enabled.
- File-type lists (dialog filter, associations, native host, extension) must be
  updated when the dispatch switch happens, not before.
- Prebuilt rootfs/initramfs images need a rebuild at that point, as with any
  worker change.
