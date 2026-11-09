"""Phase-1 checks that read live state: ports, the core router, its receive
stores, the slot list, and the CA. Imported by preflight.py; every function
here takes its inputs as arguments and its command runner as a parameter, so
each check is tested with fakes (test_preflight_live.py) and no test touches a
host.

Every check returns one Result with one outcome word:

  PASS            checked, and it holds
  FAIL            checked, and it does not hold; the code names the fix
  BLOCKED         not checked, because an input it needs is missing (GB-130)
  SKIP            not checked, on purpose or because the thing could not be
                  read; never counted as a pass
  NOT APPLICABLE  this deployment has nothing for the check to look at

Nothing here changes a host. The port probe is a read-only play
(ops/ansible/peering-preflight.yml); the router and store reads are
`docker ps`, `docker inspect` and `docker exec ... cat` against the daemon
phase 3 reads (FED_ROUTER_DOCKER_HOST, else the local one).
"""
from __future__ import annotations

import glob
import importlib.util
import json
import os
import re
import subprocess
from dataclasses import asdict, dataclass, field
from pathlib import Path

PASS, FAIL, BLOCKED, SKIP, NA = "PASS", "FAIL", "BLOCKED", "SKIP", "NOT APPLICABLE"
SCHEMA = "goat-federation-preflight/v1"


@dataclass
class Result:
    check: str
    label: str
    outcome: str
    detail: str = ""
    code: str = ""
    fix: str = ""
    facts: dict = field(default_factory=dict)

    def as_dict(self):
        d = asdict(self)
        if not d["facts"]:
            del d["facts"]
        return d


# The checks this module owns, in the order preflight.py runs them. The ids are
# what `--only` takes and what goatbuilder lists in a dry run
# (crates/goatbuilder/src/federate/preflight.rs CHECKS holds the same list; a
# test there compares the two).
CHECKS = [
    ("inputs", "every required input is present", "GB-130"),
    ("edge-ports", "the tunnel and edge router ports are free on the edge", "GB-131"),
    ("core-ports", "the republisher metrics port is free on the core", "GB-132"),
    ("slot-anchors", "the committed slot list equals the registry", "GB-134"),
    ("core-router", "the core router container is found", "GB-138"),
    ("ingress-stores", "the core's receive stores match what the edge grants", "GB-136"),
    ("signing-ca", "our CA certificate and its signing key resolve", "GB-140"),
]


def run(argv, env=None, input_bytes=None, timeout=60):
    """The default runner: argv, no shell, captured. Returns (rc, stdout, stderr)."""
    try:
        p = subprocess.run(argv, capture_output=True, env=env, input=input_bytes, timeout=timeout)
    except (OSError, subprocess.SubprocessError) as exc:
        return 127, b"", str(exc).encode()
    return p.returncode, p.stdout, p.stderr


def text(b):
    return b.decode("utf-8", "replace") if isinstance(b, (bytes, bytearray)) else (b or "")


# ---------------------------------------------------------------------------
# Values the plays will use, read where the plays read them
# ---------------------------------------------------------------------------

def _plain_yaml_value(path, key):
    """`key: value` from one YAML file, or None. A templated value (`{{ }}`)
    is None: it resolves only inside a play, and guessing it would be a claim."""
    try:
        import yaml
        data = yaml.safe_load(Path(path).read_text()) or {}
    except Exception:  # noqa: BLE001 -- an unreadable file declares nothing
        return None
    if not isinstance(data, dict) or key not in data:
        return None
    v = data[key]
    if isinstance(v, str) and "{{" in v:
        return None
    return v


def group_var(repo, group, key):
    """A group's own declared value: group_vars/<group>.yml, then every file in
    group_vars/<group>/, in name order (the last file that sets it wins, as
    Ansible merges them)."""
    gv = Path(repo) / "ops/ansible/group_vars"
    found = None
    for cand in [gv / f"{group}.yml", gv / f"{group}.yaml"] + sorted(
            Path(p) for p in glob.glob(str(gv / group / "*.y*ml"))):
        if cand.is_file():
            v = _plain_yaml_value(cand, key)
            if v is not None:
                found = v
    return found


