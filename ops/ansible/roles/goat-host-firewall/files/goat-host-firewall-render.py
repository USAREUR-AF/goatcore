#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Render the host firewall baseline of a peering edge or core host, without applying it.

One renderer for two callers: the goat-host-firewall role (before it loads anything) and
the federation planner (to report the ruleset before any change). Both get the same text
and the same hash for the same input.

Input is one JSON document (schema goat-host-firewall-input/v1, below). The deployment's
override can come inside it or from --override, a JSON or YAML file holding either the
override itself or a `goat_host_firewall_override:` key (a group_vars file works as is).

What it prints:
  --format text   the nftables ruleset, then one line `# sha256 <hex>`; the hash covers
                  every byte before that line
  --format json   {"schema": "goat-host-firewall-render/v1", "profile", "sha256",
                  "ruleset", "rules", "tightened", "loosened"}
  --list-rules    the rule names an override may use, one per line, and what each allows

Refusals, each with its code from ops/errors/catalog.yaml:
  exit 2  GB-210  the input is incomplete or malformed
  exit 3  GB-211  the override names an unknown or fixed rule, or an unknown key
  exit 3  GB-212  the override loosens a rule and records no reason
  exit 4  GB-213  the ruleset would drop a session listed in must_reach
  exit 5  GB-215  the ruleset would close a listening port nobody acknowledged

Input (goat-host-firewall-input/v1):
  {
    "schema": "goat-host-firewall-input/v1",
    "profile": "edge" | "core",
    "host": {
      "ssh_port": 22,
      "management_sources": ["198.51.100.10/32"],      # required, non-empty
      "mesh_interface": "wt0",
      "mesh_udp_ports": [51820],                       # the mesh's own WireGuard port(s)
      "fabric_tcp_port": 7447,                         # core only
      "fabric_udp_port": null,                         # core only; QUIC when enabled
      "fabric_sources": [],                            # core only; empty = any source
      "mesh_dns_port": 5353,                           # core only; null leaves it out
      "uplink_interfaces": ["eth0"],                   # optional; refused when named br-*
      "bridge_interfaces": ["docker0"],                # optional; non-docker br-* refused
      "ephemeral_ports": [32768, 60999],               # optional; UDP client sockets skipped
      "management_prefix_floor": {"ipv4": 24, "ipv6": 64},   # broader sources need a reason
      "spire_bundle_port": 8443,                       # core only, optional
      "spire_bundle_sources": ["100.127.0.20"],        # core only; the edge's mesh address
      "closed_listeners": ["tcp/8000", "chronyd"]      # optional; ports or processes closed on purpose
    },
    "peerings": [                                      # edge only, one per peering
      {"name": "wg-fed0", "transport": "wireguard", "listen_port": 51820,
       "peer_addr": "192.0.2.10", "tunnel_interface": "wg-fed0",
       "remote_link_ip": "169.254.10.0", "edge_port": 7448, "extra_tcp_ports": [],
       "spire_outbound_port": 18443, "spire_server_addr": "100.127.0.5",   # optional
       "via_forwarder": false, "carrier_interface": "wg-carry0",
       "carrier_remote_ip": "169.254.30.0", "carrier_listen_port": 51822,
       "forwarder_addr": ""},
      {"name": "ipsec-xab", "transport": "ipsec-fips", "ike_ports": [500, 4500],
       "peer_addr": "", "tunnel_interface": "ipsec-xab",
       "remote_link_ip": "169.254.11.0", "edge_port": 7448, "extra_tcp_ports": []}
    ],
    "override": {"<rule name>": {"drop": true} | {"sources": [...]} | {"ports": [...]}
                                | {"interface": "..."}, "reason": "..."},
    "must_reach": [{"saddr": "198.51.100.10", "proto": "tcp", "dport": 22, "iif": "eth0"}],
    "listening": [{"proto": "tcp", "port": 8000, "addr": "0.0.0.0", "process": "x"}],
    "listening_ss": "<the output of ss -Hlntup>"     # read when listening is absent
  }
  must_reach, listening, listening_ss, uplink_interfaces, bridge_interfaces and
  ephemeral_ports are facts of the live host: checks only, never part of the ruleset or
  its hash. A role input wider than the profile (mesh_udp_ports, a mesh_dns_port other than
  5353, a management source broader than the floor) is a loosening: the override names the
  rule with a reason, e.g. {"mesh-wireguard": {"reason": "..."}}.

usage: goat-host-firewall-render.py --input FILE|- [--override FILE] [--format text|json]
       goat-host-firewall-render.py --input FILE|- --peering-only NAME
       goat-host-firewall-render.py --input FILE|- --list-rules
