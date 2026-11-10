#!/usr/bin/env python3
"""Refuse a goatnet's config naming ANOTHER goatnet's domain.

A goatnet's group_vars describe that goatnet. When a value in them names a
host under another goatnet's domain, the deployed service reaches the other
goatnet, and nothing at deploy time looks wrong, because the other goatnet's
hosts are up and answering.

It happened: zagros-dev's `stun_hosts` named zagros's five relay-*.zagros.example
hosts from 2026-09-07 to 2026-09-15. The standup scaffold
(ops/scripts/standup-config-preflight.py --scaffold) re-threaded top-level
`key: value` lines copied from the reference goatnet but carried list items
across verbatim, and #903 later moved `relay_addresses` to zagros-dev's own relay
tier without touching `stun_hosts`. Every zagros-dev peer was handed zagros's STUN
servers, and every one of them answered.

What counts as "another goatnet's domain": each group directory that declares
`public_mgmt_fqdn` or `site_enrollment_dns` owns the last two labels of that
name (zagros_dev_hosts -> zagros-dev.example). Groups without one inherit from the
longest group-name prefix that has one (zagros_dev_relay_health -> zagros_dev_hosts).
Groups that share a domain are one family and may name each other.

A reference that is deliberately cross-goatnet stays allowed when the line says
so, inline, with a reason:

    - "relay-iad.zagros-dev.example:33073"   # cross-goatnet: zagros-dev has no obs host; watched from zagros

The marker is per line so a reviewer sees the reason at the point of use, and
so a new line copied beside an allowed one is not allowed by proximity.

Relay ADDRESSES leak the same way, and a domain check cannot see them: a bare
IPv4 names no domain. zagros-dev's mesh DNS answered its own enrolment name with
zagros's five relay IPs from 2026-09-07 until #2119 (the same scaffold copied the
`dns_extra_hosts` apex record, and #903 re-pointed `relay_addresses` but not
it). So a second pass checks addresses against the relay inventories, the one
place each relay's goatnet is declared (ops/relay/health/relays*.conf, column
`goatnet`):

  - a relay IP owned by one goatnet appearing in another goatnet's group_vars,
    mesh zone (ops/dns/<dir>/mesh-hosts), name snapshot or acceptance values
    (ops/validation/values-<env>.yaml) is RED, unless the line carries the
    same `# cross-goatnet: <reason>` marker;
  - every public address in a goatnet's enrolment-name record
    (`dns_extra_hosts` entry for `{{ site_dns_zone }}`) must be a declared
    relay of that goatnet, so an address nobody declared cannot stand in;
  - a relay-health box must probe its own goatnet's inventory
    (`relay_health_relays_conf`, default relays.conf, which is zagros's).

A goatnet's own MESH addresses (100.64.0.0/10) leak the same way, and the relay
pass cannot see them (a CGNAT address owns no relay). zagros-dev's group_vars named
zagros's relay-health box, zagros-b's Thor and zagros's Thor as its own capacity-probe
target, cross-node target and goat-map backend; every probe and the map ran
against zagros. So a third pass pins every mesh IP to a goatnet from the inventory
(doors.yaml, the per-goatnet relay-health/mesh-dns inventories, mgmt-hosts-mesh.ini)
and a mesh IP of one goatnet in another's group_vars is RED unless the line
carries the same `# cross-goatnet: <reason>` marker.

A group's goatnet is its `consistency_goatnet` (pins.yml), else that of a group
sharing its public domain, else the longest group-name prefix that has one.

Usage:
  ops/ci/check-no-foreign-goatnet-config.py              # verdict on the tree
  ops/ci/check-no-foreign-goatnet-config.py --self-test  # prove RED on a fixture (P6)
"""
import re
import sys
import tempfile
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parents[2]
MARKER = "cross-goatnet:"