def role_default(repo, role, key):
    return _plain_yaml_value(Path(repo) / "ops/ansible/roles" / role / "defaults/main.yml", key)


# (input name, role, variable, group kind). An environment variable wins (the
# script avenue, and goatbuilder's allocated values, which it exports when the
# operator has not); then the group's own value; then the role's default, which
# is a port number or an interface name and belongs to no deployment.
PORT_INPUTS = [
    ("FED_WG_INTERFACE", "goat-peering-wg", "peering_wg_interface", "edge"),
    ("FED_WG_LISTEN_PORT", "goat-peering-wg", "peering_wg_listen_port", "edge"),
    ("FED_EDGE_LISTEN_PORT", "goat-zenoh-edge", "goat_zenoh_edge_listen_port", "edge"),
    ("FED_REPUBLISHER_METRICS_PORT", "goat-fed-republisher", "goat_fed_republisher_metrics_port", "core"),
]


def resolve_port_inputs(repo, edge_group, core_group, environ=None):
    """{input: (value or None, where it came from)}."""
    environ = os.environ if environ is None else environ
    out = {}
    for name, role, var, kind in PORT_INPUTS:
        group = edge_group if kind == "edge" else core_group
        if environ.get(name, ""):
            out[name] = (str(environ[name]), f"${name}")
            continue
        v = group_var(repo, group, var)
        if v is not None:
            out[name] = (str(v), f"group_vars/{group}: {var}")
            continue
        v = role_default(repo, role, var)
        if v is not None:
            out[name] = (str(v), f"roles/{role} default {var}")
            continue
        out[name] = (None, f"{var} in group_vars/{group}/, or ${name}")
    return out


def check_inputs(missing):
    """GB-130: every required input, named with where to declare it.
    `missing` is [(name, where)]."""
    if not missing:
        return Result("inputs", "required inputs", PASS, "every required input is present")
    return Result(
        "inputs", "required inputs", FAIL,
        f"{len(missing)} missing: " + ", ".join(n for n, _ in missing),
        "GB-130",
        "\n".join(f"{n}: {w}" for n, w in missing),
    )


# ---------------------------------------------------------------------------
# Ports, from the read-only probe play's output
# ---------------------------------------------------------------------------

def probe_command(repo, inventory, limit, out_dir, aws_profile):
    """The argv of the read-only probe, run from ops/ansible. Same inventory and
    AWS wrapper the peering plays use; plain ansible-playbook rather than the
    fresh-source wrapper, because a read changes nothing a stale tree could
    get wrong."""
    return [str(Path(repo) / "ops/scripts/with-aws"), aws_profile, "--",
            "ansible-playbook", "-i", str(inventory), "--limit", limit,
            "-e", f"fed_preflight_out={out_dir}", "peering-preflight.yml"]


def load_probe(out_dir):
    """{inventory host: facts} from the probe play's per-host files."""
    facts = {}
    for p in sorted(Path(out_dir).glob("*.json")):
        try:
            d = json.loads(p.read_text())
        except (OSError, ValueError):
            continue
        if isinstance(d, dict) and d.get("host"):
            facts[d["host"]] = d
    return facts


_USERS = re.compile(r'\("([^"]+)",pid=(\d+)')


def parse_ss(out):
    """`ss -H -l -n -t -u -p` lines -> [{proto, addr, port, procs: [(name, pid)]}]."""
    rows = []
    for line in text(out).splitlines():
        parts = line.split()
        if len(parts) < 5:
            continue
        # With -t -u together the first column is the netid (tcp/udp).
        if parts[0] in ("tcp", "udp"):
            proto, local = parts[0], parts[4]
        else:
            continue
        addr, _, port = local.rpartition(":")
        if not port.isdigit():
            continue
        procs = [(n, int(p)) for n, p in _USERS.findall(line)]
        rows.append({"proto": proto, "addr": addr, "port": int(port), "procs": procs})
    return rows


