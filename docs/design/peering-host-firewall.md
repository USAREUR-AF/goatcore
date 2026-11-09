# Peering host firewall: a baseline on the edge host and the core router host

| Field | Value |
|---|---|
| Status | **Built, opt-in, off by default.** Role `roles/goat-host-firewall`, included by `ops/ansible/peering-wg.yml` and `peering-edge.yml` (profile edge, phase 11) and `peering-republisher.yml` (profile core, phase 12) when `goat_host_firewall_baseline` is true. Renderer `roles/goat-host-firewall/files/goat-host-firewall-render.py`. **Measured** in network namespaces (`tests/netns-behavior.sh`) and over SSH in containers (`ops/ci/test-host-firewall-role.sh`). Not yet run on two real environments. The planner's report of the ruleset (§7) is designed, not built |
| Decision | [ADR 1219](../adr/1219-peering-edge-on-the-mesh-reaches-only-the-core-router.md) (what the edge may reach on the mesh), [ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md) (the edge and its tunnel), [ADR 1218](../adr/1218-peering-forwarder-when-neither-edge-accepts-inbound.md) (the forwarder carrier) |
| Companions | [`peering-forwarder.md`](peering-forwarder.md), [`goatbuilder.md`](goatbuilder.md) §"Founders, joiners, host-site nodes and sovereign peering", `roles/goat-peering-wg`, `roles/goat-peering-edge` |
| Audience | Operators of a peering, and whoever accredits the edge and core hosts |

Status: WIP — gate: the port-scan scenario (§8) passes with the baseline on, and with one tightening override, from the partner side and the mesh side on two environments, and the three-party case still passes with it on.

## 1. What it is

The edge host holds a partner's certificate authority, and the core router host is the one
thing the edge dials. Both had no host firewall of their own: the tunnel table of
`goat-peering-wg` filters the tunnel and nothing else, and the mesh ACL (ADR 1219) filters the
mesh and nothing else. Anything else listening on either host answered every interface.

The baseline is a **declared profile**: inbound default deny, plus accept rules derived from the
role variables the peering already uses. Each nation brings its own prescriptive requirements, so
each deployment has an **override**: it may tighten freely, and it may loosen only a named rule,
with a reason recorded in its own configuration.

## 2. What already filtered, and what this adds

| Where | Table | Filters | This baseline |
|---|---|---|---|
| `goat-peering-wg` | `inet goat_peering_<if>` (prerouting, postrouting) | on the tunnel: only the fabric port and `peering_wg_extra_ports` from the far link address, in and out; the tunnel UDP port from the partner's literal endpoint; behind a forwarder, the carrier | leaves it as it is; adds the host's input hook |
| `goat-peering-edge` (IPsec) | `inet goat_peering` (MSS clamp only, when on) | nothing; port narrowing is left to "the host firewall" | is that host firewall |
| `goat-peering-forwarder` | `inet goat_peering_forwarder` | the forwarder host, completely | out of scope: the forwarder already has its own |
| `goat-zenoh-edge` | none; binds `goat_zenoh_edge_bind_address`, host network by default | — | admits its port on the tunnel interface only |
| `goat-zenoh` (core) | none; the router publishes `goat_zenoh_listen_host:goat_zenoh_listen_port` from a container | — | admits the fabric port; holds published container ports to the same rules |
| mesh client | its own tables | the mesh ACL | leaves it as it is |

The baseline is one table of its own, `inet goat_host_firewall`. nftables drops a packet that any
base chain drops, and an accept in one table never overrides a drop in another, so this table
only adds restrictions: it never opens what another table closes, and loading or removing it
leaves every other table untouched.

## 3. The rules

Two chains. `input` (hook input, policy drop) holds traffic to the host's own sockets.
`published` (hook forward, priority -10) holds **new connections to a port a container
publishes**, which docker translates and which therefore never reach the input hook: it applies
the same port rules to the original destination port (`ct original proto-dst`), and leaves alone
traffic from the host's own container bridges and forwarding that is not to a published port.
Outbound is unchanged: ADR 1219's outbound limits are the mesh ACL's.

