#!/usr/bin/env sh
#
# Build the initramfs for the cross-platform QEMU VM backend.
#
# The image contains a minimal Linux guest: the Alpine rootfs built by
# build-rootfs.sh (with /bin/renderer-worker), the static runsc binary, a
# static busybox (only used by /init), and the hardened OCI config.json
# rendered by the vm-image-builder helper. The guest /init mounts the base
# filesystems and execs `runsc` over the virtio-serial console, giving the
# same inner gVisor boundary as the Linux backend (ADR-003), with the VM as
# the outer boundary (ADR-006).
#
# Requirements (Linux build host only):
#   - the repo toolchain (cargo) to build the vm-image-builder helper
#   - the artifacts below; see docs/sandbox.md for pinned sources
#
# Usage:
#   ./build-vm-image.sh <output-dir> <kernel> <runsc> <busybox-static> <rootfs-dir> [modules-dir]
#
# Output: <output-dir>/initramfs.cpio.gz (boot with `-kernel <kernel> -initrd <initramfs>`).
#
# The kernel must have virtio_console support; if it ships as a module, pass
# the kernel's /lib/modules directory as [modules-dir] and /init will load it.

set -eu

case "$(uname -s)" in
  Linux) ;;
  *) echo "error: VM image generation is Linux-only" >&2; exit 1 ;;
esac

OUT_DIR="${1:?usage: build-vm-image.sh <output-dir> <kernel> <runsc> <busybox-static> <rootfs-dir> [modules-dir]}"
KERNEL="${2:?usage: build-vm-image.sh <output-dir> <kernel> <runsc> <busybox-static> <rootfs-dir> [modules-dir]}"
RUNSC="${3:?usage: build-vm-image.sh <output-dir> <kernel> <runsc> <busybox-static> <rootfs-dir> [modules-dir]}"
BUSYBOX="${4:?usage: build-vm-image.sh <output-dir> <kernel> <runsc> <busybox-static> <rootfs-dir> [modules-dir]}"
ROOTFS="${5:?usage: build-vm-image.sh <output-dir> <kernel> <runsc> <busybox-static> <rootfs-dir> [modules-dir]}"
MODULES_DIR="${6:-}"

for tool in cpio gzip find; do
  command -v "$tool" >/dev/null 2>&1 || { echo "error: $tool not found" >&2; exit 1; }
done

for artifact in "$KERNEL" "$RUNSC" "$BUSYBOX"; do
  [ -f "$artifact" ] || { echo "error: missing artifact: $artifact" >&2; exit 1; }
done
[ -f "$ROOTFS/bin/renderer-worker" ] || { echo "error: rootfs has no /bin/renderer-worker (run build-rootfs.sh first)" >&2; exit 1; }
[ -n "$MODULES_DIR" ] && [ -d "$MODULES_DIR" ] || { [ -z "$MODULES_DIR" ] || { echo "error: modules dir not found: $MODULES_DIR" >&2; exit 1; }; }

INITRAMFS_DIR="$OUT_DIR/initramfs"
rm -rf "$INITRAMFS_DIR"
rm -f "$OUT_DIR/kernel" "$OUT_DIR/initramfs.cpio.gz"
mkdir -p "$OUT_DIR" "$INITRAMFS_DIR/bin" "$INITRAMFS_DIR/bundle" \
  "$INITRAMFS_DIR/proc" "$INITRAMFS_DIR/sys" "$INITRAMFS_DIR/dev" \
  "$INITRAMFS_DIR/state" "$INITRAMFS_DIR/tmp" "$INITRAMFS_DIR/run"

echo "Assembling initramfs in $INITRAMFS_DIR..."
install -m 0755 "$BUSYBOX" "$INITRAMFS_DIR/bin/busybox"
install -m 0755 "$RUNSC" "$INITRAMFS_DIR/runsc"
cp -a "$ROOTFS/." "$INITRAMFS_DIR/bundle/rootfs"
if [ -n "$MODULES_DIR" ]; then
  mkdir -p "$INITRAMFS_DIR/lib"
  cp -a "$MODULES_DIR" "$INITRAMFS_DIR/lib/modules"