"""
import argparse
import hashlib
import ipaddress
import json
import re
import sys

INPUT_SCHEMA = "goat-host-firewall-input/v1"
RENDER_SCHEMA = "goat-host-firewall-render/v1"
TABLE = "goat_host_firewall"
IFNAME = re.compile(r"^[A-Za-z0-9_.-]{1,15}$")
RULE_NAME = re.compile(r"^[a-z0-9][a-z0-9./-]*$")
PEERING_NAME = re.compile(r"^[a-z0-9][a-z0-9.-]*$")
OVERRIDE_KEYS = {"drop", "sources", "ports", "interface", "reason"}
# What the profile itself would choose for the values a deployment may set as role
# inputs. An input wider than these loosens the baseline as much as an override does,
# and needs the same recorded reason.
PROFILE_MESH_DNS_PORT = 5353
PROFILE_MANAGEMENT_PREFIX_FLOOR = {"ipv4": 24, "ipv6": 64}
DOCKER_BRIDGE = re.compile(r"^(docker0|br-[0-9a-f]{12})$")
PROCESS_NAME = re.compile(r"^[A-Za-z0-9_.@:+-]{1,64}$")
CONTROL = re.compile(r"[\x00-\x1f\x7f]")


class Refusal(Exception):
    def __init__(self, code, exit_code, message):
        super().__init__(message)
        self.code = code
        self.exit_code = exit_code


def incomplete(msg):
    return Refusal("GB-210", 2, msg)


class Rule:
    """One accept rule. `sources` empty means any source; `iif` None means any interface.

    A rule with `expr` is a fixed expression (loopback, ICMP): an override may drop it and
    nothing else. A `locked` rule cannot be overridden at all."""

    def __init__(self, name, proto=None, ports=None, sources=None, iif=None, expr=None,
                 locked=False, published=True, note=""):
        self.name = name
        self.proto = proto
        self.ports = sorted(set(int(p) for p in (ports or [])))
        self.sources = sorted(set(sources or []), key=_net_sort_key)
        self.iif = iif
        self.expr = expr
        self.locked = locked
        self.published = published and expr is None and proto in ("tcp", "udp")
        self.note = note
        self.dropped = False
        # Set when a role input, not the override, made this rule wider than the profile.
        self.input_loosens = []

    def as_dict(self):
        d = {"name": self.name, "note": self.note}
        if self.expr is not None:
            d["expr"] = self.expr
        else:
            d.update({"proto": self.proto, "ports": self.ports,
                      "sources": self.sources, "interface": self.iif})
        if self.locked:
            d["locked"] = True
        return d


def _net_sort_key(s):
    n = ipaddress.ip_network(s, strict=False)
    return (n.version, int(n.network_address), n.prefixlen)


def _net(s, where):
    try:
        return str(ipaddress.ip_network(str(s), strict=False))
    except ValueError:
        raise incomplete(f"{where}: {s!r} is not an address or CIDR")


def _addr_or_empty(s, where):
    s = str(s or "").strip()
    if not s:
        return ""
    try:
        return str(ipaddress.ip_address(s))
    except ValueError:
        raise incomplete(f"{where}: {s!r} is not a literal address (leave it empty to admit any)")


def _port(v, where):
    try:
        p = int(v)
    except (TypeError, ValueError):
        raise incomplete(f"{where}: {v!r} is not a port number")
    if not 1 <= p <= 65535:
        raise incomplete(f"{where}: {p} is not a port number")
    return p


def _ports(vs, where):
    if not isinstance(vs, list):
        raise incomplete(f"{where}: must be a list of port numbers")
    return [_port(v, where) for v in vs]


def _ifname(v, where):
    v = str(v or "")
    if not IFNAME.match(v):
        raise incomplete(f"{where}: {v!r} is not an interface name (1-15 of A-Za-z0-9_.-)")
    return v


def _bool(v):
    return str(v).strip().lower() in ("true", "1", "yes")


def _req(d, key, where):
    if key not in d or d[key] in (None, ""):
        raise incomplete(f"{where}: {key} is required")
    return d[key]


# --------------------------------------------------------------------------- the baseline

def common_rules(host):
    """Rules both profiles carry, in order."""
    mgmt = host.get("management_sources")
    if not isinstance(mgmt, list) or not mgmt:
        raise incomplete("host.management_sources is required and must list at least one "
                         "address or CIDR: SSH is admitted from these and from nowhere else")
    mgmt = [_net(s, "host.management_sources") for s in mgmt]
    floor = dict(PROFILE_MANAGEMENT_PREFIX_FLOOR)
    floor.update(host.get("management_prefix_floor") or {})
    broad = []
    for fam in ("ipv4", "ipv6"):
        try:
            f = int(floor[fam])
        except (TypeError, ValueError):
            raise incomplete(f"host.management_prefix_floor.{fam} must be a prefix length")
        if f < PROFILE_MANAGEMENT_PREFIX_FLOOR[fam]:
            broad.append(f"management prefix floor {fam} /{f} broader than the profile's "
                         f"/{PROFILE_MANAGEMENT_PREFIX_FLOOR[fam]}")
    for m in mgmt:
        n = ipaddress.ip_network(m)
        if n.prefixlen == 0:
            raise incomplete(f"host.management_sources holds {m}, which admits SSH from "
                             f"anywhere; this is refused even with a reason")
        lim = int(floor["ipv4" if n.version == 4 else "ipv6"])
        if n.prefixlen < lim:
            broad.append(f"management source {m} broader than /{lim}")
    ssh_port = _port(host.get("ssh_port", 22), "host.ssh_port")
    rules = [
        Rule("loopback", expr='iifname "lo" accept', locked=True,
             note="the host talking to itself"),
        Rule("established", expr="ct state established,related accept", locked=True,
             note="replies to what this host or its containers opened"),
        Rule("invalid", expr="ct state invalid drop", locked=True,
             note="packets conntrack cannot place are dropped, never admitted"),
        Rule("icmp-echo", expr="meta l4proto icmp icmp type echo-request limit rate 10/second accept",
             note="ping, rate limited; errors such as fragmentation-needed arrive as related"),
        Rule("icmpv6-essential",
             expr="meta l4proto ipv6-icmp icmpv6 type { nd-neighbor-solicit, nd-neighbor-advert, "
                  "nd-router-advert, packet-too-big, mld-listener-query, echo-request } accept",
             note="IPv6 neighbor discovery, multicast listener queries and path MTU; IPv6 does "
                  "not work without them"),
        Rule("dhcp-client", expr=["udp sport 67 udp dport 68 accept",
                                  "udp sport 547 udp dport 546 accept"],
             note="DHCP and DHCPv6 replies, which conntrack cannot match to the multicast or "
                  "broadcast request"),
        Rule("container-bridges", expr=['iifname "docker0" accept', 'iifname "br-l2*" drop',
                                        'iifname "br-*" accept'],
             note="the host's own containers (docker0, br-*): to services on the host, and to "
                  "ports other containers publish; drop it on a host whose containers must not"),
        Rule("ssh-management", proto="tcp", ports=[ssh_port], sources=mgmt,
             note="SSH from the declared management sources only"),
    ]
    rules[-1].input_loosens = broad
    mesh_if = host.get("mesh_interface")
    mesh_ports = _ports(host.get("mesh_udp_ports", []), "host.mesh_udp_ports")
    if mesh_ports:
        rules.append(Rule("mesh-wireguard", proto="udp", ports=mesh_ports,
                          note="the mesh's own WireGuard port, so mesh peers can reach this "
                               "host directly; the mesh ACL decides what crosses it"))
        rules[-1].input_loosens = [f"mesh UDP ports {mesh_ports} (the profile lists none)"]
    if mesh_if:
        _ifname(mesh_if, "host.mesh_interface")
    return rules


def edge_rules(host, peerings):
    rules = common_rules(host)
    if not isinstance(peerings, list) or not peerings:
        raise incomplete("an edge profile needs at least one entry in peerings")
    seen = set()
    for i, p in enumerate(sorted(peerings, key=lambda p: str(p.get("name", "")))):
        where = f"peerings[{p.get('name', i)}]"
        name = str(_req(p, "name", where))
        if not PEERING_NAME.match(name):
            raise incomplete(f"{where}: name {name!r} must be lowercase a-z0-9.- (no '/', "
                             f"which separates a peering from its rule)")
        if name in seen:
            raise incomplete(f"{where}: two peerings are both named {name!r}")
        seen.add(name)
        transport = _req(p, "transport", where)
        tun = _ifname(_req(p, "tunnel_interface", where), f"{where}.tunnel_interface")
        remote = _addr_or_empty(_req(p, "remote_link_ip", where), f"{where}.remote_link_ip")
        edge_port = _port(_req(p, "edge_port", where), f"{where}.edge_port")
        peer = _addr_or_empty(p.get("peer_addr"), f"{where}.peer_addr")
        peer_src = [peer] if peer else []
        if transport == "wireguard":
            listen = _port(_req(p, "listen_port", where), f"{where}.listen_port")
            if _bool(p.get("via_forwarder", False)):
                carrier = _ifname(_req(p, "carrier_interface", where), f"{where}.carrier_interface")
                carrier_remote = _addr_or_empty(_req(p, "carrier_remote_ip", where),
                                                f"{where}.carrier_remote_ip")
                carrier_port = _port(_req(p, "carrier_listen_port", where),
                                     f"{where}.carrier_listen_port")
                fwd = _addr_or_empty(p.get("forwarder_addr"), f"{where}.forwarder_addr")
                rules.append(Rule(f"{name}/tunnel", proto="udp", ports=[listen],
                                  sources=[carrier_remote], iif=carrier,
                                  note="the peering tunnel port, relayed down the forwarder's carrier"))
                rules.append(Rule(f"{name}/carrier", proto="udp", ports=[carrier_port],
                                  sources=[fwd] if fwd else [],
                                  note="the carrier this edge dials to its forwarder"))
            else:
                rules.append(Rule(f"{name}/tunnel", proto="udp", ports=[listen], sources=peer_src,
                                  note="the peering WireGuard port, from the partner's endpoint "
                                       "when the package names a literal address"))
        elif transport == "ipsec-fips":
            ike = _ports(p.get("ike_ports", [500, 4500]), f"{where}.ike_ports")
            rules.append(Rule(f"{name}/ike", proto="udp", ports=ike, sources=peer_src,
                              note="IKE and NAT-T from the partner's edge"))
            rules.append(Rule(f"{name}/esp", proto="esp", sources=peer_src,
                              note="ESP from the partner's edge"))
        else:
            raise incomplete(f"{where}: transport {transport!r} is neither wireguard nor ipsec-fips")
        rules.append(Rule(f"{name}/edge-router", proto="tcp", ports=[edge_port],
                          sources=[remote] if remote else [], iif=tun,
                          note="the edge router's listener, on the tunnel interface, from the "
                               "far link address"))
        extra = _ports(p.get("extra_tcp_ports", []), f"{where}.extra_tcp_ports")
        if extra:
            rules.append(Rule(f"{name}/tunnel-extra", proto="tcp", ports=extra,
                              sources=[remote] if remote else [], iif=tun,
                              note="the other ports the transport role carries across the "
                                   "tunnel (the SPIRE bundle endpoint)"))
        if p.get("spire_outbound_port") not in (None, ""):
            sport = _port(p["spire_outbound_port"], f"{where}.spire_outbound_port")
            server = _addr_or_empty(p.get("spire_server_addr"), f"{where}.spire_server_addr")
            mesh_if = host.get("mesh_interface") or None
            if not server and not mesh_if:
                raise incomplete(f"{where}: spire_server_addr is not a literal address and "
                                 f"host.mesh_interface is empty, so the SPIRE forward would "
                                 f"be admitted from anywhere")
            rules.append(Rule(f"{name}/spire-forward", proto="tcp", ports=[sport],
                              sources=[server] if server else [],
                              iif=None if server else _ifname(mesh_if, "host.mesh_interface"),
                              note="our SPIRE server fetching the partner's bundle through the "
                                   "edge's outbound forward (goat-peering-spire, phase 13)"))
    rules += other_rules()
    _refuse_udp_overlap(rules)
    return rules


def _refuse_udp_overlap(rules):
    """A peering's UDP port that the mesh rule also admits from any source would void the
    peering rule's source restriction, and two peerings on one port cannot both work."""
    owner = {}
    for r in rules:
        if r.proto != "udp" or r.expr is not None or not r.ports:
            continue
        if r.name.startswith("other-"):
            continue
        for p in r.ports:
            if p in owner:
                raise incomplete(
                    f"UDP {p} is claimed by both {owner[p]} and {r.name}. The mesh client's "
                    f"WireGuard port and each peering's tunnel, IKE and carrier ports must all "
                    f"differ on one host (the tunnel port already held by the mesh is the "
                    f"failure preflight reports as edge ports free)")
            owner[p] = r.name


