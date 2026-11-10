# SPDX-License-Identifier: Apache-2.0
"""Render tests for goat-host-firewall-render.py: the edge and core profiles under each
transport, the override contract, the lockout check and the hash.

usage: python3 -m pytest -q ops/ansible/roles/goat-host-firewall/tests/test_render.py
"""
import copy
import importlib.util
import json
import pathlib
import subprocess
import sys

import pytest

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE.parent / "files" / "goat-host-firewall-render.py"
FIX = HERE / "fixtures"

spec = importlib.util.spec_from_file_location("hfr", SCRIPT)
hfr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hfr)


def fixture(name):
    return json.loads((FIX / f"{name}.json").read_text())


def render(doc, override_path=None):
    return hfr.run(copy.deepcopy(doc), override_path)


def refused(doc, code):
    with pytest.raises(hfr.Refusal) as e:
        render(doc)
    assert e.value.code == code, str(e.value)
    return e.value


def lines(res, chain="input"):
    text = res["ruleset"]
    start = text.index(f"  chain {chain} {{")
    end = text.index("  }", start)
    return [l.strip() for l in text[start:end].splitlines()[1:]]


def rule_line(res, name, chain="input"):
    found = [l for l in lines(res, chain) if l.endswith(f'comment "{name}"')]
    return found


# --------------------------------------------------------------------------- profiles

def test_edge_wireguard_admits_exactly_the_declared_ports():
    res = render(fixture("edge-wireguard"))
    names = [r["name"] for r in res["rules"]]
    assert names == [
        "loopback", "established", "invalid", "icmp-echo", "icmpv6-essential", "dhcp-client",
        "container-bridges", "ssh-management",
        "wg-fed0/tunnel", "wg-fed0/edge-router", "wg-fed0/tunnel-extra",
        "other-tcp", "other-udp",
    ]
    inp = lines(res)
    assert inp[0] == "type filter hook input priority filter; policy drop;"
    assert inp[-1] == 'counter drop comment "default-deny"'
    assert rule_line(res, "ssh-management") == [
        'ip saddr { 198.51.100.10/32 } tcp dport { 22 } accept comment "ssh-management"']
    assert rule_line(res, "wg-fed0/tunnel") == [
        'ip saddr { 192.0.2.10 } udp dport { 51821 } accept comment "wg-fed0/tunnel"']
    # The edge router's port is admitted on the tunnel interface only, from the far link.
    assert rule_line(res, "wg-fed0/edge-router") == [
        'iifname "wg-fed0" ip saddr { 169.254.10.0 } tcp dport { 7448 } accept '
        'comment "wg-fed0/edge-router"']
    # Nothing else on the mesh: ADR 1219 entitles only SSH from platform admins inbound.
    assert not any("7447" in l for l in inp)
    # Empty other-* rules render nothing.
    assert rule_line(res, "other-tcp") == [] and rule_line(res, "other-udp") == []


def test_edge_wireguard_published_chain_holds_the_same_ports():
    res = render(fixture("edge-wireguard"))
    pub = lines(res, "published_new")
    assert pub[:3] == ['iifname "docker0" return comment "container-bridges"',
                       'iifname "br-l2*" drop comment "container-bridges"',
                       'iifname "br-*" return comment "container-bridges"']
    assert pub[-1] == 'counter drop comment "default-deny"'
    assert ('iifname "wg-fed0" ip saddr { 169.254.10.0 } meta l4proto tcp ct original '
            'proto-dst { 7448 } accept comment "wg-fed0/edge-router"') in pub
    # Fixed expressions (loopback, ICMP, DHCP) live in the input chain only.
    assert not any("loopback" in l or "icmp" in l or "dhcp" in l for l in pub)
    assert rule_line(res, "container-bridges") == [
        'iifname "docker0" accept comment "container-bridges"',
        'iifname "br-l2*" drop comment "container-bridges"',
        'iifname "br-*" accept comment "container-bridges"']


def test_container_bridges_can_be_dropped_from_both_chains():
    doc = fixture("core")
    doc["override"] = {"container-bridges": {"drop": True}}
    res = render(doc)
    assert rule_line(res, "container-bridges") == [] and res["loosened"] == []
    assert rule_line(res, "container-bridges", "published_new") == []


