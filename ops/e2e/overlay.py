#!/usr/bin/env python3
"""overlay.py — turn a fresh clone of this repository into the hermetic set's source of record.

The hermetic tier (docs/design/deployment-e2e.md §Hermetic) runs the REAL drivers
on a clone whose declarations name three hermetic goatnets and the canary
components, and nothing live. This script writes that overlay into the clone:

  ops/pins/fleet-pins.yaml           the goatnets e2e_dev / e2e_preprod / e2e_prod
                                     and the components e2e-canary, e2e-canary-secret
  ops/release/ladder.yaml            the same three rungs, soak min_days 0
  ops/ansible/group_vars/<member>/   vars.yml per hermetic member group
  ops/ansible/inventory/mgmt-hosts.ini   the hermetic hosts, appended (ssh, the harness key)
  ops/validation/goatnet-runners.yaml    runner rows for the three goatnets (no live rows run)
  ops/validation/flow-map.yaml       one flow mapping the canary to a browser row, so the
                                     release driver's browser-toolchain preflight is exercised
  ops/ansible/roles/e2e-canary/deploy-history.yml  the hermetic hosts' connection map
  ops/consistency/intent/            the real goatnets' intents REMOVED; the hermetic ones
                                     generated and signed with a throwaway CA (ca-init)
  ops/goatbuilder/goatbuilder.yaml   regenerated from the overlaid sources

Then it commits on main and pushes to the clone's origin (the harness's bare
remote). Every write is to the clone; the repository this script lives in is
untouched. Usage:

  overlay.py <clone> --home <HOME> --ssh-key <private key>
             --host e2e_dev=<ip> --host e2e_preprod=<ip> --host e2e_prod=<ip>
             --canary-ref ghcr.io/{{ ghcr_org }}/e2e-canary:v0.0.0@sha256:…
             [--no-hosts]   (no inventory hosts: dry-run and refusal cases only)
             [--release-ref <owner>/<repo>@<tag>@sha256:…]   (a release component,
                             e2e-release, pinned there: ops/e2e/release-file.sh)
             [--public-component <name>]   (that component of this clone's own table,
                             kept in the hermetic one: ops/e2e/runner-release.sh)
             [--opt-in <goatnet>=<component>]   (the hermetic goatnet lists that
                             opt-in component under `opt_in:`)
             [--member-vars <member>=<file>]   (<file> copied into that member's
                             group_vars under its own name)
             [--ssh-user <user>]   (the hosts' login, goat by default; --host takes
                             <goatnet>=<ip> or <goatnet>=<ip>:<ssh port>)
  overlay.py <clone> --home <HOME> --repin e2e-release=<ref>
             the overlaid clone's component moved to another pin: rendered,
             signed, committed and pushed as a merged pin change would be
"""
import argparse
import os
import shutil
import subprocess
import sys

try:
    import yaml
except ImportError:
    sys.exit("overlay.py: PyYAML is required (pip install pyyaml)")


def read_text(path):
    with open(path, encoding="utf-8") as fh:
        return fh.read()

GOATNETS = ["e2e_dev", "e2e_preprod", "e2e_prod"]
RUNG = {"dev": "e2e_dev", "preprod": "e2e_preprod", "prod": "e2e_prod"}
CA_ID = "dev-e2e-ca"


def sh(args, cwd, env=None, check=True):
    r = subprocess.run(args, cwd=cwd, env=env, text=True, capture_output=True)
    if check and r.returncode != 0:
        sys.exit(f"overlay.py: {' '.join(args)} failed in {cwd}:\n{r.stdout}\n{r.stderr}")
    return r


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text)


RELEASE = "e2e-release"


def release_component(ref):
    """goatbuilder's own published releases, taken as a release component (gate J23)."""
    return {
        "var": "e2e_release_file",
        "artifact_type": "release",
        "fleet": ref,
        "release": {
            "checksums": "SHA256SUMS",
            "files": {"amd64": "goatbuilder-linux-amd64", "arm64": "goatbuilder-linux-arm64"},
            "signer": "https://github.com/%s/.github/workflows/_release-binary.yml" % ref.split("@")[0],
        },
    }


