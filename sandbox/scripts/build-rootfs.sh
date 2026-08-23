#!/usr/bin/env sh
#
# Build the minimal Alpine rootfs for the DocBunker renderer sandbox.
#
# Phase 4 deliverable: this script is written and documented but is not yet
# consumed by any code (the Phase 1 backend is the in-process mock).
#
# Requirements (Linux only):
#   - internet access to download the pinned Alpine minirootfs tarball
#   - sha256sum, tar, and the `renderer-worker` static binary
#     (built for the matching `*-unknown-linux-musl` target)
#
# Usage:
#   ./build-rootfs.sh <output-dir> [worker-binary]
#
# The result is a directory suitable for use as an OCI bundle rootfs by the
# runsc backend. It intentionally contains NO shell, NO package manager and
# NO network tools at runtime.

set -eu

# Pin a known-good Alpine release (minirootfs). Bump deliberately with review.
ALPINE_VERSION="3.20.3"
ALPINE_BRANCH="3.20"
VM_ARCH="${DOCBUNKER_VM_ARCH:-$(uname -m)}"
case "$VM_ARCH" in
  x86_64) ALPINE_ARCH="x86_64"; RUST_TARGET="x86_64-unknown-linux-musl" ;;
  aarch64|arm64) ALPINE_ARCH="aarch64"; RUST_TARGET="aarch64-unknown-linux-musl" ;;
  *) echo "error: unsupported VM architecture: $VM_ARCH" >&2; exit 1 ;;
esac
MINIROOTFS_URL="https://dl-cdn.alpinelinux.org/alpine/v${ALPINE_BRANCH}/releases/${ALPINE_ARCH}/alpine-minirootfs-${ALPINE_VERSION}-${ALPINE_ARCH}.tar.gz"
case "$ALPINE_ARCH" in
  x86_64) ALPINE_SHA256="d4e6fd67dcf75e40c451560ac7265166c2b72a0f38ddc9aae756a7de3d1efa0c" ;;
  aarch64) ALPINE_SHA256="041fa34a81788242df9e78fa69b97ab45b8ec47ddbf88864755610414a7bf3de" ;;
esac

OUT_DIR="${1:?usage: build-rootfs.sh <output-dir> [worker-binary]}"
WORKER_BIN="${2:-$(pwd)/../../target/${RUST_TARGET}/release/renderer-worker}"

case "$(uname -s)" in
  Linux) ;;
  *) echo "error: rootfs generation is Linux-only (Phase 4 deliverable)" >&2; exit 1 ;;
esac

command -v tar >/dev/null 2>&1 || { echo "error: tar not found" >&2; exit 1; }
command -v sha256sum >/dev/null 2>&1 || { echo "error: sha256sum not found" >&2; exit 1; }
[ -f "$WORKER_BIN" ] || { echo "error: worker binary not found at $WORKER_BIN (build with the musl target first)" >&2; exit 1; }

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"
TMP_TARBALL="$(mktemp)"

trap 'rm -f "$TMP_TARBALL"' EXIT

echo "Downloading Alpine minirootfs ${ALPINE_VERSION} (${ALPINE_ARCH})..."
curl -fsSL "$MINIROOTFS_URL" -o "$TMP_TARBALL" 2>/dev/null \
  || wget -q "$MINIROOTFS_URL" -O "$TMP_TARBALL" \
  || { echo "error: download failed (curl/wget only used at build time, never in the rootfs)" >&2; exit 1; }

echo "Verifying reviewed Alpine ${ALPINE_VERSION} checksum..."
echo "$ALPINE_SHA256  $TMP_TARBALL" | sha256sum -c - || { echo "error: checksum mismatch" >&2; exit 1; }

echo "Extracting..."
tar -xzf "$TMP_TARBALL" -C "$OUT_DIR"

echo "Pruning the distro rootfs to the static renderer boundary..."
rm -rf "$OUT_DIR/bin" "$OUT_DIR/dev" "$OUT_DIR/etc" "$OUT_DIR/home" \
  "$OUT_DIR/lib" "$OUT_DIR/media" "$OUT_DIR/mnt" "$OUT_DIR/opt" \
  "$OUT_DIR/proc" "$OUT_DIR/root" "$OUT_DIR/run" "$OUT_DIR/sbin" \
  "$OUT_DIR/srv" "$OUT_DIR/sys" "$OUT_DIR/tmp" "$OUT_DIR/usr" \
  "$OUT_DIR/var"
mkdir -p "$OUT_DIR/bin" "$OUT_DIR/usr/share/fonts"

echo "Installing renderer-worker..."
install -m 0755 "$WORKER_BIN" "$OUT_DIR/bin/renderer-worker"

# Scaffolding expected by the OCI runtime (mounted/created at runtime).
mkdir -p "$OUT_DIR/proc" "$OUT_DIR/dev" "$OUT_DIR/tmp" "$OUT_DIR/etc"
touch "$OUT_DIR/etc/os-release"

echo "Rootfs ready at $OUT_DIR"
du -sh "$OUT_DIR"
