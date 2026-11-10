# ADR 1218 — When neither edge accepts inbound traffic, a peering forwarder gives one side a public address; it relays ciphertext and holds nothing of the peering

| Field | Value |
|---|---|
| Status | Proposed. Built (the `goat-peering-forwarder` role); not yet run between two real environments. Nothing in a live goatnet changes on merge |
| Date | 2026-11-06 |
| Deciders | operator |
| Repo scope | goat core (federation design, runbook, package spec; the forwarder role and its CI guard when built) |
| Amends | [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) step 1 (reachability) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md); design [`peering-forwarder.md`](../design/peering-forwarder.md), [`federation-package-spec.md`](../design/federation-package-spec.md); runbook [`sovereign-federation-runbook.md`](../operations/sovereign-federation-runbook.md) |

## TL;DR (for a layperson)

Two goatnets federate over one encrypted tunnel between their two edge machines, and one of
those machines has to be reachable from the internet so the other can call it. Many sites have
no public address: a site network, a satellite terminal, a mobile provider. When neither side
has one, the call cannot be placed.

A peering forwarder is a small machine with a fixed public address. Our edge keeps a connection
open to it, and it passes the partner's call down that connection. It only ever handles traffic
it cannot open, holds no keys or certificates of the peering, and joins neither network, so
neither side has to trust it. It can run in a commercial cloud to get a peering up, and on a site
a party owns for the long term.

## Context

ADR 1157 step 1 is one dedicated tunnel between the two edge hosts. `roles/goat-peering-wg`
builds it with WireGuard: the side whose `transport_offer` says `listener: true` listens, the
other dials with a 25 s keepalive. The federation package specification refuses a pair in which
neither party listens (`compose.neither_listener`).

That leaves no path when both edges sit behind NAT with no inbound port, which is the common case
for an edge on a site LAN or behind carrier-grade NAT. The runbook's precondition, "both sides can
expose one UDP port to the other", is stronger than the design needs (one side suffices) and
unsatisfiable for these parties.

## Decision

1. **One side listens through a forwarder.** A forwarder is a host with a fixed public address. The
   listening edge keeps an outbound carrier tunnel to it (`wg-carry0`, keepalive 25 s); the
   forwarder destination- and source-translates one UDP port onto that carrier. The edge's
   `transport_offer` carries `listener: true` and the forwarder's `host:port` as `endpoint`. The
   partner dials it unchanged.
2. **The peering terminates on the two edges.** The peering WireGuard session and the fabric TLS
   session inside it are end to end. The forwarder holds its own carrier key and one nftables
   ruleset, and never a peering key, CA, SVID, fabric certificate or trust bundle, and it is a
   member of no goatnet mesh.
3. **The forwarder is outside both authorization boundaries.** It is transport. The SC-7
   enforcement points stay on the edge; on a forwarded edge the outside-interface rule binds the
   peering port to the carrier interface (`peering_wg_via_forwarder`).
4. **It is declared.** The forwarder is named in the interconnection agreement and in the partner
   declaration file (`forwarder:`). A reserved package member, `transport_offer.via`, carries it
   into the composed agreement in a later package version; it is informational and changes no
   negotiated field.
5. **Placement is the operator's choice, and the role is the same everywhere:** a commercial cloud
   VM with a static address, a party-owned site with a static address and one forwarded port, or
   one forwarder per sovereign. A party with a public address on its edge needs none.
6. **One port, one destination.** A forwarder serving several peerings uses one public port and one
   carrier per peering; the destination of a packet follows from its port alone.
7. **The runbook precondition becomes "one side can accept inbound UDP, directly or through a
   forwarder".**

## Consequences

- A peering becomes possible between any two goatnets whose edges can send outbound UDP.
- A compromised forwarder can drop, delay and count traffic and sees the partner's public address.
  It cannot read or inject. This is the position every router on an internet path already holds.
- The forwarder is an availability dependency for the link, and its operator can sever the link
  alone. The partner and our edge can each still cut unilaterally as before.
- Replacing a forwarder changes `transport_offer.endpoint`, which is a new package and a
  re-composed agreement, so its address must be static.
- The peering link MTU drops to at most 1340 on a 1500-byte path; edges behind a forwarder offer
  `measured_mtu: 1300`.
- To build: `roles/goat-peering-forwarder`; `peering_wg_via_forwarder` and the carrier interface in
  `roles/goat-peering-wg`; a CI guard that a forwarder host carries no peering key, CA, SVID or mesh
  enrolment and forwards one port per peering; `forwarder:` in `ops/federation/validate-partners.py`.

## Alternatives considered

- **An HTTP or SOCKS proxy.** Carries TCP, not WireGuard's UDP, and terminates what it proxies.
- **NAT hole punching between the edges.** Fails behind carrier-grade NAT and satellite terminals,
  and needs a public rendezvous host regardless.
- **A goatnet's NetBird relay.** Puts the peering on one party's management plane, which ADR 1157
  step 1 excludes.
- **The edge itself on a cloud host.** Valid and needs no forwarder, but moves the partner's CA and
  the fabric session into the cloud. The forwarder keeps every key on hardware the party chose.
- **A TURN server.** A protocol and a credential set for what one nftables rule does.
