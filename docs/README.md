# Documentation index

There is no strict reading order; most people start with
[overview.md](overview.md) which explains what happens when you open a
document, and then go where their interest takes them.

| Doc | What's in it |
| --- | --- |
| [overview.md](overview.md) | How the pieces fit together, with diagrams: opening a document, rendering a page, backend selection, the runsc and VM sandboxes. |
| [architecture.md](architecture.md) | Layer diagram, trust boundaries, what each crate is responsible for, session lifecycle, memory strategy. |
| [sandbox.md](sandbox.md) | The isolation layer in detail: the four backends, the OCI hardening profile, rootfs contents, VM image build. |
| [protocol.md](protocol.md) | The wire protocol: framing, handshake, messages, limits, error codes, validation rules. |
| [threat-model.md](threat-model.md) | What can go wrong (parser exploits, resource exhaustion, escape, IPC abuse) and how each is handled. |
| [roadmap.md](roadmap.md) | Phase-by-phase history (1–8), what is done and what is still missing, e.g. stale prebuilt sandbox images. |
| [adr/index.md](adr/index.md) | The decisions behind the architecture (001–009), with reasoning. |

Practically:

- To run it yourself, see "Trying it now" in [overview.md](overview.md).
- To report a security issue, see `SECURITY.md` (private channel).
- To contribute: `CONTRIBUTING.md`, then `docs/architecture.md` so you know
  which side of the boundary your change belongs on.