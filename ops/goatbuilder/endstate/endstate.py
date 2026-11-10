#!/usr/bin/env python3
"""endstate.py — a goatnet's declared end state, and the running goatnet graded against it (ADR 1206).

The end state is DERIVED, never hand-kept per goatnet: `catalog.yaml` says what each
service is (its containers, class, edges); the site's own declarations say whether it
runs it (group_vars flags, the fleet pin table's `absent:` deviations, the founder
flag) and where (has_obs_host and the inventory: a site with no obs host runs the obs
class on its one management machine). Five grades per present service — deployed,
active, healthy, functional, plus its zero-trust edges — and the negative side:
anything running that the end state does not declare is an orphan.

Both avenues run this file (ADR 1201): `ops/scripts/goatnet-endstate.sh` and
`goatbuilder endstate` call the same subcommands.

  endstate.py model <site> [--root DIR]               the derived end state, JSON
  endstate.py observe <site> [--out FILE]             read the site's hosts (read-only), facts JSON
  endstate.py grade <site> [--facts F] [--assay F]    grade; observes when no --facts is given
  endstate.py check [--root DIR]                      CI: every governed site's model renders and
                                                      the undeclared/contradiction baseline only shrinks
  endstate.py --self-test

Exit codes (goatbuilder's): 0 the site is in its declared end state; 3 anything
not deployed, not running, unhealthy, unwired, unknown, contradicted or orphaned;
1 the end state cannot be derived or the site cannot be read.
"""
import argparse
import base64
import fnmatch
import glob
import inspect
import json
import os
import re
import socket
import subprocess
import sys
from pathlib import Path

try:
    import yaml
except ImportError:  # the same one-line refusal stamp.py gives (X-PY-001)
    sys.exit("X-PY-001: this script needs PyYAML; run it under Ansible's python "
             "(head -1 \"$(command -v ansible-playbook)\") or `pip install pyyaml`")

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent.parent
sys.path.insert(0, str(HERE))
import edges as E  # noqa: E402  the zero-trust edge checks (zenoh, NetBird, secrets)
SCHEMA = "goatnet-endstate/v1"

PASS, FAIL, UNKNOWN, NA = "PASS", "FAIL", "UNKNOWN", "N/A"
# Nothing declared can observe it (no health signal, no acceptance row): a coverage
# gap, counted and listed apart from sync and health (ADR 1125), never a PASS.
UNOBSERVED = "UNOBSERVED"


# ---------------------------------------------------------------------------
# reading declarations
# ---------------------------------------------------------------------------

def _load_yaml(p):
    try:
        with open(p) as f:
            y = yaml.safe_load(f)
        return y if isinstance(y, dict) else {}
    except (OSError, yaml.YAMLError):
        return {}


def group_of(site):
    return site.replace("-", "_") + "_hosts"


def site_vars(root, group):
    """The site's group_vars, every file of the group merged (later files win, as Ansible's
    directory load does), over group_vars/all.yml."""
    v = dict(_load_yaml(root / "ops/ansible/group_vars/all.yml"))
    for f in sorted(glob.glob(str(root / f"ops/ansible/group_vars/{group}/*.yml"))):
        v.update(_load_yaml(f))
    return v


_ROLE_DEFAULTS = {}


def role_default(root, key):
    """A role default for `key`, from any role that defines it (the value a play
    falls back to when the site's group_vars say nothing)."""
    if root not in _ROLE_DEFAULTS:
        d = {}
        for f in sorted(glob.glob(str(root / "ops/ansible/roles/*/defaults/main.yml"))):
            for k, val in _load_yaml(f).items():
                d.setdefault(k, val)
        _ROLE_DEFAULTS[root] = d
    return _ROLE_DEFAULTS[root].get(key, None)


def truthy(v):
    if isinstance(v, bool):
        return v
    s = str(v).strip().lower()
    if s in ("true", "yes", "1", "on"):
        return True
    if s in ("false", "no", "0", "off", ""):
        return False
    return None  # a template the model cannot decide


def render(s, v):
    """Expand the few site facts names and surfaces are written in."""
    if not isinstance(s, str):
        return s
    s = s.replace("{site}", "{{ site_id }}").replace("{zenoh_stack}", "{{ zenoh_stack }}")
    for _ in range(3):  # a value may itself be a template (zenoh_stack: "{{ site_id }}-zenoh")
        for k in ("zenoh_stack", "site_id", "site_dns_zone"):
            val = str(v.get(k, "") or "")
            s = re.sub(r"\{\{\s*" + k + r"\s*\}\}", lambda _m: val, s)
    return s


def inventory_files(root, group):
    """The inventories that carry the group: the shared pair first (the mesh override loaded
    after the base, as the plays load them), then any site inventory of its own
    (zagros-cloud.ini). The door inventory is a transport, not a declaration of hosts."""
    d = root / "ops/ansible/inventory"
    names = ["mgmt-hosts.ini", "mgmt-hosts-mesh.ini"]
    for p in sorted(d.glob("*.ini")) if d.is_dir() else []:
        if p.name not in names and p.name != "doors-groupvars.ini" and f"[{group}]" in p.read_text():
            names.append(p.name)
    return [n for n in names if (d / n).exists()]


def inventory_hosts(root, group):
    """host name -> address, the later inventory winning (as the plays load them)."""
    out = {}
    for inv in inventory_files(root, group):
        p = root / "ops/ansible/inventory" / inv
        inside = False
        for line in p.read_text().splitlines():
            t = line.strip()
            if t.startswith("["):
                inside = t == f"[{group}]"
                continue
            if inside and t and not t.startswith("#"):
                name = t.split()[0]
                m = re.search(r"ansible_host=(\S+)", t)
                out[name] = m.group(1) if m else name
    return out


def pins_table(root):
    """fleet-pins.yaml merged with the lite sidecar (ops/pins/lite-pins.yaml, ADR
    1213): goatnets union, each component's deviations union. The same merge
    generate-pins.py and `goatbuilder pins` make; this reader only looks up
    membership and `absent:`, so it merges the two maps and validates nothing."""
    pins = _load_yaml(root / "ops/pins/fleet-pins.yaml") or {}
    lite_path = root / "ops/pins/lite-pins.yaml"
    if not lite_path.exists():
        return pins
    lite = _load_yaml(lite_path) or {}
    goatnets = dict(pins.get("goatnets") or {})
    goatnets.update(lite.get("goatnets") or {})
    components = {k: dict(v or {}) for k, v in (pins.get("components") or {}).items()}
    for cname, spec in (lite.get("components") or {}).items():
        if cname not in components:
            continue
        devs = dict(components[cname].get("deviations") or {})
        devs.update((spec or {}).get("deviations") or {})
        components[cname]["deviations"] = devs
    return {**pins, "goatnets": goatnets, "components": components}


def pin_says(pins, group, component):
    """True/False when the pin table governs this member; None when it does not."""
    comps = (pins.get("components") or {})
    members = {m for g in (pins.get("goatnets") or {}).values() for m in (g.get("members") or [])}
    if group not in members:
        return None, f"{group} is not a member of any goatnet in ops/pins/fleet-pins.yaml"
    row = comps.get(component)
    if row is None:
        return None, f"no `{component}` row in ops/pins/fleet-pins.yaml"
    # A deviation is scoped to a member group or to a whole goatnet (the shape
    # generate-pins.py resolves; the lite sidecar writes goatnet scope). Reading
    # only the group scope reported every sidecar `absent:` as present.
    devs = row.get("deviations") or {}
    goatnet = next((n for n, g in (pins.get("goatnets") or {}).items() if group in (g.get("members") or [])), None)
    dev = devs.get(group) or (devs.get(goatnet) if goatnet else None) or {}
    if dev.get("absent"):
        scope = group if devs.get(group) else goatnet
        return False, f"fleet-pins: {component} is `absent: true` for {scope}"
    return True, f"fleet-pins renders {component} for {group}"


