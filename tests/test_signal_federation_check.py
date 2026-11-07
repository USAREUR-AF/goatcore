"""Tests for ops/scripts/signal-federation-check.py — the gate that refuses a
half-configured signal federation between a joiner and its founder."""
import importlib.util
import textwrap
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("sfc", HERE.parent / "ops" / "scripts" / "signal-federation-check.py")
sfc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sfc)

RELAYS = ["relay-one.zagros.example", "relay-two.zagros.example"]

FOUNDER = {
    "mesh_ip_a": "100.127.0.10",
    "stun_hosts": RELAYS,
    "federation_peers_for_a": "federation-proxy:10001",
    "signal_federation_token": "{{ lookup('env', 'ZAGROS_SIGNAL_FEDERATION_TOKEN') }}",
    "signal_federation_ingress_enabled": True,
    "signal_federation_ingress_fqdn": "federation-a.zagros.example",
    "signal_federation_proxy_enabled": True,
    "federation_peer_mesh_addr": "100.127.0.20:10000",
    "federation_peer_public_sni": "federation-b.zagros.example",
    "federation_public_relays": "{{ stun_hosts }}",
}
JOINER = {
    "goatnet_founder": False,
    "join_founder_site": "zagros",
    "mesh_ip_a": "100.127.0.20",
    "stun_hosts": RELAYS,
    "rathole_client_service_name": "netbird-mgmt-zagros-b",
    "federation_peers_for_a": "federation-proxy:10001",
    "signal_federation_token": "{{ lookup('env', 'ZAGROS_SIGNAL_FEDERATION_TOKEN') }}",
    "signal_federation_ingress_enabled": True,
    "signal_federation_ingress_fqdn": "federation-b.zagros.example",
    "signal_federation_proxy_enabled": True,
    "federation_peer_mesh_addr": "{{ peer_ip }}:10000",
    "peer_ip": "100.127.0.10",
    "federation_peer_public_sni": "federation-a.zagros.example",
    "federation_public_relays": "{{ stun_hosts }}",
}
ALIAS = textwrap.dedent("""\
    [goat_relay_hosts:children]
    relay_servers

    [goat_relay_hosts:vars]
    rathole_ingress_dispatch_enabled=true
    rathole_ingress_backends=[{'name': 'zagros-a', 'service': 'netbird-mgmt', 'tunnel_port': 4430, 'weight': 1, 'sni_pin': ['federation-a.zagros.example']}, {'name': 'zagros-b', 'service': 'netbird-mgmt-zagros-b', 'tunnel_port': 4431, 'weight': 0, 'sni_pin': ['federation-b.zagros.example']}]
    """)


def checkout(tmp_path, founder=None, joiner=None, alias=ALIAS):
    import yaml
    gv = tmp_path / "ops" / "ansible" / "group_vars"
    for group, data in (("zagros_hosts", founder or FOUNDER), ("zagros_b_hosts", joiner or JOINER)):
        (gv / group).mkdir(parents=True)
        (gv / group / "vars.yml").write_text(yaml.safe_dump(data))
    inv = tmp_path / "ops" / "ansible" / "inventory"
    inv.mkdir(parents=True)
    if alias is not None:
        (inv / "zagros-goat-relay-alias.ini").write_text(alias)
    return tmp_path


def run(repo, **kw):
    return sfc.check(repo, "zagros-b", **kw)


def test_a_complete_pair_is_on(tmp_path):
    state, defects = run(checkout(tmp_path))
    assert (state, defects) == ("on", [])


def test_one_direction_alone_is_refused(tmp_path):
    j = dict(JOINER, federation_peers_for_a="")
    state, defects = run(checkout(tmp_path, joiner=j))
    assert state == "refused"
    assert any("one direction alone" in d and "zagros_b_hosts" in d for d in defects), defects


def test_proxy_with_no_peer_address_is_refused_even_when_off(tmp_path):
    # The state a single-node goatnet left behind: proxy on, nothing to dial.
    f = dict(FOUNDER, federation_peers_for_a="", federation_peer_mesh_addr="")
    j = dict(JOINER, federation_peers_for_a="", signal_federation_absent_reason="not yet")
    state, defects = run(checkout(tmp_path, founder=f, joiner=j))
    assert state == "refused"
    assert any("federation_peer_mesh_addr is empty" in d and "dies at startup" in d for d in defects), defects


def test_off_needs_a_declared_reason(tmp_path):
    off = {k: v for k, v in JOINER.items() if not k.startswith(("federation_", "signal_federation"))}
    foff = {k: v for k, v in FOUNDER.items() if not k.startswith(("federation_", "signal_federation"))}
    state, defects = run(checkout(tmp_path, founder=foff, joiner=off))
    assert state == "refused" and "signal_federation_absent_reason" in defects[0]
    state, defects = run(checkout(tmp_path / "x", founder=foff, joiner=dict(off, signal_federation_absent_reason="one node holds every peer")))
    assert (state, defects) == ("off", [])


