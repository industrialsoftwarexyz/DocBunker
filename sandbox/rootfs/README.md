# rootfs/
Generated content lives here (untracked, see `.gitignore`).

The rootfs is produced by `../scripts/build-rootfs.sh` and consumed by the
`runsc` backend (Phase 4). It must contain only musl, minimal fonts and the
statically-linked `renderer-worker` binary.