def core_rules(host):
    rules = common_rules(host)
    fabric = _port(_req(host, "fabric_tcp_port", "host"), "host.fabric_tcp_port")
    sources = [_net(s, "host.fabric_sources") for s in host.get("fabric_sources", [])]
    rules.append(Rule("fabric-tls", proto="tcp", ports=[fabric], sources=sources,
                      note="the core router's fabric TLS listener; the edge dials it (ADR 1219)"))
    if host.get("fabric_udp_port"):
        rules.append(Rule("fabric-quic", proto="udp",
                          ports=[_port(host["fabric_udp_port"], "host.fabric_udp_port")],
                          sources=sources, note="the core router's QUIC listener, when enabled"))
    if host.get("mesh_dns_port"):
        mesh_if = host.get("mesh_interface")
        if not mesh_if:
            raise incomplete("host.mesh_interface is required when host.mesh_dns_port is set")
        rules.append(Rule("mesh-dns", proto="udp",
                          ports=[_port(host["mesh_dns_port"], "host.mesh_dns_port")],
                          iif=_ifname(mesh_if, "host.mesh_interface"),
                          note="the mesh nameserver, on the mesh interface only (ADR 1219 "
                               "decision 4); drop it on a core that does not serve it"))
        if int(host["mesh_dns_port"]) != PROFILE_MESH_DNS_PORT:
            rules[-1].input_loosens = [f"mesh DNS port {host['mesh_dns_port']} (the profile "
                                       f"uses {PROFILE_MESH_DNS_PORT})"]
    if host.get("spire_bundle_port") not in (None, ""):
        bport = _port(host["spire_bundle_port"], "host.spire_bundle_port")
        bsrc = [_addr_or_empty(a, "host.spire_bundle_sources")
                for a in host.get("spire_bundle_sources") or []]
        bsrc = [a for a in bsrc if a]
        mesh_if = host.get("mesh_interface") or None
        if not bsrc and not mesh_if:
            raise incomplete("host.spire_bundle_sources is empty and host.mesh_interface is "
                             "empty, so the SPIRE bundle endpoint would be admitted from anywhere")
        rules.append(Rule("spire-bundle", proto="tcp", ports=[bport], sources=bsrc,
                          iif=None if bsrc else _ifname(mesh_if, "host.mesh_interface"),
                          note="our SPIRE server's bundle endpoint, for the edge's inbound "
                               "forward (goat-peering-spire), from the edge's mesh address"))
    rules += other_rules()
    return rules