def test_an_uplink_named_like_a_container_bridge_is_refused_unless_the_rule_is_dropped():
    doc = fixture("core")
    doc["host"]["uplink_interfaces"] = ["br-lan"]
    refused(doc, "GB-210")
    doc["override"] = {"container-bridges": {"drop": True}}
    render(doc)
    doc["host"]["uplink_interfaces"] = ["eth0"]
    doc["override"] = {}
    assert render(doc)["sha256"] == render(fixture("core"))["sha256"]


@pytest.mark.parametrize("port", [51820])
def test_a_peering_port_the_mesh_rule_also_holds_is_refused(port):
    # The defaults of goat-peering-wg (51820) and of a mesh client (51820) collide; the
    # mesh rule admits any source, which would void the tunnel rule's restriction.
    doc = fixture("edge-wireguard")
    doc["host"]["mesh_udp_ports"] = [51820]
    doc["override"] = {"mesh-wireguard": {"reason": "mesh peers dial this host"}}
    doc["peerings"][0]["listen_port"] = port
    e = refused(doc, "GB-210")
    assert "mesh-wireguard" in str(e) and "wg-fed0/tunnel" in str(e)


def test_two_peerings_on_one_udp_port_are_refused():
    doc = fixture("edge-wireguard")
    doc["peerings"].append(dict(doc["peerings"][0], name="wg-fed1", tunnel_interface="wg-fed1"))
    refused(doc, "GB-210")


def test_dhcp_client_admits_dhcpv4_and_dhcpv6_replies():
    res = render(fixture("core"))
    assert rule_line(res, "dhcp-client") == [
        'udp sport 67 udp dport 68 accept comment "dhcp-client"',
        'udp sport 547 udp dport 546 accept comment "dhcp-client"']


# --------------------------------------------------------------------------- listeners

LISTENING = [
    {"proto": "tcp", "port": 22, "addr": "0.0.0.0", "process": "sshd"},
    {"proto": "tcp", "port": 7447, "addr": "0.0.0.0", "process": "docker-proxy"},
    {"proto": "tcp", "port": 8000, "addr": "127.0.0.1", "process": "zenohd"},
    {"proto": "udp", "port": 53, "addr": "127.0.0.53%lo", "process": "systemd-resolve"},
    {"proto": "tcp", "port": 443, "addr": "198.51.100.1", "process": "management"},
    {"proto": "udp", "port": 3478, "addr": "::", "process": "management"},
]


def test_a_listener_the_ruleset_would_close_is_refused():
    doc = fixture("core")
    doc["listening"] = LISTENING
    e = refused(doc, "GB-215")
    assert e.exit_code == 5
    assert "tcp/443 (management on 198.51.100.1)" in str(e) and "udp/3478" in str(e)
    # Loopback listeners and admitted ports are not reported.
    assert "8000" not in str(e) and "udp/53" not in str(e) and "7447" not in str(e)


def test_listeners_closed_deliberately_or_admitted_by_a_reasoned_loosening_pass():
    doc = fixture("core")
    doc["listening"] = LISTENING
    doc["host"]["closed_listeners"] = ["udp/3478"]
    doc["override"] = {"other-tcp": {"ports": [443], "reason": "the management server"}}
    res = render(doc)
    assert res["closing"] == ["udp/3478"]
    # Listeners are a check, never part of the ruleset.
    del doc["listening"]
    assert render(doc)["sha256"] == res["sha256"]


SS = """tcp   LISTEN 0      4096         0.0.0.0:22        0.0.0.0:*    users:(("sshd",pid=1,fd=3))
tcp   LISTEN 0      4096       127.0.0.1:8000      0.0.0.0:*    users:(("zenohd",pid=2,fd=9))
tcp   LISTEN 0      4096            [::]:7447         [::]:*    users:(("docker-proxy",pid=3,fd=4))
udp   UNCONN 0      0      127.0.0.53%lo:53        0.0.0.0:*    users:(("systemd-resolve",pid=4,fd=13))
udp   UNCONN 0      0                  *:3478            *:*    users:(("management",pid=5,fd=7))
tcp   LISTEN 0      511     198.51.100.1:443       0.0.0.0:*
"""