# Generated from ops/goatnets/cross-env.yaml: the goatnet roster rendered into
# every goatnet on purpose. Naming the others is its job.
EXCLUDED_FILES = {"cross-env.generated.yml"}


def apex_of(name):
    labels = [p for p in str(name).strip().strip('"').split(".") if p]
    if len(labels) < 2 or "{{" in name:
        return None
    return ".".join(labels[-2:]).lower()


def own_apexes(gv):
    """group dir name -> apex, for groups that declare their public name."""
    found = {}
    for d in sorted(p for p in gv.iterdir() if p.is_dir() and not p.name.startswith("_")):
        for f in sorted(d.glob("*.yml")):
            try:
                data = yaml.safe_load(f.read_text(errors="ignore")) or {}
            except yaml.YAMLError:
                continue
            if not isinstance(data, dict):
                continue
            for key in ("public_mgmt_fqdn", "site_enrollment_dns"):
                a = apex_of(data.get(key, "")) if isinstance(data.get(key), str) else None
                if a:
                    found.setdefault(d.name, a)
    return found


def inherited_apex(group, declared):
    if group in declared:
        return declared[group]
    best = None
    for g in declared:
        stem = g[: -len("_hosts")] if g.endswith("_hosts") else g
        if group.startswith(stem + "_") and (best is None or len(stem) > len(best[0])):
            best = (stem, declared[g])
    return best[1] if best else None


def code_part(line):
    """The line with any YAML comment removed; '' for a comment-only line."""
    s = line.lstrip()
    if s.startswith("#"):
        return "", s
    # A ' #' outside quotes starts a comment. Values here do not contain '#'
    # inside quotes in practice; keep the split simple and conservative.
    m = re.search(r"\s#", line)
    if m:
        return line[: m.start()], line[m.start():]
    return line, ""


def scan(gv):
    declared = own_apexes(gv)
    apexes = set(declared.values())
    problems = []
    for d in sorted(p for p in gv.iterdir() if p.is_dir() and not p.name.startswith("_")):
        mine = inherited_apex(d.name, declared)
        if not mine:
            continue
        others = sorted(a for a in apexes if a != mine)
        pats = [(a, re.compile(r"(?<![\w-])(?:[\w-]+\.)*" + re.escape(a) + r"(?![\w-])", re.I)) for a in others]
        for f in sorted(d.glob("*.yml")):
            if f.name in EXCLUDED_FILES:
                continue
            for n, line in enumerate(f.read_text(errors="ignore").split("\n"), 1):
                code, comment = code_part(line)
                if not code.strip():
                    continue
                for a, pat in pats:
                    if pat.search(code) and MARKER not in comment:
                        problems.append((f.relative_to(gv.parent.parent.parent) if gv.is_relative_to(REPO) else f, n, d.name, mine, a, code.strip()))
    return problems


# --- relay addresses ----------------------------------------------------------
IPV4 = re.compile(r"(?<![\d.])((?:\d{1,3}\.){3}\d{1,3})(?![\d.])")
RELAY_DIR = Path("ops/relay/health")
DEFAULT_RELAYS_CONF = "relays.conf"   # the relay-health role default


def _norm_id(v):
    return str(v).strip().strip('"').replace("_", "-").lower() if v else None


def _public(ip):
    import ipaddress
    try:
        a = ipaddress.ip_address(ip)
    except ValueError:
        return False
    return a.version == 4 and a.is_global and a not in ipaddress.ip_network("100.64.0.0/10")


def relay_owners(root):
    """public ip -> (goatnet id, relay name, conf file), from every relays*.conf.
    An address declared by two goatnets is itself a defect, reported as such."""
    owners, clashes = {}, []
    for conf in sorted((root / RELAY_DIR).glob("relays*.conf")):
        for n, line in enumerate(conf.read_text(errors="ignore").split("\n"), 1):
            cols = line.split("#", 1)[0].split()
            if len(cols) < 5 or not IPV4.fullmatch(cols[1]):
                continue
            name, ip, gn = cols[0], cols[1], _norm_id(cols[4])
            if ip in owners and owners[ip][0] != gn:
                clashes.append((conf, n, ip, owners[ip][0], gn))
            owners.setdefault(ip, (gn, name, conf.name))
    return owners, clashes


