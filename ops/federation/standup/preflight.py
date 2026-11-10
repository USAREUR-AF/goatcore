#!/usr/bin/env python3
"""Phase 1 of a peering standup: refuse before anything is measured or deployed.

WHAT THIS CHECKS, AND WHAT IT DELIBERATELY DOES NOT. `goat federate init` already
measures the running system and refuses, by name, for anything it cannot derive
(exit 23). Duplicating those probes here would be two homes for one fact and they
would drift. This checks what init cannot: the inputs that must exist BEFORE
init is worth running, and the deployment surface that phases 8-12 will need long
after init has passed -- the ports the tunnel, the edge router and the
republisher will bind, the core router and its receive stores, the committed slot
list, and the CA phase 8 signs with. Every failure the first driven runs hit late
is checked here first (docs/design/federation-preflight.md).

Fail-closed: every missing thing is named with where to declare it. Nothing is
defaulted that belongs to a deployment, and nothing on a host is changed: the
live reads are a read-only play and `docker ps|inspect|exec cat`.

Every check prints one outcome word (PASS, FAIL, BLOCKED, SKIP, NOT APPLICABLE)
and a GB-NNN code from ops/errors/catalog.yaml when it does not pass. A skipped
check is reported as skipped, never as passed. The results are also written as
JSON (schema goat-federation-preflight/v1) to <work>/preflight/result.json.

Inputs come from the environment. goatbuilder federate derives them from the
deployment (goatbuilder.yaml peerings:, the inventory, group_vars, the allocated
peering values) and exports each one the operator has not set; the script avenue
sets them by hand. An explicitly set variable always wins.

  --list                 print the checks and exit
  --only ID[,ID...]      run only these checks (ids from --list); the phase-10
                         restart step runs `--only slot-anchors` just before the
                         core router restarts
  FED_PREFLIGHT_NO_LIVE=1  skip the live reads (each reported SKIP)
  FED_PREFLIGHT_STRICT=1   a SKIP refuses too

Exit 0 ready, 1 not ready.
"""
import argparse, json, os, re, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import preflight_live as live  # noqa: E402
from preflight_live import BLOCKED, FAIL, NA, PASS, SKIP, Result  # noqa: E402

MARK = {PASS: "[✓]", FAIL: "[✗]", BLOCKED: "[!]", SKIP: "[-]", NA: "[ ]"}
def _probe(argv, cwd, env):
    import subprocess
    try:
        p = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, timeout=600)
    except (OSError, subprocess.SubprocessError) as exc:
        return 127, b"", str(exc).encode()
    return p.returncode, p.stdout, p.stderr


PROBE = _probe  # the read-only port probe; tests replace it
problems = []   # labels of every FAIL and BLOCKED, in order
results = []    # every Result, in order


def record(r):
    results.append(r)
    word = "" if r.outcome in (PASS, FAIL) else f"{r.outcome}: "
    code = f" ({r.code})" if r.code and r.outcome != PASS else ""
    print(f"{MARK[r.outcome]} {r.label} — {word}{r.detail}{code}" if r.detail else f"{MARK[r.outcome]} {r.label}{code}")
    if r.fix and r.outcome != PASS:
        for line in r.fix.strip().splitlines():
            print(f"     {line}")
    if r.outcome in (FAIL, BLOCKED):
        problems.append(r.label)


def ok(label, detail="", check=""):
    record(Result(check or label, label, PASS, detail))


def bad(label, detail, fix="", code="", check=""):
    record(Result(check or label, label, FAIL, detail, code, fix))


def inventory_groups(text):
    """Every group of an ini inventory and its host names, `:children` sections apart."""
    groups, children, cur, kids = {}, {}, None, False
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith(("#", ";")):
            continue
        if line.startswith("["):
            name = line[1:-1].strip()
            kids = name.endswith(":children")
            cur = name[: -len(":children")] if kids else name
            (children if kids else groups).setdefault(cur, [])
            continue
        if cur is not None:
            (children if kids else groups)[cur].append(line.split()[0])
    return groups, children


