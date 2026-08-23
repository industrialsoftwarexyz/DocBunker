# VM runtime artifacts

Production bundles expect `kernel` and `initramfs.cpio.gz` in this directory.
Generate both with `sandbox/scripts/build-vm-assets.sh`; generated images are
ignored by Git and must match the target CPU architecture. See
`docs/sandbox.md` for required inputs.

QEMU is resolved from `DOCBUNKER_QEMU_BIN` or `PATH`. Redistributing QEMU in an
installer requires a separate license and dependency review.