def conf_goatnets(root, conf_name):
    conf = root / RELAY_DIR / conf_name
    if not conf.is_file():
        return None
    out = set()
    for line in conf.read_text(errors="ignore").split("\n"):
        cols = line.split("#", 1)[0].split()
        if len(cols) >= 5 and IPV4.fullmatch(cols[1]):
            out.add(_norm_id(cols[4]))
    return out


def _load(f):
    try:
        data = yaml.safe_load(f.read_text(errors="ignore")) or {}
    except yaml.YAMLError:
        return {}
    return data if isinstance(data, dict) else {}


def group_goatnets(gv):
    """group dir -> goatnet id."""
    dirs = [p for p in sorted(gv.iterdir()) if p.is_dir() and not p.name.startswith("_")]
    declared, merged = {}, {}
    for d in dirs:
        m = {}
        for f in sorted(d.glob("*.yml")):
            m.update(_load(f))
        merged[d.name] = m
        if m.get("consistency_goatnet"):
            declared[d.name] = _norm_id(m["consistency_goatnet"])
    apex = own_apexes(gv)
    by_apex = {}
    for g, gid in declared.items():
        if g in apex:
            by_apex.setdefault(apex[g], gid)
    out = {}
    for d in dirs:
        g = d.name
        gid = declared.get(g) or (by_apex.get(apex[g]) if g in apex else None)
        if not gid:
            best = None
            for k, v in declared.items():
                stem = k[: -len("_hosts")] if k.endswith("_hosts") else k
                if g.startswith(stem + "_") and (best is None or len(stem) > len(best[0])):
                    best = (stem, v)
            gid = best[1] if best else None
        if gid:
            out[g] = gid
    return out, merged


def _scan_lines(path, mine, owners, where, rel):
    problems = []
    for n, line in enumerate(path.read_text(errors="ignore").split("\n"), 1):
        code, comment = code_part(line) if not path.suffix == ".json" else (line, "")
        if path.name == "mesh-hosts" and line.lstrip().startswith("#"):
            continue
        for ip in IPV4.findall(code):
            own = owners.get(ip)
            if own and own[0] != mine and MARKER not in comment:
                problems.append((rel, n, where, mine, own[0],
                                 f"{ip} is relay-{own[1]} of {own[0]} ({own[2]})"))
    return problems