fi

echo "Rendering the guest OCI config.json..."
if [ -n "${DOCBUNKER_VM_IMAGE_BUILDER:-}" ]; then
  BUNDLE_ARG="$INITRAMFS_DIR/bundle"
  case "$DOCBUNKER_VM_IMAGE_BUILDER" in
    *.exe) BUNDLE_ARG="$(wslpath -w "$BUNDLE_ARG")" ;;
  esac
  "$DOCBUNKER_VM_IMAGE_BUILDER" "$BUNDLE_ARG" "/bundle/rootfs"
else
  cargo run -q -p docbunker-sandbox --bin vm-image-builder -- \
    "$INITRAMFS_DIR/bundle" "/bundle/rootfs"
fi

echo "Writing /init..."
cat > "$INITRAMFS_DIR/init" <<'EOF'
#!/bin/busybox sh
# Linux cannot pivot directly away from rootfs. Move PID 1 onto a real tmpfs
# root so runsc can establish its defensive minimal chroot and user namespace.
if [ "${DOCBUNKER_STAGE2:-}" != "1" ]; then
  /bin/busybox mkdir -p /newroot
  /bin/busybox mount -t tmpfs -o size=256m,mode=0755 tmpfs /newroot
  /bin/busybox cp -a /bin /bundle /runsc /init /newroot/
  /bin/busybox mkdir -p /newroot/proc /newroot/sys /newroot/dev \
    /newroot/state /newroot/tmp /newroot/run
  export DOCBUNKER_STAGE2=1
  exec /bin/busybox switch_root /newroot /init
fi
/bin/busybox mount -t proc none /proc
for limit in /proc/sys/user/max_*_namespaces; do
  echo 65536 > "$limit"
done
[ -e /proc/sys/kernel/unprivileged_userns_clone ] && \
  echo 1 > /proc/sys/kernel/unprivileged_userns_clone
[ -e /proc/sys/kernel/apparmor_restrict_unprivileged_userns ] && \
  echo 0 > /proc/sys/kernel/apparmor_restrict_unprivileged_userns
/bin/busybox mount -t sysfs none /sys
/bin/busybox mount -t devtmpfs none /dev
/bin/busybox mkdir -p /dev/pts /sys/fs/cgroup /state /tmp /run
/bin/busybox mount -t devpts devpts /dev/pts
# Use one unified hierarchy; modern runsc and kernels negotiate cgroup v2.
/bin/busybox mount -t cgroup2 none /sys/fs/cgroup
/bin/busybox mount -t tmpfs -o size=128m,mode=0700 tmpfs /tmp

# virtio-serial console may ship as a module on some kernels.
if [ ! -c /dev/vport0p1 ]; then
  /bin/busybox insmod /lib/modules/*/kernel/drivers/char/virtio_console.* 2>/dev/null || true
  /bin/busybox sleep 1
fi
if [ ! -c /dev/vport0p1 ]; then
  echo "fatal: /dev/vport0p1 not available" > /dev/ttyS0
  /bin/busybox poweroff -f
fi

# The virtio-serial port is exclusive: open it once and dup stdin to stdout
# (stderr stays on the serial console for the host log).
exec 3<> /dev/vport0p1
exec /runsc --root=/state --network=none --log=/state/runsc.log \
  run --bundle /bundle docbunker-vm <&3 >&3
EOF
chmod 0755 "$INITRAMFS_DIR/init"

echo "Packing initramfs..."
( cd "$INITRAMFS_DIR" && find . -print0 | sort -z | cpio -o -0 -H newc 2>/dev/null | gzip -9 ) \
  > "$OUT_DIR/initramfs.cpio.gz"
cp "$KERNEL" "$OUT_DIR/kernel"
rm -rf "$INITRAMFS_DIR"

echo "VM image ready:"
echo "  kernel:            $OUT_DIR/kernel"
echo "  initramfs:         $OUT_DIR/initramfs.cpio.gz"
du -h "$OUT_DIR/kernel" "$OUT_DIR/initramfs.cpio.gz"