def other_rules():
    return [
        Rule("other-tcp", proto="tcp", ports=[],
             note="empty in the baseline; a host that serves more loosens this rule by name "
                  "with a reason"),
        Rule("other-udp", proto="udp", ports=[],
             note="empty in the baseline; as other-tcp"),
    ]


def baseline(doc):
    profile = doc.get("profile")
    host = doc.get("host")
    if not isinstance(host, dict):
        raise incomplete("host is required")
    if profile == "edge":
        return edge_rules(host, doc.get("peerings"))
    if profile == "core":
        if doc.get("peerings"):
            raise incomplete("a core profile takes no peerings; the edge carries them")
        return core_rules(host)
    raise incomplete(f"profile must be edge or core, got {profile!r}")


# --------------------------------------------------------------------------- the override

def _contained(new, old):
    """Every network in `new` lies inside some network of `old`. Empty `old` is any source."""
    if not old:
        return True
    if not new:
        return False
    olds = [ipaddress.ip_network(o) for o in old]
    for n in new:
        nn = ipaddress.ip_network(n)
        if not any(nn.version == o.version and nn.subnet_of(o) for o in olds):
            return False
    return True


def apply_override(rules, override):
    """Returns (tightened, loosened). Raises Refusal before anything is changed if any entry
    is refused, so a partly applied override never exists."""
    if override in (None, {}):
        override = {}
    if not isinstance(override, dict):
        raise Refusal("GB-211", 3, "the override must be a mapping of rule name to change")
    by_name = {r.name: r for r in rules}
    plan = []
    for name in sorted(override):
        entry = override[name]
        if name not in by_name:
            raise Refusal("GB-211", 3,
                          f"override names {name!r}, which is not a rule of this baseline. "
                          f"Rules: {', '.join(r.name for r in rules if not r.locked)}")
        r = by_name[name]
        if not isinstance(entry, dict):
            raise Refusal("GB-211", 3, f"override {name!r}: must be a mapping")
        unknown = set(entry) - OVERRIDE_KEYS
        if unknown:
            raise Refusal("GB-211", 3, f"override {name!r}: unknown key(s) {sorted(unknown)}; "
                                       f"allowed: {sorted(OVERRIDE_KEYS)}")
        if r.locked:
            raise Refusal("GB-211", 3, f"override {name!r}: this rule is fixed and cannot be "
                                       f"overridden ({r.note})")
        changes = set(entry) - {"reason"}
        if not changes and not r.input_loosens:
            raise Refusal("GB-211", 3, f"override {name!r}: names no change "
                                       f"(drop, sources, ports or interface)")
        raw_reason = entry.get("reason")
        if raw_reason is not None and not isinstance(raw_reason, str):
            raise Refusal("GB-211", 3, f"override {name!r}: reason must be a string")
        if CONTROL.search(str(raw_reason or "").strip()):
            raise Refusal("GB-211", 3, f"override {name!r}: the reason holds a line break or "
                                       f"another control character; it is written into the "
                                       f"ruleset file, so it must be one line of text")
        if entry.get("drop") is not None and not isinstance(entry.get("drop"), bool):
            raise Refusal("GB-211", 3, f"override {name!r}: drop must be true or false")
        if entry.get("drop") is True and len(changes) > 1:
            raise Refusal("GB-211", 3, f"override {name!r}: drop cannot be combined with "
                                       f"other changes")
        if r.expr is not None and changes - {"drop"}:
            raise Refusal("GB-211", 3, f"override {name!r}: a fixed-expression rule can only "
                                       f"be dropped")
        loosens = []
        new = {}
        if entry.get("drop") is True:
            new["dropped"] = True
        if "sources" in entry:
            if not isinstance(entry["sources"], list):
                raise Refusal("GB-211", 3, f"override {name!r}: sources must be a list")
            try:
                srcs = sorted({str(ipaddress.ip_network(str(s), strict=False))
                               for s in entry["sources"]}, key=_net_sort_key)
            except ValueError as e:
                raise Refusal("GB-211", 3, f"override {name!r}: {e}")
            if not _contained(srcs, r.sources):
                loosens.append(f"sources {r.sources or ['any']} -> {srcs or ['any']}")
            new["sources"] = srcs
        if "ports" in entry:
            if r.proto not in ("tcp", "udp"):
                raise Refusal("GB-211", 3, f"override {name!r}: a {r.proto} rule has no ports")
            try:
                ports = sorted(set(_ports(entry["ports"], f"override {name!r}.ports")))
            except Refusal as e:
                raise Refusal("GB-211", 3, str(e))
            if not set(ports) <= set(r.ports):
                loosens.append(f"ports {r.ports} -> {ports}")
            new["ports"] = ports
        if "interface" in entry:
            try:
                iif = _ifname(entry["interface"], f"override {name!r}.interface")
            except Refusal as e:
                raise Refusal("GB-211", 3, str(e))
            if r.iif is not None and r.iif != iif:
                loosens.append(f"interface {r.iif} -> {iif}")
            new["iif"] = iif
        reason = str(entry.get("reason") or "").strip()
        if entry.get("drop") is not True:
            loosens = list(r.input_loosens) + loosens
        if loosens and not reason:
            raise Refusal("GB-212", 3,
                          f"override {name!r} loosens the baseline ({'; '.join(loosens)}) and "
                          f"records no reason. Add `reason:` to this entry in the deployment's "
                          f"configuration, or narrow instead")
        plan.append((r, new, loosens, reason))
    named = {name for name in override}
    for r in rules:
        if r.input_loosens and r.name not in named:
            raise Refusal("GB-212", 3,
                          f"{r.name}: a role input loosens the baseline "
                          f"({'; '.join(r.input_loosens)}) and records no reason. Add "
                          f"`{r.name}: {{reason: \"...\"}}` to goat_host_firewall_override")
    tightened, loosened = [], []
    for r, new, loosens, reason in plan:
        if new.get("dropped"):
            r.dropped = True
        if "sources" in new:
            r.sources = new["sources"]
        if "ports" in new:
            r.ports = new["ports"]
        if "iif" in new:
            r.iif = new["iif"]
        if loosens:
            loosened.append({"rule": r.name, "change": "; ".join(loosens), "reason": reason})
        elif new:
            tightened.append({"rule": r.name, "change": ", ".join(sorted(new))})
    return tightened, loosened