def scan_relay_ips(root=REPO):
    gv = root / "ops" / "ansible" / "group_vars"
    owners, clashes = relay_owners(root)
    problems = []
    for conf, n, ip, a, b in clashes:
        problems.append((conf.relative_to(root), n, "relay inventory", a, b,
                         f"{ip} declared by both {a} and {b}"))
    ids, merged = group_goatnets(gv)
    for d in sorted(p for p in gv.iterdir() if p.is_dir() and not p.name.startswith("_")):
        mine = ids.get(d.name)
        if not mine:
            continue
        for f in sorted(d.glob("*.yml")):
            if f.name in EXCLUDED_FILES:
                continue
            problems += _scan_lines(f, mine, owners, d.name, f.relative_to(root))
        m = merged[d.name]
        # The enrolment-name record: every public address must be a relay of mine.
        zone = m.get("site_dns_zone")
        for e in m.get("dns_extra_hosts") or []:
            if not isinstance(e, dict):
                continue
            fq = str(e.get("fqdn") or e.get("name") or "")
            if fq not in ("{{ site_dns_zone }}", str(zone)):
                continue
            for ip in e.get("ips") or []:
                ip = str(ip)
                if _public(ip) and ip not in owners:
                    problems.append((Path("ops/ansible/group_vars") / d.name, 0, d.name, mine, "undeclared",
                                     f"{ip} answers {zone or 'the enrolment name'} but is no declared relay "
                                     f"of {mine} (add it to its relays conf, or remove it)"))
        # A relay-health box probes its own goatnet's inventory.
        if m.get("relay_health_enabled") is True:
            conf = m.get("relay_health_relays_conf") or DEFAULT_RELAYS_CONF
            gns = conf_goatnets(root, conf)
            if gns is not None and gns != {mine}:
                problems.append((Path("ops/ansible/group_vars") / d.name, 0, d.name, mine, ",".join(sorted(gns)),
                                 f"relay_health_relays_conf {conf} lists relays of {sorted(gns)}, not {mine}"))
    # Generated and derived artefacts: the mesh zone, its snapshot, the acceptance values.
    for zf in sorted((root / "ops" / "dns").glob("*/mesh-hosts")) + sorted((root / "ops" / "dns").glob("*/name-snapshot.json")):
        mine = ids.get(zf.parent.name.replace("-", "_") + "_hosts")
        if mine:
            problems += _scan_lines(zf, mine, owners, zf.parent.name, zf.relative_to(root))
    for vf in sorted((root / "ops" / "validation").glob("values-*.yaml")):
        env = vf.stem[len("values-"):]
        mine = ids.get(env.replace("-", "_") + "_hosts")
        if mine:
            problems += _scan_lines(vf, mine, owners, env, vf.relative_to(root))
    return problems


def report_relay_ips(problems):
    for path, n, where, mine, other, what in problems:
        loc = f"{path}:{n}" if n else f"{path}"
        print(f"{loc}: {where} (goatnet {mine}): {what}")
    if problems:
        print()
        print(f"FAIL: {len(problems)} relay address(es) cross a goatnet boundary or are undeclared.")
        print("A peer handed another goatnet's relay reaches a relay that cannot serve")
        print("this goatnet's name (it presents its own certificate), so the fallback")
        print("fails exactly when it is needed. Use this goatnet's relays, as listed in")
        print("its ops/relay/health/relays*.conf, or mark a deliberate exception inline:")
        print(f'    - "203.0.113.7"   # {MARKER} <reason>')
        return 1
    print("ok: every relay address in goatnet config, zones, snapshots and values belongs to that goatnet")
    return 0


def report(problems):
    for path, n, group, mine, other, code in problems:
        print(f"{path}:{n}: {group} (goatnet domain {mine}) names {other}: {code[:120]}")
    if problems:
        print()
        print(f"FAIL: {len(problems)} config line(s) name another goatnet's domain.")
        print("A deployed service reaches whatever these name, and the other goatnet's")
        print("hosts answer, so nothing fails at deploy time. Point the value at this")
        print("goatnet's own host, or, if it is deliberately cross-goatnet, say why inline:")
        print(f'    - "relay-x.other.net:33073"   # {MARKER} <reason>')
        return 1
    print("ok: no goatnet config names another goatnet's domain without a cross-goatnet: reason")
    return 0


def self_test():
    with tempfile.TemporaryDirectory() as t:
        gv = Path(t) / "ops" / "ansible" / "group_vars"
        (gv / "alpha_hosts").mkdir(parents=True)
        (gv / "beta_hosts").mkdir(parents=True)
        (gv / "beta_relay_health").mkdir(parents=True)
        (gv / "alpha_hosts" / "vars.yml").write_text(
            'public_mgmt_fqdn: netbird.alpha.net\n'
            'stun_hosts:\n  - "relay-a.alpha.net"\n'
            '# a comment naming relay-b.beta.net is not config\n'
            'obs_targets:\n  - "relay-b.beta.net:33073"   # cross-goatnet: beta watched from alpha\n')
        (gv / "beta_hosts" / "vars.yml").write_text(
            'public_mgmt_fqdn: netbird.beta.net\n'
            'stun_hosts:\n  - "relay-a.alpha.net"\n'  # the zagros-dev shape: a list item
            'relay_addresses:\n  - "rels://relay-b.beta.net:33073"\n')
        (gv / "beta_relay_health" / "vars.yml").write_text(
            'probe: "https://relay-a.alpha.net/health"\n')  # inherits beta.net
        got = {(p[2], p[4]) for p in scan(gv)}
        want = {("beta_hosts", "alpha.net"), ("beta_relay_health", "alpha.net")}
        if got != want:
            print(f"self-test FAILED: got {sorted(got)}, want {sorted(want)}")
            return 1
    print("self-test ok: a foreign list item and an inherited group are RED; a marked line and a comment are not")
    return relay_self_test()


