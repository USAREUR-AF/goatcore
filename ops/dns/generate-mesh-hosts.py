#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# generate-mesh-hosts.py — renders each site's ops/dns/<dir>/mesh-hosts
# (CoreDNS zone file) from the site's service registry in
# ops/ansible/group_vars/<site>/, so "what services exist on this site"
# (docs/project/backlog.md#service-registry-consolidation): hand-synced
# lists let services deploy dark.
#
# Registry = the site's group_vars:
#   - isitup_services[].fqdn   — every declared service surface
#   - isitup_fqdn              — the status page itself (defaults to the
#                                isitup role convention isitup.<site_id>.<zone>;
#                                override in group_vars if a site diverges)
#   - dns_extra_hosts[]        — optional: mesh hostnames that are not
#                                status-page rows. Plain string = mesh_ip_a;
#                                dict {name, ip} = a different mesh peer;
#                                dict {name|fqdn, ips: [..]} = ORDERED
#                                multi-IP record set (e.g. mesh-primary +
#                                public-fallback for the enrollment apex)
#   - mesh_ip_a                — every row's address (single-host sites today;
#                                grow a per-entry ip field when a site splits)
#
# Site discovery: a group_vars dir qualifies when it defines site_id,
# site_dns_zone, mesh_ip_a and a non-empty isitup_services. The target
# file is the existing ops/dns/*/mesh-hosts whose entries carry the
# site's zone (content-addressed — no dir-name map to drift). A qualifying site with NO
# matching file is an error: create the file once with --init <site_id>
# (a new goatnet standup step), or the site's services would silently
# have no DNS.
#
# Modes:
#   (default)        rewrite the mesh-hosts files from the registry
#   --check          regenerate and fail (exit 1) if any committed file
#                    differs — wired as the dns-registry-sync CI job
#   --init <site_id> create ops/dns/<site_id>/mesh-hosts for a new site
#
# The generated files stay committed: reviewable diffs, greppable DNS
# state, and the goat-coredns play ships them unchanged. Hand-edits are
# refused by the CI gate — change the registry, rerun the generator.

import argparse
import calendar
import glob
import json
import os
import re
import sys
import time

try:
    import yaml
except ImportError:
    sys.stderr.write("ERROR: PyYAML required (pip install pyyaml)\n")
    sys.exit(2)

REPO_ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
GROUP_VARS_GLOB = os.path.join(REPO_ROOT, "ops/ansible/group_vars/*/")
MESH_HOSTS_GLOB = os.path.join(REPO_ROOT, "ops/dns/*/mesh-hosts")

VAR_RE = re.compile(r"\{\{\s*([a-zA-Z_][a-zA-Z0-9_]*)\s*\}\}")
# An external-service hostname belongs to its service and is flat or nested ONE
# level: <name>, <name>-<surface>, or <surface>.<name> (ADR 1178). Flat names are
# covered by the mesh-only certificate's *.<site>.<zone>; a nested name by one
# *.<name>.<site>.<zone> per service. No wildcard DNS is involved and none is
# possible: CoreDNS serves this zone through the `hosts` plugin, which reads
# literal records, so every name wanted is a line this generator writes out.
EXT_LABEL = r"[a-z0-9][a-z0-9-]{0,62}"


def ext_hostname_ok(label, name):
    """True when `label` is <name>, <name>-<surface> or <surface>.<name>."""
    if not isinstance(label, str) or not isinstance(name, str):
        return False
    if any(len(part) > 63 for part in label.split(".")):
        return False
    n = re.escape(name)
    return re.fullmatch(r"%s(-[a-z0-9-]+)?|%s\.%s" % (n, EXT_LABEL, n), label) is not None

MAX_FQDN = 253                                # RFC 1035 presentation length

HEADER = """\
# {site_id} mesh DNS — *.{zone} resolution. GENERATED FILE — DO NOT HAND-EDIT.
#
# Rendered by ops/dns/generate-mesh-hosts.py from the site service registry
# in ops/ansible/group_vars/{gv_dir}/ (isitup_services + isitup_fqdn +
# dns_extra_hosts). To add/rename/remove a service: edit the registry,
# rerun the generator, commit both. The dns-registry-sync CI job refuses
# drift between registry and this file (F-197: a declared service with no
# record here deploys dark).
#
# Served by CoreDNS on the site mgmt host bound to its mesh IP
# ({mesh_ip}), deployed via ops/ansible/goat-coredns.yml. Cloudflare holds
# only the public enrollment apex + relay records + ACME TXT; this file is
# the mesh-internal canon. Browser TLS comes from Traefik (LE), not DNS.
#
# Format: /etc/hosts (one IP + name per line). CoreDNS hosts plugin
# reloads every 5s; the file must stay mode 0644 (F-039: 0600 silently
# loads zero records). Gotcha: the first query for a not-yet-present name
# negative-caches upstream NXDOMAIN for up to cache-TTL — restart the
# mesh-dns container after adding records you need resolvable immediately.

"""