def parse_wg(out):
    """`wg show all listen-port` -> {interface: port}."""
    ifaces = {}
    for line in text(out).splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[1].isdigit():
            ifaces[parts[0]] = int(parts[1])
    return ifaces


def parse_pids(out):
    """`docker inspect --format '{{.State.Pid}} {{.Name}}'` lines -> {pid: name}."""
    pids = {}
    for line in text(out).splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[0].isdigit() and parts[0] != "0":
            pids[int(parts[0])] = parts[1].lstrip("/")
    return pids


def parse_containers(out):
    """`docker ps --format '{{.Names}}\t{{.Ports}}'` lines -> {name: ports}."""
    cont = {}
    for line in text(out).splitlines():
        name, _, ports = line.partition("\t")
        if name:
            cont[name.strip()] = ports.strip()
    return cont


def holders(facts, proto, port, own_container="", own_wg_interface=""):
    """Who holds `proto/port` on one host: [(description, ours)]."""
    found = []
    pids = parse_pids(facts.get("pids", ""))
    containers = parse_containers(facts.get("containers", ""))
    wg = parse_wg(facts.get("wg", "")) if proto == "udp" else {}
    published = bool(own_container) and re.search(
        rf":{port}->\d+/{proto}\b", containers.get(own_container, "")) is not None
    for row in parse_ss(facts.get("ss", "")):
        if row["proto"] != proto or row["port"] != port:
            continue
        if row["procs"]:
            for name, pid in row["procs"]:
                cname = pids.get(pid, "")
                ours = bool(own_container) and (cname == own_container or (name == "docker-proxy" and published))
                where = f"container {cname}" if cname else f"process {name} (pid {pid})"
                found.append((f"{proto}/{port} on {row['addr']} by {where}", ours))
        else:
            # A socket with no process is the kernel's: on a UDP port, a
            # WireGuard interface. `wg show` says which one.
            owners = [i for i, p in wg.items() if p == port]
            if owners:
                for i in owners:
                    found.append((f"{proto}/{port} by WireGuard interface {i}", i == own_wg_interface))
            else:
                found.append((f"{proto}/{port} on {row['addr']} by a kernel socket", False))
    # wg can hold a port ss did not list (another namespace); still a holder.
    for i, p in wg.items():
        if p == port and not any(f"interface {i}" in d for d, _ in found):
            found.append((f"udp/{port} by WireGuard interface {i}", i == own_wg_interface))
    # De-duplicate (v4 and v6 sockets of one holder).
    seen, uniq = set(), []
    for d, o in found:
        key = (re.sub(r" on \S+", "", d), o)
        if key not in seen:
            seen.add(key)
            uniq.append((d, o))
    return uniq


def _port_check(check, label, code, hosts, facts, wanted, fix):
    """wanted: [(proto, port, own_container, own_wg_interface, what)]."""
    missing = [h for h in hosts if h not in facts]
    if not hosts:
        return Result(check, label, BLOCKED, "the inventory group lists no host", "GB-130",
                      "name the host in the inventory group (the edge inventory check above says which)")
    if missing:
        return Result(check, label, FAIL, "no probe answer from " + ", ".join(missing), "GB-133",
                      "Check the host is reachable with the inventory the plays use; the probe output above\n"
                      "says why ansible could not read it.")
    collisions, own = [], []
    for h in hosts:
        if not facts[h].get("ss_ok", True):
            return Result(check, label, FAIL, f"{h}: `ss` could not list listening sockets", "GB-133",
                          "Install iproute2 on the host, or read its listening sockets by hand.")
        for proto, port, cont, iface, what in wanted:
            for desc, ours in holders(facts[h], proto, port, cont, iface):
                (own if ours else collisions).append(f"{h}: {what} {desc}")
    facts_out = {"own": own} if own else {}
    if collisions:
        return Result(check, label, FAIL, "; ".join(collisions), code, fix, facts_out)
    detail = ", ".join(f"{what} {proto}/{port} free" for proto, port, _, _, what in wanted)
    if own:
        detail += "; held by this peering's own service (a re-run): " + "; ".join(own)
    return Result(check, label, PASS, detail, facts=facts_out)


