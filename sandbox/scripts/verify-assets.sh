#!/usr/bin/env sh
#
# Verify release sandbox assets against the pinned SHA256SUMS manifest.
#
# DocBunker ships `sandbox/vm/kernel` and `sandbox/vm/initramfs.cpio.gz` with a
# reviewed SHA256SUMS (provenance from upstream release channels, never from
# the same transport that fetched the artifacts). This script is the release
# gate: distribution builds must not ship artifacts whose hashes drifted.
#
# Usage:
#   ./verify-assets.sh [assets-dir]   (default: sandbox/vm)
#
# Exit status is non-zero if any expected file is missing or mismatched.

set -eu

ASSETS_DIR="${1:-$(CDPATH= cd -- "$(dirname -- "$0")/../vm" && pwd)}"
MANIFEST="$ASSETS_DIR/SHA256SUMS"

[ -f "$MANIFEST" ] || { echo "error: $MANIFEST not found" >&2; exit 1; }
command -v sha256sum >/dev/null 2>&1 || { echo "error: sha256sum not found" >&2; exit 1; }

echo "Verifying sandbox assets in $ASSETS_DIR ..."
# `sha256sum -c` checks every file listed in the manifest (the `*` marker is
# the binary-mode prefix, accepted as an entry field) and fails on missing
# files by default — a removed or renamed asset fails the gate.
(
  cd "$ASSETS_DIR"
  sha256sum -c "$MANIFEST" >/dev/null
) || { echo "error: sandbox asset verification failed" >&2; exit 1; }

echo "OK: all sandbox assets match the pinned SHA256SUMS manifest"