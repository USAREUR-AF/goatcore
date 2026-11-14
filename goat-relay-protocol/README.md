# goat-relay-protocol

Wire-protocol library for the Netbird relay (`rels://` over WebSocket and QUIC).
Clean-room reimplementation; Apache-2.0.

This crate is the parse / serialize / state-machine layer. It owns no I/O and
no async runtime — the async transport binary lives elsewhere (Block 33B work).

## Status

**Implemented, version 0.1.0.** The modules (framing, messages, handshake,
keepalive, transport) implement the spec doc, and the relay server `goat-relay`
is built on this crate and in service.

## Where to start reading

- Spec: [`../docs/design/netbird-relay-wire-protocol.md`](../docs/design/netbird-relay-wire-protocol.md)
- Working state: `../docs/project/block-33-progress.md`
- Plan + sub-block status: `../docs/project/implementation-plan.md` §"Block 33"

## Build

```
cargo check
cargo test
```

`cargo test` runs the unit tests. The conformance tool under `src/bin/` parses
each fixture frame, serializes it again and requires identical bytes.

## License

Apache-2.0. See [`LICENSE-APACHE`](LICENSE-APACHE).

The protocol semantics this crate implements derive from reading upstream
`netbirdio/netbird/` source — `relay/` (AGPLv3) for routing and lifecycle,
`shared/relay/messages/` (BSD-3-Clause) for wire-format. Per Oracle v Google
and Sega v Accolade, protocols are not copyrightable subject matter; this
crate transliterates no upstream code. See
[ADR 0110](../docs/adr/0110-license-posture-agplv3-forked-components.md) for
the project-wide license posture this crate sits within.