def load_site(gv_dir):
    scalars = {}
    lists = {"isitup_services": [], "dns_extra_hosts": [], "external_services": []}
    for path in sorted(glob.glob(os.path.join(gv_dir, "*.yml"))):
        with open(path) as f:
            data = yaml.safe_load(f) or {}
        if not isinstance(data, dict):
            continue
        for k, v in data.items():
            if isinstance(v, str):
                scalars[k] = v
            # name_snapshot_* toggles/overrides are bool/int, not templated
            # strings; render() never touches non-str values so this is safe.
            elif k.startswith("name_snapshot_") and isinstance(v, (bool, int)):
                scalars[k] = v
            # External-service declarations (ADR 1178): one mapping per
            # off-box service the mgmt host fronts, in the site's
            # group_vars/external-service-*.yml. Its `name` is the mesh
            # hostname's first label; the record points at the mgmt host
            # (Traefik terminates there), exactly like an isitup row.
            elif k.startswith("external_service_") and isinstance(v, dict):
                lists["external_services"].append((k, v))
        for key in ("isitup_services", "dns_extra_hosts"):
            if isinstance(data.get(key), list):
                lists[key].extend(data[key])
    return scalars, lists


def render(value, scalars, depth=0):
    if depth > 5:
        return value

    def sub(m):
        v = scalars.get(m.group(1))
        return v if isinstance(v, str) else m.group(0)

    out = VAR_RE.sub(sub, value)
    if out != value and VAR_RE.search(out):
        return render(out, scalars, depth + 1)
    return out