def relay_self_test():
    with tempfile.TemporaryDirectory() as t:
        root = Path(t)
        gv = root / "ops" / "ansible" / "group_vars"
        rh = root / RELAY_DIR
        rh.mkdir(parents=True)
        (rh / "relays.conf").write_text("a1  198.51.100.1  r  i-1  alpha\n")
        (rh / "relays-beta.conf").write_text("b1  198.51.100.2  r  i-2  beta\n")
        for g, gid, apex in (("alpha_hosts", "alpha", "alpha.net"), ("beta_hosts", "beta", "beta.net")):
            (gv / g).mkdir(parents=True)
            (gv / g / "pins.yml").write_text(f'consistency_goatnet: "{gid}"\n')
        (gv / "alpha_hosts" / "vars.yml").write_text(
            'public_mgmt_fqdn: netbird.alpha.net\nsite_dns_zone: netbird.alpha.net\n'
            'dns_extra_hosts:\n  - fqdn: "{{ site_dns_zone }}"\n    ips:\n'
            '      - "100.127.0.1"\n      - "198.51.100.1"\n')
        (gv / "beta_hosts" / "vars.yml").write_text(
            'public_mgmt_fqdn: netbird.beta.net\nsite_dns_zone: netbird.beta.net\n'
            'dns_extra_hosts:\n  - fqdn: "{{ site_dns_zone }}"\n    ips:\n'
            '      - "100.127.0.2"\n      - "198.51.100.1"\n'        # the zagros-dev shape
            '      - "192.0.2.200"\n'                                # TEST-NET: not global, ignored
            '      - "8.8.8.8"\n'                                    # public, declared by nobody
            'obs_targets:\n  - "198.51.100.1"   # cross-goatnet: watched from beta\n')
        (gv / "beta_relay_health").mkdir(parents=True)
        (gv / "beta_relay_health" / "vars.yml").write_text("relay_health_enabled: true\n")  # default conf = alpha's
        (root / "ops" / "dns" / "beta").mkdir(parents=True)
        (root / "ops" / "dns" / "beta" / "mesh-hosts").write_text("100.127.0.2 netbird.beta.net\n198.51.100.1 netbird.beta.net\n")
        got = sorted((str(p[2]), p[4], p[5].split()[0]) for p in scan_relay_ips(root))
        want = sorted([("beta", "alpha", "198.51.100.1"), ("beta_hosts", "alpha", "198.51.100.1"),
                       ("beta_hosts", "undeclared", "8.8.8.8"), ("beta_relay_health", "alpha", "relay_health_relays_conf")])
        if got != want:
            print(f"relay self-test FAILED:\n got  {got}\n want {want}")
            return 1
    print("relay self-test ok: a foreign relay IP in group_vars and in a zone, an undeclared apex address and a")
    print("  relay-health box on another goatnet's inventory are RED; a marked line and the owner's own use are not")
    return 0