def shared_inventory_names(text, mine):
    """Host names in one of `mine` (this peering's edge and core groups) that also sit in
    another peering group. Ansible merges the group_vars of every group an inventory host
    is in, the group sorting last winning a conflict, so such a host's run reads the other
    peering's interface, ports and certificate paths. Returns [(host, [groups])]."""
    groups, children = inventory_groups(text)
    peering = set(children.get("peering_edges", [])) | set(children.get("peering_cores", []))
    found = []
    for g in mine:
        for h in groups.get(g, []):
            # The same peering's own edge and core groups naming one host is one
            # peering's variables, not two: not a conflict.
            others = sorted(o for o in peering if o not in mine and h in groups.get(o, []))
            if others and (h, [g] + others) not in found:
                found.append((h, [g] + others))
    return found


def check_local_docker():
    """Refuse an unverified phase-12 dependency before any deployment."""
    import shutil
    import subprocess

    label = "docker compose plugin (phase 12)"
    if not shutil.which("docker"):
        bad(label, "no docker on this host", code="GB-146")
        return
    try:
        result = subprocess.run(["docker", "compose", "version"],
                                capture_output=True, text=True, timeout=15)
    except (OSError, subprocess.SubprocessError) as exc:
        bad(label, str(exc), code="GB-146")
        return
    if result.returncode != 0:
        bad(label, "docker has no compose plugin",
            "The edge role deploys with `docker compose`. Install\n"
            "docker-compose-plugin, or drop the plugin binary at\n"
            "/usr/libexec/docker/cli-plugins/docker-compose.", code="GB-146")
    elif not result.stdout.strip():
        bad(label, "docker compose version returned no version", code="GB-146")
    else:
        ok(label, result.stdout.strip().splitlines()[0])


def group_hosts(inv_text, group):
    """Hosts listed directly under [group]."""
    want, hosts = False, []
    for line in inv_text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("["):
            want = line[1:-1].strip() == group
            continue
        if want:
            hosts.append(line.split()[0])
    return hosts


def group_children(inv_text, parent):
    children, inside = [], False
    for line in inv_text.splitlines():
        line = line.strip()
        if line.startswith("["):
            inside = line == f"[{parent}:children]"
        elif inside and line and not line.startswith("#"):
            children.append(line.split()[0])
    return children


def peering_environment(repo, peering):
    """The declared environment of this peering (goatbuilder.yaml peerings:)."""
    try:
        import yaml
        cfg = yaml.safe_load((Path(repo) / "ops/goatbuilder/goatbuilder.yaml").read_text()) or {}
    except Exception:  # noqa: BLE001
        return ""
    for p in cfg.get("peerings") or []:
        if isinstance(p, dict) and p.get("with") == peering:
            return str(p.get("environment") or "")
    return ""