def declared_want(root, group, component, pins):
    row = (pins.get("components") or {}).get(component) or {}
    var = row.get("var")
    if not var:
        return None
    p = root / "ops/ansible/group_vars" / group / "pins.yml"
    if not p.exists():
        return None
    for line in p.read_text().splitlines():
        if line.startswith(f"{var}:"):
            return line.split(":", 1)[1].strip().strip('"').strip("'") or None
    return None


# ---------------------------------------------------------------------------
# the model
# ---------------------------------------------------------------------------

def catalog(root):
    c = _load_yaml(root / "ops/goatbuilder/endstate/catalog.yaml")
    if not c.get("services"):
        c = _load_yaml(REPO / "ops/goatbuilder/endstate/catalog.yaml")
    return c


def presence(root, svc, spec, v, group, pins):
    """(present: True/False/None, contradiction: bool, declarations: [...])."""
    decls = []
    founder = truthy(v.get("goatnet_founder", True))
    if spec.get("founder") and founder is False:
        return False, False, [{"kind": "founder", "says": False, "why": "founder-only service; this site joins a goatnet (goatnet_founder: false)"}]
    if "pin" in spec:
        says, why = pin_says(pins, group, spec["pin"])
        if says is not None:
            decls.append({"kind": "pin", "name": spec["pin"], "says": says, "why": why})
    if "var" in spec:
        key = spec["var"]
        if key in v:
            says, why = truthy(v[key]), f"group_vars/{group}: {key}: {v[key]}"
        else:
            rd = role_default(root, key)
            if rd is not None:
                says, why = truthy(rd), f"role default {key}: {rd}"
            elif "default" in spec:
                says, why = bool(spec["default"]), f"{key} unset; the catalog default for {svc} is {spec['default']}"
            else:
                says, why = None, f"{key} is set nowhere: declare it in group_vars/{group}/"
        if says is not None:
            decls.append({"kind": "var", "name": key, "says": says, "why": why})
        else:
            decls.append({"kind": "var", "name": key, "says": None, "why": why})
    # One fact, one home (#2140): the pin table declares VERSIONS and renders every
    # component for every founder member; only its `absent: true` is a statement of
    # presence. So where the service has its own flag and the flag answers, a pin that
    # merely renders the component is recorded as the version it is, not as a vote. An
    # explicit `absent: true` against a flag that says on is still a contradiction.
    flag = next((d for d in decls if d["kind"] == "var" and d["says"] is not None), None)
    if flag is not None:
        for d in decls:
            if d["kind"] == "pin" and d["says"] is True:
                d["says"] = None
                d["why"] += " (a version, not a presence vote: the flag decides)"
    if spec.get("founder") and not decls:
        return True, False, [{"kind": "founder", "says": True, "why": "every founder runs it"}]
    answers = {d["says"] for d in decls if d["says"] is not None}
    if not answers:
        return None, False, decls
    if len(answers) > 1:
        return None, True, decls
    return answers.pop(), False, decls


def _proto_port(x):
    proto, _, port = str(x).partition("/")
    return (proto, int(port))


_SERVED = None


def _served_mod():
    """ops/scripts/served-payloads.py, the one judge of what a serving surface serves (#2138)."""
    global _SERVED
    if _SERVED is None:
        import importlib.util
        spec = importlib.util.spec_from_file_location("served_payloads", REPO / "ops/scripts/served-payloads.py")
        _SERVED = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(_SERVED)
    return _SERVED


def served_declared(root, ledger, group):
    """{payload: tag|"absent"} for a role that serves release assets; {} for every other."""
    if not ledger:
        return {}
    try:
        return _served_mod().declared(str(root), ledger, group) or {}
    except Exception as e:  # an unresolvable declaration is reported by the grade, not guessed
        return {"?": f"unresolvable: {str(e)[:120]}"}


def deploy_play(root, name, spec, pins):
    """The play that deploys the service, resolved the way deploy-component.sh resolves it."""
    cands = []
    if spec.get("play"):
        cands.append(spec["play"])
    pin = (spec.get("present") or {}).get("pin")
    row = (pins.get("components") or {}).get(pin) or {} if pin else {}
    if row.get("deploy_playbook"):
        cands.append(row["deploy_playbook"])
    for n in (spec.get("ledger"), name, pin):
        if n:
            cands.append(f"{n}-deploy.yml")
    for c in cands:
        if (Path(root) / "ops/ansible" / c).is_file():
            return f"ops/ansible/{c}"
    return None


def model(site, root=REPO):
    root = Path(root)
    group = group_of(site)
    if not (root / "ops/ansible/group_vars" / group).is_dir():
        raise SystemExit(f"GB-066: no goatnet site named `{site}`: ops/ansible/group_vars/{group}/ does not exist")
    v = site_vars(root, group)
    if not v.get("zenoh_stack") and role_default(root, "zenoh_stack"):
        v["zenoh_stack"] = role_default(root, "zenoh_stack")
    pins = pins_table(root)
    cat = catalog(root)
    mgmt = inventory_hosts(root, group)
    obs_group = group.replace("_hosts", "_obs_hosts")
    obs = inventory_hosts(root, obs_group) if truthy(v.get("has_obs_host", False)) else {}
    hosts = {"mgmt": sorted(mgmt), "obs": sorted(obs) if obs else sorted(mgmt)}
    machines = sorted(set(hosts["mgmt"]) | set(hosts["obs"]))
    shape = "single-host" if len(machines) == 1 else ("no-host" if not machines else f"{len(machines)}-host")
    addresses = {**mgmt, **obs}

    surfaces = {}
    for s in (v.get("isitup_services") or []):
        if isinstance(s, dict) and s.get("id"):
            surfaces[str(s["id"])] = {
                "id": str(s["id"]),
                "fqdn": render(s.get("fqdn"), v),
                "container": render(s.get("container"), v),
                "port": s.get("port"),
                "health_path": s.get("health_path"),
                # isitup's own rules (roles/isitup/files/check.sh): the codes that mean up,
                # and whether the name is a front door at all (F-430: `probe_front_door`
                # defaults to the row's `probe_gate`).
                "expect_codes": str(s.get("expect_codes") or "2|3"),
                "front_door": truthy(s.get("probe_front_door", s.get("probe_gate", True))) is not False,
            }

    services = {}
    claimed_surfaces = set()
    for name, spec in (cat.get("services") or {}).items():
        present, contradiction, decls = presence(root, name, spec.get("present") or {}, v, group, pins)
        cls = spec.get("class", "mgmt")
        row = {
            "present": present,
            "contradiction": contradiction,
            "declarations": decls,
            "class": cls,
            "hosts": hosts["obs" if cls == "obs" else "mgmt"],
            "containers": [render(c, v) for c in (spec.get("containers") or [])],
            "oneshot": [render(c, v) for c in (spec.get("oneshot") or [])],
            "ledger": spec.get("ledger"),
            "want": None,
            "surfaces": [],
            "oidc_clients": list(spec.get("oidc_clients") or []),
            "spiffe": list(spec.get("spiffe") or []),
            "assay": list(spec.get("assay") or []),
            "assay_exclude": list(spec.get("assay_exclude") or []),
            "zenoh": None,
            "mesh": None,
            "play": None,
            "roles": [r for r in ([spec.get("role")] if spec.get("role") else [name, spec.get("ledger") or ""]) if r],
        }
        z = spec.get("zenoh") or {}
        if z:
            row["zenoh"] = {"put": [render(k, v) for k in z.get("put") or []], "sub": [render(k, v) for k in z.get("sub") or []]}
        row["play"] = deploy_play(root, name, spec, pins)
        row["served_want"] = served_declared(root, spec.get("ledger"), group) if present is True else {}
        if spec.get("present", {}).get("pin"):
            row["want"] = declared_want(root, group, spec["present"]["pin"], pins)
        for sid in (spec.get("surfaces") or []):
            claimed_surfaces.add(sid)
            row["surfaces"].append(surfaces.get(sid) or {"id": sid, "undeclared": True})
        mesh = spec.get("mesh") or {}
        reach = [_proto_port(x) for x in mesh.get("reach") or []]
        if any(sf.get("fqdn") and sf.get("front_door", True) for sf in row["surfaces"]) and ("tcp", 443) not in reach:
            reach.append(("tcp", 443))   # a front door is reached through traefik on :443
        internal = [_proto_port(x) for x in mesh.get("internal") or []]
        if reach or internal:
            row["mesh"] = {"reach": reach, "internal": internal, "from": list(mesh.get("from") or ["PLATFORM-ADMIN"]),
                           "dest": mesh.get("dest") or "INFRA"}
        services[name] = row

    unowned = sorted(sid for sid, s in surfaces.items() if sid not in claimed_surfaces and s.get("container") not in (None, "None", ""))
    return {
        "schema": SCHEMA,
        "site": site,
        "group": group,
        "shape": shape,
        "founder": truthy(v.get("goatnet_founder", True)) is not False,
        "hosts": hosts,
        "addresses": addresses,
        "inventories": sorted(set(inventory_files(root, group)) | set(inventory_files(root, obs_group))),
        "mesh_dns": v.get("mesh_ip_a"),
        # the NetBird account this site's peers live in: a joiner shares its founder's
        "acl_site": (str(v.get("join_founder_site")) if truthy(v.get("goatnet_founder", True)) is False and v.get("join_founder_site") else site),
        "mgmt_url": f"https://{v['public_mgmt_fqdn']}" if v.get("public_mgmt_fqdn") else None,
        "zenoh_router": render("{zenoh_stack}-router", v) if v.get("zenoh_stack") else None,
        "services": services,
        "unowned_surfaces": unowned,
    }