def test_ss_output_is_read_when_no_listening_list_is_given():
    doc = fixture("core")
    doc["listening_ss"] = SS
    e = refused(doc, "GB-215")
    assert "tcp/443 (unknown process on 198.51.100.1)" in str(e)
    assert "udp/3478 (management on *)" in str(e)
    assert "8000" not in str(e) and "udp/53" not in str(e) and "7447" not in str(e)


def test_closed_listeners_must_be_proto_and_port():
    doc = fixture("core")
    doc["listening"] = LISTENING
    doc["host"]["closed_listeners"] = ["3478"]
    refused(doc, "GB-210")


def test_edge_wireguard_without_a_literal_endpoint_admits_any_source_to_the_tunnel():
    doc = fixture("edge-wireguard")
    doc["peerings"][0]["peer_addr"] = ""
    res = render(doc)
    assert rule_line(res, "wg-fed0/tunnel") == [
        'udp dport { 51821 } accept comment "wg-fed0/tunnel"']


def test_edge_wireguard_via_forwarder_binds_the_tunnel_to_the_carrier():
    doc = fixture("edge-wireguard")
    doc["peerings"][0].update({
        "via_forwarder": "True", "carrier_interface": "wg-carry0",
        "carrier_remote_ip": "169.254.30.0", "carrier_listen_port": 51822,
        "forwarder_addr": "203.0.113.5"})
    res = render(doc)
    assert rule_line(res, "wg-fed0/tunnel") == [
        'iifname "wg-carry0" ip saddr { 169.254.30.0 } udp dport { 51821 } accept '
        'comment "wg-fed0/tunnel"']
    assert rule_line(res, "wg-fed0/carrier") == [
        'ip saddr { 203.0.113.5 } udp dport { 51822 } accept comment "wg-fed0/carrier"']


def test_edge_spire_forward_admits_our_server_only():
    doc = fixture("edge-wireguard")
    doc["peerings"][0].update(spire_outbound_port="18443", spire_server_addr="100.127.0.5")
    res = render(doc)
    assert rule_line(res, "wg-fed0/spire-forward") == [
        'ip saddr { 100.127.0.5 } tcp dport { 18443 } accept comment "wg-fed0/spire-forward"']
    doc["peerings"][0].update(spire_outbound_port="")
    assert rule_line(render(doc), "wg-fed0/spire-forward") == []


def test_edge_ipsec_admits_ike_and_esp_from_the_partner_only():
    res = render(fixture("edge-ipsec"))
    assert rule_line(res, "ipsec-xab/ike") == [
        'ip saddr { 192.0.2.20 } udp dport { 500, 4500 } accept comment "ipsec-xab/ike"']
    assert rule_line(res, "ipsec-xab/esp") == [
        'ip saddr { 192.0.2.20 } meta l4proto esp accept comment "ipsec-xab/esp"']
    assert rule_line(res, "ipsec-xab/edge-router") == [
        'iifname "ipsec-xab" ip saddr { 169.254.11.0 } tcp dport { 7448 } accept '
        'comment "ipsec-xab/edge-router"']
    assert not any("51821" in l for l in lines(res))
    # ESP has no port, so it is not in the published chain.
    assert rule_line(res, "ipsec-xab/esp", "published_new") == []


def test_core_admits_fabric_mesh_and_management():
    res = render(fixture("core"))
    names = [r["name"] for r in res["rules"]]
    assert "fabric-tls" in names and "mesh-dns" in names and "fabric-quic" not in names
    assert rule_line(res, "fabric-tls") == ['tcp dport { 7447 } accept comment "fabric-tls"']
    assert rule_line(res, "mesh-dns") == [
        'iifname "wt0" udp dport { 5353 } accept comment "mesh-dns"']
    assert not any("7448" in l for l in lines(res))


def test_core_with_quic_adds_the_udp_listener():
    doc = fixture("core")
    doc["host"]["fabric_udp_port"] = 7448
    res = render(doc)
    assert rule_line(res, "fabric-quic") == ['udp dport { 7448 } accept comment "fabric-quic"']


def test_core_refuses_peerings():
    doc = fixture("core")
    doc["peerings"] = fixture("edge-wireguard")["peerings"]
    refused(doc, "GB-210")


