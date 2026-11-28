# ADR 1050 — Reticulum bridge: a denied-edge bearer for goat (below the IP line)

**Status:** Accepted — the bridge pattern stands. The first bridge (Python, on `rns`/`lxmf`) is removed because of its license ([Amendment 1](#amendment-1-2026-11-28--the-reticulum-license-and-the-rust-rebuild)); the rebuild in Rust on Reticulum-rs is designed-not-built; field RF bearers and goat-identity mapping are designed-not-built.
**Repo scope:** Sibling repo `goat-reticulum` (the bridge and its proof, Rust, on Reticulum-rs and `zenoh` =1.9.0; Amendment 1). goat core: docs only ([`reticulum-bridge.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/reticulum-bridge.md), this ADR, a CLAUDE.md pointer). `ice-goat`: southbound-contract reference (PR).
**Related:** [ADR 0858](0858-zenoh-image-upstream-pivot.md) (the build-from-source cost this avoids) · [ADR 0217 addendum](0217-shim-repo-containment.md) (sibling-repo containment) · [ADR 1032](1032-zenoh-volatile-non-persisted-namespace-class.md) (the `volatile/blob` chunking the large-payload roadmap mirrors) · [ADR 1033](1033-zenoh-qos-prioritization-reserved-mission-slice.md) (rate-shaping onto a scarce bearer) · [`reticulum-bridge.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/reticulum-bridge.md) (the design) · ice-goat `docs/interfaces/southbound.md` (the adapter contract).

## Plain-language summary

goat moves data over an IP overlay (WireGuard mesh) and the Zenoh fabric. Below
the IP line — RF, LoRa, packet radio, serial, denied infrastructure — it has no
bearer. Reticulum (RNS) is built for exactly that regime. This ADR adopts a
**bridge** between the two stacks: a Zenoh peer on the goat side, a Reticulum
node on the denied-edge side, translating fabric samples to Reticulum traffic and
back. goat does not reimplement Reticulum; the bridge is the seam. A
self-contained, no-hardware proof shows data crossing the boundary in both
directions on current `rns` and `eclipse-zenoh`.

## Context

- goat's edge-resilience — "the tactical edge runs itself" — is real **through a
  management-plane outage** (tunnels persist, same-LAN peers keep talking) but
  carries an **unstated dependency on IP reachability between peers**. WireGuard
  P2P and relay fallback both need IP. There is no bearer below the IP line.
- Reticulum is coordination-less, cryptographic-identity-addressed (no IP/DNS/
  central authority), with transport interfaces over almost any medium down to a
  few bits per second — the complement to goat, not a competitor.
- ice-goat's southbound adapter contract already defines the plug-point for
  transport/RF/MANET gateways and explicitly flags a "Zenoh over serial/RF"
  tracked dependency that would need the non-default `transport_serial` cargo
  feature and reopen goat's build-from-source cost (ADR 0858).
- A working proof was built first (no design-only commitment): `make proof` in
  the sibling repo passes both directions, three consecutive runs, with the edge's
  Reticulum identity carried end-to-end onto the Zenoh key.

## Decision

1. **Bridge, don't reimplement.** A single bridge process holds both stacks: a
   Zenoh peer (egress subscribe / ingress publish) and a Reticulum `IN`/`SINGLE`
   destination edge peers Link to. `rns`/`lxmf` are runtime dependencies, never
   vendored or forked.
2. **Zenoh stays on IP; Reticulum owns the cross-to-RF hop.** This sidesteps the
   `transport_serial` shape entirely — **no zenohd fork, no non-default Zenoh
   feature, no reopened build-from-source cost.** If a tier only needs *data*
   (not Zenoh's exact wire) to reach a denied edge, this is the cheaper bearer.
3. **The bearer is opaque.** The Reticulum interface (TCP loopback in the proof;
   `RNode`/LoRa, KISS/packet-radio, serial in the field) is config, not bridge
   code — the same bridge runs over real RF unchanged. This mirrors ice-goat's
   southbound boundary exactly.
4. **Identity crosses the seam.** Edge peers prove a Reticulum identity on the
   Link; the bridge labels ingress data by that identity hash, so edge-sourced
   data is attributable on the fabric (`goat/rns/ingress/<edge-id>`), not
   anonymous. Both legs are encrypted end-to-end (RNS Link + goat WireGuard).
5. **Sibling repo, not goat core.** It pulls a Python RF stack neither goat core
   (Rust/Zenoh) nor ice-goat should depend on, and it is a *reference* bearer —
   so it lives beside the contract it implements (ADR 0217 addendum; demand-gated
   radio tier). goat core carries docs + a pointer only.
6. **Positioned as ice-goat's first denied-edge southbound gateway.** It presents
   an IP-reachable endpoint upward and keeps the RF bearer opaque below, per the
   southbound contract; the media-plane obligations (PTT/COR, link-health) are
   the growth path toward ice-goat's voice use case.

## Alternatives considered

- **Carry Zenoh directly over serial/RF (`transport_serial`) — rejected for now.**
  It is the "purest" answer but reopens ADR 0858's build-from-source cost
  (non-default cargo feature → zenohd fork → maintenance/ABI burden). The bridge
  delivers denied-edge reach without any of that. `transport_serial` stays a
  tracked option for a future tier that genuinely needs Zenoh's wire on the RF
  link.
- **Build our own RF/LoRa stack — rejected.** Reticulum already exists, is
  mature, and is the regime's reference implementation. Reimplementing it would
  violate the complexity budget and the "don't rebuild what exists" posture.
- **Put the bridge in goat core or ice-goat — rejected.** Neither should take a
  Python RF dependency; a reference bearer belongs beside the contract, not
  inside it.
- **An accredited cross-domain solution — out of scope (different problem).** A
  separate conversation; the denied-edge bearer is about *transport below IP*,
  not *transfer between security domains*.

## Consequences

- goat gains a path below the IP line: the honest gap in "the tactical edge runs
  itself" is closeable, proven in principle, on current upstreams — at zero cost
  to the goat substrate's build (no Zenoh fork).
- ice-goat gets its first reference denied-edge southbound gateway and the
  lower-cost resolution of its "Zenoh over serial/RF" tracked dependency.
- The capability is contained: a sibling repo, demand-gated, no new dependency in
  goat core or ice-goat.
- **Built since acceptance:** the large-payload path (`RNS.Resource`, chunked) for
  payloads above the ~383-byte single-packet ceiling — auto-selected, transparent
  in both directions, proven bidirectionally over loopback (a 50 KB ≈ 108-part
  transfer). Field-confirmation over RF is part of the gate below.
- **Gate to canon (move out of in-flight):** a real RF bearer (LoRa/packet-radio/
  serial) proven on hardware (the large-payload Resource path confirmed over it),
  multi-edge routing, a Reticulum-identity → goat-slot-anchor mapping for full
  attribution, and rate-shaping onto the scarce bearer (ADR 1033). Until then it
  is a proven bridge pattern, not a fielded bearer.

## Amendment 1 (2026-11-28) — the Reticulum License and the Rust rebuild

**The license fact.** The Python reference implementation `rns` and `lxmf` are
not MIT. From rns 0.9.4 (2025-04-15) onward they are under the "Reticulum
License": the MIT License plus use conditions. One of them reads:

> The Software shall not be used in any kind of system which includes amongst its
> functions the ability to purposefully do harm to human beings.

The condition attaches to use, not only to redistribution, so installing and
running the unmodified package is covered. The first bridge required
`rns>=1.3.5` and `lxmf>=1.0.1`, so it was inside the condition. The same license
covers Reticulum-Go. The Reticulum protocol itself is in the public domain; the
license covers the implementations.

**The rule (operator, 2026-11-28, verbatim):** "okay then only do reads and
gleans and clean room rebuilds against anythign with this license", and, for the
implementation, "other than that go with the recommendations".

| Action on Reticulum-License code | Allowed |
|---|---|
| Read its code and documentation to learn the protocol | Yes |
| Depend on it, vendor it, copy or translate it | No |
| Run it as part of goat, its demos, CI or tests, including as an interop peer | No |
| Redistribute it | No |

**The implementation choice.** A Rust implementation under a plain license is
used when its provenance is clean; otherwise goat writes a clean-room
implementation from the protocol description in the Reticulum manual and from
observed behavior on the wire. Two candidates were checked by matching every
comment line against the text of rns 0.9.3 (the last MIT release) and of current
rns, and by dating each match:

| Candidate | License | Result |
|---|---|---|
| Reticulum-rs | MIT | **Used.** Idiomatic async Rust; 20 of 356 comment lines match rns text, all of it present in 0.9.3 (MIT) and added in 2024; none matches text that exists only in Reticulum-License releases. Pinned to a reviewed commit, re-checked before every bump. Its `python-tests` feature runs the Python stack and is never enabled. |
| microReticulum (C++) | Apache-2.0 | **Not used.** It describes itself as a port of the Python stack and carries comment text that first appears in rns 0.9.6 and 1.1.7. |

Reticulum-rs does not implement Resources or the serial, KISS and RNode
interfaces. goat writes those as clean-room modules in the bridge repo, from
the manual, the KISS protocol and the RNode firmware's host command table, not
from rns source, under MIT so they can go upstream to Reticulum-rs. Messages
larger than one packet are split into chunks sized for the bearer MTU and sent
over a Reticulum Channel (reliable, ordered, windowed), which also retransmits
small messages lost on the bearer.

Reticulum-rs is vendored in the bridge repo under `third_party/reticulum-rs`
(MIT, its LICENSE kept), pinned to its upstream commit, with each goat change
listed in a `PATCHES.md`. The provenance check that cleared it is committed
beside it and runs against the vendored tree; a bump of the pin is gated on it.

**Interop scope.** Interoperating with Python `rns` networks is not a goal. The
bridge and its edges are Reticulum-rs and goat nodes. Python transport nodes
from rns 1.6.2 on are announced to drop announces, path requests and link
requests that lack network stamps (proof-of-work); Reticulum-rs does not emit
them, so such nodes would not carry this traffic. That does not affect a
bearer made of goat and Reticulum-rs nodes.

**What changes in the Decision.**

- Decision 1 now reads: the bridge is one process holding a Zenoh peer and a
  Reticulum destination on Reticulum-rs. `rns` and `lxmf` are not used in any
  form.
- Decision 5: the sibling repo holds Rust, not Python. goat core still takes no
  Reticulum dependency.
- Alternatives, "Build our own RF/LoRa stack": still rejected as a whole stack.
  The clean-room modules above are the subset Reticulum-rs lacks, no more.

**The rebuild.** The bridge is rebuilt in Rust on Reticulum-rs, with the same
key spaces (`goat/rns/egress/**`, `goat/rns/ingress/<edge-identity-hash>`) and
the same proof: bidirectional, one-packet and chunked messages, over shaped and
lossy links. Interop peers are Reticulum-rs nodes and goat's own nodes only. The
bridge's own handling of a message is held to 1 ms or less per message (p99).
Each side queues messages per peer identity across Link loss and resends
what was not confirmed, so delivery is at-least-once; every message carries
its sender's message id so the receiving side can drop repeats.
The rebuild lives in the internal `goat-reticulum` repository, the one that held the first bridge. The first bridge's code is removed from `goat-reticulum`; its design and proof
documents stay there, marked superseded, beside a `LICENSING.md` stating this
rule.
