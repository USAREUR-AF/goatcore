# Mesh DNS redundancy — a 3-server nameserver group

**Status:** built. **ADR:** [1056](../adr/1056-mesh-dns-redundancy-three-server-group.md). **Companion:** `dns-service-discovery.md` (the single-resolver architecture this makes redundant).

## Problem

With a single resolver the mesh-internal DNS is a **single point of failure**: one CoreDNS container on the goatnet's mgmt host, bound to one mesh IP, advertised to every peer as a **1-server** netbird nameserver group. When that host — or the relayed path to it — drops, **all service-subdomain resolution fails goatnet-wide**. (Netbird's built-in *peer* DNS keeps resolving peer-named hosts; it's the nameserver-group services that die — see `dns-service-discovery.md` §"peer DNS vs nameserver-group DNS".)

This is structural, not a bug: nothing made DNS survive losing one host.

## What makes redundancy cheap here

1. **The zone is deterministically generated** from the service registry (`ops/dns/generate-mesh-hosts.py` → `ops/dns/<site>/mesh-hosts`, CI-gated by `dns-registry-sync`). The same input yields a byte-identical zone, so **N CoreDNS replicas serve identical answers with zero drift** — no primary/secondary zone transfer, no consensus; just ship the same file.
2. **NetBird nameserver groups already support exactly 1 or 3 servers** (`dns-service-discovery.md` §"Nameserver group limits"). Going from 1 → 3 is a config change, not new netbird capability. NetBird's resolver tries the servers and fails over.
3. **CoreDNS is already a turnkey role** (`mesh-coredns` + `goat-coredns.yml`): static container, mounts the zone file, `reload 5s`. Standing up another replica is the same role on another host.

## Design

**A 3-server netbird nameserver group**, each server a CoreDNS replica on a distinct **mesh-resident** host, all serving the identical generated zone.

```
                 netbird nameserver group "<site>-mesh-dns"  (3 servers)
                 ┌──────────────┬──────────────┬──────────────┐
   peers ───────▶│  CoreDNS A   │  CoreDNS B   │  CoreDNS C   │   (failover by resolver)
   (resolver      │ mesh-ip-A:5353│ mesh-ip-B:5353│ mesh-ip-C:5353│
    fails over)   └──────┬───────┴──────┬───────┴──────┬───────┘
                         │ identical zone (generated, dns-registry-sync gated)
                    ops/dns/<site>/mesh-hosts  ◀── single source of truth
```

- **Servers must be mesh peers.** A peer queries a nameserver by *mesh IP* over the overlay, so each replica host must be on the mesh. **Relays are NOT mesh peers** (and their public-IP DNS would leak mesh service names onto the internet — violates the no-public-secure-plane invariant), so **relays are not eligible**. Eligible hosts: the mgmt host(s), per-site standby mgmt nodes (the HA-failover Orins, `ha-failover.md`), and any always-on mesh-joined service node (e.g. the collab-services pattern).
- **Identical zone everywhere.** Each replica gets the same `ops/dns/<site>/mesh-hosts` via the `mesh-coredns` role; `reload 5s` picks up regenerated zones. No replica is authoritative-primary — they're interchangeable.
- **The group is registered with all 3 IPs.** `goat-coredns.yml`'s nameserver-group POST/PUT (`/api/dns/nameservers`, `<SITE>_NB_API_TOKEN`) sends a 3-element `nameservers` array instead of 1.
- **Honest degradation.** 3 → 2 → 1 reachable servers still resolves (netbird fails over); only 0 reachable = outage.

## Implementation (role + playbook deltas) — BUILT 2026-06-20

The `mesh-coredns` role already deploys one CoreDNS; the work was to (a) drive it across a host *list* and (b) register the group with the list. All in `goat-coredns.yml`:

- **`mesh_dns_servers`** — an ordered list of `{host, mesh_ip}` (1 or 3 entries; netbird rejects 2). **Default = the single targeted host** (`[{host: inventory_hostname, mesh_ip: mesh_ip_a}]` — today's behavior, fully backward-compatible). Set it in group_vars or `-e` for a 3-server group.
- **Role apply is per-host already** — `mesh-coredns` binds each host's own `mesh_ip_a`, so running the play with `-l` across all 3 hosts deploys a replica on each. No role change needed.
- **Nameserver-group registration** builds the `nameservers` array from `mesh_dns_servers` (portable loop → `[{ip, ns_type: udp, port}]`), and the whole GET/POST/PUT block is `run_once` (delegated to localhost) so a 3-host play registers the group exactly once. The 1-entry default is functionally identical to the prior single-server registration.
- **Validation:** asserts `mesh_dns_servers | length in [1, 3]` (netbird's limit) at playbook start, fail-closed with a clear message.
- **No zone change:** `generate-mesh-hosts.py` + `dns-registry-sync` are unchanged; every replica mounts the same output.

## One group per site that replicates (ADR 1235)

Status: **built** on the fixture for `join-node`, **not measured** on a joined site. [ADR 1235](../adr/1235-per-site-mesh-dns-nameserver-groups.md) amends ADR 1056: every site that replicates runs a replica of the goatnet's zone, as its own nameserver group `<site>-mesh-dns`.

- **The list.** `goat-coredns.yml` with `mesh_dns_peer_replica_group=<founder>_hosts` lists the targeted host first and the first host of that group second. Without it, the list is the targeted host alone, as before (`tests/test_coredns_peer_replica.py`).
- **The distribution.** A new group goes to the NetBird groups `netbird_nameserver_distribution_groups` names, by default `All`. Every peer then asks every site's group for the zone. The NetBird client merges groups for the same domain and queries them in parallel, so a site cut off from its founder answers from its own replica. A name NetBird does not hold is refused before the group is created.
- **The writer.** A joining site's group is registered through the founder's management API (`netbird_api`, `netbird_api_token_envvar`), the one writer. `join-node` phase 13 runs it for a joiner's core node.

## Non-goals / boundaries

- **Not** anycast or a DNS load-balancer — netbird's resolver-side failover across the group is the mechanism; no new component.
- **Not** zone transfer / primary-secondary — the generated file *is* the replication; shipping it is deterministic and CI-gated.
- **Not** relay-hosted DNS — relays aren't mesh peers and public DNS leaks names.
- Cross-site DNS federation (a peer at site X resolving site Y's services) rides the same generated-zone + Spock-replicated registry story and is out of scope here (it's the ADR 0216 federation plane).
