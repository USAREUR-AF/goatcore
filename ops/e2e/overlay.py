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


def fleet_pins(canary_ref, secret_ref):
    members = {g: [f"{g}_hosts"] for g in GOATNETS}
    goatnets = {}
    for g in GOATNETS:
        goatnets[g] = {
            "description": f"hermetic goatnet {g} — the deployment e2e harness's {g.split('_')[1]} rung; one host, nothing live",
            "members": members[g],
            "ca_id_var": "e2e_ca_id",
        }
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
    header = ("# ops/pins/fleet-pins.yaml — HERMETIC overlay written by ops/e2e/overlay.py.\n"
              "# Three hermetic goatnets, two canary components, nothing live\n"
              "# (docs/design/deployment-e2e.md). The real table is on main.\n")
    return header + yaml.safe_dump({"goatnets": goatnets, "components": components}, sort_keys=False, width=120)


def ladder(clone):
    p = os.path.join(clone, "ops/release/ladder.yaml")
    d = yaml.safe_load(read_text(p))
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


def inventory(clone, hosts, ssh_key):
    p = os.path.join(clone, "ops/ansible/inventory/mgmt-hosts.ini")
    lines = ["", "# --- HERMETIC hosts appended by ops/e2e/overlay.py (never on main) ---"]
    for g in GOATNETS:
        lines.append(f"[{g}_hosts]")
        ip = hosts.get(g)
        if ip:
            lines.append(f"e2e-{g.split('_')[1]}-01 ansible_host={ip} ansible_user=goat ansible_python_interpreter=/usr/bin/python3 "
                         f"ansible_ssh_private_key_file={ssh_key} ansible_ssh_common_args=\"-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null\" "
                         f"router_role=A site_name={g.replace('_', '-')}")
        lines.append("")
    with open(p, "a", encoding="utf-8") as fh:
        fh.write("\n".join(lines) + "\n")


def runners(clone):
    p = os.path.join(clone, "ops/validation/goatnet-runners.yaml")
    d = yaml.safe_load(read_text(p))
    for g in GOATNETS:
        d.setdefault("goatnets", {})[g] = {"name": g.replace("_", "-"), "goat_via": "", "intake_resolve": "",
                                           "values": "ops/validation/values-zagros-dev.yaml"}
    write(p, "# HERMETIC overlay (ops/e2e/overlay.py): runner rows for the hermetic goatnets.\n" + yaml.safe_dump(d, sort_keys=False, width=120))


def flow_map(clone):
    p = os.path.join(clone, "ops/validation/flow-map.yaml")
    d = yaml.safe_load(read_text(p))
    d.setdefault("flows", []).append({
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
    pem = os.path.join(clone, "ops/enrollment/public-keys", f"{CA_ID}.pem")
    if os.path.exists(pem):
        os.remove(pem)
    sh(["go", "run", "./cmd/ca-init", "--mode", "dev", "--out", key, "--recipients", pub, "--allow-single-recipient", "--id", "e2e-ca",
        "--public-key-dir", os.path.join(clone, "ops/enrollment/public-keys")], cwd=os.path.join(clone, "ops/enrollment"), env=env)
    return key, ident


def intents(clone, home, env, key, ident):
    idir = os.path.join(clone, "ops/consistency/intent")
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
    header = [l for l in read_text(up).splitlines(True) if l.startswith("#")]
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


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("clone")
    ap.add_argument("--home", required=True)
    ap.add_argument("--ssh-key", required=True)
    ap.add_argument("--host", action="append", default=[], help="goatnet=ip")
    ap.add_argument("--canary-ref", required=True)
    ap.add_argument("--no-hosts", action="store_true")
    a = ap.parse_args()
    clone = os.path.abspath(a.clone)
    hosts = {} if a.no_hosts else dict(h.split("=", 1) for h in a.host)
    env = dict(os.environ, HOME=a.home)
    env.setdefault("GOMODCACHE", os.path.join(os.environ.get("HOME", a.home), "go/pkg/mod"))
    env.setdefault("GOCACHE", os.path.join(os.environ.get("HOME", a.home), "Library/Caches/go-build")
                   if sys.platform == "darwin" else os.path.join(os.environ.get("HOME", a.home), ".cache/go-build"))
    # The secret canary's fleet tag is the commit under test (sha-<short>), the
    # base affected-flows.py diffs from; its digest is the canary's (the role pulls nothing).
    base = sh(["git", "rev-parse", "--short=7", "HEAD"], cwd=clone, env=env).stdout.strip()
    secret_ref = "ghcr.io/{{ ghcr_org }}/e2e-canary-secret:sha-%s@%s" % (base, a.canary_ref.split("@")[-1])
    write(os.path.join(clone, "ops/pins/fleet-pins.yaml"), fleet_pins(a.canary_ref, secret_ref))
    ladder(clone)
    group_vars(clone)
    inventory(clone, hosts, a.ssh_key)
    runners(clone)
    flow_map(clone)
    deploy_history(clone, hosts)
    key, ident = ca(clone, a.home, env)
    intents(clone, a.home, env, key, ident)
    commit(clone, env, "e2e: hermetic overlay — three goatnets, two canaries, a throwaway CA\n\n[track: deploy-e2e-harness]")
    print(f"overlay: {clone} is the hermetic source of record ({', '.join(GOATNETS)}; CA {CA_ID}; key {key})")


if __name__ == "__main__":
    main()
