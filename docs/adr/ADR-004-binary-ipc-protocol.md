# ADR-004: Binary IPC protocol

## Context
Host and worker must exchange documents (in) and raster pages (out). If this channel is JSON with
arbitrary structures, both sides get a parser with dynamic allocation and no natural size limits.

## Decision
Use a **versioned, binary, framed protocol** (`crates/protocol`) shared by host and worker:
- `[u32 LE payload_len][version: u8][discriminator: u8][postcard body]`
- `postcard` (serde-based, compact, strict — rejects trailing bytes) for struct serialization.
- Hard caps (`MAX_IPC_MESSAGE`, `MAX_DOCUMENT_SIZE`, `MAX_PIXEL_BUFFER`, ...) enforced before any
  attacker-influenced allocation, with checked arithmetic.
- A `Hello`/`HelloOk` handshake negotiates limits as the minimum of both sides' caps; every later
  message is validated by **both** sides.
- Transport-agnostic (`SandboxTransport`): pipes today, Unix sockets soon, `virtio-vsock`/
  `virtio-serial` behind a VM later.

## Alternatives considered
- JSON/msgpack with schema validation: rejected — dynamic, no natural caps, larger parsers.
- `bincode 2`: fine, but `postcard` is more widely audited and simpler for embedded-style payloads.
- Hand-rolled zero-dep codec: more code to audit; `postcard`+serde is a maintained, widely used
  base with strict behavior we control at the framing layer.

## Security implications
The protocol defines the complete set of operations the untrusted worker can express. Fuzzing
targets cover framing and decode (see `crates/protocol/fuzz/`). The worker cannot smuggle HTML,
paths or URLs through this channel because there is no such field, and unknown discriminators/
fields are rejected.

## Consequences
Small payload surface, explicit versioning, easy audit. Adding features later means extending the
message enums + bumping the protocol version when incompatible — a deliberate, reviewed process.
