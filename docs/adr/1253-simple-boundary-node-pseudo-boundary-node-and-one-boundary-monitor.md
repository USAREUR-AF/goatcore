# ADR 1253 — A simple boundary node sends released data as plain messages one way, a pseudo boundary node crosses to another environment of ours, and one monitor shows every boundary

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-19 |
| Deciders | operator |
| Repo scope | goat core: `ops/simple-bn/`, boundary-node design |
| Supersedes | — |
| Superseded by | — |
| Amended by | [ADR 1263](1263-simple-boundary-node-partner-ipsec-gateway-fails-closed-and-real-data-needs-a-validated-module.md) (decision 3: stage 2 with IPsec; real data across an organizational boundary uses stage 2 on a validated module) |
| Related | [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) (the canonical boundary node), [ADR 1208](1208-sovereign-environments-release-namespaces-and-grant-encryption.md) (only released data crosses), [ADR 1178](1178-external-service-golden-path.md) (how the monitor is fronted), [ADR 1117](1117-per-env-doors-cross-env-management.md) (one client per environment), [ADR 1186](1186-credentials-at-rest-are-references-only.md) (keys and addresses as references), design [`simple-boundary-node.md`](../design/simple-boundary-node.md) |

## TL;DR (for a layperson)

Some systems that need our data are not goat and cannot join a goat fabric, for example an air-defence centre that
takes radar reports as standard ASTERIX messages on one network port. For them goat now has a simple boundary node: one
dedicated machine that sends chosen, released data to that one receiver as plain messages, in one direction only. It
starts unencrypted, for testing with simulated data only, and moves to a private WireGuard tunnel whose keys the two sides
swap out of band before any real data flows. When data has to move to another goat environment of our own rather than to
a partner, a pseudo boundary node does it, and how that crossing is protected is decided case by case. Every kind of
boundary reports to the same small status page, visible only inside the goatnet.

## Context

The only boundary goat had was the canonical boundary node: Zenoh federation through a dedicated edge router per partner
(ADR 1157). It assumes the far side runs goat, or at least Zenoh, and shares SPIRE trust. A receiver that takes UDP
datagrams in a standard surveillance format fits none of that. Before this decision, the only ways to feed one were a
process on whatever host was handy, or a core host sending to an outside address. Both put an outside-facing sender on a
machine the goatnet depends on.

Data also has to move between our own environments, for example from a test goatnet to a pre-production one. That
crossing needs neither the trust negotiation of a cross-sovereign peering nor the far side's ignorance of goat.

## Decision

1. **Simple boundary node (`simple-bn`).** One process on a dedicated host sends named feeds to one declared receiver as
   UDP datagrams (TCP designed), in the receiver's format. For radar this is ASTERIX CAT034 service messages and CAT048
   target reports.
   - Each feed has its own UDP socket and its own SAC/SIC.
   - The receiver's address is a store reference, never a committed value.
   - The only input that may leave is declared `release/<set>/**` keys. Until the fabric subscription is built, the
     only input is notional sensor feeds, and every report they produce sets the SIM bit.
2. **Host rule.** A boundary host is in the goatnet, always up and connected, and is never a core-services machine, a
   host-site node or a person's laptop.
3. **Encryption stages.**
   - Stage 0, plain UDP, carries synthetic data only.
   - Stage 1 is a point-to-point WireGuard tunnel to the receiver, separate from NetBird. Each side keeps its own private
     key, the public keys are exchanged out of band, and the role is `goat-peering-wg`.
   - Stage 2 is the receiver's own protected transport when they mandate one.
   - Real data never uses stage 0.
   - Amended by ADR 1263: stage 2 for an IPsec gateway runs on the boundary host and fails closed, and real data to a
     receiver outside our organization uses stage 2 on a validated module; stage 1 carries real data only between our own
     goatnets or where an authorizing official has accepted it.
4. **Pseudo boundary node (`pseudo-bn`).** A crossing to another goatnet of the same sovereign declares a
   `crossing.mode`:
   - `join`: enrolled in both goatnets, a second NetBird client in its own network namespace, admitted on the far side
     as a machine with a scoped write grant.
   - `tunnel`: stage 1.
   - `plain`: synthetic data only.

   The mode is chosen per crossing and recorded in the declaration.
5. **One boundary monitor.** Every boundary service writes a `goat.boundary-status/v1` document. `bn-monitor` serves the
   documents for its node as one page plus `/api/status`. It is bound to the node's mesh address only, and fronted with
   `door: group-gated` through the external-service declaration. The canonical boundary node gets an adapter that writes
   the same document from its edge router's admin space.

## Alternatives considered

- **Send from a core host.** It needs no new machine, but it puts an outside-facing sender on a machine the goatnet
  depends on, and a fault in the sender or its traffic would then hit the core. Rejected.
- **A forwarding Zenoh bridge on the receiver's side.** It would reuse the canonical design, but the receiver does not run
  Zenoh and should not have to. Rejected.
- **Encrypt from the first packet.** It removes the plain stage, but it delays the first integration test until a key
  exchange with the receiver completes. The plain stage is allowed only for synthetic data, which carries no information,
  so the delay buys nothing. Rejected.
- **Always join the far goatnet for internal crossings.** It is simplest, but a host in two goatnets is a path between
  them, and some pairs must stay apart. The mode is therefore a per-crossing decision, not a default.
- **A monitor per boundary kind.** Each could show more native detail, but it would mean three pages to watch and three
  things to build. One schema and one page keep the operator's view the same whatever the boundary kind.

## Consequences

- A receiver can be fed without running goat software and without any goat identity on its side.
- Each new receiver costs one declaration in the private overlay and one store item. The host and the code are shared.
- Stage 0 is a real outside data path. It is safe only because its input is synthetic. The release-key subscription and
  the `synthetic_only` guard keep it that way.
- UDP gives no acknowledgement. The monitor says so and shows only port refusal, until a receiver agrees to an application
  acknowledgement.
- `bn-monitor` adds a mesh-only HTTP port (8740) on every boundary host.