# ---------------------------------------------------------------------------
# observing (read-only)
# ---------------------------------------------------------------------------

REMOTE = r'''
import json, subprocess, glob, base64, re
inp = json.loads(base64.b64decode("%s").decode())
def sh(c, t=30):
    try:
        r = subprocess.run(c, shell=True, capture_output=True, text=True, timeout=t)
        return r.returncode, r.stdout
    except Exception:
        return 1, ""
out = {"containers": {}, "ledgers": {}, "hydra": None, "spire": None, "routes": {}, "health": {}}
rc, o = sh("docker ps -aq")
ids = o.split()
if ids:
    fmt = "{{.Name}}|{{.State.Status}}|{{.State.ExitCode}}|{{if .State.Health}}{{.State.Health.Status}}{{end}}|{{range .NetworkSettings.Networks}}{{.IPAddress}} {{end}}"
    rc, o = sh("docker inspect --format '%%s' %%s" %% (fmt, " ".join(ids)))
    for l in o.splitlines():
        p = l.split("|")
        if len(p) == 5:
            out["containers"][p[0].lstrip("/")] = {"status": p[1], "exit": int(p[2] or 0), "health": p[3], "ips": p[4].split()}
for f in glob.glob("/var/lib/goat/*/deploy-history.jsonl"):
    rc, o = sh("tail -n 1 %%s" %% f)
    try:
        out["ledgers"][f.split("/")[-2]] = json.loads(o)
    except Exception:
        out["ledgers"][f.split("/")[-2]] = {"unparseable": o[:200]}
rc, o = sh("curl -s -m 5 'http://127.0.0.1:4445/admin/clients?page_size=500'")
try:
    out["hydra"] = [{"id": c["client_id"], "redirects": c.get("redirect_uris") or []} for c in json.loads(o)]
except Exception:
    hyd = [n for n in out["containers"] if n.endswith("hydra-1")]
    if hyd:
        rc, o = sh("docker exec %%s hydra list clients --endpoint http://127.0.0.1:4445 --format json" %% hyd[0])
        try:
            out["hydra"] = [{"id": c["client_id"], "redirects": c.get("redirect_uris") or []} for c in json.loads(o)["items"]]
        except Exception:
            pass
if "spire-server" in out["containers"]:
    rc, o = sh("docker exec spire-server /opt/spire/bin/spire-server entry show -output json")
    try:
        out["spire"] = sorted({e["spiffe_id"]["path"] for e in json.loads(o)["entries"]})
    except Exception:
        pass
for h in inp.get("health", []):
    c = out["containers"].get(h["container"]) or {}
    ip = (c.get("ips") or [""])[0]
    if not ip or c.get("status") != "running":
        continue
    rc, o = sh("curl -s -o /dev/null -w '%%%%{http_code}' -m 5 http://%%s:%%s%%s" %% (ip, h["port"], h["path"]), 10)
    out["health"][h["container"] + ":" + str(h["port"]) + h["path"]] = o.strip() or "000"
dns = inp.get("dns")
for r in inp.get("routes", []):
    fq = r["fqdn"]
    res = {"dns": [], "tls": None, "http": None}
    rc, o = sh("getent ahostsv4 %%s | awk '{print $1}' | sort -u" %% fq, 8)
    res["dns"] = [x for x in o.split() if x[:1].isdigit()]
    if res["dns"]:
        ip = res["dns"][0]
        rc, o = sh("echo | openssl s_client -connect %%s:443 -servername %%s -verify_hostname %%s -verify_return_error 2>&1 | grep -E 'Verify return code'" %% (ip, fq, fq), 12)
        res["tls"] = o.strip()
        rc, o = sh("curl -s -o /dev/null -w '%%%%{http_code}' -m 6 --resolve %%s:443:%%s https://%%s%%s" %% (fq, ip, fq, r.get("path") or "/"), 10)
        res["http"] = o.strip() or "000"
    out["routes"][fq] = res
#ZENOH#
print("GBES " + json.dumps(out))
'''

# Appended to REMOTE unformatted (it carries no %% escapes): read the router's live
# access_control and each fabric client's certificate. Read-only.
ZENOH_REMOTE = r'''
z = inp.get("zenoh") or {}
if z.get("router"):
    zo = {"acl": None, "acl_err": None, "clients": {}}
    def _inspect(name):
        rc, o = sh("docker inspect " + name)
        try:
            return json.loads(o)[0]
        except Exception:
            return None
    def _host(path, info):
        best = None
        for m in (info or {}).get("Mounts") or []:
            d = m.get("Destination") or ""
            if path == d or path.startswith(d.rstrip("/") + "/"):
                if best is None or len(d) > len(best[0]):
                    best = (d, m.get("Source") or "")
        return best[1] + path[len(best[0]):] if best else None
    ri = _inspect(z["router"])
    ca_host = None
    cfg_dest = None
    for m in (ri or {}).get("Mounts") or []:
        if (m.get("Destination") or "").endswith("config.json5"):
            cfg_dest = m["Destination"]
    if ri is None:
        zo["acl_err"] = "router container %s not found" % z["router"]
    elif not cfg_dest:
        zo["acl_err"] = "the router mounts no config.json5"
    else:
        try:
            cfg = json5_lite(open(_host(cfg_dest, ri)).read())
            acl = cfg.get("access_control") or {"enabled": False}
            tls = ((cfg.get("transport") or {}).get("link") or {}).get("tls") or {}
            ca = tls.get("root_ca_certificate")
            ca_host = _host(ca, ri) if ca else None
        except Exception as e:
            acl = None
            zo["acl_err"] = "the router config could not be parsed: %s" % str(e)[:160]
    cns = []
    for svc, names in (z.get("clients") or {}).items():
        cl = {"err": None}
        info = None
        for n in names:
            info = _inspect(n)
            if info:
                break
        if not info:
            cl["err"] = "no running container to read its certificate from"
        else:
            env = dict(e.split("=", 1) for e in (info.get("Config") or {}).get("Env") or [] if "=" in e)
            cert = next((v for k, v in env.items() if v.endswith(".pem") and re.search(r"CERT(_FILE|_PATH)?$", k) and "CA" not in k), None)
            hp = _host(cert, info) if cert else None
            if not hp:
                # No cert in the environment (grant-keyd reads its own config file): the
                # client cert is the *cert.pem in a mounted directory named for it.
                import os as _os
                base = (info.get("Name") or "").lstrip("/")
                for m in info.get("Mounts") or []:
                    src = m.get("Source") or ""
                    if _os.path.isdir(src):
                        for fn in sorted(_os.listdir(src)):
                            if fn.endswith("cert.pem") and "ca" not in fn.split("-")[0].lower() and base.split("-")[0] in fn:
                                hp = _os.path.join(src, fn)
                                break
                    if hp:
                        break
            if not hp:
                cl["err"] = "no client certificate in its environment"
            else:
                rc, o = sh("openssl x509 -noout -subject -enddate -nameopt RFC2253 -in %s" % hp)
                m = re.search(r"CN=([^,\n]+)", o)
                cl["cn"] = m.group(1).strip() if m else ""
                m = re.search(r"notAfter=(.+)", o)
                cl["not_after"] = m.group(1).strip() if m else ""
                rc, _ = sh("openssl x509 -noout -checkend 0 -in %s" % hp)
                cl["expired"] = rc != 0
                if ca_host:
                    rc, o = sh("openssl verify -CAfile %s %s 2>&1" % (ca_host, hp))
                    cl["verify"] = "OK" if rc == 0 else (o.strip().splitlines() or ["verify failed"])[-1][:200]
                else:
                    cl["verify"] = "the router's root CA could not be located"
                cns.append(cl["cn"])
        zo["clients"][svc] = cl
    if acl is not None:
        zo["acl"] = prune_acl(acl, cns)
    out["zenoh"] = zo
'''


