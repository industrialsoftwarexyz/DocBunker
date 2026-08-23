#!/usr/bin/env sh
# Build the architecture-matched worker, rootfs, kernel bundle and initramfs.

set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"
OUTPUT_DIR="${1:-$ROOT_DIR/sandbox/vm}"
VM_ARCH="${DOCBUNKER_VM_ARCH:-$(uname -m)}"

case "$VM_ARCH" in
  x86_64) RUST_TARGET="x86_64-unknown-linux-musl" ;;
  aarch64|arm64) RUST_TARGET="aarch64-unknown-linux-musl" ;;
  *) echo "error: unsupported VM architecture: $VM_ARCH" >&2; exit 1 ;;
esac

KERNEL="${DOCBUNKER_VM_KERNEL_SOURCE:?set DOCBUNKER_VM_KERNEL_SOURCE}"
RUNSC="${DOCBUNKER_RUNSC_BIN:?set DOCBUNKER_RUNSC_BIN}"
BUSYBOX="${DOCBUNKER_BUSYBOX_BIN:?set DOCBUNKER_BUSYBOX_BIN}"
KERNEL_SHA256="${DOCBUNKER_VM_KERNEL_SHA256:?set DOCBUNKER_VM_KERNEL_SHA256}"
RUNSC_SHA256="${DOCBUNKER_RUNSC_SHA256:?set DOCBUNKER_RUNSC_SHA256}"
BUSYBOX_SHA256="${DOCBUNKER_BUSYBOX_SHA256:?set DOCBUNKER_BUSYBOX_SHA256}"
CARGO="${CARGO:-cargo}"

command -v sha256sum >/dev/null 2>&1 || { echo "error: sha256sum not found" >&2; exit 1; }

verify_artifact() {
  artifact_name="$1"
  artifact_path="$2"
  expected_sha256="$3"

  [ -f "$artifact_path" ] || { echo "error: $artifact_name not found at $artifact_path" >&2; exit 1; }
  [ "${#expected_sha256}" -eq 64 ] || { echo "error: invalid SHA-256 for $artifact_name" >&2; exit 1; }
  case "$expected_sha256" in
    *[!0-9A-Fa-f]*) echo "error: invalid SHA-256 for $artifact_name" >&2; exit 1 ;;
  esac
  printf '%s  %s\n' "$expected_sha256" "$artifact_path" | sha256sum -c -
}

verify_artifact "kernel" "$KERNEL" "$KERNEL_SHA256"
verify_artifact "runsc" "$RUNSC" "$RUNSC_SHA256"
verify_artifact "busybox" "$BUSYBOX" "$BUSYBOX_SHA256"

if [ -n "${DOCBUNKER_WORKER_BIN:-}" ]; then
  WORKER="$DOCBUNKER_WORKER_BIN"
else
  "$CARGO" build -p docbunker-renderer-worker --target "$RUST_TARGET" --release
  WORKER="$ROOT_DIR/target/$RUST_TARGET/release/renderer-worker"
fi

if [ -n "${DOCBUNKER_VM_IMAGE_BUILDER:-}" ]; then
  IMAGE_BUILDER="$DOCBUNKER_VM_IMAGE_BUILDER"
else
  "$CARGO" build -p docbunker-sandbox --bin vm-image-builder
  IMAGE_BUILDER="$ROOT_DIR/target/debug/vm-image-builder"
fi

ROOTFS="$ROOT_DIR/sandbox/tmp/rootfs-$VM_ARCH"
DOCBUNKER_VM_ARCH="$VM_ARCH" \
  "$ROOT_DIR/sandbox/scripts/build-rootfs.sh" "$ROOTFS" "$WORKER"
DOCBUNKER_VM_IMAGE_BUILDER="$IMAGE_BUILDER" \
  "$ROOT_DIR/sandbox/scripts/build-vm-image.sh" \
    "$OUTPUT_DIR" "$KERNEL" "$RUNSC" "$BUSYBOX" "$ROOTFS"

[ -f "$OUTPUT_DIR/kernel" ] || { echo "error: kernel was not generated" >&2; exit 1; }
[ -f "$OUTPUT_DIR/initramfs.cpio.gz" ] || { echo "error: initramfs was not generated" >&2; exit 1; }
(CDPATH= cd -- "$OUTPUT_DIR" && sha256sum kernel initramfs.cpio.gz > SHA256SUMS)

if [ "${DOCBUNKER_VM_E2E:-0}" = "1" ]; then
  DOCBUNKER_VM_KERNEL="$OUTPUT_DIR/kernel" \
  DOCBUNKER_VM_INITRD="$OUTPUT_DIR/initramfs.cpio.gz" \
    "$CARGO" test -p docbunker-sandbox qemu_vm_end_to_end -- --ignored
fi
