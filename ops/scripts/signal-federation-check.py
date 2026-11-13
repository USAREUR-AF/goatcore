#!/usr/bin/env python3
"""signal-federation-check — refuse a half-configured signal federation between
a joiner site and the founder it joins, before any play runs.

Signal federation lets a peer whose signal home is one management node reach a
peer homed on the other. It needs eight facts kept in step across two sites'
group_vars, the founder's relay ingress inventory and public DNS. Each one
missing fails differently and quietly: a proxy with no peer address dies at
startup, a public ingress with no token is an unauthenticated Forward surface,
one direction on and the other off leaves cross-shard offers undeliverable,
and a federation hostname the relays do not pin lands on the wrong node.

    ops/scripts/signal-federation-check.py <joiner-site> [--peer <founder-site>]
        [--resolve] [--require-env] [--repo DIR]

Per side (group `<site>_hosts`):
  - signal_federation_token is an env reference (lookup('env', NAME)), never a
    literal; both sides name the SAME variable
  - federation on (`federation_peers_for_a` set) means the dual-path proxy:
    signal_federation_proxy_enabled, peers = federation-proxy:10001,
    federation_peer_mesh_addr = <the peer's mesh_ip_a>:10000,
    federation_peer_public_sni = the peer's signal_federation_ingress_fqdn,
    federation_public_relays non-empty, signal_federation_ingress_enabled with
    its own fqdn
  - proxy enabled with an empty peer address, SNI or relay list is refused
    whatever the peers line says (the proxy container cannot start)
Across the pair:
  - both sides on, or both off; off needs `signal_federation_absent_reason`
    declared on the joiner, so leaving it off is a decision someone wrote down
  - a third site: the joiner off with its reason declared, while the founder
    federates with another site, passes; it is refused only when the founder's
    peer address or SNI names this joiner, because then the founder expects it
  - the founder's relay inventory `inventory/<founder>-goat-relay-alias.ini`
    runs ingress dispatch and pins each side's federation hostname to that
    side's backend only (the backend whose service is the side's
    rathole_client_service_name)
  --resolve      each federation hostname resolves, and only to relay addresses
  --require-env  the token variable is set in this environment (value unread)

Exit 0 = consistent (on, or off with a reason). 1 = refused, one line per
defect naming the key and file to fix. 2 = usage. Reads files only, except
--resolve, which asks DNS.
"""
import argparse
import ast
import os
import re
import socket
import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    sys.exit("signal-federation-check: needs PyYAML (pip3 install pyyaml)")

PROXY_PEER = "federation-proxy:10001"
SIGNAL_PORT = "10000"
ENV_REF = re.compile(r"""^\{\{\s*lookup\(\s*['"]env['"]\s*,\s*['"]([A-Za-z_][A-Za-z0-9_]*)['"]\s*\)\s*\}\}$""")
WHOLE_VAR = re.compile(r"^\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*\}\}$")
INNER_VAR = re.compile(r"\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*\}\}")


class EnvRef(str):
    """A value that is `lookup('env', NAME)`; str() is the variable name."""


def group_of(site):
    return site.replace("-", "_") + "_hosts"


def load_group(gv_root, group):
    """Merged vars of one group: group_vars/<group>.yml and group_vars/<group>/*.yml."""
    files = []
    single = gv_root / f"{group}.yml"
    if single.is_file():
        files.append(single)
    d = gv_root / group
    if d.is_dir():
        files += sorted(p for p in d.iterdir() if p.suffix in (".yml", ".yaml") and p.is_file())
    if not files:
        return None
    out = {}
    for f in files:
        try:
            doc = yaml.safe_load(f.read_text()) or {}
        except yaml.YAMLError as e:
            raise SystemExit(f"signal-federation-check: {f} does not parse: {e}")
        if isinstance(doc, dict):
            out.update(doc)
    return out


def resolve(vars_, value, depth=0):
    """Resolve the template forms group_vars use here: a whole `{{ var }}`, an
    inline `{{ var }}` inside a string, and `lookup('env', NAME)`. Anything else
    is returned as written."""
    if depth > 6 or not isinstance(value, str):
        return value
    v = value.strip()
    m = ENV_REF.match(v)
    if m:
        return EnvRef(m.group(1))
    m = WHOLE_VAR.match(v)
    if m and m.group(1) in vars_:
        return resolve(vars_, vars_[m.group(1)], depth + 1)

    def sub(mm):
        name = mm.group(1)
        if name in vars_ and isinstance(vars_[name], (str, int)):
            r = resolve(vars_, str(vars_[name]), depth + 1)
            return str(r)
        return mm.group(0)

    return INNER_VAR.sub(sub, v)