# --- deploy targets -------------------------------------------------------------
# A goatnet's own config telling the reader to deploy with `-e target=<group>`
# of ANOTHER goatnet. Comments count here: a "Deploy:" line is an instruction an
# operator copies, and a service entry whose deploy line names another goatnet
# was copied from it (#2117: zagros-dev declared zagros's Compliance Dashboard front,
# with `compliance-proxy-deploy.yml -e target=zagros_hosts`, and never ran it).
TARGET = re.compile(r"\btarget=([a-z0-9_]+)")


def scan_deploy_targets(root=REPO):
    gv = root / "ops" / "ansible" / "group_vars"
    ids, _ = group_goatnets(gv)
    problems = []
    for d in sorted(p for p in gv.iterdir() if p.is_dir() and not p.name.startswith("_")):
        mine = ids.get(d.name)
        if not mine:
            continue
        for f in sorted(d.glob("*.yml")):
            if f.name in EXCLUDED_FILES:
                continue
            for n, line in enumerate(f.read_text(errors="ignore").split("\n"), 1):
                if MARKER in line:
                    continue
                for g in TARGET.findall(line):
                    other = ids.get(g)
                    if other and other != mine:
                        problems.append((f.relative_to(root), n, d.name, mine, other, line.strip()))
    return problems


def report_deploy_targets(problems):
    for path, n, where, mine, other, line in problems:
        print(f"{path}:{n}: {where} (goatnet {mine}) deploys to {other}: {line[:120]}")
    if problems:
        print()
        print(f"FAIL: {len(problems)} line(s) in a goatnet's config deploy to another goatnet's hosts.")
        print("An operator following the line deploys the other goatnet, and a service entry")
        print("carrying one was copied from that goatnet and is likely not this one's. Name")
        print("this goatnet's own group, drop the copied entry, or mark it inline:")
        print(f"    # ... -e target=other_hosts   # {MARKER} <reason>")
        return 1
    print("ok: no goatnet config names another goatnet's hosts as a deploy target")
    return 0


def deploy_target_self_test():
    with tempfile.TemporaryDirectory() as t:
        root = Path(t)
        gv = root / "ops" / "ansible" / "group_vars"
        for g, gid in (("alpha_hosts", "alpha"), ("beta_hosts", "beta")):
            (gv / g).mkdir(parents=True)
            (gv / g / "vars.yml").write_text(f"consistency_goatnet: {gid}\nsite_dns_zone: {gid}.net\n")
        (gv / "alpha_hosts" / "svc.yml").write_text(
            "# Deploy: x-deploy.yml -e target=beta_hosts\n"
            "# Deploy: y-deploy.yml -e target=alpha_hosts\n"
            "# z-deploy.yml -e target=beta_hosts   # cross-goatnet: shared box\n")
        got = scan_deploy_targets(root)
        assert [(p[1], p[4]) for p in got] == [(1, "beta")], got
    print("deploy-target self-test ok: another goatnet's group as a deploy target is RED; own group and a marked line are not")
    return 0


# --- mesh addresses -----------------------------------------------------------
# A goatnet's own MESH addresses (100.64.0.0/10) leak the way relay public
# addresses do, and the scan above cannot see them: _public() excludes the
# CGNAT range, so a bare mesh IP names no owner there. zagros-dev's group_vars
# named zagros's relay-health box as its capacity-probe target, zagros-b's Thor as
# its cross-node target and zagros's Thor as its goat-map backend: every probe and
# the map ran against zagros, and zagros answered, so nothing failed at deploy time.
#
# Each goatnet's mesh hosts are declared once, in the inventory. Three sources,
# each pinning an IP to a goatnet without inference:
#   - doors.yaml: doors.<goatnet>.mgmt[].mesh_ip (the door registry, ADR 1117);
#   - the per-goatnet relay-health / mesh-dns inventories: site_id + mesh_ip_a;
#   - mgmt-hosts-mesh.ini: a [<group>] section, mapped to its goatnet by this
#     repo's group_goatnets (the same map the relay-address scan uses).
# A mesh IP the inventory assigns to one goatnet, appearing in ANOTHER goatnet's
# group_vars, is RED unless the line carries the same `# cross-goatnet: <reason>`
# marker. The cross-env roster (cross-env.generated.yml) names every goatnet on
# purpose and stays excluded, as it is for the scans above; today no hand-written
# group_vars line is a deliberate cross-goatnet mesh reference.
INV_DIR = Path("ops/ansible/inventory")
MESH_NET = "100.64.0.0/10"
_KV = re.compile(r"([A-Za-z_][\w]*)=(\S+)")