@pytest.mark.parametrize("mutate", [
    lambda d: d["host"].update(management_sources=[]),
    lambda d: d["host"].pop("management_sources"),
    lambda d: d["host"].update(management_sources=["not-an-address"]),
    lambda d: d.update(profile="forwarder"),
    lambda d: d.update(schema="v0"),
    lambda d: d.update(peerings=[]),
    lambda d: d["peerings"][0].update(transport="gre"),
    lambda d: d["peerings"][0].update(tunnel_interface="an-interface-name-too-long"),
    lambda d: d["peerings"][0].update(listen_port=70000),
    lambda d: d["peerings"][0].update(peer_addr="partner.example.org"),
    lambda d: d["peerings"].append(dict(d["peerings"][0])),
])
def test_incomplete_input_is_refused(mutate):
    doc = fixture("edge-wireguard")
    mutate(doc)
    refused(doc, "GB-210")


def test_ipv6_management_source_renders_an_ip6_rule():
    doc = fixture("core")
    doc["host"]["management_sources"] = ["198.51.100.10/32", "2001:db8::/64"]
    res = render(doc)
    assert rule_line(res, "ssh-management") == [
        'ip saddr { 198.51.100.10/32 } tcp dport { 22 } accept comment "ssh-management"',
        'ip6 saddr { 2001:db8::/64 } tcp dport { 22 } accept comment "ssh-management"',
    ]


# --------------------------------------------------------------------------- several peerings

def test_two_peerings_on_one_edge_admit_both_and_order_does_not_matter():
    doc = fixture("edge-wireguard")
    second = dict(doc["peerings"][0], name="wg-fed1", tunnel_interface="wg-fed1",
                  listen_port=51823, peer_addr="192.0.2.30", remote_link_ip="169.254.12.0")
    a = copy.deepcopy(doc)
    a["peerings"].append(second)
    b = copy.deepcopy(doc)
    b["peerings"].insert(0, second)
    ra, rb = render(a), render(b)
    assert ra["sha256"] == rb["sha256"]
    assert rule_line(ra, "wg-fed0/tunnel") and rule_line(ra, "wg-fed1/tunnel")


def test_wireguard_and_ipsec_peerings_on_one_edge():
    doc = fixture("edge-wireguard")
    doc["peerings"] += fixture("edge-ipsec")["peerings"]
    res = render(doc)
    assert rule_line(res, "wg-fed0/tunnel") and rule_line(res, "ipsec-xab/esp")


# --------------------------------------------------------------------------- hash

def test_hash_is_stable_and_covers_the_text():
    doc = fixture("edge-wireguard")
    r1, r2 = render(doc), render(doc)
    assert r1["sha256"] == r2["sha256"] == hfr.sha256(r1["ruleset"])
    # Types as Ansible may hand them (strings for numbers) do not change the hash.
    doc2 = copy.deepcopy(doc)
    doc2["host"]["ssh_port"] = "22"
    doc2["peerings"][0]["listen_port"] = "51821"
    assert render(doc2)["sha256"] == r1["sha256"]
    # The managing session is a check, never part of the ruleset.
    doc3 = copy.deepcopy(doc)
    doc3["must_reach"] = [{"saddr": "198.51.100.10", "proto": "tcp", "dport": 22, "iif": "eth0"}]
    assert render(doc3)["sha256"] == r1["sha256"]


def test_cli_text_ends_with_the_hash_of_what_precedes_it():
    out = subprocess.run([sys.executable, str(SCRIPT), "--input", str(FIX / "core.json")],
                         check=True, capture_output=True, text=True).stdout
    body, last = out.rsplit("# sha256 ", 1)
    assert hfr.sha256(body) == last.strip()


# --------------------------------------------------------------------------- the override

