# ADR 1264 — A microcontroller is an attached device of one goat node, not a peer and not a build target

| Field | Value |
|---|---|
| Status | Accepted (2026-11-27, operator: "proceed with these"). Nothing changes in a goatnet on merge: no firmware, role or check is added by this record; the goatbuilder check of Decision 7 is designed, not built |
| Date | 2026-11-26 |
| Deciders | operator |
| Repo scope | goat core: `docs/design/goatbuilder-build-targets.md`; a firmware repository to be created by a later decision |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1257](1257-every-machine-kind-is-a-goatbuilder-build-target-type-declared-as-data.md) (machine kinds), [ADR 1248](1248-goat-follows-stock-upstream-netbird-and-wireguard-and-retires-its-departures.md) (stock mesh client), [ADR 1023](1023-zenoh-rest-pub-bridge-for-non-rust-publishers.md) (publish bridge), [`zenoh-data-fabric.md`](../design/zenoh-data-fabric.md) |

## TL;DR (for a layperson)

Some units cannot buy or ship small computers, and a board that costs about as much as a
cable can still do part of a field node's job: listen to a sensor, pass its data on, convert
a simple message format. Such a board has no operating system, cannot join the mesh and
cannot hold our credentials safely. This decision says how it is allowed into a goatnet
anyway: as a part attached to one named goat node, which owns it, vouches for nothing it
says, and is the only way its data reaches the fabric. The board is counted and checked like
equipment, not trusted like a member.

## Context

[ADR 1257](1257-every-machine-kind-is-a-goatbuilder-build-target-type-declared-as-data.md)
makes every machine a platform family plus a target type, and goatbuilder builds and checks
each one over SSH with Ansible plays. A microcontroller fits the words of that model and not
its mechanics: it has no operating system, no SSH, no disk to encrypt, and its "install" is a
firmware image written at a bench.

One board was measured on a bench on 2026-11-26, an ESP32-P4 development board with one
100 Mb/s Ethernet port, cabled to a Linux machine:

| Measured | Result |
|---|---|
| Forwarding a sensor's UDP datagrams to the Linux machine | no loss to 10,000 datagrams per second at 64 bytes and 8,000 at 1,400 bytes; 3,600,000 of 3,600,000 delivered in a 60-minute run at 1,000 per second |
| Receiving traffic addressed to other machines (promiscuous) | all frames at 97 Mb/s of full-size frames; no loss to 10,000 small frames per second, 1% loss at 12,000 |
| Publishing to a Zenoh router with zenoh-pico (1.9.0 and 1.10.2, each against both router versions) | works over plain TCP; the library has no TLS on this platform |
| Converting a binary message by a compiled description (`gt-core`) | 509 of 509 outputs byte-identical to the reference engine, 232 microseconds per message, no heap |
| Firmware update pulled from the Linux machine, with rollback | a bad image rolls back by itself; a wrong hash or signature is refused |
| Reproducible build | two builds give identical images |

Three facts decide the design:

- **It cannot be a mesh peer.** The mesh client has no microcontroller build, and
  [ADR 1248](1248-goat-follows-stock-upstream-netbird-and-wireguard-and-retires-its-departures.md)
  keeps goat on the stock client.
- **It cannot authenticate to a goatnet router.** Routers accept mutual TLS only
  ([`zenoh-data-fabric.md`](../design/zenoh-data-fabric.md)); the client library on this
  platform has none.
- **It cannot keep a secret from someone holding it.** The chip's key storage and secure boot
  are one-way fuse settings, parts of them are withdrawn or restricted for this chip in the
  vendor's current release, and published attacks on sibling chips recover keys with
  inexpensive equipment. No validated cryptographic module covers the platform, and its
  post-quantum support is an evaluation component.

## Decision

1. **A microcontroller is an attached device.** It belongs to exactly one goat node, its
   **parent**, which is an ordinary machine of an ordinary target type. The device is not a
   peer, holds no goatnet identity or credential, and is not a goatbuilder target type or a
   model of a platform family.
