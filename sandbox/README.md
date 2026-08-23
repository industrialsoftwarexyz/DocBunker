# sandbox/

Runtime artifacts for the isolated rendering environment.

| Path | Purpose |
| --- | --- |
| `rootfs/` | Generated minimal Linux rootfs (Alpine + musl + fonts + `renderer-worker`). **Never commit generated content.** |
| `scripts/build-rootfs.sh` | Builds the rootfs from a pinned Alpine minirootfs. Linux only; Phase 4 deliverable. |
| `config/oci-config.json.tmpl` | OCI bundle template used by the `runsc` backend (Phase 4). |

## Current status (Phase 1)

The scripts and templates are written and documented but **not yet executed or wired**:
the mock backend does not use them. Running `build-rootfs.sh` now on Linux produces a rootfs
that is not yet consumed by any code — it is a Phase 4 deliverable. On non-Linux hosts the
script exits with a clear error.

## What the rootfs must contain (hardening rules)

Only:

- musl (static `renderer-worker` binary is preferred; nothing else needed at runtime)
- minimal fonts
- scaffolding directories (`/proc`, `/dev`, `/tmp`, `/etc`)

Explicitly **excluded**: SSH, curl, wget, compilers, Node, Python, package managers at runtime,
network services, daemons, admin tools.

## Security rules enforced by the manager (Phase 4)

- read-only rootfs; no host mounts
- unprivileged user, no capabilities
- `--network none`
- cgroup limits: memory, CPU, pids
- private size-capped tmpfs `/tmp`
- host-side wall-clock timeouts on every operation
- all `runsc` invocations with separated arguments (no shell)

See `docs/sandbox.md` for the full design.