def fleet_pins(canary_ref, secret_ref, release_ref=None, extra=None, opt_in=None):
    members = {g: [f"{g}_hosts"] for g in GOATNETS}
    goatnets = {}
    for g in GOATNETS:
        goatnets[g] = {
            "description": f"hermetic goatnet {g} — the deployment e2e harness's {g.split('_')[1]} rung; one host, nothing live",
            "members": members[g],
            "ca_id_var": "e2e_ca_id",
        }
        if (opt_in or {}).get(g):
            goatnets[g]["opt_in"] = opt_in[g]
    components = {
        "e2e-canary": {
            "var": "e2e_canary_image",
            "image_workflow": ".github/workflows/e2e-canary-image.yml",
            "ledger_component": "e2e-canary",
            "fleet": canary_ref,
        },
        "e2e-canary-secret": {
            "var": "e2e_canary_secret_image",
            # Named so rc.sh derives the image name; the tag is the base commit
            # (sha-<short>) so affected-flows.py can diff from it, and the flow map
            # below maps this canary to a browser row: the release driver's
            # toolchain preflight is exercised on it, never on e2e-canary.
            "image_workflow": ".github/workflows/e2e-canary-secret-image.yml",
            "deploy_playbook": "e2e-canary-secret-deploy.yml",
            "ledger_component": "e2e-canary-secret",
            "fleet": secret_ref,
        },
    }
    if release_ref:
        components[RELEASE] = release_component(release_ref)
    for name, spec in (extra or {}).items():
        components[name] = spec
    header = ("# ops/pins/fleet-pins.yaml — HERMETIC overlay written by ops/e2e/overlay.py.\n"
              "# Three hermetic goatnets, two canary components, nothing live\n"
              "# (docs/design/deployment-e2e.md). The real table is on main.\n")
    return header + yaml.safe_dump({"goatnets": goatnets, "components": components}, sort_keys=False, width=120)


def ladder(clone):
    p = os.path.join(clone, "ops/release/ladder.yaml")
    d = yaml.safe_load(read_text(p)) or {}
    if not d.get("rungs"):
        # A checkout that declares no deployment ships an empty ladder; the
        # hermetic one is the three-rung ladder with the gates a deployment
        # declares (docs/design/release-graduation.md).
        d["rungs"] = [{"name": n, "goatnet": g, "status": "live", "role": f"hermetic {n} rung"} for n, g in RUNG.items()]
        d["hops"] = [
            {"from": "dev", "to": "preprod", "status": "live",
             "gates": [{"id": "candidate-record"}, {"id": "validation-evidence"}]},
            {"from": "preprod", "to": "prod", "status": "live",
             "gates": [{"id": "candidate-record"}, {"id": "soak", "min_days": 1}, {"id": "convergence-evidence"},
                       {"id": "validation-evidence"}, {"id": "operator-ack"}]},
        ]
    for r in d.get("rungs") or []:
        if r.get("name") in RUNG:
            r["goatnet"] = RUNG[r["name"]]
            r["status"] = "live"
    # The soak stays as declared (min_days 1 — goatbuilder's schema refuses zero,
    # by design); the harness lets it elapse by dating the candidate record back
    # one day before `--from prod`, a declared step in the run record.
    write(p, "# HERMETIC overlay (ops/e2e/overlay.py): the ladder's rungs point at the\n"
             "# hermetic goatnets; the soak gate is the real one.\n"
             + yaml.safe_dump(d, sort_keys=False, width=120))


def group_vars(clone):
    for g in GOATNETS:
        member = f"{g}_hosts"
        write(os.path.join(clone, "ops/ansible/group_vars", member, "vars.yml"),
              "---\n"
              f"# HERMETIC member group {member} (ops/e2e/overlay.py).\n"
              f"e2e_ca_id: \"{CA_ID}\"\n"
              "ghcr_org: \"usareur-af\"\n"
              "e2e_canary_secret_token: \"{{ lookup('env', 'E2E_CANARY_TOKEN') }}\"\n")