def _reach(addr, port=22):
    try:
        with socket.create_connection((addr, port), timeout=3):
            return True
    except OSError:
        return False


# The AWS profile with-aws reads the hosts through: GOAT_AWS_PROFILE, then
# deploy.aws_profile in the tree's ops/goatbuilder/project.yml
# (ops/scripts/goatnet-facts.py deploy aws_profile), else this one.
DEFAULT_AWS_PROFILE = "usareur-af"


def aws_profile(root=REPO):
    env = os.environ.get("GOAT_AWS_PROFILE", "").strip()
    if env:
        return env
    try:
        p = subprocess.run([sys.executable, str(Path(root) / "ops/scripts/goatnet-facts.py"), "deploy", "aws_profile"],
                           capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return DEFAULT_AWS_PROFILE
    return (p.stdout.strip() if p.returncode == 0 else "") or DEFAULT_AWS_PROFILE


def observe(m, root=REPO):
    root = Path(root)
    profile = aws_profile(root)
    facts = {"site": m["site"], "hosts": {}}
    first_mgmt = m["hosts"]["mgmt"][0] if m["hosts"]["mgmt"] else None
    routes, health = [], []
    for name, s in m["services"].items():
        if s["present"] is not True:
            continue
        for sf in s["surfaces"]:
            if sf.get("fqdn") and sf.get("front_door", True) and sf.get("fqdn") not in [r["fqdn"] for r in routes]:
                routes.append({"fqdn": sf["fqdn"], "path": sf.get("health_path") or "/"})
            if sf.get("container") and sf.get("port"):
                health.append({"container": sf["container"], "port": sf["port"], "path": sf.get("health_path") or "/"})
    zclients = {name: s["containers"] for name, s in m["services"].items() if s["present"] is True and s.get("zenoh")}
    zremote = E.JSON5_LITE + "\n" + inspect.getsource(E._subject_matches) + "\n" + inspect.getsource(E.prune_acl) + ZENOH_REMOTE
    for host in sorted(set(m["hosts"]["mgmt"]) | set(m["hosts"]["obs"])):
        inp = {"dns": m.get("mesh_dns"), "routes": routes if host == first_mgmt else [], "health": health}
        if host == first_mgmt and m.get("zenoh_router") and zclients:
            inp["zenoh"] = {"router": m["zenoh_router"], "clients": zclients}
        script = (REMOTE % base64.b64encode(json.dumps(inp).encode()).decode()).replace("#ZENOH#", zremote)
        cmd_py = "python3 -c " + _shq("import base64;exec(base64.b64decode('%s').decode())" % base64.b64encode(script.encode()).decode())
        addr = m["addresses"].get(host, host)
        reach = _reach(addr)
        invs = [a for inv in m.get("inventories") or ["mgmt-hosts.ini"]
                if reach or inv != "mgmt-hosts-mesh.ini" for a in ("-i", f"inventory/{inv}")]
        argv = [str(root / "ops/scripts/with-aws"), profile, "--", "ansible", *invs, host, "-b", "-m", "shell", "-a", cmd_py]
        try:
            p = subprocess.run(argv, cwd=root / "ops/ansible", capture_output=True, text=True, timeout=600,
                               env={**os.environ, "OBJC_DISABLE_INITIALIZE_FORK_SAFETY": "YES", "ANSIBLE_HOST_KEY_CHECKING": "False"})
            text = p.stdout
        except (OSError, subprocess.TimeoutExpired) as e:
            text = f"ERR {e}"
        line = next((l for l in text.splitlines() if l.startswith("GBES ")), None)
        if line:
            facts["hosts"][host] = json.loads(line[5:])
        else:
            facts["hosts"][host] = {"unreachable": (text.strip().splitlines() or ["no output"])[0][:300]}
    facts["netbird"] = observe_netbird(m, root)
    facts["secrets"] = observe_deploy_inputs(m, root)
    return facts


def observe_netbird(m, root=REPO):
    """The site's declared ACL rules, and the live account's policies when its token is set."""
    root = Path(root)
    site = m.get("acl_site") or m["site"]
    out = {"rules": None, "why": None, "live": None, "live_why": None, "undeclared": [], "all_group": []}
    try:
        mx = _load_yaml(root / "ops/ansible/netbird_acl_matrix.yml").get("netbird_acl_matrix") or {}
    except Exception as e:
        out["why"] = f"ops/ansible/netbird_acl_matrix.yml could not be read: {e}"
        return out
    if site not in mx:
        out["why"] = f"ops/ansible/netbird_acl_matrix.yml declares no ACL for {site}"
        return out
    out["rules"] = mx[site].get("rules") or []
    declared = {r.get("name") for r in out["rules"]}
    try:
        import importlib.util
        spec = importlib.util.spec_from_file_location("nb_posture", root / "ops/scripts/netbird-policy-posture.py")
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        mtx, tax = mod.load()
        declared |= mod.declared_names(site, mtx, tax)   # exercise scopes this site hosts
    except Exception as e:  # without the helper, exercise scopes are not added; say so
        out["why"] = f"exercise scopes not read: {str(e)[:120]}"
    var = site.replace("-", "_").upper() + "_NB_API_TOKEN"
    has_pat = bool(os.environ.get(var))
    if not m.get("mgmt_url"):
        out["live_why"] = "the site declares no public_mgmt_fqdn"
    elif not has_pat:
        out["live_why"] = f"{var} is not set (run under ops/scripts/with-secrets)"
    else:
        try:
            live, allg = E.netbird_live(m["mgmt_url"], var)
            out["live"], out["all_group"] = live, allg
            out["undeclared"] = sorted(n for n, en in live.items() if en and n not in declared)
        except Exception as e:
            out["live_why"] = f"GET {m['mgmt_url']}/api/policies failed: {str(e)[:160]}"
    return out


def observe_deploy_inputs(m, root=REPO):
    """Per present service: the env-bound secrets its deploy play templates, and which the
    declared posture can supply. Names only."""
    root = Path(root)
    have, store, err, legacy = None, None, None, set()
    try:
        import importlib.util
        spec = importlib.util.spec_from_file_location("goat_posture", root / "ops/secrets/posture.py")
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        store = mod.store(str(root))
        have = set(mod.available(str(root))) | {k for k, v in os.environ.items() if v}
        legacy = set()
        # What a deploy from this checkout actually gets: ansible-preflight.sh loads the
        # main checkout's .envrc.local.d — references (*.refs), and the legacy plaintext
        # *.envrc unless GOAT_REFERENCES_ONLY=1. Names only; no value is kept.
        main = mod._main_checkout(str(root)) or str(root)
        d = Path(main) / ".envrc.local.d"
        if store == "openbao":
            d = d / "openbao"
        if store in ("1password", "openbao") and d.is_dir():
            for f in sorted(d.glob("*.refs")):
                for line in f.read_text(errors="replace").splitlines():
                    mm = re.match(r"\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)=['\"]?(op|bao)://", line)
                    if mm:
                        have.add(mm.group(1))
            if os.environ.get("GOAT_REFERENCES_ONLY", "0") != "1":
                for f in sorted(d.glob("*.envrc")):
                    for line in f.read_text(errors="replace").splitlines():
                        mm = re.match(r"\s*(?:export\s+)?([A-Z][A-Z0-9_]*)=(\S)", line)
                        if mm and mm.group(2) not in ("#",) and not line.split("=", 1)[1].strip() in ('""', "''"):
                            if mm.group(1) not in have:
                                legacy.add(mm.group(1))
        have = sorted(have | legacy)
    except Exception as e:
        err = f"the secrets posture could not be read: {str(e)[:160]}"
        legacy = set()
    binds = E.env_bindings(str(root), m["group"])
    out = {}
    for name, s in m["services"].items():
        if s["present"] is not True:
            continue
        if err:
            out[name] = {"err": err}
            continue
        if not s.get("play"):
            out[name] = {"play": None}
            continue
        words = E.play_words(str(root), str(root / s["play"]), s.get("roles") or [name])
        needed = {var: env for var, env in binds.items() if var in words}
        out[name] = {"play": s["play"], "needed": needed, "have": [e for e in have if e in needed.values()], "store": store,
                     "legacy": sorted(e for e in legacy if e in needed.values())}
    return out


def _shq(s):
    return "'" + s.replace("'", "'\\''") + "'"


# ---------------------------------------------------------------------------
# grading
# ---------------------------------------------------------------------------

def _match(name, pats):
    return any(fnmatch.fnmatchcase(name, p) for p in pats)


def _grade(status, why):
    return {"status": status, "why": why}


def _repo_of(ref):
    return re.sub(r"[:@].*$", "", ref.split("/")[-1]) if ref else ""


def _ledger_grade(row, want):
    if not isinstance(row, dict) or "unparseable" in row:
        return _grade(UNKNOWN, "ledger row unreadable")
    act = row.get("action")
    ref = str(row.get("ref") or row.get("image") or "")
    if want and ref and "/" in ref and _repo_of(want) and _repo_of(ref) != _repo_of(want):
        return _grade(UNKNOWN, f"the shared ledger's newest row records another image ({_repo_of(ref)}); this service runs {_repo_of(want)}")
    if act == "pending":
        return _grade(FAIL, f"newest ledger row is `pending` for {ref[-19:]}: the deploy play did not finish")
    if act == "rollback":
        return _grade(FAIL, f"rolled back from {str(row.get('prior_ref',''))[-19:]} to {ref[-19:]}; not at the declared ref")
    if act != "deploy":
        return _grade(UNKNOWN, f"newest ledger row action `{act}`")
    if want and "sha256:" in want:
        d = want.split("sha256:")[1][:12]
        if d not in ref:
            return _grade(FAIL, f"deployed {ref.split('@')[-1][:19]}, declared sha256:{d}")
    return _grade(PASS, f"ledger: deploy {ref.split('@')[-1][:19] or ref[:40]}")


def grade(m, facts, assay_rows=None, strict=False):
    hosts = facts.get("hosts") or {}
    unreachable = {h for h, f in hosts.items() if "unreachable" in f}
    allc = {}
    for h, f in hosts.items():
        for n, c in (f.get("containers") or {}).items():
            allc.setdefault(n, []).append((h, c))
    first = m["hosts"]["mgmt"][0] if m["hosts"]["mgmt"] else None
    hf = hosts.get(first) or {}
    hydra = hf.get("hydra")
    spire = hf.get("spire")
    rows = []
    by_token = {}
    for r in (assay_rows or []):
        for t in r.get("requires") or []:
            by_token.setdefault(t, []).append(r)

    present_pats = []
    for name, s in m["services"].items():
        res = {"service": name, "class": s["class"], "hosts": s["hosts"]}
        if s["contradiction"]:
            res["presence"] = _grade(FAIL, "declarations disagree: " + "; ".join(d["why"] for d in s["declarations"]))
            present_pats += s["containers"] + s["oneshot"]  # undecided, so not an orphan either
            rows.append(res)
            continue
        if s["present"] is None:
            res["presence"] = _grade(UNKNOWN, "presence undeclared: " + ("; ".join(d["why"] for d in s["declarations"]) or "no declaration"))
            present_pats += s["containers"] + s["oneshot"]  # whatever runs is not an orphan of an undeclared service
            rows.append(res)
            continue
        if s["present"] is False:
            res["presence"] = _grade(NA, "declared absent: " + "; ".join(d["why"] for d in s["declarations"]))
            rows.append(res)
            continue
        res["presence"] = _grade(PASS, "declared present: " + "; ".join(d["why"] for d in s["declarations"]))
        present_pats += s["containers"] + s["oneshot"]
        on = [h for h in s["hosts"] if h in hosts and h not in unreachable]
        # active
        if not on:
            res["active"] = _grade(UNKNOWN, "no host of its class could be read")
        else:
            missing, down = [], []
            for pat in s["containers"]:
                hits = [(n, c) for h in on for n, c in (hosts[h].get("containers") or {}).items() if fnmatch.fnmatchcase(n, pat)]
                if not hits:
                    missing.append(pat)
                for n, c in hits:
                    if c["status"] != "running":
                        down.append(f"{n} {c['status']} (exit {c['exit']})")
            for pat in s["oneshot"]:
                for h in on:
                    for n, c in (hosts[h].get("containers") or {}).items():
                        if fnmatch.fnmatchcase(n, pat) and c["status"] == "exited" and c["exit"] != 0:
                            down.append(f"{n} one-shot exited {c['exit']}")
            if missing or down:
                res["active"] = _grade(FAIL, "; ".join([f"no container {p}" for p in missing] + down))
            else:
                res["active"] = _grade(PASS, f"{len(s['containers'])} container pattern(s) running")
        # deployed
        if s["ledger"] is None:
            res["deployed"] = _grade(UNOBSERVED, "the service writes no deploy ledger")
        elif not on:
            res["deployed"] = _grade(UNKNOWN, "host unreadable")
        else:
            led = (hosts[on[0]].get("ledgers") or {}).get(s["ledger"])
            res["deployed"] = _ledger_grade(led, s.get("want")) if led else _grade(UNKNOWN, f"no ledger at /var/lib/goat/{s['ledger']}/")
            want_served = s.get("served_want") or {}
            if led and res["deployed"]["status"] == PASS and want_served:
                if "?" in want_served:
                    res["deployed"] = _grade(UNKNOWN, f"what it should serve cannot be resolved: {want_served['?']}")
                else:
                    rc, line = _served_mod().verdict(want_served, led)
                    if rc != 0:
                        res["deployed"] = _grade(FAIL if "DRIFT" in line else UNKNOWN, line)
                    else:
                        res["deployed"] = _grade(PASS, res["deployed"]["why"] + "; " + line)
        # healthy
        hs = []
        for pat in s["containers"]:
            for h in on:
                for n, c in (hosts[h].get("containers") or {}).items():
                    if fnmatch.fnmatchcase(n, pat) and c.get("health"):
                        hs.append((n, c["health"]))
        probes = []
        for sf in s["surfaces"]:
            if sf.get("container") and sf.get("port"):
                key = f"{sf['container']}:{sf['port']}{sf.get('health_path') or '/'}"
                for h in on:
                    code = (hosts[h].get("health") or {}).get(key)
                    if code:
                        probes.append((key, code, sf.get("expect_codes") or "2|3"))
        bad = [f"{n} {st}" for n, st in hs if st != "healthy"] + [f"{k} -> {c} (up is {ec})" for k, c, ec in probes if not re.match(f"^({ec})", c)]
        if bad:
            res["healthy"] = _grade(FAIL, "; ".join(bad))
        elif hs or probes:
            res["healthy"] = _grade(PASS, ", ".join([f"{n} healthy" for n, _ in hs] + [f"{k} {c}" for k, c, _ in probes]))
        elif res.get("active", {}).get("status") == FAIL:
            res["healthy"] = _grade(FAIL, "not running")
        else:
            res["healthy"] = _grade(UNOBSERVED, "no health signal: no docker healthcheck and no isitup row with a health path")
        # functional
        frows = list({r["id"]: r for t in s["assay"] for r in by_token.get(t, []) if r["id"] not in s.get("assay_exclude", [])}.values())
        if not s["assay"]:
            res["functional"] = _grade(UNOBSERVED, "no acceptance row exercises it")
        elif assay_rows is None:
            res["functional"] = _grade(UNKNOWN, f"assay not run; rows requiring {', '.join(s['assay'])} decide it (--assay)")
        elif not frows:
            res["functional"] = _grade(UNKNOWN, f"no assay row ran for {', '.join(s['assay'])}")
        else:
            st = [r.get("status") for r in frows]
            if FAIL in st:
                res["functional"] = _grade(FAIL, ", ".join(r["id"] for r in frows if r.get("status") == FAIL))
            elif all(x in (PASS, NA, "SKIPPED") for x in st) and PASS in st:
                res["functional"] = _grade(PASS, f"{st.count(PASS)} acceptance row(s) pass")
            elif all(x in (NA, "SKIPPED") for x in st):
                res["functional"] = _grade(UNOBSERVED, "no acceptance row that exercises it applies to this site")
            else:
                res["functional"] = _grade(UNKNOWN, ", ".join(f"{r['id']}={r.get('status')}" for r in frows))
        # edges
        edges = []
        seen = set()
        for sf in s["surfaces"]:
            if sf.get("fqdn") in seen:
                continue
            seen.add(sf.get("fqdn"))
            if not sf.get("undeclared") and not sf.get("front_door", True):
                continue  # not a front door by declaration (probe_front_door / probe_gate false)
            if sf.get("undeclared"):
                edges.append({"edge": f"route {sf['id']}", **_grade(FAIL, f"surface `{sf['id']}` is not in this site's isitup_services")})
                continue
            r = (hf.get("routes") or {}).get(sf["fqdn"])
            if r is None:
                edges.append({"edge": f"route {sf['fqdn']}", **_grade(UNKNOWN, "not probed")})
            elif not r["dns"]:
                edges.append({"edge": f"route {sf['fqdn']}", **_grade(FAIL, "the name does not resolve on the site's own host")})
            elif "(ok)" not in (r.get("tls") or ""):
                edges.append({"edge": f"route {sf['fqdn']}", **_grade(FAIL, f"TLS for the name does not verify: {r.get('tls') or 'no answer on :443'}")})
            elif (r.get("http") or "000") in ("404", "502", "503", "000"):
                edges.append({"edge": f"route {sf['fqdn']}", **_grade(FAIL, f"name and cert are right; the front door answers {r.get('http')}")})
            else:
                edges.append({"edge": f"route {sf['fqdn']}", **_grade(PASS, f"{r['dns'][0]}, cert verifies, HTTP {r['http']}")})
        for cid in s["oidc_clients"]:
            if hydra is None:
                edges.append({"edge": f"oidc-client {cid}", **_grade(UNKNOWN, "Hydra's client list could not be read")})
                continue
            hit = next((c for c in hydra if c["id"] == cid), None)
            if not hit:
                edges.append({"edge": f"oidc-client {cid}", **_grade(FAIL, f"client `{cid}` is not registered in Hydra: every login to it fails invalid_client")})
            else:
                want_hosts = {sf.get("fqdn") for sf in s["surfaces"] if sf.get("fqdn")}
                rh = {re.sub(r"^https?://([^/]+).*$", r"\1", u) for u in hit["redirects"]}
                if want_hosts and not (want_hosts & rh):
                    edges.append({"edge": f"oidc-client {cid}", **_grade(FAIL, f"registered, but its redirects ({', '.join(sorted(rh)) or 'none'}) are not on {', '.join(sorted(want_hosts))}")})
                else:
                    edges.append({"edge": f"oidc-client {cid}", **_grade(PASS, "registered in Hydra" + (f", redirect on {', '.join(sorted(want_hosts & rh))}" if want_hosts else ""))})
        for path in s["spiffe"]:
            if spire is None:
                edges.append({"edge": f"spiffe {path}", **_grade(UNKNOWN, "SPIRE's entries could not be read")})
            elif path in spire:
                edges.append({"edge": f"spiffe {path}", **_grade(PASS, "registration entry present")})
            else:
                edges.append({"edge": f"spiffe {path}", **_grade(FAIL, f"no SPIRE registration entry for {path}: the workload gets no SVID")})
        zf = hf.get("zenoh") or {}
        if s.get("zenoh"):
            edges += E.zenoh_edges(s["zenoh"], {"acl": zf.get("acl"), "acl_err": zf.get("acl_err") or ("the router was not read" if not zf else None),
                                                "client": (zf.get("clients") or {}).get(name)})
        nb = facts.get("netbird")
        if s.get("mesh"):
            edges += E.netbird_edges(s["mesh"], nb)
        if name == "netbird-mgmt" or (name == m.get("netbird_service")):
            edges.append(E.netbird_site_edge(nb))
        inputs = (facts.get("secrets") or {}).get(name) if "secrets" in facts else {"err": "the deploy inputs were not derived (no observe)"}
        edges.append(E.deploy_inputs_edge(inputs))
        res["edges"] = edges
        rows.append(res)

    # orphans: what runs that the end state does not declare
    orphans = []
    absent_owner = {}
    for name, s in m["services"].items():
        if s["present"] is False:
            for p in s["containers"] + s["oneshot"]:
                absent_owner[p] = name
    for n, where in sorted(allc.items()):
        if _match(n, present_pats):
            continue
        owner = next((svc for p, svc in absent_owner.items() if fnmatch.fnmatchcase(n, p)), None)
        st = ", ".join(f"{h}: {c['status']}" for h, c in where)
        orphans.append({"kind": "container", "name": n, "why": (f"service `{owner}` is declared absent here" if owner else "no service in the catalog owns it") + f" ({st})"})
    claimed_clients = {c for s in m["services"].values() if s["present"] is not False for c in s["oidc_clients"]}
    for c in (hydra or []):
        if c["id"] not in claimed_clients:
            orphans.append({"kind": "oidc-client", "name": c["id"], "why": "registered in Hydra; no present service declares it"})
    claimed_paths = {p for s in m["services"].values() if s["present"] is not False for p in s["spiffe"]}
    for p in (spire or []):
        if not (p.startswith("/agent/") or p.startswith("/spire/")) and p not in claimed_paths:
            orphans.append({"kind": "spiffe-entry", "name": p, "why": "a SPIRE entry no present service declares: it can still mint an SVID"})

    counts = {PASS: 0, FAIL: 0, UNKNOWN: 0, NA: 0, UNOBSERVED: 0}
    for r in rows:
        for k in ("presence", "active", "deployed", "healthy", "functional"):
            if k in r:
                counts[r[k]["status"]] += 1
        for e in r.get("edges", []):
            counts[e["status"]] += 1
    green = counts[FAIL] == 0 and counts[UNKNOWN] == 0 and not orphans and not unreachable and not (strict and counts[UNOBSERVED])
    return {
        "schema": SCHEMA + "+grade",
        "site": m["site"],
        "shape": m["shape"],
        "hosts": m["hosts"],
        "unreachable": sorted(unreachable),
        "counts": counts,
        "orphans": orphans,
        "services": rows,
        "unowned_surfaces": m["unowned_surfaces"],
        "exit": 0 if green else 3,
    }


def print_grade(g, out=sys.stdout):
    c = g["counts"]
    print(f"end state of {g['site']} ({g['shape']}: {', '.join(sorted(set(g['hosts']['mgmt']) | set(g['hosts']['obs'])))})", file=out)
    for r in g["services"]:
        p = r["presence"]
        if p["status"] == NA:
            continue
        parts = [f"{k}={r[k]['status']}" for k in ("active", "deployed", "healthy", "functional") if k in r]
        edges = r.get("edges", [])
        ok_e = sum(1 for e in edges if e["status"] == PASS)
        line = f"  {r['service']:26} {p['status']:7} " + " ".join(parts) + (f" edges={ok_e}/{len(edges)}" if edges else "")
        print(line, file=out)
        for k in ("presence", "active", "deployed", "healthy", "functional"):
            if k in r and r[k]["status"] in (FAIL, UNKNOWN, UNOBSERVED):
                print(f"      {k}: {r[k]['status']} — {r[k]['why']}", file=out)
        for e in edges:
            if e["status"] != PASS:
                print(f"      {e['edge']}: {e['status']} — {e['why']}", file=out)
    absent = [r["service"] for r in g["services"] if r["presence"]["status"] == NA]
    if absent:
        print(f"  declared absent ({len(absent)}): {', '.join(absent)}", file=out)
    if g["orphans"]:
        print(f"  orphans ({len(g['orphans'])}):", file=out)
        for o in g["orphans"]:
            print(f"      {o['kind']} {o['name']}: {o['why']}", file=out)
    if g["unreachable"]:
        print(f"  unreachable: {', '.join(g['unreachable'])}", file=out)
    print(f"  PASS {c[PASS]} · FAIL {c[FAIL]} · UNKNOWN {c[UNKNOWN]} · UNOBSERVED {c[UNOBSERVED]} · N/A {c[NA]} · orphans {len(g['orphans'])}", file=out)
    verdict = "in its declared end state" if g["exit"] == 0 else "NOT in its declared end state"
    print(f"  {g['site']} is {verdict}" + (f"; {c[UNOBSERVED]} grade(s) have no signal to read (coverage)" if c[UNOBSERVED] else ""), file=out)


# ---------------------------------------------------------------------------
# CI: the model renders for every governed site; the gaps only shrink
# ---------------------------------------------------------------------------

def governed_sites(root):
    pins = pins_table(root)
    groups = [m for g in (pins.get("goatnets") or {}).values() for m in (g.get("members") or [])]
    return [g[: -len("_hosts")].replace("_", "-") for g in groups if g.endswith("_hosts")]


def gaps(root):
    out = set()
    for site in governed_sites(root):
        m = model(site, root)
        for name, s in m["services"].items():
            if s["contradiction"]:
                out.add(f"contradiction {m['group']} {name}")
            elif s["present"] is None:
                out.add(f"undeclared {m['group']} {name}")
    return out


def check(root=REPO):
    root = Path(root)
    errs = []
    cat = catalog(root)
    pins = pins_table(root)
    comps = set((pins.get("components") or {}).keys())
    tokens = set()
    acc = _load_yaml(root / "ops/validation/acceptance-tests.yaml")
    for t in (acc.get("tests") or acc.get("rows") or []):
        if isinstance(t, dict):
            tokens |= set(t.get("requires") or [])
    for name, spec in (cat.get("services") or {}).items():
        p = spec.get("present") or {}
        if not ({"founder", "pin", "var"} & set(p)):
            errs.append(f"catalog: {name} has no presence declaration (founder/pin/var)")
        if p.get("pin") and p["pin"] not in comps:
            errs.append(f"catalog: {name} names pin `{p['pin']}`, which is not a component in ops/pins/fleet-pins.yaml")
        if spec.get("class", "mgmt") not in ("mgmt", "obs"):
            errs.append(f"catalog: {name} class `{spec.get('class')}` is not mgmt or obs")
        if not spec.get("containers"):
            errs.append(f"catalog: {name} names no containers")
    for site in governed_sites(root):
        try:
            m = model(site, root)
        except SystemExit as e:
            errs.append(str(e))
            continue
        for name, s in m["services"].items():
            if s["present"] and not s["hosts"]:
                errs.append(f"{site}: {name} is declared present but the site has no {s['class']} host in the inventory")
            for c in s["containers"]:
                if "{{" in c or c.startswith("-") or "{" in c:
                    errs.append(f"{site}: {name} container `{c}` did not render")
    base_p = HERE / "baseline.txt"
    base = {l.strip() for l in base_p.read_text().splitlines() if l.strip() and not l.startswith("#")} if base_p.exists() else set()
    now = gaps(root)
    for g in sorted(now - base):
        kind, grp, svc = g.split()
        hint = (f"declare it: set the service's flag in ops/ansible/group_vars/{grp}/ (catalog: ops/goatbuilder/endstate/catalog.yaml)"
                if kind == "undeclared" else f"make the declarations agree (fleet-pins `absent:` for {grp}, or the flag)")
        errs.append(f"GB-067: new {kind}: {grp} {svc} — {hint}")
    for g in sorted(base - now):
        errs.append(f"GB-067: baseline row no longer true, remove it from ops/goatbuilder/endstate/baseline.txt: {g}")
    for e in errs:
        print(f"FAIL {e}")
    if not errs:
        print(f"ok: {len(governed_sites(root))} sites model; {len(now)} declared gaps, none new")
    return 1 if errs else 0


# ---------------------------------------------------------------------------

def _edge_unit_tests():
    """The edge primitives on their own: the JSON5 reader, key-expression inclusion, the
    zenoh decision order and the NetBird matcher. Returns the number of failures."""
    bad = 0
    def t(cond, what):
        nonlocal bad
        if not cond:
            print(f"FAIL edges: {what}")
            bad += 1
    cfg = E.json5_lite("""{
      // a comment, and 'single' quotes
      mode: "router", id: 'abc', url: "https://x.test//not-a-comment",
      access_control: { enabled: true, default_permission: "deny",
        rules: [ { id: "r", key_exprs: ["ops/**",], }, ], /* trailing */ },
    }""")
    t(cfg["url"] == "https://x.test//not-a-comment" and cfg["id"] == "abc", "json5: a // inside a string is not a comment")
    t(cfg["access_control"]["rules"][0]["key_exprs"] == ["ops/**"], "json5: unquoted keys and trailing commas")
    t(E.ke_includes("ops/**", "ops/mesh/health/x"), "ke: ** covers deeper chunks")
    t(E.ke_includes("*/**", "a"), "ke: */** covers a one-chunk key")
    t(not E.ke_includes("**", "x/@adv/pub"), "ke: ** does not match a verbatim @ chunk")
    t(E.ke_includes("**/@adv/**", "x/@adv/pub"), "ke: **/@adv/** matches the verbatim chunk")
    t(E.ke_includes("ops/$*-health/x", "ops/mesh-health/x") and not E.ke_includes("ops/*", "ops/a/b"), "ke: $* and * stay in one chunk")
    acl = {"default_permission": "deny",
           "subjects": [{"id": "u", "link_protocols": ["tls"]}, {"id": "w", "cert_common_names": ["w"]}, {"id": "e", "cert_common_names": []}],
           "policies": [{"rules": ["allow-ops", "deny-audit"], "subjects": ["u"]}, {"rules": ["allow-audit"], "subjects": ["w"]}],
           "rules": [{"id": "allow-ops", "messages": ["put"], "flows": ["ingress"], "permission": "allow", "key_exprs": ["ops/**"]},
                     {"id": "deny-audit", "messages": ["put"], "permission": "deny", "key_exprs": ["ops/audit/operator/**"]},
                     {"id": "allow-audit", "messages": ["put"], "flows": ["ingress"], "permission": "allow", "key_exprs": ["ops/audit/operator/**"]}]}
    t(E.zenoh_decide(acl, "x", "put", "ingress", "ops/audit/operator/k")[0] == "deny", "zenoh: a deny inside the subject wins over its allow")
    t(E.zenoh_decide(acl, "w", "put", "ingress", "ops/audit/operator/k")[0] == "allow", "zenoh: across subjects the first allow wins")
    t(E.zenoh_decide(acl, "x", "put", "egress", "elsewhere/k")[0] == "deny", "zenoh: no rule means default_permission")
    t(not E._subject_matches(acl["subjects"][2], "w"), "zenoh: an empty cert_common_names list matches nobody")
    rules = [{"name": "a", "sources": ["INFRA"], "destinations": ["X"], "protocol": "all", "action": "accept", "bidirectional": True},
             {"name": "b", "sources": ["OBSERVER"], "destinations": ["INFRA"], "protocol": "tcp", "ports": ["8000-8100"], "action": "accept"}]
    t(E.netbird_admitting(rules, "X", "INFRA", "udp", 53) == ["a"], "netbird: a bidirectional rule admits the reverse path")
    t(E.netbird_admitting(rules, "OBSERVER", "INFRA", "tcp", 8080) == ["b"] and not E.netbird_admitting(rules, "OBSERVER", "INFRA", "tcp", 443), "netbird: port ranges")
    if not bad:
        print("ok   edges: json5 reader, key expressions, zenoh decision order, NetBird matcher")
    return bad


def self_test():
    fx = REPO / "ops/goatbuilder/fixtures/endstate"
    failed = _edge_unit_tests()
    for case in sorted(p for p in fx.iterdir() if p.is_dir()):
        exp = json.loads((case / "expected.json").read_text())
        m = model(exp["site"], case)
        facts = json.loads((case / "facts.json").read_text())
        assay = None
        if (case / "assay.jsonl").exists():
            assay = [json.loads(l) for l in (case / "assay.jsonl").read_text().splitlines() if l.strip()]
        g = grade(m, facts, assay)
        got = {
            "shape": g["shape"],
            "exit": g["exit"],
            "absent": sorted(r["service"] for r in g["services"] if r["presence"]["status"] == NA),
            "orphans": sorted(f"{o['kind']} {o['name']}" for o in g["orphans"]),
            "fail": sorted(
                [f"{r['service']} {k}" for r in g["services"] for k in ("presence", "active", "deployed", "healthy", "functional") if k in r and r[k]["status"] == FAIL]
                + [f"{r['service']} {e['edge']}" for r in g["services"] for e in r.get("edges", []) if e["status"] == FAIL]),
        }
        bad = 0
        for k, want in exp["expect"].items():
            if got.get(k) != want:
                print(f"FAIL {case.name}: {k}: got {got.get(k)!r}, want {want!r}")
                bad += 1
        failed += bad
        if not bad:
            print(f"ok   {case.name}")
    # A serving surface's ledger row carries what it serves (#2138): a payload that differs
    # from main fails `deployed` even when the image is the declared one.
    m = model("alpha", fx / "single-host")
    facts = json.loads((fx / "single-host" / "facts.json").read_text())
    m["services"]["core"]["served_want"] = {"goat-cli": "goat-cli-v0.5.14"}
    facts["hosts"]["h1"]["ledgers"]["core"]["served"] = {"goat-cli": "goat-cli-v0.5.6"}
    row = next(r for r in grade(m, facts, None)["services"] if r["service"] == "core")
    if row["deployed"]["status"] != FAIL or "DRIFT" not in row["deployed"]["why"]:
        print(f"FAIL served payload drift: deployed = {row['deployed']}")
        failed += 1
    else:
        facts["hosts"]["h1"]["ledgers"]["core"]["served"] = {"goat-cli": "goat-cli-v0.5.14"}
        row = next(r for r in grade(m, facts, None)["services"] if r["service"] == "core")
        if row["deployed"]["status"] != PASS:
            print(f"FAIL served payload as declared: deployed = {row['deployed']}")
            failed += 1
        else:
            print("ok   a served payload that differs from main fails deployed; as declared it passes")
    # The assay runner reads the model: a row needing only declared-absent services is N/A.
    import importlib.util
    spec = importlib.util.spec_from_file_location("run_acceptance", REPO / "ops/validation/run-acceptance.py")
    ra = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(REPO / "ops/validation"))
    spec.loader.exec_module(ra)
    import tempfile
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as fh:
        json.dump(model("gamma", fx / "orphans-and-breaks"), fh)
    gone = ra.declared_absent(fh.name)
    os.unlink(fh.name)
    if gone != set():  # sandbox carries no acceptance token; core and app are present
        print(f"FAIL run-acceptance --endstate: declared_absent = {gone}, want set()")
        failed += 1
    m2 = model("beta", fx / "no-grafana-no-sandbox")
    m2["services"]["app"]["present"] = False
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as fh:
        json.dump(m2, fh)
    gone = ra.declared_absent(fh.name)
    os.unlink(fh.name)
    if gone != {"app"}:
        print(f"FAIL run-acceptance --endstate: declared_absent = {gone}, want {{'app'}}")
        failed += 1
    else:
        print("ok   run-acceptance reads the model: a row needing only an absent service is N/A")
    return 1 if failed else 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--self-test", action="store_true")
    sub = ap.add_subparsers(dest="cmd")
    for c in ("model", "observe", "grade"):
        p = sub.add_parser(c)
        p.add_argument("site")
        p.add_argument("--root", default=str(REPO))
        if c == "observe":
            p.add_argument("--out")
        if c == "grade":
            p.add_argument("--facts")
            p.add_argument("--assay", help="the assay's rows (goatbuilder assay --rows-out)")
            p.add_argument("--json", action="store_true")
            p.add_argument("--facts-out")
            p.add_argument("--strict", action="store_true", help="a grade with no signal (UNOBSERVED) fails too")
            p.add_argument("--json-out", help="also write the grades to this file as JSON")
    p = sub.add_parser("check")
    p.add_argument("--root", default=str(REPO))
    a = ap.parse_args(argv)
    if a.self_test:
        return self_test()
    if a.cmd == "check":
        return check(a.root)
    if not a.cmd:
        ap.print_help()
        return 2
    m = model(a.site, Path(a.root))
    if a.cmd == "model":
        print(json.dumps(m, indent=2))
        return 0
    if a.cmd == "observe":
        f = observe(m, Path(a.root))
        text = json.dumps(f, indent=2)
        if a.out:
            Path(a.out).write_text(text)
        else:
            print(text)
        return 0
    facts = json.loads(Path(a.facts).read_text()) if a.facts else observe(m, Path(a.root))
    if a.facts_out:
        Path(a.facts_out).write_text(json.dumps(facts, indent=2))
    assay = None
    if a.assay:
        assay = []
        for l in Path(a.assay).read_text().splitlines():
            if l.strip():
                r = json.loads(l)
                assay.append(r)
    g = grade(m, facts, assay, strict=a.strict)
    if a.json_out:
        Path(a.json_out).write_text(json.dumps(g, indent=2))
    if a.json:
        print(json.dumps(g, indent=2))
    else:
        print_grade(g)
    return g["exit"]


if __name__ == "__main__":
    sys.exit(main())