def truthy(v):
    if isinstance(v, bool):
        return v
    return str(v).strip().lower() in ("true", "yes", "1", "on")


class Side:
    def __init__(self, site, gv_root):
        self.site = site
        self.group = group_of(site)
        self.path = f"group_vars/{self.group}"
        raw = load_group(gv_root, self.group)
        self.missing = raw is None
        self.raw = raw or {}

    def get(self, key, default=""):
        if key not in self.raw:
            return default
        v = resolve(self.raw, self.raw[key])
        return "" if v is None else v

    @property
    def peers(self):
        return str(self.get("federation_peers_for_a")).strip()

    @property
    def on(self):
        return bool(self.peers)

    @property
    def proxy(self):
        return truthy(self.get("signal_federation_proxy_enabled", False))

    @property
    def ingress(self):
        return truthy(self.get("signal_federation_ingress_enabled", False))

    @property
    def fqdn(self):
        return str(self.get("signal_federation_ingress_fqdn")).strip()

    @property
    def mesh_addr(self):
        return str(self.get("federation_peer_mesh_addr")).strip()

    @property
    def sni(self):
        return str(self.get("federation_peer_public_sni")).strip()

    @property
    def relays(self):
        v = self.get("federation_public_relays", [])
        if isinstance(v, str):
            v = [x for x in re.split(r"[,\s]+", v) if x and "{{" not in x]
        return [str(x) for x in (v or [])]

    @property
    def token(self):
        return self.get("signal_federation_token")

    @property
    def mesh_ip(self):
        return str(self.get("mesh_ip_a")).strip()

    @property
    def service(self):
        return str(self.get("rathole_client_service_name") or "netbird-mgmt").strip()


def side_defects(s):
    """Defects one side has on its own: the states its containers cannot run."""
    out = []
    p = s.path
    if s.proxy:
        if not s.mesh_addr:
            out.append(f"{p}: signal_federation_proxy_enabled is true and federation_peer_mesh_addr is empty — the federation-proxy renders a server line with no address and dies at startup. Set it to <peer mesh_ip_a>:{SIGNAL_PORT}, or turn the proxy off")
        if not s.sni:
            out.append(f"{p}: signal_federation_proxy_enabled is true and federation_peer_public_sni is empty — the backup legs have no TLS name to verify. Set it to the peer's signal_federation_ingress_fqdn")
        if not s.relays:
            out.append(f"{p}: signal_federation_proxy_enabled is true and federation_public_relays is empty — the proxy has no public backup leg. Set it to \"{{{{ stun_hosts }}}}\" (the relay fleet)")
    if PROXY_PEER.split(":")[0] in s.peers and not s.proxy:
        out.append(f"{p}: federation_peers_for_a points signal at {PROXY_PEER} but signal_federation_proxy_enabled is false — signal dials a container that is not running")
    if s.ingress and not s.fqdn:
        out.append(f"{p}: signal_federation_ingress_enabled is true and signal_federation_ingress_fqdn is empty — Traefik gets a router with no hostname")
    tok = s.token
    if (s.ingress or s.on) and not tok:
        out.append(f"{p}: federation is configured (ingress or peers) and signal_federation_token is unset — the federation surface would accept unauthenticated Forwards. Set signal_federation_token: \"{{{{ lookup('env', '<GOATNET>_SIGNAL_FEDERATION_TOKEN') }}}}\", the same variable on both sides")
    elif tok and not isinstance(tok, EnvRef):
        out.append(f"{p}: signal_federation_token is a literal value — credentials are references only. Use lookup('env', '<GOATNET>_SIGNAL_FEDERATION_TOKEN')")
    return out