2. **Its owner is its parent's owner.** The parent's record lists each attached device by
   serial number, hardware model, firmware version and firmware hash. No device is in service
   without that record.
3. **Its data enters the fabric as the parent's publication.** The device hands data to the
   parent on a wire between them; the parent publishes under its own identity and marks the
   data's origin as the attached device. Nothing the device says is authenticated: the parent
   and every reader treat it as unauthenticated input, rate-limited and checked for shape.
4. **The parent never takes direction from the device.** The device is not a source of
   identity, time, configuration or commands for anything. The parent listens for it on a
   dedicated port or interface and offers it one service: firmware and plan updates.
5. **No secret on the device.** Firmware carries no credential, key or name of a goatnet.
   Where the device must protect data it carries or records, it seals each payload to a public
   key whose private half is elsewhere. A device holding its own scoped credential is a
   separate tier that needs its own decision and a chip on which the protections work.
6. **Firmware is a signed, reproducible release** built from one repository, flashed at a
   bench, and updated by the device pulling from its parent, with automatic rollback. The
   device reports its version and hash in a health beacon, which the parent republishes under
   its own health key.
7. **goatbuilder checks the device through the parent.** A target type may declare attached
   devices; `doctor --target` then compares each device's reported serial, version and hash
   with the parent's record, and reports a device that is silent, unknown or running
   unrecorded firmware. goatbuilder does not flash or configure the device.
8. **The first roles are fixed-function:** forward a sensor's datagrams, hold a sensor link
   open, capture frames for a tap or decoder running on the parent, convert a binary message
   by a compiled description, record to removable storage. A device that sends commands to a
   sensor is out of scope until a later decision.

A unit with no Linux node at all may still use such a device alone, on a wire it controls,
for data that is perishable or sealed. That use is outside a goatnet and outside this record.

## Alternatives considered

| Alternative | Why not |
|---|---|
| A `microcontroller` platform family with its own target types | Every mechanic of a target type assumes a reachable Linux host: an Ubuntu version, Ansible plays, SSH checks, disk encryption. A family with none of them would be a family in name only |
| A mesh peer through third-party WireGuard code on the chip | Departs from the stock client (ADR 1248), puts a private key on a device that cannot protect it, and the available code is unaudited |
| Its own fabric identity (certificate and TLS on the chip) | Not buildable today, and the key would sit in storage a holder of the device can read. Revisit as the separate tier of Decision 5 |
| Refuse microcontrollers | Units without the money or supply for small computers would wire them in anyway, with no record, owner or check |
| A secure element beside the chip | It protects an identity key, not the data passing through, and adds a part and a supplier. Nothing in the first roles needs a key on the device |

## Consequences

**Positive.** A unit can add capture and sensor points for the price of a cable while every
goatnet control stays on a machine that can enforce it. Every device has an owner and a
recorded firmware hash. A captured device yields its firmware and nothing else.

**Negative.** The parent gains a listener and an update server to maintain. Data from an
attached device is only as trustworthy as the wire and the device, and readers must be told
so by its origin marking. A firmware repository, a signing key and a bench procedure are new
things to keep. Anyone with the device's USB port can reflash it; the control is the hash
check and a physical seal, not the chip.

## Assessor view (SCA-R)

The device sits outside the authorization boundary's trusted components and is treated as
untrusted equipment attached to a system component (the parent). Identification and
authentication (IA) controls are met by the parent, not the device. Configuration management
(CM-2, CM-8) is met by the inventory record and the reported firmware hash; system and
information integrity (SI-7) by signed, reproducible firmware and the parent-side check.
Media and physical protection of the device itself is procedural (seal, inspection). Residual
risk: false or altered data from a substituted device, bounded by origin marking, rate limits
and the reader's treatment of the source as unauthenticated.

## Trigger conditions for revisit

- A chip revision and vendor release on which secure boot and hardware key storage are both
  supported, and a Zenoh client with TLS on the platform: revisit the credential-holding tier.
- A validated cryptographic module that lists the platform.
- A role that must send commands to a sensor.
- A second vendor's board in use, to test that the decision is not specific to one chip.