def inventory(clone, hosts, ssh_key, ssh_user="goat"):
    p = os.path.join(clone, "ops/ansible/inventory/mgmt-hosts.ini")
    lines = ["", "# --- HERMETIC hosts appended by ops/e2e/overlay.py (never on main) ---"]
    for g in GOATNETS:
        lines.append(f"[{g}_hosts]")
        ip = hosts.get(g)
        if ip:
            ip, _, port = ip.partition(":")
            at = f" ansible_port={port}" if port else ""
            lines.append(f"e2e-{g.split('_')[1]}-01 ansible_host={ip}{at} ansible_user={ssh_user} ansible_python_interpreter=/usr/bin/python3 "
                         f"ansible_ssh_private_key_file={ssh_key} ansible_ssh_common_args=\"-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null\" "
                         f"router_role=A site_name={g.replace('_', '-')}")
        lines.append("")
    with open(p, "a", encoding="utf-8") as fh:
        fh.write("\n".join(lines) + "\n")


def runners(clone):
    p = os.path.join(clone, "ops/validation/goatnet-runners.yaml")
    d = yaml.safe_load(read_text(p))
    if not d.get("goatnets"):   # a checkout that declares no goatnet lists none
        d["goatnets"] = {}
    for g in GOATNETS:
        d["goatnets"][g] = {"name": g.replace("_", "-"), "goat_via": "", "intake_resolve": "",
                                           "values": "ops/validation/values-zagros-dev.yaml"}
    write(p, "# HERMETIC overlay (ops/e2e/overlay.py): runner rows for the hermetic goatnets.\n" + yaml.safe_dump(d, sort_keys=False, width=120))


def flow_map(clone):
    p = os.path.join(clone, "ops/validation/flow-map.yaml")
    d = yaml.safe_load(read_text(p))
    if not d.get("flows"):
        d["flows"] = []
    d["flows"].append({
        "id": "e2e-canary-secret",
        "what": "the secret-bearing hermetic canary; mapped to a browser row so the release driver's toolchain preflight is exercised",
        "paths": ["ops/e2e/canary/**"],
        "cheap": [],
        "live": ["intake-form-browser-submit"],
    })
    write(p, yaml.safe_dump(d, sort_keys=False, width=120))
    # Nothing on main touches ops/e2e/canary/**: the harness tags the secret
    # canary's candidate on a side commit that does (hermetic.sh), so only that
    # candidate is "affected" and every e2e-canary candidate maps to no live row.


def deploy_history(clone, hosts):
    rows = []
    for g in GOATNETS:
        ip = hosts.get(g)
        if not ip:
            continue
        name = f"e2e-{g.split('_')[1]}-01"
        rows.append({"host": ip, "ssh_user": "goat", "container_name": f"e2e-canary-{name}",
                     "compose_path": "/opt/e2e-canary/docker-compose.yml", "site": g.replace("_", "-")})
    write(os.path.join(clone, "ops/ansible/roles/e2e-canary/deploy-history.yml"),
          "# HERMETIC connection map (ops/e2e/overlay.py).\n" + yaml.safe_dump({"service": "e2e-canary", "hosts": rows, "history": []}, sort_keys=False))


def state_dir(clone, env):
    """Where rc.sh looks for the CA keys under the harness's HOME: the clone's
    deploy.state_dir (ops/goatbuilder/project.yml, which gen-config.py carries
    into goatbuilder.yaml), else ~/.goat/state, the builder's rule."""
    r = sh([sys.executable, "ops/scripts/goatnet-facts.py", "deploy", "state_dir"], cwd=clone, env=env, check=False)
    return r.stdout.strip() if r.returncode == 0 and r.stdout.strip() else os.path.join(env["HOME"], ".goat/state")