| Rule | Profile | Admits | Derived from | Override |
|---|---|---|---|---|
| `loopback` | both | `iifname lo` | — | fixed |
| `established` | both | replies (`ct state established,related`) | — | fixed |
| `invalid` | both | drops `ct state invalid` | — | fixed |
| `icmp-echo` | both | ICMP echo request, 10/s | — | drop only |
| `icmpv6-essential` | both | neighbor discovery, multicast listener query, packet-too-big, echo | — | drop only |
| `dhcp-client` | both | DHCP (UDP 67 → 68) and DHCPv6 (UDP 547 → 546) replies | — | drop only |
| `container-bridges` | both | the host's own containers (`docker0`, `br-*`) reaching services on the host, and in `published` their connections to ports other containers publish | — | drop only; dropping it removes both. A host whose uplink is named `br-*` is refused (GB-210) unless it is dropped |
| `ssh-management` | both | TCP SSH from the management sources only | `goat_host_firewall_management_sources` (required), `goat_host_firewall_ssh_port` (`ansible_port`) | yes |
| `mesh-wireguard` | both | the mesh client's own WireGuard UDP port, when listed (empty by default: the mesh client's own outbound connections are answered as established) | `goat_host_firewall_mesh_udp_ports` | yes |
| `<if>/tunnel` | edge, wireguard | the tunnel UDP port from the partner's literal endpoint (any source when the package names a host name) | `peering_wg_listen_port`, the counterparty package's `transport_offer.endpoint` | yes |
| `<if>/tunnel` | edge, wireguard via forwarder | the tunnel UDP port on the carrier, from the forwarder's carrier address | `peering_wg_carrier_interface`, `peering_wg_carrier_remote_ip` | yes |
| `<if>/carrier` | edge, via forwarder | the carrier's own UDP port, from the forwarder's literal address | `peering_wg_carrier_listen_port`, `peering_wg_forwarder_endpoint` | yes |
| `<if>/ike`, `<if>/esp` | edge, ipsec-fips | IKE (UDP `peering_edge_ike_port`, 4500) and ESP from the partner's literal address | `peering_edge_ike_port`, `peering_edge_remote_addr` | yes |
| `<if>/edge-router` | edge | the edge router's port, **on the tunnel interface only**, from the far link address | `goat_zenoh_edge_listen_port` (`peering_wg_fabric_port`), the tunnel interface, `goat_peering_remote_link_ip` | yes |
| `<if>/tunnel-extra` | edge | the other ports the tunnel carries (the SPIRE bundle endpoint), on the tunnel only | `peering_wg_extra_ports` | yes |
| `<if>/spire-forward` | edge, when `peering_spire_enabled` | the port of goat-peering-spire's outbound forward, from our SPIRE server's literal address; phase 13 opens it after phase 11 loads the baseline, so it is admitted ahead of time. ADR 1219 does not list this reach; it is the SPIRE step's existing design, made visible here | `peering_spire_outbound_port`, `peering_spire_server_addr` | yes |
| `fabric-tls` | core | the core router's fabric TLS port, any source (mutual TLS decides) | `goat_zenoh_listen_port` | yes |
| `fabric-quic` | core | the QUIC listener, when `goat_zenoh_quic_enabled` | `goat_zenoh_quic_port` | yes |
| `mesh-dns` | core | the mesh nameserver, on the mesh interface only (ADR 1219 decision 4) | `goat_host_firewall_mesh_dns_port`, `goat_zenoh_mesh_iface` | yes; drop it on a core that does not serve it |
| `other-tcp`, `other-udp` | both | nothing in the baseline | — | the named rules a host that serves more loosens, with a reason |

`<if>` is the peering's tunnel interface (`wg-fed0`, `ipsec-<name>`). On the edge, ADR 1219
entitles nothing else inbound from the mesh: platform admins reach it on SSH, which
`ssh-management` covers when the management sources hold them.

**One UDP port, one owner.** The mesh port and every peering's tunnel, IKE and carrier ports must
differ on one host; the renderer refuses an overlap (GB-210). The mesh rule admits any source, so
an overlap would void the tunnel rule's source restriction, and two peerings on one port cannot
both work. goat-peering-wg and a mesh client both default to UDP 51820, which is the collision the
9 November run hit.

**Several peerings on one edge.** Each peering's run stores its entry on the host
(`/etc/goat-host-firewall/peerings/<if>.json`), and the edge's ruleset admits every peering stored
there. A second peering's run therefore never cuts the first. Before loading, the role lists the
peering services on the host (`goat-peering-wg@*`, `goat-peering-link@*`) and refuses (GB-210)
while any of them has no stored entry; the refused run stores its own entry first. So switching
the baseline on for an edge that already carries several peerings means running each peering's
play once, and the last of those runs loads the ruleset for all of them. Taking a peering out has
to remove its file as well, or its rules stay: not built.

**Services sharing the host.** A core router host often runs more (a management server, a
portal). Before loading, the role reads the host's listening sockets (`ss -Hlntup`) and refuses
(GB-215) a ruleset that would close a port listening on a non-loopback address unless a rule
admits it (loosen `other-tcp` or `other-udp`, with a reason) or
`goat_host_firewall_closed_listeners` names it as closed on purpose. The ports it closes are
printed and returned in the renderer's JSON as `closing`.

**One profile per host.** The host records its profile; an edge play on a host that carries the
core profile, or the reverse, is refused (GB-210).

## 4. The override contract

`goat_host_firewall_override`, in the deployment's group_vars, maps a rule name to a change:

```yaml
goat_host_firewall_override:
  icmp-echo: {drop: true}                       # tighten: no reason needed
  fabric-tls: {sources: [100.127.0.0/16]}       # tighten: narrower than any source
  other-tcp:                                    # loosen: a named rule, with a reason
    ports: [9100]
    reason: "node exporter, scraped by the site's monitoring host"
```

| Change | Tightening when | Otherwise |
|---|---|---|
| `drop: true` | always | — |
| `sources: [...]` | every new source lies inside a baseline source (a baseline of any source is narrowed by any list) | loosening; an empty list means any source |
| `ports: [...]` | a subset of the baseline's ports | loosening |
| `interface: <if>` | the baseline had no interface | loosening when it names another |

