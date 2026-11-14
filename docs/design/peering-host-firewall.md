# Peering host firewall: a baseline on the edge host and the core router host

| Field | Value |
|---|---|
| Status | **Built: required on an edge, opt-in on a core, off by default.** Role `roles/goat-host-firewall`, included by `ops/ansible/peering-wg.yml` and `peering-edge.yml` (profile edge, phase 11) and `peering-republisher.yml` (profile core, phase 12) when `goat_host_firewall_baseline` is true. The edge plays (phases 2, 11 and 12) refuse an edge with it off (**GB-279**, §6): built, unit-tested by running the plays (`roles/goat-host-firewall/tests/test_edge_requires_baseline.py`), not yet measured in the federation test environment. Renderer `roles/goat-host-firewall/files/goat-host-firewall-render.py`. **Measured** in network namespaces (`tests/netns-behavior.sh`) and over SSH in containers (`ops/ci/test-host-firewall-role.sh`). Not yet run on two real environments. The planner's report of the ruleset (§7): not built in this branch; the planner records the render command as its plan stage 6.4 |
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
does not change another table's rules. It does change what reaches the services behind those
tables: traffic that docker, the mesh client or goat-peering-wg would admit is still dropped
here unless a rule of this table admits it too. That is its purpose, and its cost: for
example, with `goat_host_firewall_mesh_udp_ports` empty, a mesh peer cannot open a direct UDP
path to this host, so traffic between the two goes through the mesh's relay (slower, still
working) unless the host itself opened the path first.

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
| `container-bridges` | both | the host's own containers (`docker0`, `br-*`) reaching services on the host, and in `published` their connections to ports other containers publish. A pattern, not a list of names, because compose creates new docker networks after this phase (phase 12). `br-l2*` (VXLAN segment bridges) is dropped ahead of the pattern, so a segment created after a run stays closed until the next | — | drop only; dropping it removes both. Refused (GB-210) while the host has any link named `br-*` that is not docker's (`br-<12 hex>`), such as a VXLAN segment bridge or Open vSwitch's `br-int` / `br-ex`, or a `br-*` uplink, unless it is dropped |
| `ssh-management` | both | TCP SSH from the management sources only | `goat_host_firewall_management_sources` (required), `goat_host_firewall_ssh_port` (`ansible_port`) | yes. A `/0` source is refused (GB-210); a source broader than `goat_host_firewall_management_prefix_floor` (/24, /64), or a floor set broader than those, is a loosening that needs `ssh-management: {reason: ...}` (GB-212) |
| `mesh-wireguard` | both | the mesh client's own WireGuard UDP port, when listed. Empty by default: the mesh client's own outbound connections are answered as established, and a peer's direct path that this host did not open goes through the mesh relay instead | `goat_host_firewall_mesh_udp_ports` | yes. Listing any port is a loosening that needs `mesh-wireguard: {reason: ...}` (GB-212) |
| `<if>/tunnel` | edge, wireguard | the tunnel UDP port from the partner's literal endpoint (any source when the package names a host name) | `peering_wg_listen_port`, the counterparty package's `transport_offer.endpoint` | yes |
| `<if>/tunnel` | edge, wireguard via forwarder | the tunnel UDP port on the carrier, from the forwarder's carrier address | `peering_wg_carrier_interface`, `peering_wg_carrier_remote_ip` | yes |
| `<if>/carrier` | edge, via forwarder | the carrier's own UDP port, from the forwarder's literal address | `peering_wg_carrier_listen_port`, `peering_wg_forwarder_endpoint` | yes |
| `<if>/ike`, `<if>/esp` | edge, ipsec-fips | IKE (UDP `peering_edge_ike_port`, 4500) and ESP from the partner's literal address | `peering_edge_ike_port`, `peering_edge_remote_addr` | yes |
| `<if>/edge-router` | edge | the edge router's port, **on the tunnel interface only**, from the far link address | `goat_zenoh_edge_listen_port` (`peering_wg_fabric_port`), the tunnel interface, `goat_peering_remote_link_ip` | yes |
| `<if>/tunnel-extra` | edge | the other ports the tunnel carries (the SPIRE bundle endpoint), on the tunnel only | `peering_wg_extra_ports` | yes |
| `<if>/spire-forward` | edge, when `peering_spire_enabled` | the port of goat-peering-spire's outbound forward, from our SPIRE server's literal address, else on the mesh interface only; phase 13 opens it after phase 11 loads the baseline, so it is admitted ahead of time. ADR 1219 does not list this reach; it is the SPIRE step's existing design, made visible here | `peering_spire_outbound_port`, `peering_spire_server_addr` | yes |
| `fabric-tls` | core | the core router's fabric TLS port, any source (mutual TLS decides) | `goat_zenoh_listen_port` | yes |
| `fabric-quic` | core | the QUIC listener, when `goat_zenoh_quic_enabled` | `goat_zenoh_quic_port` | yes |
| `mesh-dns` | core | the mesh nameserver, on the mesh interface only (ADR 1219 decision 4) | `goat_host_firewall_mesh_dns_port`, `goat_host_firewall_mesh_interface` (`goat_zenoh_mesh_iface`) | yes; drop it on a core that does not serve it. A port other than 5353 needs `mesh-dns: {reason: ...}` (GB-212). A mesh interface that is the uplink is refused (GB-210) |
| `spire-bundle` | core, when `peering_spire_enabled` and this host is `peering_spire_server_host` | our SPIRE server's bundle endpoint, reached by the edge's inbound forward (goat-peering-spire), on the mesh interface only (the edge's inventory address may be an underlay address) | `peering_spire_bundle_port`, `peering_spire_server_host` | yes |
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
while any of them has no stored entry; the refused run first checks its own entry alone
(`--peering-only`, which leaves out override entries naming another peering's rules) and stores
it, creating `/etc/goat-host-firewall` (0700) on a fresh host. So switching
the baseline on for an edge that already carries several peerings means running each peering's
play once, and the last of those runs loads the ruleset for all of them. Taking a peering out has
to remove its file as well, or its rules stay: not built.

**Services sharing the host.** A core router host often runs more (a management server, a
portal). Before loading, the role reads the host's listening sockets (`ss -Hlntup`) and refuses
(GB-215) a ruleset that would close a port listening on a non-loopback address unless a rule
admits it (loosen `other-tcp` or `other-udp`, with a reason) or
`goat_host_firewall_closed_listeners` names it as closed on purpose, by `tcp/<port>`,
`udp/<port>` or process name. The DHCP client sockets (UDP 68 and 546) count as admitted while
`dhcp-client` is, and UDP sockets on the host's ephemeral ports (`ip_local_port_range`) are
client sockets and are not counted. The ports it closes are
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
- Loosening needs `reason:`, a non-empty string on one line. It is written into the ruleset
  file (`# loosened <rule>: <change>; reason: <reason>`) and into the renderer's JSON, so the
  reason travels with the evidence. Without one: **GB-212**. A reason holding a line break or
  another control character (a YAML `|` block, for one) is refused (**GB-211**), because a line
  break would end the comment and let the rest run as nftables commands; a folded `>` reason,
  which ends in one newline, is accepted with that newline trimmed.
- A **role input** wider than the profile is a loosening too, and needs the same reason, given
  as an entry that names the rule and only a reason: `goat_host_firewall_mesh_udp_ports` not
  empty (`mesh-wireguard`), a `goat_host_firewall_mesh_dns_port` other than 5353
  (`mesh-dns`), a management source broader than `goat_host_firewall_management_prefix_floor`
  (`ssh-management`), or a floor set broader than /24 or /64 (`ssh-management`). Without one: **GB-212**. A `/0` management source, and a mesh interface
  that is the host's uplink, are refused outright (**GB-210**).
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
   that timer must fire or be confirmed first. Twice `goat_host_firewall_confirm_timeout`
   plus 15 s must be below `goat_host_firewall_rollback_seconds` (**GB-210**).
4. Loading is one `nft -f` transaction (the file declares, deletes and re-declares the one table).
   Before it, the role copies the last confirmed ruleset (or, the first time, a file that removes
   the table) to `rollback.nft` and arms
   `systemd-run --on-active=goat_host_firewall_rollback_seconds` to load it. After it, the role
   drops its SSH connection and confirms with a fresh Ansible connection made with
   multiplexing off (`-o ControlMaster=no -o ControlPath=none` ahead of the inventory's own ssh
   arguments), so it uses everything Ansible resolves: user, key or password, port, ssh_config,
   ansible.cfg, jump host, host key policy. A host managed with a password works. Only when that
   succeeds, and while the timer is still armed, does it stop the timer, keep the file for boot (`goat-host-firewall.service`,
   ordered after `nftables.service`, whose stock file flushes the ruleset) and record the hash.
   If the confirmation fails, or the timer has already fired, the play fails (**GB-214**)
   and the last confirmed ruleset is what is loaded. The rollback and the boot unit run the
   `nft` that `command -v nft` finds on the host.
5. Idempotent: a run whose rendered hash equals the recorded one, and whose loaded table (listed
   without counters) equals what the last run recorded, loads nothing and changes nothing. A hand
   edit or a rollback that fired shows as a difference and is loaded over.

## 6. Off by default

`goat_host_firewall_baseline: false` is the role's default, and it does not change. The plays
include the role only when it is true.

**The peering plays require the host firewall baseline on an edge.** Every play a peering
standup runs on an edge (`peering-prepare.yml` in phase 2, `peering-wg.yml` or
`peering-edge.yml` in phase 11, `peering-zenoh-edge.yml` in phase 12) runs
`roles/goat-host-firewall/tasks/require.yml` as its first task, before anything on the host
changes. With `goat_host_firewall_baseline` not true for the edge, it refuses (**GB-279**),
naming the host, the setting and the fix: set `goat_host_firewall_baseline: true` and
`goat_host_firewall_management_sources` in the edge's group_vars
(`group_vars/<edge group>/host-firewall.yml`, from
`group_vars/_examples/peering-host-firewall.example.yml`), then run the peering again. Phase
11's play applies the baseline; there is no separate play for it. `goatbuilder federate plan`
shows such an edge as BLOCKED with GB-279 ([federation-plan.md](federation-plan.md), "Host
firewall"). A core host keeps the baseline opt-in. The back-out (`peering-backout.yml`) does not
check it, so a peering can always be taken down.

An existing deployment whose edges have the baseline off is refused at its next peering run
until the edge's group_vars switch it on.

Turning the switch back to false does not unload a baseline a host carries: its table stays,
and `goat-host-firewall.service` reloads it at boot. On an edge the switch cannot be false for a
peering run (GB-279). On a core, with the switch false, the core play reads `/etc/goat-host-firewall/profile` (one read-only task) and refuse (**GB-210**) while it
exists, naming the command. To take the baseline off a host:
`systemctl disable --now goat-host-firewall` (its stop removes the table), then
`rm -rf /etc/goat-host-firewall`, then run with the switch false.

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
(`must_reach`, `listening` or `listening_ss`, `uplink_interfaces`, `bridge_interfaces`,
`ephemeral_ports`) are checks only, never part of the ruleset or its hash, so a planner that
cannot read them gets the same hash the role loads. `--peering-only <name>` validates one
peering's entry alone; the role uses it before storing an entry, and nothing it renders is loaded. The planner (`goatbuilder federate plan`,
stage 6) calls this and reports the ruleset and hash before anything is applied; the planner
records it as its plan stage 6.4, and that call is not built in this branch.

## 8. How it is proven

| Check | What | State |
|---|---|---|
| `roles/goat-host-firewall/tests/test_render.py` | render of edge and core under wireguard, wireguard via forwarder and ipsec-fips; two peerings on one edge; UDP port overlap; every override outcome; reasons with line breaks; role inputs that loosen (management prefix floor, mesh port, mesh DNS port, mesh interface on the uplink); lockout; listeners (DHCP, ephemeral, process names); non-docker bridges; SPIRE rules; `--peering-only`; hash stability | measured |
| `roles/goat-host-firewall/tests/test_play_vars.py` | the core play puts `spire-bundle` only on the SPIRE server host and never sources it from the edge's inventory address | measured |
| `roles/goat-host-firewall/tests/netns-behavior.sh` | the rendered rulesets loaded into throwaway network namespaces and scanned from the management source, an outsider, the partner endpoint, the tunnel and the mesh; a tightening override; a loosening through a published (DNAT) port | measured |
| `ops/ci/test-host-firewall-role.sh` | the role over SSH into a systemd container: converge, second run changes nothing, check mode loads nothing, scans, each refusal (GB-210..215) leaves the loaded table unchanged, tightening and loosening, the boot unit running the host's own `nft`, a non-bridge link named `br-*` refused, a confirm timeout that does not fit twice before the rollback refused, a password-managed host loading and confirming with multiplexing off; from a host with no `/etc/goat-host-firewall`, a peering the host carries without an entry refused while the refused run stores its own (even with an override naming the other peering), the other peering's run then loading both; the rollback timer restoring SSH after a cutting ruleset armed by the same commands the role runs; the switch turned off refused while the baseline is loaded. The role's own failed-confirm path is not exercised | measured |
| port-scan scenario in the federation test environment | from the partner side and the mesh side, the edge and core answer on exactly the declared ports; baseline and one tightening override; the three-party case with the baseline on | requested, not run |