# --------------------------------------------------------------------------- lockout

def check_must_reach(rules, must_reach):
    for i, m in enumerate(must_reach or []):
        where = f"must_reach[{i}]"
        if not isinstance(m, dict):
            raise incomplete(f"{where}: must be a mapping")
        try:
            saddr = ipaddress.ip_address(str(_req(m, "saddr", where)))
        except ValueError:
            raise incomplete(f"{where}: saddr is not an address")
        proto = _req(m, "proto", where)
        dport = _port(_req(m, "dport", where), f"{where}.dport")
        iif = m.get("iif") or None
        ok = False
        for r in rules:
            if r.dropped or r.expr is not None or r.proto != proto or dport not in r.ports:
                continue
            if r.iif is not None and r.iif != iif:
                continue
            if r.sources and not any(
                    saddr.version == ipaddress.ip_network(s).version
                    and saddr in ipaddress.ip_network(s) for s in r.sources):
                continue
            ok = True
            break
        if not ok:
            raise Refusal("GB-213", 4,
                          f"the ruleset would drop the managing session {saddr} -> "
                          f"{proto}/{dport}{' on ' + iif if iif else ''}: no accept rule "
                          f"admits it. Add the session's source to "
                          f"goat_host_firewall_management_sources (or fix the override); "
                          f"nothing was applied")