def ca(clone, home, env):
    """A throwaway dev CA: ca-init writes the age-encrypted key under $HOME the way rc.sh expects it, and the SPKI into the clone."""
    ident = os.path.join(home, ".config/age/keys.txt")
    os.makedirs(os.path.dirname(ident), exist_ok=True)
    if not os.path.exists(ident):
        sh(["age-keygen", "-o", ident], cwd=clone, env=env)
    pub = [l.split(":", 1)[1].strip() for l in read_text(ident).splitlines(True) if l.lower().startswith("# public key")][0]
    keydir = os.path.join(state_dir(clone, env), "e2e-ca")
    os.makedirs(keydir, exist_ok=True)
    key = os.path.join(keydir, "ca.key.age")
    pubdir = os.path.join(clone, "ops/enrollment/public-keys")
    os.makedirs(pubdir, exist_ok=True)   # a checkout that declares no goatnet carries no CA public keys
    pem = os.path.join(pubdir, f"{CA_ID}.pem")
    if os.path.exists(pem):
        os.remove(pem)
    sh(["go", "run", "./cmd/ca-init", "--mode", "dev", "--out", key, "--recipients", pub, "--allow-single-recipient", "--id", "e2e-ca",
        "--public-key-dir", os.path.join(clone, "ops/enrollment/public-keys")], cwd=os.path.join(clone, "ops/enrollment"), env=env)
    return key, ident


def intents(clone, home, env, key, ident):
    idir = os.path.join(clone, "ops/consistency/intent")
    os.makedirs(idir, exist_ok=True)   # none on a checkout that declares no goatnet
    for d in os.listdir(idir):
        full = os.path.join(idir, d)
        if os.path.isdir(full) and d not in GOATNETS:
            shutil.rmtree(full)
    # The release records and graduation sets of the real components would fail
    # graduate.py check (C2/C7: unknown component) against the hermetic table;
    # the hermetic ladder starts with no history.
    for sub in ("records", "sets"):
        d = os.path.join(clone, "ops/release", sub)
        if os.path.isdir(d):
            for entry in os.listdir(d):
                full = os.path.join(d, entry)
                if os.path.isdir(full):
                    shutil.rmtree(full)
                elif entry.endswith((".yaml", ".yml")):
                    os.remove(full)
            write(os.path.join(d, ".gitkeep"), "")
    # The unmigrated-pins allowlist names rows of the real goatnets, which the
    # hermetic table no longer lists; an empty list keeps the ratchet honest here
    # (0 rows, baseline untouched) without a stale row failing the check.
    up = os.path.join(clone, "ops/pins/unmigrated-pins.txt")
    # A checkout that declares no goatnet carries no allowlist; the hermetic one
    # starts empty at baseline 0.
    header = [l for l in read_text(up).splitlines(True) if l.startswith("#")] if os.path.exists(up) else ["# baseline: 0\n"]
    write(up, "".join(header) + "# HERMETIC overlay (ops/e2e/overlay.py): the hermetic goatnets carry no unmigrated pin.\n")
    sh([sys.executable, "ops/pins/generate-pins.py"], cwd=clone, env=env)
    for g in GOATNETS:
        intent = os.path.join(idir, g, "intent.json")
        sh(["go", "run", "./cmd/intent-sign", "--intent", intent, "--root-key", key, "--age-identity", ident],
           cwd=os.path.join(clone, "ops/enrollment"), env=env)
    sh([sys.executable, "ops/pins/generate-pins.py", "--check"], cwd=clone, env=env)
    sh([sys.executable, "ops/release/graduate.py", "check"], cwd=clone, env=env)
    sh([sys.executable, "ops/goatbuilder/gen-config.py"], cwd=clone, env=env)


def commit(clone, env, message):
    sh(["git", "add", "-A"], cwd=clone, env=env)
    sh(["git", "-c", "user.name=e2e harness", "-c", "user.email=e2e@localhost", "commit", "-q", "-s", "-m", message], cwd=clone, env=env)
    sh(["git", "push", "-q", "origin", "main"], cwd=clone, env=env)