def test_override_tightens_without_a_reason():
    doc = fixture("core")
    doc["override"] = {
        "icmp-echo": {"drop": True},
        "fabric-tls": {"sources": ["100.127.0.0/16"]},
        "ssh-management": {"sources": ["198.51.100.10/32"], "interface": "wt0"},
    }
    res = render(doc)
    assert [t["rule"] for t in res["tightened"]] == [
        "fabric-tls", "icmp-echo", "ssh-management"]
    assert res["loosened"] == []
    assert "icmp-echo" in res["dropped"]
    assert rule_line(res, "icmp-echo") == []
    assert rule_line(res, "fabric-tls") == [
        'ip saddr { 100.127.0.0/16 } tcp dport { 7447 } accept comment "fabric-tls"']
    assert rule_line(res, "ssh-management") == [
        'iifname "wt0" ip saddr { 198.51.100.10/32 } tcp dport { 22 } accept '
        'comment "ssh-management"']
    assert "# tightened fabric-tls: sources" in res["ruleset"]
    assert render(fixture("core"))["sha256"] != res["sha256"]


def test_override_narrowing_a_cidr_is_tightening():
    doc = fixture("core")
    doc["host"]["management_sources"] = ["198.51.100.0/24"]
    doc["override"] = {"ssh-management": {"sources": ["198.51.100.10/32"]}}
    assert render(doc)["loosened"] == []


def test_override_loosening_with_a_reason_is_recorded():
    doc = fixture("core")
    doc["override"] = {
        "other-tcp": {"ports": [9100], "reason": "node exporter, scraped by the site monitor"},
        "ssh-management": {"sources": ["198.51.100.0/24"], "reason": "a second management host"},
    }
    res = render(doc)
    assert {l["rule"] for l in res["loosened"]} == {"other-tcp", "ssh-management"}
    assert rule_line(res, "other-tcp") == ['tcp dport { 9100 } accept comment "other-tcp"']
    assert ("# loosened other-tcp: ports [] -> [9100]; reason: node exporter, scraped by the "
            "site monitor") in res["ruleset"]


@pytest.mark.parametrize("entry", [
    {"ports": [9100]},                                   # other-tcp opened
    {"ports": [9100], "reason": "   "},                  # a blank reason is no reason
])
def test_override_loosening_without_a_reason_is_refused(entry):
    doc = fixture("core")
    doc["override"] = {"other-tcp": entry}
    e = refused(doc, "GB-212")
    assert e.exit_code == 3


@pytest.mark.parametrize("name,entry", [
    ("ssh-management", {"sources": ["0.0.0.0/0"]}),       # wider sources
    ("fabric-tls", {"ports": [7447, 8000]}),             # another port
    ("wg-fed0/edge-router", {"interface": "eth0"}),      # another interface
    ("wg-fed0/tunnel", {"sources": []}),                 # empty = any, wider than the partner
])
def test_each_kind_of_loosening_needs_a_reason(name, entry):
    doc = fixture("edge-wireguard") if name.startswith("wg-") else fixture("core")
    doc["override"] = {name: entry}
    refused(doc, "GB-212")


@pytest.mark.parametrize("override", [
    {"fabric-tsl": {"drop": True}},                      # unknown name, even to tighten
    {"node-exporter": {"ports": [9100], "reason": "x"}}, # a new rule is not a named rule
    {"loopback": {"drop": True}},                        # fixed
    {"established": {"drop": True}},                     # fixed
    {"icmp-echo": {"sources": ["198.51.100.0/24"]}},     # fixed expression: drop only
    {"fabric-tls": {"open": True}},                      # unknown key
    {"fabric-tls": {"reason": "nothing changes"}},       # no change
    {"fabric-tls": {"drop": True, "ports": [7447]}},     # drop with another change
    {"fabric-tls": "drop"},                              # not a mapping
])
def test_override_unknown_or_fixed_is_refused(override):
    doc = fixture("core")
    doc["override"] = override
    e = refused(doc, "GB-211")
    assert e.exit_code == 3


def test_a_refused_entry_refuses_the_whole_override():
    doc = fixture("core")
    doc["override"] = {"icmp-echo": {"drop": True}, "zzz-unknown": {"drop": True}}
    refused(doc, "GB-211")


def test_override_from_a_group_vars_yaml_file(tmp_path):
    pytest.importorskip("yaml")
    gv = tmp_path / "host-firewall.yml"
    gv.write_text("---\ngoat_host_firewall_baseline: true\n"
                  "goat_host_firewall_override:\n"
                  "  icmp-echo: {drop: true}\n"
                  "  other-udp:\n    ports: [161]\n    reason: \"site SNMP poller\"\n")
    res = render(fixture("core"), str(gv))
    assert "icmp-echo" in res["dropped"]
    assert res["loosened"][0]["rule"] == "other-udp"