def _mesh(ip):
    import ipaddress
    try:
        a = ipaddress.ip_address(ip)
    except ValueError:
        return False
    return a.version == 4 and a in ipaddress.ip_network(MESH_NET)


def mesh_ip_owners(root, ids):
    """mesh ip -> (goatnet id, source inventory file). Only addresses the
    inventory pins to a KNOWN goatnet enter the map, so an unmapped host (e.g. a
    breakglass node labelled by host site) never stands in as a false owner. A
    clash (one IP, two goatnets) is an inventory defect, returned to be reported."""
    invdir = root / INV_DIR
    known = set(ids.values())
    owners, clashes = {}, []

    def claim(ip, gn, src):
        if not (gn and _mesh(ip)):
            return
        gn = _norm_id(gn)
        if gn not in known:
            return
        if ip in owners and owners[ip][0] != gn:
            clashes.append((src, ip, owners[ip][0], gn))
            return
        owners.setdefault(ip, (gn, src))

    doors = invdir / "doors.yaml"
    if doors.is_file():
        for gn, d in (_load(doors).get("doors") or {}).items():
            if not isinstance(d, dict):
                continue
            for m in d.get("mgmt") or []:
                if isinstance(m, dict) and m.get("mesh_ip"):
                    claim(str(m["mesh_ip"]), gn, "doors.yaml")

    # Every inventory .ini: host lines under a [<section>], with site_id/site_name
    # either on the line or in the section's [<section>:vars]. A host's goatnet is
    # that site label, else (for the mgmt-hosts groups) its group_goatnets id.
    for ini in sorted(invdir.glob("*.ini")):
        svars, hosts = {}, []
        section = None
        for line in ini.read_text(errors="ignore").split("\n"):
            s = line.split("#", 1)[0].strip()
            if not s:
                continue
            m = re.match(r"\[([^\]]+)\]", s)
            if m:
                section = m.group(1)
                continue
            if not section:
                continue
            kv = dict(_KV.findall(s))
            if section.endswith(":vars"):
                svars.setdefault(section[: -len(":vars")], {}).update(kv)
            else:
                hosts.append((section, kv))
        for sec, kv in hosts:
            gn = (kv.get("site_id") or kv.get("site_name")
                  or svars.get(sec, {}).get("site_id")
                  or svars.get(sec, {}).get("site_name")
                  or ids.get(sec))
            # ansible_host is the mesh IP on the mgmt-hosts lines but an SSM
            # instance id on the relay-health/mesh-dns lines (whose mesh IP is in
            # mesh_ip_a); claim every value that is a mesh address.
            for key in ("ansible_host", "mesh_ip_a", "mesh_ip"):
                if kv.get(key):
                    claim(kv[key], gn, ini.name)
    return owners, clashes


def scan_mesh_ips(root=REPO):
    gv = root / "ops" / "ansible" / "group_vars"
    ids, _ = group_goatnets(gv)
    owners, clashes = mesh_ip_owners(root, ids)
    problems = []
    for src, ip, a, b in clashes:
        problems.append((INV_DIR / src, 0, "inventory", a, b,
                         f"{ip} is declared for both {a} and {b}"))
    for d in sorted(p for p in gv.iterdir() if p.is_dir() and not p.name.startswith("_")):
        mine = ids.get(d.name)
        if not mine:
            continue
        for f in sorted(d.glob("*.yml")):
            if f.name in EXCLUDED_FILES:
                continue
            for n, line in enumerate(f.read_text(errors="ignore").split("\n"), 1):
                code, comment = code_part(line)
                if not code.strip():
                    continue
                for ip in sorted(set(IPV4.findall(code))):
                    own = owners.get(ip)
                    if own and own[0] != mine and MARKER not in comment:
                        problems.append((f.relative_to(root), n, d.name, mine, own[0],
                                         f"{ip} is a mesh host of {own[0]} ({own[1]})"))
    return problems


