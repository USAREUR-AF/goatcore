# ADR 1141 — A host site is four nodes: a core pair and an observability pair

- **Status:** Accepted — **amended by [ADR 1183](1183-rescue-nodes-are-part-of-the-site-baseline.md)**, which adds the two rescue roles this baseline never named: a site declares its break-glass node ([ADR 1114](1114-breakglass-node-class-and-two-path-reach.md)) and its network-jump host ([ADR 1131](1131-network-jump-class-and-herd-gear-scope.md)), or declares why it has neither. The core pair and obs pair below stand unchanged.
- **Deciders:** operator
- **Amends:** ADR 0826 (per-site three-node topology — the node count and the deferred fourth node; everything else in 0826 stands)
- **Related:** [ADR 0811](0811-audit-log-integrity.md) (audit-log integrity — the cross-mount durability property this restores), [ADR 1111](1111-cutover-gated-service-class-for-standby-nodes.md) (how a backup node stays safe while dormant), [ADR 0205](0205-intra-site-vrrp-failover.md) (VRRP removed; promotion is operator-fired)

## Context

ADR 0826 set the per-site shape at three hosts: a management pair (one active, one passive) plus one observability and storage host. It considered a fourth node, an observability partner, and deferred it on hardware cost.

Three things made the three-node shape insufficient:

- **The audit record lost its second copy.** [ADR 0811](0811-audit-log-integrity.md) relied on MinIO being cross-mounted across the management pair, so the loss of one host did not lose the audit record. Moving MinIO to a single observability host dropped that property: that host became the only custodian of the Object-Lock audit bucket. A compliance-mode bucket with one physical copy does not survive the loss of a host.
- **An observability host can fail on its own.** A stalled encrypted data volume on that one host is enough to lose the dashboards and the only copy of the audit record together.
- **The observability host stopped being only the observability host.** It also carries the in-site Ansible bastion, the apt and OCI pull-through caches, the site's git remote, the signing host for the development CA, the operator sign-in and local inference. Losing it costs the site its deploy workspace and package caches during exactly the window an operator needs them.

## Decision

**The minimum host-site shape is four nodes: a core pair and an observability pair.**

| Node | Role | Posture |
|---|---|---|
| core-A | management control plane — NetBird management and signal, Postgres with Spock, Ory, Traefik, the Zenoh router | active |
| core-B | same stack | warm standby; promotion is operator-fired |
| obs-A | observability, storage and the site utility workloads | active |
| obs-B | same stack | warm standby, cutover-gated per [ADR 1111](1111-cutover-gated-service-class-for-standby-nodes.md); MinIO runs as a **replication target**, which restores the [ADR 0811](0811-audit-log-integrity.md) two-copy property |

Three points of definition:

1. **Roles, not hardware.** "core" and "obs" name roles. The hardware for each role is a procurement choice; the fleet vocabulary is "four-node site", not a count of a particular device.
2. **The fourth node is a warm partner, not a second live observability plane.** Everything on obs-B is dormant except the standby-safe classes defined by [ADR 1111](1111-cutover-gated-service-class-for-standby-nodes.md) and `ops/standby/service-classes.yaml`: reveal-only self-publishers, host-local probes, a listen-only router and passive replication receivers. MinIO replication is a passive receiver, so nothing here creates a second active write surface.
3. **An existing three-node site is a deviation and is declared.** It does not pass silently as compliant: it carries a declared, reasoned exclusion until it is retrofitted or the exclusion is made permanent.

## Consequences

- **Cost.** Four nodes per site instead of three: a third more hardware, power, thermal budget and rack or case space. It is paid to keep the audit record through a host loss and to keep the site's deploy workspace, not to raise availability numbers.
- **Failure-domain accounting.** Losing one core node costs an operator-fired promotion. Losing one obs node costs the live dashboards and the active caches until cutover, but no longer the only copy of the audit record. Losing a whole site remains a rebuild, not an availability event.
- **What this does not buy.** No automatic failover on either pair, and no protection against losing the site. A warm partner that has never been promoted is an assumption, not a capability, until a drill promotes it.

## Alternatives considered

- **Keep three nodes and accept a single copy of the audit record.** Rejected: the obs host carries far more than dashboards.
- **Cross-site MinIO replication instead of a fourth node.** Worth doing on its own merits and not excluded, but it does not help the common case, a live site whose single obs host is wedged, and it makes audit durability depend on a WAN link that island mode assumes can be absent.
- **Move observability back onto the core pair.** Rejected: disk and IOPS contention against the Postgres WAL, and two failure domains merged into one.
- **Five or more nodes per site.** The fourth node closes a named gap; a fifth has no gap to close.