# --------------------------------------------------------------------------- lockout

def test_lockout_refused_when_the_session_is_not_admitted():
    doc = fixture("edge-wireguard")
    doc["must_reach"] = [{"saddr": "203.0.113.9", "proto": "tcp", "dport": 22, "iif": "eth0"}]
    e = refused(doc, "GB-213")
    assert e.exit_code == 4


def test_lockout_refused_when_an_override_drops_ssh():
    doc = fixture("core")
    doc["must_reach"] = [{"saddr": "198.51.100.10", "proto": "tcp", "dport": 22, "iif": "eth0"}]
    doc["override"] = {"ssh-management": {"drop": True}}
    refused(doc, "GB-213")


def test_lockout_refused_when_an_override_binds_ssh_to_another_interface():
    doc = fixture("core")
    doc["must_reach"] = [{"saddr": "198.51.100.10", "proto": "tcp", "dport": 22, "iif": "eth0"}]
    doc["override"] = {"ssh-management": {"interface": "wt0"}}
    refused(doc, "GB-213")


def test_lockout_refused_when_ssh_is_on_another_port():
    doc = fixture("core")
    doc["host"]["ssh_port"] = 2222
    doc["must_reach"] = [{"saddr": "198.51.100.10", "proto": "tcp", "dport": 22, "iif": "eth0"}]
    refused(doc, "GB-213")


def test_session_from_a_management_source_passes():
    doc = fixture("core")
    doc["must_reach"] = [{"saddr": "198.51.100.10", "proto": "tcp", "dport": 22, "iif": "eth0"}]
    render(doc)


# --------------------------------------------------------------------------- CLI

def test_cli_exit_codes_and_messages(tmp_path):
    doc = fixture("core")
    doc["override"] = {"other-tcp": {"ports": [9100]}}
    p = tmp_path / "in.json"
    p.write_text(json.dumps(doc))
    r = subprocess.run([sys.executable, str(SCRIPT), "--input", str(p)],
                       capture_output=True, text=True)
    assert r.returncode == 3 and r.stdout == ""
    assert r.stderr.startswith("goat-host-firewall: REFUSED (GB-212): ")


def test_cli_json_from_stdin():
    r = subprocess.run([sys.executable, str(SCRIPT), "--input", "-", "--format", "json"],
                       input=(FIX / "edge-ipsec.json").read_text(),
                       check=True, capture_output=True, text=True)
    res = json.loads(r.stdout)
    assert res["schema"] == "goat-host-firewall-render/v1" and res["profile"] == "edge"
    assert res["sha256"] == hfr.sha256(res["ruleset"])


def test_cli_list_rules_marks_fixed_rules():
    r = subprocess.run([sys.executable, str(SCRIPT), "--input", str(FIX / "core.json"),
                        "--list-rules"], check=True, capture_output=True, text=True)
    names = [l.split("\t")[0] for l in r.stdout.splitlines()]
    assert "loopback (fixed)" in names and "fabric-tls" in names


# --------------------------------------------------------------------------- review of 4232de2

# 2. A reason is written into the ruleset file: a newline in it would end the comment and
#    let the rest of the reason run as nft commands.
@pytest.mark.parametrize("reason", [
    "ok\nflush ruleset",                              # injection
    "first line of a YAML | block\nsecond line\n",    # what `reason: |` produces
    "tab\tinside",
    "nul\x00inside",
])
def test_a_reason_with_control_characters_is_refused(reason):
    doc = fixture("core")
    doc["override"] = {"other-tcp": {"ports": [9100], "reason": reason}}
    refused(doc, "GB-211")


def test_a_folded_reason_with_only_a_trailing_newline_is_accepted():
    doc = fixture("core")
    doc["override"] = {"other-tcp": {"ports": [9100], "reason": "a folded YAML reason\n"}}
    res = render(doc)
    assert "# loosened other-tcp: ports [] -> [9100]; reason: a folded YAML reason\n" in res["ruleset"]