def check_edge_ports(facts, edge_hosts, wg_port, wg_interface, edge_port, edge_container, edge_group):
    wanted = [("udp", wg_port, "", wg_interface, "tunnel listen port"),
              ("tcp", edge_port, edge_container, "", "edge router listen port")]
    fix = (f"Choose ports nothing else on the edge holds (NetBird holds UDP 51820 on a mesh host):\n"
           f"pin them in the peering declaration (goatbuilder.yaml peerings: values: wg_listen_port /\n"
           f"edge_listen_port), or set peering_wg_listen_port / goat_zenoh_edge_listen_port in\n"
           f"group_vars/{edge_group}/, then run phase 1 again.")
    return _port_check("edge-ports", "edge ports free", "GB-131", edge_hosts, facts, wanted, fix)


def check_core_ports(facts, core_hosts, metrics_port, republisher_container, core_group):
    wanted = [("tcp", metrics_port, republisher_container, "", "republisher metrics port")]
    fix = (f"Choose a metrics port nothing else on the core holds: pin republisher_metrics_port in the\n"
           f"peering declaration (goatbuilder.yaml peerings: values:), or set\n"
           f"goat_fed_republisher_metrics_port in group_vars/{core_group}/. One port per peering on a\n"
           f"shared core host.")
    return _port_check("core-ports", "core metrics port free", "GB-132", core_hosts, facts, wanted, fix)


# ---------------------------------------------------------------------------
# The core router: the selection contract of goat federate init
# (crates/goat-federation/src/derive.rs select_router,
# docs/design/federation-router-discovery.md). fixtures/router-discovery.json
# holds cases both implementations are tested against.
# ---------------------------------------------------------------------------

CORE_ROLE = "zenoh-core"
ROUTER_LIST_FORMAT = '{{.ID}}\t{{.Names}}\t{{.Label "org.goat.component"}}'
ROUTER_INSPECT_FORMAT = {"docker": "{{.Id}}\t{{.State.Running}}\t{{.Config.Image}}",
                         "podman": "{{.ID}}\t{{.State.Running}}\t{{.Config.Image}}"}
_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]*$")


def container_name(v):
    return bool(v) and bool(_NAME.match(v))


def select_router(selected, tools, call):
    """call(tool, args) -> (ok, stdout_or_error). Returns ("ok", id, tool) or
    ("none"|"many"|"invalid", message, None)."""
    if selected and not container_name(selected):
        return ("invalid", "FED_ROUTER_CONTAINER must be a container name or ID, not an option or expression", None)
    found, attempts = [], []
    for tool in tools:
        if selected:
            candidates = [selected]
        else:
            ok, out = call(tool, ["ps", "--no-trunc", "--format", ROUTER_LIST_FORMAT])
            if not ok:
                attempts.append(f"{tool}: {out}")
                continue
            candidates = []
            for line in out.splitlines():
                if not line:
                    continue
                parts = line.split("\t")
                if len(parts) != 3 or not container_name(parts[0]) or not container_name(parts[1]):
                    return ("invalid", f"{tool}: could not parse the running container listing", None)
                if parts[2] == CORE_ROLE or parts[1] == "goat-zenoh" or parts[1].endswith("-zenoh-router"):
                    candidates.append(parts[0])
        for name in candidates:
            ok, out = call(tool, ["inspect", "--type", "container", "--format", ROUTER_INSPECT_FORMAT[tool], name])
            parts = out.strip().split("\t") if ok else []
            if not ok or len(parts) != 3 or not container_name(parts[0]) or parts[1] != "true" or not parts[2]:
                attempts.append(f"{tool}: " + (out if not ok else f"container {name}: no complete running-container observation"))
                continue
            if not any(f[0] == parts[0] for f in found):
                found.append((parts[0], tool))
    if len(found) == 1:
        return ("ok", found[0][0], found[0][1])
    if not found:
        return ("none", "no running core router found; discovery accepts org.goat.component=zenoh-core, "
                "standard *-zenoh-router names and legacy goat-zenoh. Attempts: " + " | ".join(attempts), None)
    return ("many", "multiple running core routers found (" + ", ".join(f"{t}:{i}" for i, t in found) + ")", None)