def check_uplinks(rules, host):
    uplinks = host.get("uplink_interfaces") or []
    if not isinstance(uplinks, list):
        raise incomplete("host.uplink_interfaces must be a list of interface names")
    bridges = sorted({u for u in (_ifname(u, "host.uplink_interfaces") for u in uplinks)
                      if u == "docker0" or u.startswith("br-")})
    mesh_if = host.get("mesh_interface")
    if mesh_if and mesh_if in uplinks:
        raise incomplete(f"host.mesh_interface {mesh_if} is the host's uplink, so the rules "
                         f"bound to the mesh interface would be open on the uplink; set it to "
                         f"the mesh client's interface")
    present = host.get("bridge_interfaces") or []
    if not isinstance(present, list):
        raise incomplete("host.bridge_interfaces must be a list of interface names")
    foreign = sorted({b for b in present
                      if str(b).startswith("br-") and not DOCKER_BRIDGE.match(str(b))})
    cb = next((r for r in rules if r.name == "container-bridges"), None)
    if foreign and cb is not None and not cb.dropped:
        raise incomplete(
            f"this host has bridge(s) {', '.join(foreign)} that are not docker's, and the "
            f"container-bridges rule (br-*) would admit everything arriving on them. Drop "
            f"that rule in the override (container-bridges: {{drop: true}})")
    if bridges and cb is not None and not cb.dropped:
        raise incomplete(
            f"the host's uplink {', '.join(bridges)} is named like a container bridge, so the "
            f"container-bridges rule would admit everything arriving on it. Drop that rule in "
            f"the override (container-bridges: {{drop: true}})")


def _listener_key(proto, port):
    return f"{proto}/{int(port)}"


def parse_ss(text):
    """`ss -Hlntup` lines -> [{"proto", "port", "addr", "process"}]."""
    out = []
    for line in (text or "").splitlines():
        cols = line.split()
        if len(cols) < 5 or cols[0] not in ("tcp", "udp"):
            continue
        local = cols[4]
        if ":" not in local:
            continue
        addr, _, port = local.rpartition(":")
        if not port.isdigit():
            continue
        m = re.search(r'users:\(\("([^"]+)"', line)
        out.append({"proto": cols[0], "port": int(port), "addr": addr.strip("[]"),
                    "process": m.group(1) if m else ""})
    return out


