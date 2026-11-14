# ADR 1235 — Every site that replicates runs its own mesh DNS, as its own nameserver group

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-14 |
| Repo scope | goat core (goatbuilder, ansible) |
| Amends | ADR-1056 |
| Supersedes | — |
| Superseded by | — |
| Related | ADR-1165, ADR-1206, ADR-1233, ADR-1084 |

## TL;DR (for a layperson)

Every machine on a goatnet looks up the goatnet's service names through mesh DNS, a small name server. Today only the first site, the founder, runs one, so a second site cut off from the founder cannot look up any of the goatnet's names. Under this decision, every site that joins runs its own copy of the name server, serving the same names. Its machines are told to use their own site's copy first and the founder's second. A site cut off from the founder keeps resolving names, which is part of being able to run on its own ("island mode").

## Context

Mesh DNS is a CoreDNS replica serving the goatnet's generated zone (`ops/dns/<dir>/mesh-hosts`, from `ops/dns/generate-mesh-hosts.py`). Peers reach it through a NetBird nameserver group. [ADR 1056](1056-mesh-dns-redundancy-three-server-group.md) made that group hold up to three replicas, because one replica was a single point of failure. The group is one per goatnet, `<site_id>-mesh-dns` at the founder. Joining sites have run no replica at all: standup skips mesh DNS for a joiner ("joiners use the founder's DNS"), and `join-node` has no mesh DNS step.

The goatnet's decision of record is that every site is fully island-mode capable ([ADR 1165](1165-island-mode-multi-writer-every-site-admits.md); gate group I). A site cut off from the founder with no resolver of its own fails that: no service name resolves there. The end-state ruling of 14 November 2026 therefore names mesh DNS absent at a joined site a defect, closed by gate row I3.

Two facts shape the fix:
- A goatnet's NetBird groups are per environment, with role suffixes (`ops/ansible/netbird_acl_taxonomy.yml`). No group means "this site's peers", so a nameserver group cannot yet be aimed at one site's peers only.
- The NetBird client merges every nameserver group that targets the same domain into one handler, and queries the groups in parallel (`client/internal/dns/server.go`, `buildMergedDomainHandler`). So two groups for the goatnet's zone, both given to every peer, both get asked, and either one answering is enough.

## Decision

- **Every site that replicates runs a mesh DNS replica** on its core node. The replica serves the goatnet's whole generated zone, the same file every replica serves. `join-node` deploys it in phase 13 for a joining site's core node, after the replication passes and signal federation. `standup` deploys it for a joining site at phase 12, as it does for the founder. A site with no zone file to serve is refused when the plan is made (GB-280).
- **Each site has its own nameserver group,** `<site>-mesh-dns`, for the goatnet's zone. It lists the site's own replica first and the founder's second (`goat-coredns.yml`, `mesh_dns_peer_replica_group`). The founder's group is unchanged.
- **Distribution is declared.** A group is distributed to the groups named in `netbird_nameserver_distribution_groups`, by default `All`, today's behaviour. With both groups on `All`, every peer asks both, and a site cut off from the founder answers from its own replica. A goatnet that later creates per-site peer groups names them there, with no code change.
- **One writer.** The group is registered through the founder's management API, with the founder's token (`<FOUNDER>_NB_API_TOKEN`), as standup registers the founder's group today. A second site writes no NetBird configuration of its own. The token resolves through `with-secrets`, and an unset or unresolved token is refused before the step runs (GB-281). Moving this write to each site belongs to the admin-surface class switch of ADR 1165 (gate row I4), not to this decision.
- **When cut off, a replica keeps every record.** The founder's names still resolve, and connections to them fail at the address, so reconnecting needs no DNS change. The role's no-drop guard already refuses to push a zone that removes live records.

This amends ADR 1056 in one point: a goatnet has one nameserver group per site that replicates, each with up to three servers, rather than one group for the goatnet.

## Alternatives considered

**One goatnet group, with every site's replica in it (ADR 1056 as written).** It needs no new group and no change to distribution. NetBird caps a group at three servers, so it fills at the third site, and a peer has no way to prefer its own site's replica. Rejected because it stops scaling at the size of a goatnet that already exists (a founder, a joiner and a third site).

**Per-site groups distributed only to that site's peers.** This is the cleanest form: each peer gets exactly its site's group. No NetBird group means "this site's peers" today, and adding one is a change to the ACL taxonomy with its own decision. Kept open: `netbird_nameserver_distribution_groups` lets a goatnet move to it by naming the groups, once they exist.

**Each site registers its own group through its own management.** It removes the dependence on the founder's API and token for a joiner's group. It is an admin write at a second site to a NetBird database that Spock replicates both ways, which is exactly the multi-writer question ADR 1165 settles class by class. Rejected for now. It belongs to gate row I4.

**Joiners keep using the founder's DNS.** It is what was built. A site cut off from the founder then resolves nothing, so it is not island-mode capable. Rejected by the ruling.

## Consequences

**Positive.** A joined site cut off from its founder keeps resolving every goatnet name. Losing any one replica no longer takes the zone away from any peer. The join path runs mesh DNS with no step outside the command. The founder stays the only writer of NetBird configuration.

**Negative.**
- Every peer asks every site's group for the goatnet's zone, so lookups go to more than one replica. CoreDNS's cache and the client's merge keep this small, but it grows with the number of sites until per-site distribution exists.
- A joining site's mesh DNS depends on the founder's API token being stored for the operator who joins it.
- The merge of same-domain groups is the NetBird client's behaviour, not a documented contract. A client that does not merge would use one group and lose the island property. The live read checks it on the deployed client.
- A joining site's own service names enter the zone only when the generator renders more than one site into one zone file. Today it renders one site per zone, so a replica at a joined site serves the founder's names. Its own names are reached through NetBird's peer DNS and their mesh addresses.
- A replica binds the site's `mesh_ip_a`, so a second core node at a site would bind the first node's address. That is the role's existing limit, not new.

## Assessor view (SCA-R)

The resolver stays on the mesh only: it binds loopback and mesh addresses, never a public interface (SC-7). The zone is generated from the declared service registry and checked in CI, so every replica serves the same reviewed records (CM-2, CM-3). The NetBird configuration has one writer, the founder's management, through a token the declared secrets posture holds by reference and resolves per command (IA-5, AC-6). A site cut off from its founder keeps name resolution for its own continuity (CP-2, SC-20/SC-21 for the internal resolver). Residual risk: a site whose replica fails and is cut off from the founder has no resolver until either returns. The compensating control is the founder's replica, which every peer also asks.

## Trigger conditions for revisit

- When per-site peer groups exist in the NetBird ACL taxonomy: distribute each site's group only to its peers.
- When ADR 1165's admin-surface class switch (gate row I4) moves NetBird configuration writes to each site: register each site's group through its own management.
- When a NetBird client release stops merging nameserver groups that target the same domain.
- When the zone generator renders more than one site into one zone, so each site's own names are in the zone it serves.