# 3. Non-docker bridges (the vxlan role's br-l2seg*) would be admitted by br-*.
def test_a_non_docker_br_bridge_is_refused_unless_container_bridges_is_dropped():
    doc = fixture("core")
    doc["host"]["bridge_interfaces"] = ["docker0", "br-0123456789ab", "br-l2seg0"]
    e = refused(doc, "GB-210")
    assert "br-l2seg0" in str(e)
    doc["override"] = {"container-bridges": {"drop": True}}
    render(doc)


def test_docker_bridges_only_pass_and_do_not_change_the_hash():
    doc = fixture("core")
    doc["host"]["bridge_interfaces"] = ["docker0", "br-0123456789ab"]
    assert render(doc)["sha256"] == render(fixture("core"))["sha256"]


# 4. GB-215 false positives.
def test_dhcp_client_sockets_count_as_admitted():
    doc = fixture("core")
    doc["listening"] = [{"proto": "udp", "port": 68, "addr": "0.0.0.0", "process": "dhclient"},
                        {"proto": "udp", "port": 546, "addr": "::", "process": "dhcpcd"}]
    assert render(doc)["closing"] == []


def test_ephemeral_udp_client_sockets_are_skipped():
    doc = fixture("core")
    doc["host"]["ephemeral_ports"] = [32768, 60999]
    doc["listening"] = [{"proto": "udp", "port": 41234, "addr": "0.0.0.0", "process": "x"}]
    assert render(doc)["closing"] == ["udp/41234"]
    # A TCP listener in that range is a service, not a client socket.
    doc["listening"] = [{"proto": "tcp", "port": 41234, "addr": "0.0.0.0", "process": "x"}]
    refused(doc, "GB-215")


def test_closed_listeners_take_process_names():
    doc = fixture("core")
    doc["listening"] = [{"proto": "udp", "port": 323, "addr": "::1", "process": "chronyd"},
                        {"proto": "udp", "port": 123, "addr": "0.0.0.0", "process": "chronyd"}]
    refused(doc, "GB-215")
    doc["host"]["closed_listeners"] = ["chronyd"]
    assert render(doc)["closing"] == ["udp/123"]


# 5. Role inputs that loosen the baseline need a reason too.
@pytest.mark.parametrize("src", ["0.0.0.0/0", "::/0"])
def test_management_source_any_is_refused_even_with_a_reason(src):
    doc = fixture("core")
    doc["host"]["management_sources"] = [src]
    doc["override"] = {"ssh-management": {"reason": "anyone"}}
    refused(doc, "GB-210")


def test_management_source_broader_than_the_floor_needs_a_reason():
    doc = fixture("core")
    doc["host"]["management_sources"] = ["198.51.0.0/16"]
    refused(doc, "GB-212")
    doc["override"] = {"ssh-management": {"reason": "the site's whole management network"}}
    res = render(doc)
    assert res["loosened"][0]["rule"] == "ssh-management"
    # Moving the floor instead moves the loosening onto the floor; it still needs a reason.
    doc["host"]["management_prefix_floor"] = {"ipv4": 16, "ipv6": 48}
    doc["override"] = {}
    refused(doc, "GB-212")


def test_mesh_udp_ports_need_a_reason():
    doc = fixture("core")
    doc["host"]["mesh_udp_ports"] = [51820]
    e = refused(doc, "GB-212")
    assert "mesh-wireguard" in str(e)
    doc["override"] = {"mesh-wireguard": {"reason": "mesh peers dial this host directly"}}
    assert [l["rule"] for l in render(doc)["loosened"]] == ["mesh-wireguard"]


def test_a_mesh_dns_port_other_than_the_profile_default_needs_a_reason():
    doc = fixture("core")
    doc["host"]["mesh_dns_port"] = 53
    refused(doc, "GB-212")
    doc["override"] = {"mesh-dns": {"reason": "the mesh nameserver listens on 53 here"}}
    render(doc)


def test_a_mesh_interface_that_is_the_uplink_is_refused():
    doc = fixture("core")
    doc["host"]["mesh_interface"] = "eth0"
    doc["host"]["uplink_interfaces"] = ["eth0"]
    refused(doc, "GB-210")