def test_a_public_ingress_without_a_token_is_refused(tmp_path):
    j = dict(JOINER)
    del j["signal_federation_token"]
    state, defects = run(checkout(tmp_path, joiner=j))
    assert any("unauthenticated Forwards" in d for d in defects), defects


def test_a_literal_token_is_refused(tmp_path):
    state, defects = run(checkout(tmp_path, joiner=dict(JOINER, signal_federation_token="s3cret")))
    assert any("literal value" in d for d in defects), defects


def test_the_two_sides_must_name_the_same_token(tmp_path):
    j = dict(JOINER, signal_federation_token="{{ lookup('env', 'OTHER_TOKEN') }}")
    state, defects = run(checkout(tmp_path, joiner=j))
    assert any("same token" in d for d in defects), defects


def test_crossed_names_and_addresses_are_refused(tmp_path):
    j = dict(JOINER, federation_peer_public_sni="federation-x.zagros.example", federation_peer_mesh_addr="100.127.0.99:10000")
    state, defects = run(checkout(tmp_path, joiner=j))
    assert any("is not group_vars/zagros_hosts's signal_federation_ingress_fqdn" in d for d in defects), defects
    assert any("is not group_vars/zagros_hosts's mesh_ip_a 100.127.0.10" in d for d in defects), defects


def test_the_watcher_must_revive_the_proxy(tmp_path):
    j = dict(JOINER, mgmt_host_compose_watch_services="postgres-a signal-a traefik")
    state, defects = run(checkout(tmp_path, joiner=j))
    assert any("does not list federation-proxy" in d for d in defects), defects
    j["mgmt_host_compose_watch_services"] += " federation-proxy"
    assert run(checkout(tmp_path / "ok", joiner=j)) == ("on", [])


def test_peers_must_dial_the_local_proxy(tmp_path):
    state, defects = run(checkout(tmp_path, joiner=dict(JOINER, federation_peers_for_a="100.127.0.10:10000")))
    assert any("dials its local dual-path proxy" in d for d in defects), defects


def test_the_relays_must_pin_each_hostname_to_its_own_tunnel(tmp_path):
    state, defects = run(checkout(tmp_path, alias=None))
    assert any("zagros-goat-relay-alias.ini is missing" in d for d in defects), defects
    unpinned = ALIAS.replace("'sni_pin': ['federation-b.zagros.example']", "'sni_pin': []")
    state, defects = run(checkout(tmp_path / "u", alias=unpinned))
    assert any("no backend pins federation-b.zagros.example" in d for d in defects), defects
    wrong = ALIAS.replace("'federation-a.zagros.example']", "'federation-a.zagros.example', 'federation-b.zagros.example']").replace("'sni_pin': ['federation-b.zagros.example']", "'sni_pin': []")
    state, defects = run(checkout(tmp_path / "w", alias=wrong))
    assert any("pinned to backend zagros-a" in d for d in defects), defects
    nodispatch = ALIAS.replace("rathole_ingress_dispatch_enabled=true", "rathole_ingress_dispatch_enabled=false")
    state, defects = run(checkout(tmp_path / "n", alias=nodispatch))
    assert any("rathole_ingress_dispatch_enabled=true is not set" in d for d in defects), defects


def test_dns_must_resolve_to_relays_only(tmp_path):
    repo = checkout(tmp_path)
    table = {"relay-one.zagros.example": {"192.0.2.1"}, "relay-two.zagros.example": {"192.0.2.2"},
             "federation-a.zagros.example": {"192.0.2.1", "192.0.2.2"}}
    state, defects = run(repo, resolve_dns=True, lookup=lambda n: table.get(n, set()))
    assert state == "refused" and any("federation-b.zagros.example does not resolve" in d for d in defects), defects
    table["federation-b.zagros.example"] = {"198.51.100.7"}
    state, defects = run(repo, resolve_dns=True, lookup=lambda n: table.get(n, set()))
    assert any("198.51.100.7, which is not a relay" in d for d in defects), defects
    table["federation-b.zagros.example"] = {"192.0.2.2"}
    assert run(repo, resolve_dns=True, lookup=lambda n: table.get(n, set())) == ("on", [])


def test_the_token_must_be_in_the_environment_when_asked(tmp_path):
    repo = checkout(tmp_path)
    state, defects = run(repo, require_env=True, environ={})
    assert any("ZAGROS_SIGNAL_FEDERATION_TOKEN is not set" in d for d in defects), defects
    assert run(repo, require_env=True, environ={"ZAGROS_SIGNAL_FEDERATION_TOKEN": "x"}) == ("on", [])


def test_cli_exit_codes(tmp_path, capsys):
    repo = checkout(tmp_path)
    assert sfc.main(["zagros-b", "--repo", str(repo)]) == 0
    assert "ON, both sides consistent" in capsys.readouterr().out
    repo2 = checkout(tmp_path / "r", joiner=dict(JOINER, federation_peers_for_a=""))
    assert sfc.main(["zagros-b", "--repo", str(repo2)]) == 1
    assert "REFUSED" in capsys.readouterr().out