def check_listeners(rules, listening, closed, ephemeral=None):
    """Every socket listening on a non-loopback address must be admitted by some rule (from
    some source, on some interface), or named in host.closed_listeners as a port the
    deployment means to close. Returns the ports this ruleset closes."""
    if not listening:
        return []
    if not isinstance(closed, list):
        raise incomplete("host.closed_listeners must be a list such as [\"tcp/8000\"]")
    acknowledged, processes = set(), set()
    for c in closed:
        m = re.match(r"^(tcp|udp)/(\d+)$", str(c))
        if m:
            acknowledged.add(_listener_key(m.group(1), m.group(2)))
        elif PROCESS_NAME.match(str(c)) and "/" not in str(c) and not str(c).isdigit():
            processes.add(str(c))
        else:
            raise incomplete(f"host.closed_listeners: {c!r} is neither tcp/<port>, udp/<port> "
                             f"nor a process name")
    lo = hi = None
    if ephemeral:
        if not (isinstance(ephemeral, list) and len(ephemeral) == 2):
            raise incomplete("host.ephemeral_ports must be [low, high]")
        lo, hi = (_port(x, "host.ephemeral_ports") for x in ephemeral)
    admitted = set()
    dhcp = next((r for r in rules if r.name == "dhcp-client"), None)
    if dhcp is not None and not dhcp.dropped:
        admitted |= {"udp/68", "udp/546"}
    for r in rules:
        if r.dropped or r.expr is not None or r.proto not in ("tcp", "udp"):
            continue
        for p in r.ports:
            admitted.add(_listener_key(r.proto, p))
    closing, unacknowledged = [], []
    for item in listening:
        if not isinstance(item, dict):
            raise incomplete("listening entries must be mappings")
        proto = str(item.get("proto", ""))
        if proto not in ("tcp", "udp"):
            continue
        key = _listener_key(proto, _port(item.get("port"), "listening.port"))
        addr = str(item.get("addr", "")).strip("[]")
        try:
            if addr and ipaddress.ip_address(addr.split("%")[0]).is_loopback:
                continue
            if addr.endswith("%lo"):
                continue
        except ValueError:
            pass
        if key in admitted or key in closing:
            continue
        closing.append(key)
        if proto == "udp" and lo is not None and lo <= int(item.get("port")) <= hi:
            continue   # most likely a client socket: recorded as closed, not refused
        if key not in acknowledged and str(item.get("process") or "") not in processes:
            unacknowledged.append(f"{key} ({item.get('process') or 'unknown process'} on "
                                  f"{addr or '*'})")
    if unacknowledged:
        raise Refusal(
            "GB-215", 5,
            f"the ruleset would close {len(unacknowledged)} port(s) this host listens on: "
            f"{'; '.join(sorted(unacknowledged))}. Admit each by loosening other-tcp or "
            f"other-udp with a reason, or name it in goat_host_firewall_closed_listeners to "
            f"close it deliberately; nothing was applied")
    return sorted(closing)


# --------------------------------------------------------------------------- nftables text

def _src_matches(sources):
    v4 = [s for s in sources if ipaddress.ip_network(s).version == 4]
    v6 = [s for s in sources if ipaddress.ip_network(s).version == 6]
    out = []
    if v4:
        out.append("ip saddr { " + ", ".join(v4) + " } ")
    if v6:
        out.append("ip6 saddr { " + ", ".join(v6) + " } ")
    return out or [""]


def _rule_lines(r, published):
    if r.dropped:
        return []
    if r.expr is not None:
        exprs = r.expr if isinstance(r.expr, list) else [r.expr]
        return [] if published else [f'{e} comment "{r.name}"' for e in exprs]
    if r.proto in ("tcp", "udp") and not r.ports:
        return []
    if published and not r.published:
        return []
    iif = f'iifname "{r.iif}" ' if r.iif else ""
    lines = []
    for src in _src_matches(r.sources):
        if r.proto == "esp":
            match = "meta l4proto esp "
        elif published:
            match = f"meta l4proto {r.proto} ct original proto-dst {{ {', '.join(map(str, r.ports))} }} "
        else:
            match = f"{r.proto} dport {{ {', '.join(map(str, r.ports))} }} "
        lines.append(f'{iif}{src}{match}accept comment "{r.name}"')
    return lines


def render(profile, rules, tightened, loosened):
    out = [
        "#!/usr/sbin/nft -f",
        f"# goat-host-firewall baseline, profile {profile}. Rendered by",
        "# roles/goat-host-firewall/files/goat-host-firewall-render.py; do not edit by hand.",
        "# Inbound only: default deny on the input hook, and on connections forwarded to a",
        "# published container port. Outbound is unchanged. Other tables are untouched.",
    ]
    for t in tightened:
        out.append(f"# tightened {t['rule']}: {t['change']}")
    for lo in loosened:
        out.append(f"# loosened {lo['rule']}: {lo['change']}; reason: {lo['reason']}")
    out += [
        f"table inet {TABLE}",
        f"delete table inet {TABLE}",
        f"table inet {TABLE} {{",
        "  chain input {",
        "    type filter hook input priority filter; policy drop;",
    ]
    for r in rules:
        out += ["    " + line for line in _rule_lines(r, published=False)]
    out += [
        '    counter drop comment "default-deny"',
        "  }",
        "  # A port a container publishes is DNAT-ed and never reaches the input hook, so new",
        "  # connections to one are held to the same rules here. Traffic from the host's own",
        "  # container bridges, and forwarding that is not to a published port, is untouched.",
        "  chain published {",
        "    type filter hook forward priority -10; policy accept;",
        "    ct state established,related accept",
        "    ct status dnat ct direction original jump published_new",
        "  }",
        "  chain published_new {",
    ]
    cb = next((r for r in rules if r.name == "container-bridges"), None)
    if cb is not None and not cb.dropped:
        # A VXLAN segment bridge (br-l2*) made after this run stays closed until the next.
        out += ['    iifname "docker0" return comment "container-bridges"',
                '    iifname "br-l2*" drop comment "container-bridges"',
                '    iifname "br-*" return comment "container-bridges"']
    for r in rules:
        out += ["    " + line for line in _rule_lines(r, published=True)]
    out += [
        '    counter drop comment "default-deny"',
        "  }",
        "}",
    ]
    return "\n".join(out) + "\n"