def pair_defects(j, f):
    """Defects of the pair (joiner j, founder f) when federation is on."""
    out = []
    for a, b in ((j, f), (f, j)):
        p = a.path
        if not a.on:
            out.append(f"{p}: federation_peers_for_a is empty while {b.path} federates — one direction alone leaves offers to the other side's peers undeliverable. Set federation_peers_for_a: \"{PROXY_PEER}\" on both sides in the same change")
            continue
        if a.peers != PROXY_PEER:
            out.append(f"{p}: federation_peers_for_a is \"{a.peers}\"; a federating management node dials its local dual-path proxy, \"{PROXY_PEER}\"")
        if not a.proxy:
            out.append(f"{p}: signal_federation_proxy_enabled must be true when the site federates (mesh primary + relay backup; ADR 1085 Amendment 1)")
        if not a.ingress:
            out.append(f"{p}: signal_federation_ingress_enabled must be true — the peer's backup leg arrives at this site's Traefik on {a.fqdn or 'its federation hostname'}")
        host, _, port = a.mesh_addr.rpartition(":")
        if a.mesh_addr and port != SIGNAL_PORT:
            out.append(f"{p}: federation_peer_mesh_addr \"{a.mesh_addr}\" — the peer's signal listens on :{SIGNAL_PORT}")
        if a.mesh_addr and b.mesh_ip and "{{" not in b.mesh_ip and host and host != b.mesh_ip:
            out.append(f"{p}: federation_peer_mesh_addr host {host} is not {b.path}'s mesh_ip_a {b.mesh_ip}")
        if a.mesh_addr and not b.mesh_ip:
            out.append(f"{b.path}: mesh_ip_a is unset — the peer's mesh address cannot be checked; fill it after the node enrols")
        watch = str(a.get("mgmt_host_compose_watch_services")).split()
        if watch and "federation-proxy" not in watch:
            out.append(f"{p}: mgmt_host_compose_watch_services does not list federation-proxy — a proxy that exits is never revived and the node stops forwarding")
        if a.sni and b.fqdn and a.sni != b.fqdn:
            out.append(f"{p}: federation_peer_public_sni \"{a.sni}\" is not {b.path}'s signal_federation_ingress_fqdn \"{b.fqdn}\"")
    if j.fqdn and j.fqdn == f.fqdn:
        out.append(f"{j.path} and {f.path} declare the same signal_federation_ingress_fqdn \"{j.fqdn}\" — each node needs its own, pinned to its own tunnel")
    jt, ft = j.token, f.token
    if isinstance(jt, EnvRef) and isinstance(ft, EnvRef) and str(jt) != str(ft):
        out.append(f"signal_federation_token: {j.path} reads {jt}, {f.path} reads {ft} — both sides must carry the same token")
    return out


def relay_defects(repo, j, f):
    """The founder's relay ingress pins each federation hostname to its side's tunnel."""
    alias = repo / "ops" / "ansible" / "inventory" / f"{f.site}-goat-relay-alias.ini"
    rel = f"inventory/{alias.name}"
    if not alias.is_file():
        return [f"{rel} is missing — the founder's relays route by SNI only with ingress dispatch (ADR 1085). Create it with rathole_ingress_dispatch_enabled=true and one backend per management node; relay-ingress-add-site.yml adds the joiner's"]
    text = alias.read_text()
    out = []
    if not re.search(r"^\s*rathole_ingress_dispatch_enabled\s*=\s*true\s*$", text, re.M):
        out.append(f"{rel}: rathole_ingress_dispatch_enabled=true is not set — without the dispatcher every SNI reaches the founder's tunnel")
    m = re.search(r"^\s*rathole_ingress_backends\s*=\s*(\[.*\])\s*$", text, re.M)
    if not m:
        return out + [f"{rel}: no rathole_ingress_backends line"]
    try:
        backends = ast.literal_eval(m.group(1))
    except (ValueError, SyntaxError) as e:
        return out + [f"{rel}: rathole_ingress_backends does not parse as a Python literal: {e}"]
    for s in (j, f):
        if not s.fqdn:
            continue
        pinned = [b for b in backends if s.fqdn in (b.get("sni_pin") or [])]
        if not pinned:
            out.append(f"{rel}: no backend pins {s.fqdn} — the peer's backup leg falls into the balanced pool and reaches the wrong node. Add it to the sni_pin of the backend with service {s.service}")
        elif len(pinned) > 1:
            out.append(f"{rel}: {s.fqdn} is pinned to {len(pinned)} backends ({', '.join(b.get('name', '?') for b in pinned)}); pin it to one")
        elif pinned[0].get("service") != s.service:
            out.append(f"{rel}: {s.fqdn} is pinned to backend {pinned[0].get('name')} (service {pinned[0].get('service')}), but {s.path} registers rathole service {s.service}")
    return out


def addrs(name):
    try:
        return {ai[4][0] for ai in socket.getaddrinfo(name, 443, proto=socket.IPPROTO_TCP)}
    except socket.gaierror:
        return set()


def dns_defects(j, f, lookup=addrs):
    out = []
    relay_ips = set()
    for r in set(j.relays) | set(f.relays):
        relay_ips |= lookup(r)
    for s in (j, f):
        if not s.fqdn:
            continue
        got = lookup(s.fqdn)
        if not got:
            out.append(f"{s.fqdn} does not resolve — declare its A records over the relay addresses in the goatnet's DNS tree (one record per relay, not a CNAME to the apex) and apply it")
        elif relay_ips and not got <= relay_ips:
            stray = ", ".join(sorted(got - relay_ips))
            out.append(f"{s.fqdn} resolves to {stray}, which is not a relay in federation_public_relays")
    return out