def site_fqdns(site_id, scalars, lists, mesh_ip):
    """Ordered (hostname, ip) pairs for one site. Fail closed on unresolved vars.

    Every row is the mgmt host's mesh_ip by default (single-host sites). A
    dns_extra_hosts entry may instead be a dict {name, ip} to point a mesh
    hostname at a DIFFERENT peer than the mgmt host — e.g. a service that runs
    on its own EC2 and joins the mesh (Zulip: chat.<site> -> the Zulip peer's
    mesh IP, not the Thor). A dict entry may instead carry ips: [..] — an
    ORDERED list emitted as multiple A records in file order, for a name whose
    answer is a candidate set (the enrollment apex: mesh IP first, relay
    public IPs as fallback — mesh-control-path plan Phase 1). Plain-string
    dns_extra_hosts entries keep the mesh_ip default. Name dedup is
    first-entry-wins across ALL sources: a name claimed by dns_extra_hosts
    keeps its full IP set; a later isitup_services row for the same name is
    dropped (Zulip relies on this)."""
    entries = []  # (source_id, name_template, [ips] in answer order)
    isitup_fqdn = scalars.get("isitup_fqdn", "isitup.{{ site_id }}.{{ site_dns_zone }}")
    entries.append(("isitup", isitup_fqdn, [mesh_ip]))
    for extra in lists["dns_extra_hosts"]:
        if isinstance(extra, dict):
            name = extra.get("name") or extra.get("fqdn")
            ips = extra.get("ips") if isinstance(extra.get("ips"), list) else \
                ([extra["ip"]] if "ip" in extra else None)
            if not name or not ips:
                sys.stderr.write(f"ERROR: {site_id}: dns_extra_hosts dict entry must carry "
                                 f"name/fqdn plus ip or a non-empty ips list: {extra!r}\n")
                sys.exit(2)
            entries.append(("dns_extra_hosts", str(name), [str(ip) for ip in ips]))
        else:
            entries.append(("dns_extra_hosts", str(extra), [mesh_ip]))
    # A service row may only claim a MESH record if its name is inside this
    # site's own zone. isitup lists off-mesh surfaces too, which live on other
    # infrastructure entirely. Emitting a
    # mesh A record for one does not "add" a name: mesh CoreDNS is
    # authoritative for what it answers, so it would OVERRIDE the real public
    # answer with the mgmt host's mesh IP, for every peer on the mesh, and the
    # service would go dark fleet-wide while its own infrastructure stayed
    # perfectly healthy.
    #
    #
    # `dns_extra_hosts` is deliberately NOT filtered: it is an explicit
    # operator declaration and is the escape hatch for the legitimate case of
    # a mesh name outside the zone pattern (Zulip on its own peer).
    zone_suffix = "." + str(scalars["site_dns_zone"]).lower().lstrip(".")
    for svc in lists["isitup_services"]:
        if isinstance(svc, dict) and "fqdn" in svc:
            svc_name = render(str(svc["fqdn"]), scalars).lower()
            if not svc_name.endswith(zone_suffix):
                continue
            entries.append((svc.get("id", "?"), str(svc["fqdn"]), [mesh_ip]))
    # External-service declarations (ADR 1178). Always inside the zone by
    # construction (<label>.<site_id>.<zone>). A declaration whose name an
    # isitup row already claims is deduplicated below (first entry wins),
    # which is what lets a service keep a hand-written status row while its
    # edge moves onto the declaration.
    #
    # `hostnames:` (default [name]) lets ONE declaration carry several labels
    # — a partner platform whose api/audit/authentication surfaces are one
    # service with one withdrawal, not N declarations with N status rows.
    #
    # `tls:` decides WHERE the record points, and it is the whole difference
    # between the two modes:
    #   edge    (default) -> the mgmt host. Traefik terminates, the mesh-only
    #                        allowlist is the edge gate, the partner holds no
    #                        key for this zone. Today's behaviour, unchanged.
    #   partner           -> the partner's own mesh IP (`mesh_ip`, required).
    #                        They terminate TLS with their own certificate;
    #                        goat renders no router and no SAN for these names.
    #                        Reachability control moves to their host — that is
    #                        the cost, and the guide states it.
    for var_name, ext in lists.get("external_services", []):
        if not ext.get("enabled", True):
            continue
        tls_mode = str(ext.get("tls", "edge")).lower()
        if tls_mode not in ("edge", "partner"):
            sys.stderr.write(f"ERROR: {site_id}: {var_name}: `tls` must be edge or partner, "
                             f"got {ext.get('tls')!r}\n")
            sys.exit(2)
        if tls_mode == "partner":
            target_ip = ext.get("mesh_ip")
            if not isinstance(target_ip, str) or not target_ip.strip():
                sys.stderr.write(f"ERROR: {site_id}: {var_name}: `tls: partner` requires "
                                 f"`mesh_ip` (the partner host's mesh address)\n")
                sys.exit(2)
        else:
            target_ip = mesh_ip
        labels = ext.get("hostnames") or [ext.get("name")]
        if not isinstance(labels, list) or not labels:
            sys.stderr.write(f"ERROR: {site_id}: {var_name}: `hostnames` must be a non-empty list\n")
            sys.exit(2)
        for label in labels:
            if not ext_hostname_ok(label, ext.get("name")):
                sys.stderr.write(f"ERROR: {site_id}: {var_name}: every hostname is one DNS "
                                 f"label of this service: <name>, <name>-<surface> or <surface>.<name>, got {label!r}\n")
                sys.exit(2)
            entries.append((var_name, label + ".{{ site_id }}.{{ site_dns_zone }}", [target_ip]))

    rendered, seen = [], set()
    for sid, tpl, ips in entries:
        name = render(tpl, scalars).lower()
        if VAR_RE.search(name):
            sys.stderr.write(f"ERROR: {site_id}: cannot render '{tpl}' (entry '{sid}') — "
                             f"unresolved variable; the generator must be complete\n")
            sys.exit(2)
        if len(name) > MAX_FQDN:
            sys.stderr.write(f"ERROR: {site_id}: '{sid}' builds {name!r}, {len(name)} characters; "
                             f"a DNS name is at most {MAX_FQDN}\n")
            sys.exit(2)
        if name in seen:
            continue
        seen.add(name)
        for ip_tpl in ips:
            ip = render(ip_tpl, scalars)
            if VAR_RE.search(ip):
                sys.stderr.write(f"ERROR: {site_id}: cannot render ip '{ip_tpl}' (entry '{sid}') — "
                                 f"unresolved variable; the generator must be complete\n")
                sys.exit(2)
            rendered.append((name, ip))
    return rendered


def discover_sites():
    sites = []
    for gv_dir in sorted(glob.glob(GROUP_VARS_GLOB)):
        scalars, lists = load_site(gv_dir)
        if not lists["isitup_services"]:
            continue
        if not all(scalars.get(k) for k in ("site_id", "site_dns_zone", "mesh_ip_a")):
            continue
        sites.append((gv_dir, scalars, lists))
    return sites


