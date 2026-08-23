//! Build-time helper that renders the guest's OCI `config.json`.
//!
//! Used by `sandbox/scripts/build-vm-image.sh` (Linux build hosts). The
//! container rootfs path is written verbatim because the bundle lives inside
//! the VM initramfs and does not exist on the build host.
//!
//! The session limits baked into the image are the crate defaults; the host
//! backend sizes the VM from its own session config and enforces the same
//! limits on its side.

use std::path::PathBuf;

use docbunker_sandbox::config::SandboxConfig;
use docbunker_sandbox::runsc_bundle::OciBundle;

fn main() {
    let mut args = std::env::args().skip(1);
    let bundle_dir = args
        .next()
        .expect("usage: vm-image-builder <bundle-dir> <rootfs-path-in-guest>");
    let rootfs_in_guest = args
        .next()
        .expect("usage: vm-image-builder <bundle-dir> <rootfs-path-in-guest>");

    OciBundle::write_with_guest_rootfs(
        PathBuf::from(bundle_dir).as_path(),
        PathBuf::from(rootfs_in_guest).as_path(),
        &SandboxConfig::default(),
    )
    .expect("write guest oci config");
}