def env_defects(j, f, environ=os.environ):
    out = []
    for name in sorted({str(t) for t in (j.token, f.token) if isinstance(t, EnvRef)}):
        if not environ.get(name):
            out.append(f"{name} is not set in this environment — run under ops/scripts/with-secrets with the goatnet's team-store reference for it")
    return out


def third_site_defects(j, f):
    """Defects of a joiner j that declares federation off while its founder f
    federates: none unless f's federation names j, which would leave f
    forwarding offers to a side that never answers."""
    out = []
    host = f.mesh_addr.rpartition(":")[0]
    if host and j.mesh_ip and "{{" not in j.mesh_ip and host == j.mesh_ip:
        out.append(f"{f.path}: federation_peer_mesh_addr {f.mesh_addr} names {j.path}'s mesh_ip_a, but {j.path} declares signal federation off — turn it on at both, or point {f.path} at the site it federates with")
    if f.sni and j.fqdn and f.sni == j.fqdn:
        out.append(f"{f.path}: federation_peer_public_sni names {j.path}'s signal_federation_ingress_fqdn \"{j.fqdn}\", but {j.path} declares signal federation off")
    return out


def check(repo, site, peer=None, resolve_dns=False, require_env=False, lookup=addrs, environ=os.environ):
    """(state, defects). state is 'on', 'off' or 'refused'."""
    gv_root = repo / "ops" / "ansible" / "group_vars"
    j = Side(site, gv_root)
    if j.missing:
        return "refused", [f"group_vars/{j.group} not found under {gv_root}"]
    if not peer:
        peer = str(j.get("join_founder_site")).strip()
    if not peer:
        return "refused", [f"{j.path}: no join_founder_site and no --peer — name the site this one federates with"]
    f = Side(peer, gv_root)
    if f.missing:
        return "refused", [f"group_vars/{f.group} not found (the peer {peer})"]
    defects = side_defects(j) + side_defects(f)
    if not j.on and not f.on:
        reason = str(j.get("signal_federation_absent_reason")).strip()
        if not reason:
            defects.append(f"{j.path}: signal federation is off on both sides and no signal_federation_absent_reason is declared. Turn it on (both sides, docs/tutorials/02-first-federated-node.md) or write down why it stays off")
        return ("off" if not defects else "refused"), defects
    if not j.on and str(j.get("signal_federation_absent_reason")).strip():
        # A third site at a goatnet whose founder already federates with
        # another joiner: off here is a decision, and the founder's federation
        # is the other pair's. Judged on this side and on whether the founder
        # names it, never on the founder's other pair.
        defects = side_defects(j) + third_site_defects(j, f)
        return ("off" if not defects else "refused"), defects
    defects += pair_defects(j, f)
    defects += relay_defects(repo, j, f)
    if resolve_dns:
        defects += dns_defects(j, f, lookup)
    if require_env:
        defects += env_defects(j, f, environ)
    return ("on" if not defects else "refused"), defects


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("site", help="the joiner site (group <site>_hosts)")
    ap.add_argument("--peer", help="the site it federates with; default its join_founder_site")
    ap.add_argument("--repo", default=str(Path(__file__).resolve().parents[2]), help="the checkout (default: this script's)")
    ap.add_argument("--resolve", action="store_true", help="also check each federation hostname resolves to relay addresses")
    ap.add_argument("--require-env", action="store_true", help="also check the token variable is set (its value is not read)")
    a = ap.parse_args(argv)
    state, defects = check(Path(a.repo), a.site, a.peer, a.resolve, a.require_env)
    peer = a.peer or str(Side(a.site, Path(a.repo) / "ops" / "ansible" / "group_vars").get("join_founder_site")) or "?"
    if state == "refused":
        for d in defects:
            print(f"FAIL {d}")
        print(f"signal federation {a.site} <-> {peer}: REFUSED ({len(defects)} defect(s))")
        return 1
    if state == "off":
        print(f"signal federation {a.site} <-> {peer}: OFF at {a.site}, reason declared — nothing to converge"
              + ("" if not Side(peer, Path(a.repo) / "ops" / "ansible" / "group_vars").on
                 else f" ({peer} federates with another site, not this one)"))
    else:
        print(f"signal federation {a.site} <-> {peer}: ON, both sides consistent{' (DNS checked)' if a.resolve else ''}{' (token present)' if a.require_env else ''}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