def docker_caller(docker_host, runner=None):
    runner = runner or run
    env = dict(os.environ)
    if docker_host:
        env["DOCKER_HOST"] = docker_host

    def call(tool, args):
        rc, out, err = runner([tool] + list(args), env=env)
        if rc != 0:
            return False, (text(err).strip() or f"{tool} exited {rc}")
        return True, text(out)
    return call


def router_tools(docker_host):
    # An explicit daemon is Docker only: an unreachable remote core is never
    # replaced by a local Podman observation (derive.rs router_tools).
    return ["docker"] if docker_host else ["docker", "podman"]


def check_core_router(docker_host, selected, call):
    verdict, what, tool = select_router(selected, router_tools(docker_host), call)
    where = f"DOCKER_HOST={docker_host}" if docker_host else "the local container runtime"
    if verdict == "ok":
        return Result("core-router", "core router found", PASS, f"{tool} container {what[:12]} on {where}",
                      facts={"id": what, "tool": tool})
    if verdict == "many":
        return Result("core-router", "core router found", FAIL, what, "GB-139",
                      "Set FED_ROUTER_CONTAINER to the core router's name or ID.")
    return Result("core-router", "core router found", FAIL, f"{what} (looked on {where})", "GB-138",
                  "Phase 3 reads the core router the same way. Point FED_ROUTER_DOCKER_HOST at the core host\n"
                  "(ssh://<user>@<core>; goatbuilder derives it from the inventory group <peering>_core),\n"
                  "or set FED_ROUTER_CONTAINER to its name or ID.")


# ---------------------------------------------------------------------------
# Receive stores (docs/design/federation-ingress-storage-preflight.md)
# ---------------------------------------------------------------------------