def list_checks():
    print("phase-1 checks (--only takes the ids in the second group):")
    for name in ("declaration", "profile certificate", "CA root bundle", "signing key", "peering directory",
                 "working directory", "edge inventory", "republisher inventory", "one inventory name per peering", "declared",
                 "core certificate SANs (phase 8)", "docker compose plugin (phase 12)"):
        print(f"  always  {name}")
    for cid, label, code in live.CHECKS:
        print(f"  --only  {cid:<15} {label} ({code})")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("peering")
    ap.add_argument("--work", required=True)
    # Under `sudo -E` the invoking user's HOME survives, so Path.home() is
    # /home/<user> while the profile the run will actually read is root's. That
    # mismatch made the first driven run report three missing inputs that were
    # all present. Follow the effective user instead.
    ap.add_argument("--profile-dir", default=os.environ.get("GOAT_PROFILE_DIR", ""))
    ap.add_argument("--context", default=os.environ.get("GOAT_CONTEXT", "default"))
    ap.add_argument("--edge-group", default="")
    ap.add_argument("--core-group", default=os.environ.get("FED_CORE_GROUP", ""))
    ap.add_argument("--repo", default=str(Path(__file__).resolve().parents[3]))
    ap.add_argument("--only", default="")
    ap.add_argument("--list", action="store_true")
    # Where the results go; the restart recheck writes its own file so it never
    # reads, or overwrites, the full phase-1 result.
    ap.add_argument("--result", default="")
    a = ap.parse_args(argv)
    if a.list:
        return list_checks()

    env = os.environ
    only = {x.strip() for x in a.only.split(",") if x.strip()}
    unknown = only - {c for c, _, _ in live.CHECKS}
    if unknown:
        print(f"preflight: unknown check id(s) {', '.join(sorted(unknown))}; --list names them", file=sys.stderr)
        return 2

    def want(cid):
        return not only or cid in only

    static = not only
    no_live = env.get("FED_PREFLIGHT_NO_LIVE", "") == "1"
    profile_dir = a.profile_dir or ("/root/.goat" if os.geteuid() == 0
                                    else str(Path.home() / ".goat"))
    print(f"preflight — peering {a.peering}" + (f" (only {', '.join(sorted(only))})" if only else ""))
    ctx = Path(profile_dir) / "contexts" / a.context
    group = a.edge_group or f"{a.peering}_edge"
    core = a.core_group or f"{a.peering}_core"
    counterparty = env.get("FED_COUNTERPARTY_ID", "") or a.peering
    inv = Path(a.repo) / "ops/ansible/inventory/peering-edges.ini"
    inv_text = inv.read_text() if inv.is_file() else ""
    decl = ctx / "federation.toml"
    decl_text = decl.read_text() if decl.is_file() else ""
    missing = []   # (input, where to declare it)

    m = re.search(r'^\s*release\s*=\s*\[([^\]]*)\]', decl_text, re.M)
    offers = bool(m and m.group(1).strip().strip(","))
    receives = bool(re.search(r'^\s*accept_(hz|bytes_per_message|distinct_keys)\s*=', decl_text, re.M))

    if static:
        # 1 — the declaration. init reads it and refuses without it, but it refuses
        #     AFTER printing a template, which reads like an error rather than a
        #     first step. Saying it here, first, is the difference.
        if not decl.is_file():
            bad("declaration", f"no federation.toml at {decl}",
                "It holds only what no probe can produce: the organisation's name, the term,\n"
                "the cut contact, and the link parameters two parties agree rather than one\n"
                "observes. `goat federate init` prints a template when it is absent.", "GB-144")
        else:
            m = re.search(r'^\s*sovereign_id\s*=\s*"([^"]+)"', decl_text, re.M)
            if not m:
                bad("declaration", f"{decl} has no sovereign_id", code="GB-144")
            else:
                sid = m.group(1)
                if not (2 <= len(sid) <= 16 and re.fullmatch(r"[a-z0-9-]+", sid)):
                    bad("sovereign_id", f"`{sid}` is not 2-16 chars of lowercase a-z0-9-", code="GB-144")
                else:
                    ok("declaration", f"sovereign_id `{sid}`")
                    # ADVISORY, not a refusal: the shape is legal but the VALUE may
                    # be an environment name, which the spec warns against because a
                    # sandbox and a production fabric share one sovereign id.
                    for e in ("dev", "prod", "test", "sandbox", "staging", "lab"):
                        if sid == e or sid.endswith("-" + e):
                            print(f"     note: `{sid}` looks like an ENVIRONMENT name. A sovereign id is the")
                            print("     organisation, and a sandbox and a production fabric share one.")
                            break
            # A capability typed into the declaration is a claim; init refuses to
            # read one, but the operator finds out only after a full run.
            for leaf in ("transports", "fabric_version", "fabric_fips_image", "pqc_capable"):
                if re.search(rf'^\s*{re.escape(leaf)}\s*=', decl_text, re.M):
                    bad("declaration", f"`{leaf}` is MEASURED and must not be declared",
                        "A capability typed by an operator is a claim, and a wrong claim stays\n"
                        "wrong until the link fails in front of a partner.", "GB-144")

        # 2 — profile identity. init derives trust_domain and ca_fingerprint from
        #     these two files; without them phase 3 cannot start.
        cert, roots = ctx / "mtls.cert.pem", ctx / "ca-roots.pem"
        if not cert.is_file():
            bad("profile certificate", f"none at {cert}", "Run `goat profile init <bundle.cbor>` first.", "GB-144")
        elif b"-----BEGIN CERTIFICATE-----" not in cert.read_bytes():
            bad("profile certificate", f"{cert} is not a PEM certificate", code="GB-144")
        else:
            ok("profile certificate", str(cert))
        if not roots.is_file():
            bad("CA root bundle", f"none at {roots}", "Run `goat profile init <bundle.cbor>` first.", "GB-144")
        else:
            ok("CA root bundle", str(roots))

        # 3 — our signing key. Without it init prints the bytes to sign and stops,
        #     which is correct behaviour and a confusing way to discover the gap.
        key = env.get("FED_SIGNING_KEY", "")
        if not key:
            missing.append(("FED_SIGNING_KEY", f"goat_federation_signing_key in group_vars/{group}/ (a path to our "
                            "Ed25519 key: 64 hex or 44 base64), or export it"))
        elif Path(key).is_file():
            raw = Path(key).read_text().strip()
            if not (re.fullmatch(r"[0-9a-fA-F]{64}", raw) or len(raw) == 44):
                bad("signing key", f"{key} is neither 64 hex characters nor 44 base64", code="GB-142")
            else:
                ok("signing key", key)
        else:
            bad("signing key", f"FED_SIGNING_KEY points at {key}, which does not exist", code="GB-142")

        # 3b — the peering directory. Phase 5 exchanges packages and CA certificates
        #     through it. FED_NO_DIRECTORY=1 is the one exception: a counterparty that
        #     can reach no directory, where phase 5 names the files to hand over.
        if env.get("FED_NO_DIRECTORY", "") == "1":
            ok("peering directory", "FED_NO_DIRECTORY=1: phase 5 names the files to hand over directly")
        else:
            url = env.get("FED_DIRECTORY_URL", "")
            plain = re.compile(r"^[A-Za-z0-9._:/?=&%~+@-]*$")
            values = [url, env.get("FED_COUNTERPARTY_ID", ""), env.get("FED_PACKAGE_URL", ""), env.get("FED_CA_CERT", "")]
            if not url:
                missing.append(("FED_DIRECTORY_URL", f"goat_federation_directory_url in group_vars/{group}/ (https://"
                                "<directory>), or goat_federation_no_directory: true for a counterparty that can "
                                "reach none"))
            elif not url.startswith("https://"):
                bad("peering directory", f"FED_DIRECTORY_URL is {url}, not an https:// address", code="GB-147")
            elif not env.get("FED_COUNTERPARTY_ID", ""):
                missing.append(("FED_COUNTERPARTY_ID", "the counterparty's sovereign_id; goatbuilder derives it "
                                "from the peering's `with`"))
            elif not all(plain.match(v) for v in values):
                bad("peering directory", "FED_DIRECTORY_URL, FED_PACKAGE_URL, FED_COUNTERPARTY_ID or "
                    "FED_CA_CERT holds characters outside a plain URL", code="GB-147")
            else:
                ok("peering directory", f"{url}, counterparty {env['FED_COUNTERPARTY_ID']}")

        # 4 — somewhere to work.
        work = Path(a.work)
        try:
            work.mkdir(parents=True, exist_ok=True)
            (work / ".w").write_text("x"); (work / ".w").unlink()
            ok("working directory", str(work))
        except OSError as e:
            bad("working directory", f"{work} is not writable: {e}", code="GB-148")

        # 5 — the deployment surface phases 11-13 need. Checked NOW because it is
        #     free to check and expensive to discover at phase 11.
        if not inv.is_file():
            bad("edge inventory", f"no {inv}", code="GB-143")
        elif f"[{group}]" not in inv_text:
            bad("edge inventory", f"{inv} has no [{group}] group",
                f"Phases 2 and 11-13 deploy with --limit {group}; a missing group deploys to nothing\n"
                f"and reports success.", "GB-143")
        else:
            # A group that EXISTS and is empty is the worse case: `--limit` matches
            # it, ansible runs against nothing, and the play reports success.
            hosts = group_hosts(inv_text, group)
            if hosts:
                ok("edge inventory", f"[{group}] -> {', '.join(hosts)}")
                # Every peering play runs `hosts: peering_edges`. A group that is not a
                # child of it matches `--limit` and none of the plays' hosts.
                if group in group_children(inv_text, "peering_edges"):
                    ok("edge inventory", f"[{group}] is a child of [peering_edges]")
                else:
                    bad("edge inventory", f"[{group}] is not listed under [peering_edges:children] in {inv}",
                        "Every peering play targets peering_edges, so `--limit` on this group\n"
                        "runs nothing and reports success.", "GB-143")
            else:
                bad("edge inventory", f"[{group}] exists but lists no hosts",
                    "`--limit` on an empty group runs against nothing and reports SUCCESS,\n"
                    "so phase 12 would 'pass' having deployed no edge at all.", "GB-143")

        # 5b — where the outbound republisher runs, when the declaration offers a
        #      release set: our core host, --limit <peering>_core, under
        #      [peering_cores:children].
        if offers and inv.is_file():
            hosts = group_hosts(inv_text, core)
            if not hosts:
                bad("republisher inventory", f"{inv} has no [{core}] group with a host in it",
                    f"The declaration offers a release set, so phase 12 deploys the outbound\n"
                    f"republisher on our core host with --limit {core}. Name the core host the\n"
                    f"edge dials there; an empty or missing group deploys nothing and reports success.", "GB-143")
            elif core not in group_children(inv_text, "peering_cores"):
                bad("republisher inventory", f"[{core}] is not listed under [peering_cores:children] in {inv}",
                    "peering-republisher.yml targets peering_cores, so `--limit` on this group\n"
                    "runs nothing and reports success.", "GB-143")
            else:
                ok("republisher inventory", f"[{core}] -> {', '.join(hosts)}")

        # 5c — one inventory name per peering. Two peerings on one machine are two inventory
        #      names for it (same ansible_host): one name in two peering groups merges both
        #      groups' variables, and a run for one peering reads the other's values.
        if inv.is_file():
            shared = shared_inventory_names(inv_text, [group, core])
            for h, gs in shared:
                bad("one inventory name per peering", f"{h} is in {', '.join(gs)}",
                    "Ansible merges the group_vars of every group a host is in and the group that\n"
                    "sorts last wins, so this peering's run would read another peering's interface,\n"
                    f"ports and certificate paths, and re-running either redeploys the other's values.\n"
                    f"Give this peering its own inventory name for the machine, e.g. {h}-{a.peering},\n"
                    "with the same ansible_host.", "GB-143")
            if not shared:
                ok("one inventory name per peering", "no host of this peering is in another peering's group")

        # C1 — declared first.
        cfg = Path(a.repo) / "ops/goatbuilder/goatbuilder.yaml"
        if cfg.is_file():
            names = re.findall(r'^\s*-?\s*with:\s*["\']?([^"\'\s#]+)', cfg.read_text(), re.M)
            if a.peering in names:
                ok("declared", f"`{a.peering}` is in {cfg} peerings:")
            else:
                bad("declared", f"`{a.peering}` is not in {cfg} peerings: (declared: {', '.join(names) or 'none'})",
                    "Declare the peering first (C1). A peering nobody declared is one nobody reviewed.", "GB-149")

    # ---- the inputs the live checks and phases 8-12 need ------------------------
    ca_cert, ca_key = env.get("FED_CA_CERT", ""), env.get("FED_CA_KEY", "")
    ca_key_age, age_id = env.get("FED_CA_KEY_AGE", ""), env.get("FED_AGE_IDENTITY", "")
    if want("signing-ca"):
        if not ca_cert:
            missing.append(("FED_CA_CERT", f"goat_federation_ca_cert in group_vars/{group}/; goatbuilder derives "
                            "ops/ansible/enrollment/ca/<site>/root-cert.pem"))
        if not ca_key and not ca_key_age:
            missing.append(("FED_CA_KEY or FED_CA_KEY_AGE", "goat_federation_ca_key (PEM) or goat_federation_ca_key_age "
                            "in group_vars; goatbuilder derives <deploy.state_dir>/<site>-ca/ca.key.age"))

    ports = live.resolve_port_inputs(a.repo, group, core, env)
    need_ports = [n for n in ("FED_WG_INTERFACE", "FED_WG_LISTEN_PORT", "FED_EDGE_LISTEN_PORT")] if want("edge-ports") else []
    if want("core-ports") and offers:
        need_ports.append("FED_REPUBLISHER_METRICS_PORT")
    unusable = []  # inputs a live port check cannot run on
    for n in need_ports:
        v, where = ports[n]
        if v is None:
            missing.append((n, where))
            unusable.append(n)
        elif n != "FED_WG_INTERFACE" and not (v.isdigit() and 0 < int(v) < 65536):
            missing.append((n, f"{v} (from {where}) is not a port number from 1 to 65535"))
            unusable.append(n)
        elif n == "FED_WG_INTERFACE" and not live.container_name(v):
            missing.append((n, f"{v} (from {where}) is not an interface name"))
            unusable.append(n)

    slot_env = env.get("FED_SLOT_ENV", "") or peering_environment(a.repo, a.peering)
    hosts_group = f"{slot_env.replace('-', '_')}_hosts" if slot_env else ""
    gv = (lambda k: live.group_var(a.repo, hosts_group, k)) if hosts_group else (lambda k: None)
    registry = {
        "store": env.get("FED_SLOT_REGISTRY_STORE", "") or str(gv("portal_store") or ""),
        "db": env.get("FED_SLOT_DB", ""),
        "pg_ssh": env.get("FED_SLOT_PG_SSH", ""),
        "pg_container": env.get("FED_SLOT_PG_CONTAINER", "") or str(gv("portal_db_container") or ""),
        "pg_db": env.get("FED_SLOT_PG_DB", "") or str(gv("portal_db_name") or ""),
        "pg_user": env.get("FED_SLOT_PG_USER", "") or str(gv("portal_db_user") or ""),
    }
    slot_applicable = bool(slot_env) and live.slot_anchor_file(a.repo, slot_env).is_file()
    if want("slot-anchors") and slot_applicable:
        if (registry["store"] or "sqlite") == "sqlite" and not registry["db"]:
            missing.append(("FED_SLOT_DB", f"the portal registry.db of {slot_env} (or FED_SLOT_REGISTRY_STORE=postgres "
                            f"with FED_SLOT_PG_SSH; group_vars/{hosts_group}: portal_store says which)"))
        if registry["store"] == "postgres" and not registry["pg_ssh"]:
            missing.append(("FED_SLOT_PG_SSH", f"the management host that runs {slot_env}'s portal database"))

    # The core certificate must carry a SAN for every address the edge will dial.
    endpoints = []
    dep = Path(a.repo) / "ops/ansible/group_vars" / group / "deployment.yml"
    if dep.is_file():
        for line in dep.read_text().splitlines():
            m = re.search(r'"(?:tls|tcp)/([^:"/#]+):', line)
            if m:
                endpoints.append(m.group(1))
    san = env.get("FED_CORE_SAN", "")
    if static and endpoints and not san:
        missing.append(("FED_CORE_SAN", f"the addresses the edge dials ({', '.join(endpoints)}); goatbuilder derives "
                        f"them from group_vars/{group}/deployment.yml"))

    if want("inputs"):
        record(live.check_inputs(missing))

    # ---- live reads --------------------------------------------------------------
    def skipped(cid, label):
        record(Result(cid, label, SKIP, "FED_PREFLIGHT_NO_LIVE=1: not checked"))

    edge_hosts = group_hosts(inv_text, group)
    core_hosts = group_hosts(inv_text, core)
    port_checks = [c for c in ("edge-ports", "core-ports") if want(c)]
    core_na = "core-ports" in port_checks and not offers
    if core_na:
        port_checks.remove("core-ports")
    if port_checks and no_live:
        for c in port_checks:
            skipped(c, "edge ports free" if c == "edge-ports" else "core metrics port free")
    elif port_checks:
        blocked = [n for n in need_ports if n in unusable]
        if blocked:
            for c in port_checks:
                record(Result(c, "edge ports free" if c == "edge-ports" else "core metrics port free", BLOCKED,
                              "no usable " + ", ".join(blocked) + " (see required inputs)", "GB-130"))
        elif not inv.is_file():
            for c in port_checks:
                record(Result(c, "edge ports free" if c == "edge-ports" else "core metrics port free", BLOCKED,
                              f"no inventory at {inv}", "GB-143"))
        else:
            out_dir = Path(a.work) / "preflight" / "probe"
            out_dir.mkdir(parents=True, exist_ok=True)
            for old in out_dir.glob("*.json"):
                old.unlink()  # a previous run's answer is not this run's evidence
            groups = [group] + ([core] if "core-ports" in port_checks else [])
            argv = live.probe_command(a.repo, "inventory/peering-edges.ini", ":".join(groups), out_dir,
                                      env.get("GOAT_AWS_PROFILE", "") or "default")
            print(f"     probing {', '.join(groups)} read-only: ansible-playbook ... peering-preflight.yml")
            rc, out, err = PROBE(argv, cwd=str(Path(a.repo) / "ops/ansible"),
                                 env=dict(env, OBJC_DISABLE_INITIALIZE_FORK_SAFETY="YES"))
            facts = live.load_probe(out_dir)
            if rc != 0 and not facts:
                tail = (live.text(out) + live.text(err)).strip().splitlines()[-3:]
                print("     probe: " + " / ".join(tail))
            if "edge-ports" in port_checks:
                bind, _ = live.edge_bind_address(a.repo, a.work, group, env)
                record(live.check_edge_ports(
                    facts, edge_hosts, int(ports["FED_WG_LISTEN_PORT"][0]), ports["FED_WG_INTERFACE"][0],
                    int(ports["FED_EDGE_LISTEN_PORT"][0]),
                    str(live.group_var(a.repo, group, "goat_zenoh_edge_stack") or f"goat-zenoh-edge-{counterparty}"),
                    group, bind))
            if "core-ports" in port_checks:
                record(live.check_core_ports(
                    facts, core_hosts, int(ports["FED_REPUBLISHER_METRICS_PORT"][0]),
                    str(live.group_var(a.repo, core, "goat_fed_republisher_stack") or f"goat-fed-republisher-{counterparty}"),
                    core))

    if core_na:
        record(Result("core-ports", "core metrics port free", NA,
                      "the declaration offers no release set, so no republisher runs on the core"))

    if want("slot-anchors"):
        if no_live and slot_applicable:
            skipped("slot-anchors", "slot list equals registry")
        else:
            record(live.check_slot_anchors(a.repo, slot_env, registry))

    router = None
    docker_host = live.effective_docker_host(env.get("FED_ROUTER_DOCKER_HOST", ""), env)
    call = live.docker_caller(docker_host)
    if want("core-router") or want("ingress-stores"):
        if no_live:
            if want("core-router"):
                skipped("core-router", "core router found")
        else:
            r = live.check_core_router(docker_host, env.get("FED_ROUTER_CONTAINER", ""), call)
            if want("core-router"):
                record(r)
            if r.outcome == PASS:
                router = (r.facts["tool"], r.facts["id"])
    if want("ingress-stores"):
        if no_live and receives:
            skipped("ingress-stores", "core receive stores")
        else:
            required, source = live.required_ingress_scopes(a.repo, group, counterparty, a.work)
            record(live.check_ingress_stores(a.repo, receives, required, source, router, call,
                                             env.get("FED_CORE_CONFIG_PATH", "") or "/etc/zenoh/config.json5",
                                             counterparty))

    if want("signing-ca"):
        record(live.check_signing_ca(ca_cert, ca_key, ca_key_age, age_id))

    if static:
        if endpoints and san:
            not_covered = [e for e in endpoints if e not in san.split(",")]
            if not_covered:
                bad("core certificate SANs (phase 8)", f"FED_CORE_SAN does not cover: {', '.join(not_covered)}",
                    "The edge dials those; the core certificate must name them.", "GB-145")
            else:
                ok("core certificate SANs (phase 8)", san)
        # Phase 12 deploys with `docker compose`, and the module's error when the
        # plugin is absent arrives only after the config has been written.
        if env.get("FED_SKIP_LOCAL_DOCKER_CHECK", "") != "1":
            check_local_docker()

    skips = [r for r in results if r.outcome == SKIP]
    strict = env.get("FED_PREFLIGHT_STRICT", "") == "1"
    ready = not problems and not (strict and skips)
    result_path = Path(a.result) if a.result else Path(a.work) / "preflight" / "result.json"
    try:
        result_path.parent.mkdir(parents=True, exist_ok=True)
        result_path.write_text(json.dumps({
            "schema": live.SCHEMA, "peering": a.peering, "only": sorted(only), "ready": ready,
            "results": [r.as_dict() for r in results]}, indent=2) + "\n")
    except OSError as e:
        print(f"[✗] result file — {result_path} could not be written: {e}")
        ready = False
        problems.append("result file")
    print()
    if not ready:
        n = len(problems) + (len(skips) if strict else 0)
        print(f"NOT READY — {n} thing(s) above. Nothing was measured, signed or deployed.")
        return 1
    if skips:
        print(f"READY, with {len(skips)} check(s) SKIPPED and not verified: "
              + ", ".join(r.label for r in skips) + ". FED_PREFLIGHT_STRICT=1 refuses on a skip.")
        return 0
    print("READY — phase 1 can measure this host." if not only else "READY — the checks above hold.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