def target_file(zone, site_id):
    matches = []
    for path in sorted(glob.glob(MESH_HOSTS_GLOB)):
        with open(path) as f:
            for line in f:
                line = line.strip()
                if line and not line.startswith("#") and line.split()[-1].endswith(zone):
                    matches.append(path)
                    break
    if len(matches) > 1:
        sys.stderr.write(f"ERROR: zone {zone} appears in multiple mesh-hosts files: {matches}\n")
        sys.exit(2)
    return matches[0] if matches else None


# --- name-fallback snapshot (docs/design/name-fallback-resolver.md, ADR 1082) ---
# Sites that opt in via name_snapshot_enabled: true additionally get a signed,
# versioned ops/dns/<dir>/name-snapshot.json next to their mesh-hosts file:
# the same registry rendering, wrapped with a monotonic DNS-SOA-style serial +
# declared TTL, for goat-cli's fallback resolver when mesh DNS is down. The
# serial (and generated_at) bump ONLY when the record set changes, so an
# unchanged registry regenerates byte-identically. The detached .sig (ECDSA
# P-256 over the file bytes, minted by ops/enrollment/cmd/name-snapshot-sign)
# goes stale on every content change — the sig gate is the separate
# ops/ci/check-name-snapshot-sig.sh; this generator gates content drift only.

SNAPSHOT_FORMAT = "goat-name-snapshot-v1"
SNAPSHOT_DEFAULT_TTL = 30 * 24 * 3600  # 30 days