def sha256(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


# --------------------------------------------------------------------------- main

def load_override(path):
    with open(path) as f:
        raw = f.read()
    try:
        data = json.loads(raw)
    except json.JSONDecodeError:
        try:
            import yaml
        except ImportError:
            raise incomplete(f"{path} is not JSON and PyYAML is not installed to read it as YAML")
        try:
            data = yaml.safe_load(raw)
        except yaml.YAMLError as e:
            raise incomplete(f"{path}: {e}")
    if isinstance(data, dict) and "goat_host_firewall_override" in data:
        data = data["goat_host_firewall_override"]
    return data or {}


def run(doc, override_path=None, peering_only=None):
    if not isinstance(doc, dict):
        raise incomplete("the input must be a JSON object")
    if doc.get("schema") != INPUT_SCHEMA:
        raise incomplete(f"schema must be {INPUT_SCHEMA}, got {doc.get('schema')!r}")
    override = doc.get("override") or {}
    if override_path:
        override = load_override(override_path)
    if peering_only is not None:
        # Validate one peering's entry before the role stores it: that peering alone,
        # the override without the entries that name another peering's rules, and none
        # of the live-host checks. Nothing rendered this way is loaded.
        doc = dict(doc)
        doc["peerings"] = [p for p in doc.get("peerings") or []
                           if str(p.get("name")) == peering_only]
        if not doc["peerings"]:
            raise incomplete(f"--peering-only {peering_only!r}: no such peering in the input")
        if isinstance(override, dict):
            # A peering rule is named <peering>/<rule>; the other peerings may not be
            # stored yet, so every entry for a peering other than this one is left out.
            override = {k: v for k, v in override.items()
                        if "/" not in str(k) or str(k).split("/", 1)[0] == peering_only}
        for k in ("must_reach", "listening", "listening_ss"):
            doc.pop(k, None)
    rules = baseline(doc)
    tightened, loosened = apply_override(rules, override)
    check_uplinks(rules, doc["host"])
    check_must_reach(rules, doc.get("must_reach"))
    listening = doc.get("listening")
    if listening is None and doc.get("listening_ss") is not None:
        listening = parse_ss(doc["listening_ss"])
    closing = check_listeners(rules, listening, doc.get("host", {}).get(
        "closed_listeners", []), doc.get("host", {}).get("ephemeral_ports"))
    text = render(doc["profile"], rules, tightened, loosened)
    return {
        "schema": RENDER_SCHEMA,
        "profile": doc["profile"],
        "sha256": sha256(text),
        "ruleset": text,
        "rules": [r.as_dict() for r in rules if not r.dropped],
        "dropped": [r.name for r in rules if r.dropped],
        "tightened": tightened,
        "loosened": loosened,
        "closing": closing,
    }


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--input", required=True, help="input JSON file, or - for stdin")
    ap.add_argument("--override", help="override file (JSON or YAML); replaces the input's")
    ap.add_argument("--format", choices=("text", "json"), default="text")
    ap.add_argument("--list-rules", action="store_true")
    ap.add_argument("--peering-only", metavar="NAME",
                    help="validate one peering's entry alone (the role's check before it "
                         "stores the entry); nothing rendered this way is loaded")
    a = ap.parse_args(argv)
    try:
        try:
            raw = sys.stdin.read() if a.input == "-" else open(a.input).read()
            doc = json.loads(raw)
        except (OSError, json.JSONDecodeError) as e:
            raise incomplete(f"cannot read the input: {e}")
        if a.list_rules:
            if not isinstance(doc, dict) or not isinstance(doc.get("host"), dict):
                raise incomplete("host is required")
            for r in baseline(doc):
                tag = " (fixed)" if r.locked else ""
                print(f"{r.name}{tag}\t{r.note}")
            return 0
        res = run(doc, a.override, a.peering_only)
    except Refusal as e:
        print(f"goat-host-firewall: REFUSED ({e.code}): {e}", file=sys.stderr)
        return e.exit_code
    if a.format == "json":
        print(json.dumps(res, indent=2, sort_keys=True))
    else:
        sys.stdout.write(res["ruleset"])
        print(f"# sha256 {res['sha256']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