def _ingress_filter(repo):
    path = Path(repo) / "ops/ansible/filter_plugins/zenoh_ingress.py"
    spec = importlib.util.spec_from_file_location("goat_preflight_zenoh_ingress", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.zenoh_ingress_storage_check


def required_ingress_scopes(repo, edge_group, counterparty, work):
    """What the edge admits from them, and where that came from: the rendered
    edge vars when apply has run (deployed first, then this run's), else the
    spec's one rule, `from/<their sovereign_id>/**` (compose publish_under)."""
    name = f"federation-{counterparty}.yml"
    for p in [Path(repo) / "ops/ansible/group_vars" / edge_group / name,
              Path(work) / "applied/ops/ansible/group_vars" / edge_group / name]:
        v = _plain_yaml_value(p, "goat_zenoh_edge_ingress_key_exprs")
        if isinstance(v, list) and v:
            return [str(x) for x in v], str(p)
    return [f"from/{counterparty}/**"], "the spec's wire name for the counterparty (from/<them>/**)"


def check_ingress_stores(repo, receives, required, source, router, call, config_path):
    """router: (tool, id) or None."""
    label = "core receive stores"
    if not receives:
        return Result("ingress-stores", label, NA, "the declaration accepts nothing ([data] accept_* absent)")
    if router is None:
        return Result("ingress-stores", label, BLOCKED, "no core router to read (see the core router check)", "GB-138")
    tool, rid = router
    ok, out = call(tool, ["exec", rid, "cat", config_path])
    if not ok:
        # Never echo the read: a core config can hold credentials.
        return Result("ingress-stores", label, SKIP,
                      f"the core configuration at {config_path} could not be read; receive scopes NOT verified",
                      "GB-137",
                      "Set FED_CORE_CONFIG_PATH to where the core router reads its configuration, or\n"
                      "inspect the core's storages through its own approved workflow.")
    report = _ingress_filter(repo)(out, required)
    if report["error"]:
        return Result("ingress-stores", label, FAIL, report["error"], "GB-136",
                      "Fix the core's storage_manager configuration so it can be inspected.")
    if report["missing"]:
        wide = [s for s in report["observed"] if s.endswith("/**") and any(
            r.startswith(s[:-2]) and r != s for r in report["missing"])]
        detail = "no store for " + ", ".join(report["missing"]) + f" (required by {source})"
        if wide:
            detail += "; the wider " + ", ".join(wide) + " does not count: the edge refuses a subscriber wider than its grant"
        return Result("ingress-stores", label, FAIL, detail, "GB-136",
                      "Add a receive store on the core whose key_expr equals each admitted scope (the goat-zenoh\n"
                      "role renders one per partner); keep a historical from/** store beside it if you need its data.",
                      {"observed": report["observed"], "missing": report["missing"]})
    return Result("ingress-stores", label, PASS, "a store matches each admitted scope: " + ", ".join(required))


# ---------------------------------------------------------------------------
# Slot anchors (ops/scripts/sync-slot-anchors.py --check)
# ---------------------------------------------------------------------------

def slot_anchor_file(repo, env):
    return Path(repo) / "ops/ansible/group_vars" / f"{env.replace('-', '_')}_hosts" / "zenoh-lazy-storage.yml"


def slot_check_command(repo, env, registry):
    """registry: {store, db, pg_ssh, pg_container, pg_db, pg_user} (empty = not given)."""
    argv = ["python3", str(Path(repo) / "ops/scripts/sync-slot-anchors.py"), "--env", env, "--check"]
    store = registry.get("store") or "sqlite"
    if store == "postgres":
        argv += ["--registry-store", "postgres"]
        for flag, key in (("--pg-ssh", "pg_ssh"), ("--pg-container", "pg_container"),
                          ("--pg-db", "pg_db"), ("--pg-user", "pg_user")):
            if registry.get(key):
                argv += [flag, registry[key]]
    else:
        argv += ["--db", registry.get("db", "")]
    return argv


def check_slot_anchors(repo, env, registry, runner=None):
    """The committed slot list against the registry. Callable on its own: the
    phase-10 restart step calls it again just before the restart
    (`preflight.py <peering> --work <dir> --only slot-anchors`)."""
    runner = runner or run
    label = "slot list equals registry"
    if not env:
        return Result("slot-anchors", label, BLOCKED, "no environment to read the slot list of", "GB-130",
                      "the peering's environment (goatbuilder.yaml peerings: environment), or $FED_SLOT_ENV")
    f = slot_anchor_file(repo, env)
    if not f.is_file():
        return Result("slot-anchors", label, NA, f"no {f.relative_to(repo)}: this deployment declares no lazy slot storage")
    store = registry.get("store") or "sqlite"
    if store == "sqlite" and not registry.get("db"):
        return Result("slot-anchors", label, BLOCKED, "the slot registry is not named", "GB-130",
                      "FED_SLOT_DB (the portal registry.db), or FED_SLOT_REGISTRY_STORE=postgres with\n"
                      "FED_SLOT_PG_SSH (group_vars: portal_store, portal_db_* are read for the rest)")
    rc, out, err = runner(slot_check_command(repo, env, registry), timeout=180)
    said = (text(out) + text(err)).strip()
    if rc == 0:
        return Result("slot-anchors", label, PASS, said.splitlines()[0] if said else "in sync")
    if rc == 1:
        return Result("slot-anchors", label, FAIL, " / ".join(said.splitlines()[:8]), "GB-134",
                      f"Regenerate the list and commit it before the core router restarts:\n"
                      f"  {' '.join(a for a in slot_check_command(repo, env, registry) if a != '--check')}\n"
                      f"(the same command without --check writes it). Slots minted between this check and the\n"
                      f"restart drift it again; the restart step runs this check once more.")
    return Result("slot-anchors", label, FAIL, " / ".join(said.splitlines()[-3:]) or f"exit {rc}", "GB-135",
                  "The registry could not be read, which is not a verdict on drift: check the registry path or\n"
                  "the management host named by FED_SLOT_PG_SSH.")


# ---------------------------------------------------------------------------
# Our CA and its signing key (what phase 8 mints with)
# ---------------------------------------------------------------------------

def _pubkey_of_cert(cert, runner):
    rc, out, _ = runner(["openssl", "x509", "-in", str(cert), "-noout", "-pubkey"])
    return text(out).strip() if rc == 0 else None


def _pubkey_of_key(key_bytes, runner):
    rc, out, _ = runner(["openssl", "pkey", "-pubout"], input_bytes=key_bytes)
    return text(out).strip() if rc == 0 else None


def check_signing_ca(ca_cert, ca_key, ca_key_age, age_identity, runner=None):
    runner = runner or run
    label = "signing CA (phase 8)"
    if not ca_cert:
        return Result("signing-ca", label, BLOCKED, "no CA certificate named", "GB-130",
                      "FED_CA_CERT, or goat_federation_ca_cert in the edge group's vars (goatbuilder derives\n"
                      "it from ops/ansible/enrollment/ca/<site>/root-cert.pem, the portal's own CA)")
    if not Path(ca_cert).is_file():
        return Result("signing-ca", label, FAIL, f"{ca_cert} does not exist", "GB-140",
                      "Name our own CA certificate (never the counterparty's).")
    rc, out, _ = runner(["openssl", "x509", "-in", str(ca_cert), "-noout", "-ext", "basicConstraints"])
    if rc != 0:
        return Result("signing-ca", label, FAIL, f"{ca_cert} is not a PEM certificate openssl can read", "GB-140",
                      "Name our own CA certificate (PEM).")
    if "CA:TRUE" not in text(out).replace(" ", ""):
        return Result("signing-ca", label, FAIL, f"{ca_cert} is not a CA certificate (basicConstraints CA:FALSE or absent)",
                      "GB-140", "Name the CA that issues our leaves, not a leaf.")
    cert_pub = _pubkey_of_cert(ca_cert, runner)
    if ca_key:
        if not Path(ca_key).is_file():
            return Result("signing-ca", label, FAIL, f"{ca_key} does not exist", "GB-141",
                          "FED_CA_KEY names our CA's private key (PEM).")
        key_pub = _pubkey_of_key(Path(ca_key).read_bytes(), runner)
        how = ca_key
    elif ca_key_age:
        if not Path(ca_key_age).is_file():
            return Result("signing-ca", label, FAIL, f"{ca_key_age} does not exist", "GB-141",
                          "The deployment keeps its CA key at <deploy.state_dir>/<site>-ca/ca.key.age; restore it\n"
                          "there, or name it with goat_federation_ca_key_age / FED_CA_KEY_AGE.")
        if not age_identity or not Path(age_identity).is_file():
            return Result("signing-ca", label, FAIL, f"no age identity at {age_identity or '(unset)'} to open {ca_key_age}",
                          "GB-141", "FED_AGE_IDENTITY, or goat_federation_age_identity (the portal role reads\n"
                          "~/.config/age/keys.txt).")
        rc, key_bytes, err = runner(["age", "-d", "-i", str(age_identity), str(ca_key_age)])
        if rc != 0:
            return Result("signing-ca", label, FAIL, f"{ca_key_age} did not decrypt with {age_identity}: "
                          f"{text(err).strip()[:200]}", "GB-141",
                          "Use the age identity the key was encrypted to.")
        # In memory only: the plaintext key is piped to openssl and dropped.
        key_pub = _pubkey_of_key(key_bytes, runner)
        key_bytes = None
        how = f"{ca_key_age} (age)"
    else:
        return Result("signing-ca", label, BLOCKED, "no CA signing key named", "GB-130",
                      "FED_CA_KEY (PEM) or FED_CA_KEY_AGE (the deployment's ca.key.age, derived from\n"
                      "deploy.state_dir and the peering's site)")
    if not key_pub:
        return Result("signing-ca", label, FAIL, f"{how} is not a private key openssl can read", "GB-141",
                      "Name our CA's private key.")
    if cert_pub is None or key_pub != cert_pub:
        return Result("signing-ca", label, FAIL, f"{how} does not match {ca_cert}", "GB-141",
                      "The key and the certificate must be one CA.")
    return Result("signing-ca", label, PASS, f"{ca_cert}, key {how}")