- Tightening needs nothing else.
- Loosening needs `reason:`, a non-empty string. It is written into the ruleset file
  (`# loosened <rule>: <change>; reason: <reason>`) and into the renderer's JSON, so the reason
  travels with the evidence. Without one: **GB-212**.
- An unknown rule name, a fixed rule, an unknown key, a change a fixed-expression rule cannot
  take, or an entry that changes nothing: **GB-211**. A new allow rule is not a named rule, so it
  is refused; a service the host also runs goes in `other-tcp` or `other-udp`.
- One refused entry refuses the whole override, before anything is applied.
- `--list-rules` prints the names a given input produces.

## 5. Never cutting the managing session

1. `goat_host_firewall_management_sources` is required; empty is **GB-210**.
2. Before rendering, the role reads the SSH session running the play (`SSH_CONNECTION`, read
   without become) and the interface the host routes the reply out of. The renderer refuses a
   ruleset that no accept rule admits that session through: **GB-213**. Over SSH (`ssh`,
   `paramiko`, with or without the `ansible.builtin.` prefix), a session that cannot be read is
   also GB-213. Over a local or container connection there is no SSH session to protect.
3. A run that starts while an earlier run's rollback is still armed is refused (**GB-214**):
   that timer must fire or be confirmed first.
4. Loading is one `nft -f` transaction (the file declares, deletes and re-declares the one table).
   Before it, the role copies the last confirmed ruleset (or, the first time, a file that removes
   the table) to `rollback.nft` and arms
   `systemd-run --on-active=goat_host_firewall_rollback_seconds` to load it. After it, the role
   drops its SSH connection and makes a fresh one; only when that succeeds, and while the timer
   is still armed, does it stop the timer, keep the file for boot (`goat-host-firewall.service`,
   ordered after `nftables.service`, whose stock file flushes the ruleset) and record the hash.
   If the fresh connection fails, or the timer has already fired, the play fails (**GB-214**)
   and the last confirmed ruleset is what is loaded.
5. Idempotent: a run whose rendered hash equals the recorded one, and whose loaded table (listed
   without counters) equals what the last run recorded, loads nothing and changes nothing. A hand
   edit or a rollback that fired shows as a difference and is loaded over.

## 6. Off by default

`goat_host_firewall_baseline: false`. The plays include the role only when it is true, so an
existing deployment sees no change. Switching it on by default is a separate later change.

To take it off a host: `systemctl disable --now goat-host-firewall` (its stop removes the table),
then set the switch back to false.

## 7. Rendering without applying

```
python3 ops/ansible/roles/goat-host-firewall/files/goat-host-firewall-render.py \
  --input <input.json|-> [--override <group_vars file>] [--format text|json] [--list-rules]
```

Input is one JSON document per host, schema `goat-host-firewall-input/v1` (documented at the top
of the script; fixtures in `roles/goat-host-firewall/tests/fixtures/`). `--override` takes a JSON
or YAML file holding the override or a `goat_host_firewall_override:` key, so a group_vars file
works as is. Text output is the ruleset followed by `# sha256 <hex>` over every byte before it;
JSON output (`goat-host-firewall-render/v1`) carries the same hash, the rules, and what was
tightened and loosened, with reasons. The role calls the same script on the controller before it
touches the host, and prints the hash it loads, so the planner and the deploy agree on the bytes.
Exit 0, or 2 (GB-210), 3 (GB-211, GB-212), 4 (GB-213), 5 (GB-215). The live-host facts
(`must_reach`, `listening` or `listening_ss`, `uplink_interfaces`) are checks only, never part of
the ruleset or its hash, so a planner that cannot read them gets the same hash the role loads. The planner (`goatbuilder federate plan`,
stage 6) is to call this and report the ruleset and hash before anything is applied: designed,
not built.

## 8. How it is proven

| Check | What | State |
|---|---|---|
| `roles/goat-host-firewall/tests/test_render.py` | render of edge and core under wireguard, wireguard via forwarder and ipsec-fips; two peerings on one edge; UDP port overlap; every override outcome; lockout; listeners; bridge-named uplinks; hash stability | measured |
| `roles/goat-host-firewall/tests/netns-behavior.sh` | the rendered rulesets loaded into throwaway network namespaces and scanned from the management source, an outsider, the partner endpoint, the tunnel and the mesh; a tightening override; a loosening through a published (DNAT) port | measured |
| `ops/ci/test-host-firewall-role.sh` | the role over SSH into a systemd container: converge, second run changes nothing, check mode loads nothing, scans, each refusal (GB-210..215) leaves the loaded table unchanged, tightening and loosening, a peering the host carries without an entry refused, a second peering keeps the first, and the rollback timer restoring SSH after a cutting ruleset armed by the same commands the role runs. The role's own failed-confirm path is not exercised | measured |
| port-scan scenario in the federation test environment | from the partner side and the mesh side, the edge and core answer on exactly the declared ports; baseline and one tightening override; the three-party case with the baseline on | requested, not run |