def snapshot_render(scalars, records, prev, bindings=None):
    """Render the snapshot dict; serial/generated_at carry over unless content changed."""
    site_id, zone = scalars["site_id"], scalars["site_dns_zone"]
    ca_id = scalars.get("name_snapshot_ca_id") or scalars.get("goat_ca_id")
    if not ca_id:
        sys.stderr.write(f"ERROR: {site_id}: name_snapshot_enabled but no "
                         f"name_snapshot_ca_id / goat_ca_id in group_vars — the snapshot "
                         f"must name its signing CA (fail closed)\n")
        sys.exit(2)
    body = {
        "format": SNAPSHOT_FORMAT,
        "site_id": site_id,
        "zone": zone,
        "serial": 1,
        "generated_at": "",
        "generated_at_unix": 0,
        "ttl_seconds": int(scalars.get("name_snapshot_ttl_seconds", SNAPSHOT_DEFAULT_TTL)),
        "ca_id": ca_id,
        "records": [{"name": name, "ip": ip} for name, ip in records],
    }
    # Self-claims (ADR 1082 Amendment 2): registry-derived name→anchor
    # bindings ride the signed snapshot so claim verification is fully
    # offline. Emitted ONLY when a bindings file exists — sites without
    # one regenerate byte-identically (v1-compatible readers ignore it).
    if bindings:
        body["peer_bindings"] = bindings
    content_keys = ("format", "site_id", "zone", "ttl_seconds", "ca_id", "records",
                    "peer_bindings")
    if prev and all(prev.get(k) == body.get(k) for k in content_keys) \
            and isinstance(prev.get("serial"), int) and prev["serial"] >= 1:
        body["serial"] = prev["serial"]
        body["generated_at"] = prev.get("generated_at", "")
        body["generated_at_unix"] = prev.get("generated_at_unix", 0)
    else:
        body["serial"] = (prev["serial"] + 1) if prev and isinstance(prev.get("serial"), int) else 1
        now = time.gmtime()
        body["generated_at"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", now)
        body["generated_at_unix"] = calendar.timegm(now)
    return body


def load_peer_bindings(site_dir):
    """Optional ops/dns/<site>/peer-bindings.json: [{"name", "anchor"}, …],
    registry-produced (portal export). Sorted by name for determinism;
    malformed input fails closed (a wrong bindings table would let the
    wrong anchor claim a name)."""
    path = os.path.join(site_dir, "peer-bindings.json")
    if not os.path.exists(path):
        return None
    with open(path) as f:
        raw = json.load(f)
    if not isinstance(raw, list):
        sys.stderr.write(f"ERROR: {path}: expected a JSON array of "
                         f"{{name, anchor}} objects\n")
        sys.exit(2)
    out = []
    for i, b in enumerate(raw):
        if not (isinstance(b, dict) and isinstance(b.get("name"), str) and b["name"]
                and isinstance(b.get("anchor"), str) and b["anchor"]):
            sys.stderr.write(f"ERROR: {path}[{i}]: each binding needs non-empty "
                             f"string fields name + anchor\n")
            sys.exit(2)
        out.append({"name": b["name"], "anchor": b["anchor"]})
    out.sort(key=lambda b: b["name"])
    names = [b["name"] for b in out]
    if len(set(names)) != len(names):
        sys.stderr.write(f"ERROR: {path}: duplicate name bindings (single-owner "
                         f"register — one anchor per name)\n")
        sys.exit(2)
    return out


def snapshot_emit(scalars, records, mesh_hosts_path, check):
    path = os.path.join(os.path.dirname(mesh_hosts_path), "name-snapshot.json")
    rel = os.path.relpath(path, REPO_ROOT)
    prev = None
    if os.path.exists(path):
        try:
            with open(path) as f:
                prev = json.load(f)
        except ValueError:
            prev = None
    bindings = load_peer_bindings(os.path.dirname(mesh_hosts_path))
    body = snapshot_render(scalars, records, prev, bindings)
    text = json.dumps(body, indent=2) + "\n"
    if os.path.exists(path):
        with open(path) as source_file:
            current = source_file.read()
    else:
        current = ""
    if current == text:
        print(f"OK: {rel} in sync (serial {body['serial']})")
        return 0
    if check:
        sys.stderr.write(f"DRIFT: {rel} does not match the registry — run "
                         f"ops/dns/generate-mesh-hosts.py, re-sign with "
                         f"ops/enrollment/cmd/name-snapshot-sign, commit both\n")
        return 1
    with open(path, "w") as f:
        f.write(text)
    print(f"WROTE: {rel} (serial {body['serial']}) — signature is now stale; re-sign with "
          f"ops/enrollment/cmd/name-snapshot-sign before committing")
    return 0


def generate(check=False):
    sites = discover_sites()
    if not sites:
        sys.stderr.write("ERROR: no sites discovered — generator is miswired\n")
        sys.exit(2)
    drift = 0
    for gv_dir, scalars, lists in sites:
        site_id, zone, mesh_ip = scalars["site_id"], scalars["site_dns_zone"], scalars["mesh_ip_a"]
        path = target_file(zone, site_id)
        if path is None:
            sys.stderr.write(
                f"ERROR: {site_id}: declares {len(lists['isitup_services'])} services but no "
                f"ops/dns/*/mesh-hosts carries zone {zone}. Run --init {site_id} first "
                f"(new-site standup step) — skipping would re-open F-197.\n")
            sys.exit(2)
        gv_name = os.path.basename(gv_dir.rstrip("/"))
        records = site_fqdns(site_id, scalars, lists, mesh_ip)
        body = HEADER.format(site_id=site_id, zone=zone, gv_dir=gv_name, mesh_ip=mesh_ip)
        body += "".join(f"{ip}  {name}\n" for name, ip in records)
        rel = os.path.relpath(path, REPO_ROOT)
        if os.path.exists(path):
            with open(path) as source_file:
                current = source_file.read()
        else:
            current = ""
        if current == body:
            print(f"OK: {rel} in sync with {gv_name} registry")
        elif check:
            drift = 1
            sys.stderr.write(f"DRIFT: {rel} does not match the {gv_name} registry — "
                             f"run ops/dns/generate-mesh-hosts.py and commit the result\n")
        else:
            with open(path, "w") as f:
                f.write(body)
            print(f"WROTE: {rel} ({gv_name} registry)")
        if scalars.get("name_snapshot_enabled") is True:
            drift |= snapshot_emit(scalars, records, path, check)
    return drift


def init_site(site_id):
    for gv_dir, scalars, lists in discover_sites():
        if scalars["site_id"] == site_id:
            path = os.path.join(REPO_ROOT, "ops/dns", site_id, "mesh-hosts")
            if os.path.exists(path):
                sys.stderr.write(f"ERROR: {path} already exists\n")
                sys.exit(2)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            # Seed with one real row so content-addressed discovery finds it,
            # then immediately regenerate in full.
            with open(path, "w") as f:
                f.write(f"{scalars['mesh_ip_a']}  isitup.{site_id}.{scalars['site_dns_zone']}\n")
            return generate(check=False)
    sys.stderr.write(f"ERROR: no qualifying group_vars site with site_id={site_id}\n")
    sys.exit(2)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--check", action="store_true", help="fail if committed files drift from the registry")
    ap.add_argument("--init", metavar="SITE_ID", help="create mesh-hosts for a new site, then generate")
    args = ap.parse_args()
    if args.init:
        sys.exit(init_site(args.init))
    sys.exit(generate(check=args.check))


if __name__ == "__main__":
    main()
