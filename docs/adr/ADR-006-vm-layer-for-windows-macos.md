# ADR-006: VM layer for Windows/macOS backends (Phase 6)

> Status: accepted and implemented as one cross-platform `QemuVmBackend`:
> QEMU + WHPX/HVF/KVM + prebuilt Linux initramfs, outer VM + inner gVisor.

## Context

On Windows and macOS the Linux `runsc` backend cannot run natively; the
architecture requires a minimal Linux VM as the *outer* isolation boundary,
with gVisor as the *inner* boundary:

```
Windows/macOS host
  └─ hypervisor (outer boundary)
       └─ minimal Linux guest (rootfs + renderer-worker)
            └─ runsc → renderer-worker   (inner boundary, ADR-003)
```

## Decision

**Evaluate before implementing** (per the roadmap), with this priority order:

1. **QEMU** as the guest launcher, accelerated by:
   - macOS: **Hypervisor.framework** (`hvf` accel) — native, no extra kernel
     module, sandbox entitlement required.
   - Windows: **WHPX** (Windows Hypervisor Platform).
   - Linux: **KVM**.
2. **Boot strategy**: a tiny initramfs containing the static `renderer-worker`
   (no full distro install), sharing the Phase 4 rootfs tooling.

The guest runs the same OCI bundle + `runsc` stack as the Linux backend; the
transport seam (`SandboxTransport`) already accommodates `virtio-vsock` /
`virtio-serial`, so protocol code does not change.

## Alternatives considered

- Raw `Virtualization.framework` / Hyper-V VMCX/WMI APIs without QEMU: more
  control but much more code to audit and maintain; deferred.
- A second `SandboxBackend` implementation directly over a VM library per
  platform: premature until the QEMU path is proven.
- Windows/macOS native sandboxing (AppContainer/seatbelt + deps): does not
  satisfy the "Linux + gVisor" requirement; rejected.

## Security implications

The hypervisor becomes an additional layer *outside* gVisor: a gVisor escape
inside the guest is contained by the VM; a VM escape is the residual Critical
risk on these platforms. The guest must be minimal (same hardening rules as
the rootfs), networkless (vsock/serial only), with memory/CPU caps set from
`SandboxConfig`.

## Consequences

The same backend and guest image contract are used on all desktop platforms.
Release automation must provide architecture-matched kernels and initramfs
files and execute the ignored host end-to-end test before publication.