def repin(clone, env, home, spec):
    comp, ref = spec.split("=", 1)
    p = os.path.join(clone, "ops/pins/fleet-pins.yaml")
    d = yaml.safe_load(read_text(p))
    d["components"][comp]["fleet"] = ref
    write(p, "# HERMETIC overlay (ops/e2e/overlay.py), re-pinned.\n" + yaml.safe_dump(d, sort_keys=False, width=200))
    key = os.path.join(state_dir(clone, env), "e2e-ca/ca.key.age")
    intents(clone, home, env, key, os.path.join(home, ".config/age/keys.txt"))
    commit(clone, env, "e2e: %s moves to another pin" % comp)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("clone")
    ap.add_argument("--home", required=True)
    ap.add_argument("--ssh-key")
    ap.add_argument("--host", action="append", default=[], help="goatnet=ip")
    ap.add_argument("--canary-ref")
    ap.add_argument("--release-ref")
    ap.add_argument("--public-component", action="append", default=[], help="a component of the clone's table, kept")
    ap.add_argument("--opt-in", action="append", default=[], help="goatnet=component: listed under the goatnet's opt_in:")
    ap.add_argument("--member-vars", action="append", default=[], help="member=file: copied into its group_vars")
    ap.add_argument("--ssh-user", default="goat")
    ap.add_argument("--repin", help="component=ref, on a clone already overlaid")
    ap.add_argument("--no-hosts", action="store_true")
    a = ap.parse_args()
    clone = os.path.abspath(a.clone)
    hosts = {} if a.no_hosts else dict(h.split("=", 1) for h in a.host)
    env = dict(os.environ, HOME=a.home)
    env.setdefault("GOMODCACHE", os.path.join(os.environ.get("HOME", a.home), "go/pkg/mod"))
    env.setdefault("GOCACHE", os.path.join(os.environ.get("HOME", a.home), "Library/Caches/go-build")
                   if sys.platform == "darwin" else os.path.join(os.environ.get("HOME", a.home), ".cache/go-build"))
    if a.repin:
        repin(clone, env, a.home, a.repin)
        return
    if not a.ssh_key or not a.canary_ref:
        ap.error("--ssh-key and --canary-ref are required")
    # The secret canary's fleet tag is the commit under test (sha-<short>), the
    # base affected-flows.py diffs from; its digest is the canary's (the role pulls nothing).
    base = sh(["git", "rev-parse", "--short=7", "HEAD"], cwd=clone, env=env).stdout.strip()
    secret_ref = "ghcr.io/{{ ghcr_org }}/e2e-canary-secret:sha-%s@%s" % (base, a.canary_ref.split("@")[-1])
    public = (yaml.safe_load(read_text(os.path.join(clone, "ops/pins/fleet-pins.yaml"))) or {}).get("components") or {}
    extra = {}
    for name in a.public_component:
        if name not in public:
            ap.error("--public-component %s: the clone's table has no such component" % name)
        extra[name] = public[name]
    opt_in = {}
    for o in a.opt_in:
        g, name = o.split("=", 1)
        if g not in GOATNETS:
            ap.error("--opt-in %s: not a hermetic goatnet" % g)
        opt_in.setdefault(g, []).append(name)
    write(os.path.join(clone, "ops/pins/fleet-pins.yaml"), fleet_pins(a.canary_ref, secret_ref, a.release_ref, extra, opt_in))
    ladder(clone)
    group_vars(clone)
    for m in a.member_vars:
        member, path = m.split("=", 1)
        if member not in ["%s_hosts" % g for g in GOATNETS]:
            ap.error("--member-vars %s: not a hermetic member group" % member)
        write(os.path.join(clone, "ops/ansible/group_vars", member, os.path.basename(path)), read_text(path))
    inventory(clone, hosts, a.ssh_key, a.ssh_user)
    runners(clone)
    flow_map(clone)
    deploy_history(clone, hosts)
    key, ident = ca(clone, a.home, env)
    intents(clone, a.home, env, key, ident)
    commit(clone, env, "e2e: hermetic overlay — three goatnets, two canaries, a throwaway CA\n\n[track: deploy-e2e-harness]")
    print(f"overlay: {clone} is the hermetic source of record ({', '.join(GOATNETS)}; CA {CA_ID}; key {key})")


if __name__ == "__main__":
    main()