# 8. SPIRE: the core admits the bundle endpoint from the edge; spire-forward falls back to
#    the mesh interface.
def test_core_spire_bundle_from_the_edge_mesh_address():
    doc = fixture("core")
    doc["host"]["spire_bundle_port"] = 8443
    doc["host"]["spire_bundle_sources"] = ["100.127.0.20"]
    assert rule_line(render(doc), "spire-bundle") == [
        'ip saddr { 100.127.0.20 } tcp dport { 8443 } accept comment "spire-bundle"']
    doc["host"]["spire_bundle_sources"] = []
    assert rule_line(render(doc), "spire-bundle") == [
        'iifname "wt0" tcp dport { 8443 } accept comment "spire-bundle"']


def test_spire_forward_without_a_literal_server_is_bound_to_the_mesh_interface():
    doc = fixture("edge-wireguard")
    doc["peerings"][0].update(spire_outbound_port=18443, spire_server_addr="")
    assert rule_line(render(doc), "wg-fed0/spire-forward") == [
        'iifname "wt0" tcp dport { 18443 } accept comment "wg-fed0/spire-forward"']


# 1. The role validates one peering alone before storing it, so an override that names a
#    peering not stored yet does not stop the entry from being stored.
def test_peering_only_ignores_override_entries_for_other_peerings():
    doc = fixture("edge-wireguard")
    doc["peerings"].append(dict(doc["peerings"][0], name="wg-fed1", tunnel_interface="wg-fed1",
                                listen_port=51823, remote_link_ip="169.254.12.0"))
    doc["override"] = {"wg-fed1/edge-router": {"sources": ["169.254.12.0"]},
                       "icmp-echo": {"drop": True}}
    only = copy.deepcopy(doc)
    only["peerings"] = only["peerings"][:1]
    refused(only, "GB-211")          # the full render of wg-fed0 alone refuses
    res = hfr.run(copy.deepcopy(only), None, "wg-fed0")
    assert "icmp-echo" in res["dropped"]
    with pytest.raises(hfr.Refusal):
        hfr.run(copy.deepcopy(only), None, "wg-fed9")


# --------------------------------------------------------------------------- recheck of acbac9d

# 3. A VXLAN segment bridge created after the load must stay closed between runs.
def test_br_l2_is_dropped_before_the_container_bridge_accept():
    res = render(fixture("core"))
    inp = lines(res)
    drop = inp.index('iifname "br-l2*" drop comment "container-bridges"')
    accept = inp.index('iifname "br-*" accept comment "container-bridges"')
    assert drop < accept
    pub = lines(res, "published_new")
    assert pub.index('iifname "br-l2*" drop comment "container-bridges"') < \
        pub.index('iifname "br-*" return comment "container-bridges"')


@pytest.mark.parametrize("link", ["br-int", "br-ex", "br-l2seg3"])
def test_any_non_docker_br_link_is_refused(link):
    doc = fixture("core")
    doc["host"]["bridge_interfaces"] = [link]
    refused(doc, "GB-210")


# 5. The floor itself is a role input: one broader than the profile's needs a reason.
@pytest.mark.parametrize("floor", [{"ipv4": 1}, {"ipv4": 16}, {"ipv6": 48}])
def test_a_floor_broader_than_the_profile_needs_a_reason(floor):
    doc = fixture("core")
    doc["host"]["management_prefix_floor"] = floor
    e = refused(doc, "GB-212")
    assert "ssh-management" in str(e)
    doc["override"] = {"ssh-management": {"reason": "the site's management network is a /16"}}
    render(doc)


def test_a_floor_narrower_than_the_profile_is_free():
    doc = fixture("core")
    doc["host"]["management_prefix_floor"] = {"ipv4": 28, "ipv6": 96}
    render(doc)


# Minor: a peering name holding "/" would make its rule names ambiguous.
def test_a_peering_name_with_a_slash_is_refused():
    doc = fixture("edge-wireguard")
    doc["peerings"][0]["name"] = "wg/fed0"
    refused(doc, "GB-210")


# Minor: a UDP listener on an ephemeral port is not refused, but it is recorded as closed.
def test_an_ephemeral_udp_listener_is_recorded_in_closing():
    doc = fixture("core")
    doc["host"]["ephemeral_ports"] = [32768, 60999]
    doc["listening"] = [{"proto": "udp", "port": 41234, "addr": "0.0.0.0", "process": "x"}]
    assert render(doc)["closing"] == ["udp/41234"]