def report_mesh_ips(problems):
    for path, n, where, mine, other, what in problems:
        loc = f"{path}:{n}" if n else f"{path}"
        print(f"{loc}: {where} (goatnet {mine}): {what}")
    if problems:
        print()
        print(f"FAIL: {len(problems)} mesh address(es) in a goatnet's config belong to another goatnet.")
        print("A service pointed at another goatnet's mesh host reaches it, and that host")
        print("answers, so nothing fails at deploy time while the data crosses the boundary.")
        print("Point the value at this goatnet's own mesh host (its inventory: doors.yaml,")
        print("its relay-health/mesh-dns ini, mgmt-hosts-mesh.ini), or mark it inline:")
        print(f'    some_target: "100.127.0.7"   # {MARKER} <reason>')
        return 1
    print("ok: every mesh address in goatnet group_vars belongs to that goatnet")
    return 0


def mesh_ip_self_test():
    with tempfile.TemporaryDirectory() as t:
        root = Path(t)
        gv = root / "ops" / "ansible" / "group_vars"
        inv = root / INV_DIR
        inv.mkdir(parents=True)
        (inv / "mgmt-hosts-mesh.ini").write_text(
            "[alpha_hosts]\nmgmt-alpha ansible_host=100.127.0.1\n"
            "[beta_hosts]\nmgmt-beta ansible_host=100.127.0.2\n")
        (inv / "beta-relay-health.ini").write_text(
            "[beta_relay_health]\nrh-beta ansible_host=i-1 mesh_ip_a=100.127.0.3\n"
            "[beta_relay_health:vars]\nsite_id=beta\n")
        for g, gid in (("alpha_hosts", "alpha"), ("beta_hosts", "beta"), ("beta_relay_health", "beta")):
            (gv / g).mkdir(parents=True)
            (gv / g / "pins.yml").write_text(f'consistency_goatnet: "{gid}"\n')
        (gv / "alpha_hosts" / "vars.yml").write_text(
            'public_mgmt_fqdn: netbird.alpha.net\n'
            'own_bind: "100.127.0.1"\n'                                      # own IP: ok
            'probe_target: "100.127.0.3"\n'                                  # beta's relay-health box: RED
            'map_backend: "http://100.127.0.2:8081"\n'                       # beta's Thor: RED
            'watch: "100.127.0.2"   # cross-goatnet: alpha watches beta\n')  # marked: ok
        got = sorted((p[2], p[4], p[5].split()[0]) for p in scan_mesh_ips(root))
        want = sorted([("alpha_hosts", "beta", "100.127.0.2"),
                       ("alpha_hosts", "beta", "100.127.0.3")])
        if got != want:
            print(f"mesh-ip self-test FAILED:\n got  {got}\n want {want}")
            return 1
    print("mesh-ip self-test ok: another goatnet's mesh host in group_vars is RED; the own IP and a marked line are not")
    return 0


def main():
    if "--self-test" in sys.argv[1:]:
        return self_test() or deploy_target_self_test() or mesh_ip_self_test()
    rc = report(scan(REPO / "ops" / "ansible" / "group_vars"))
    rc = report_deploy_targets(scan_deploy_targets(REPO)) or rc
    rc = report_relay_ips(scan_relay_ips(REPO)) or rc
    return report_mesh_ips(scan_mesh_ips(REPO)) or rc


if __name__ == "__main__":
    sys.exit(main())
