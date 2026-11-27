#!/usr/bin/env python3
"""Standup phase 11: mint the goatnet's bootstrap NetBird identity (ADR 1274).

usage: standup-bootstrap-token.py <site> [--group <group>] [--host <user@address>]

The bootstrap identity (`netbird-infra` in ops/netbird/service-identities.yaml)
is the admin service user whose token phase 12 uses to mint every other NetBird
identity. This makes it, on a founder site, with the fleet management image's
own `service-user` commands, and stores its token under the declared secrets
posture before the token is valid:

  1. the site declares goatnet_founder: true (GB-435);
  2. the posture keeps a store copy: env-file, sops, git-crypt, or 1password
     (GB-436 otherwise; phase 11 stays manual there);
  3. a stored token that authenticates as the service user: nothing to do. A
     stored one that does not, or a token row with no stored copy: GB-437;
  4. the running management-a is the fleet pin, the image that has the
     commands (GB-438);
  5. `service-user bootstrap-user` makes the service user and a PENDING token in
     /run/goat-bootstrap on the host (a one-shot container of management-a: the
     same image, environment and data volume as the server);
  6. the token is read over ssh into this process, stored under the posture
     and read back from where it was written; anything but an equal read-back
     stops here with nothing switched on (GB-439), and a re-run resumes;
  7. `service-user bootstrap-token-install` switches it on, given the read-back
     value on stdin.

The token is held only in this process's memory and in the pipes to and from
ssh. It is never printed, never an argument or an environment variable, and
never in a file this script writes (under a value store it is in the posture's
own file; under 1password only a reference is on disk, ADR 1186).

Exit: 0 minted or already in place; 1 refused (the message starts with its GB
code) or a step failed; 2 usage.
"""
import argparse
import importlib.util
import json
import re
import subprocess
import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    sys.exit("standup-bootstrap-token: needs PyYAML (pip3 install pyyaml)")

REPO = Path(__file__).resolve().parents[2]
BOOT_DIR = "/run/goat-bootstrap"
CONTAINER = "management-a"
STORE_COPY = ("env-file", "sops", "git-crypt", "1password")
TOKEN_SHAPE = re.compile(r"^nbp_[A-Za-z0-9]{36}$")


class Refused(Exception):
    """A refusal with its catalog code; the message never holds a token."""

    def __init__(self, code, msg):
        super().__init__(f"{code}: {msg}")
        self.code = code


def load_identities():
    path = REPO / "ops" / "scripts" / "netbird-service-identities.py"
    spec = importlib.util.spec_from_file_location("netbird_service_identities", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def say(msg):
    print(f"standup-bootstrap-token: {msg}", flush=True)


def fleet_digest():
    """The sha256 digest the fleet pins netbird-mgmt at."""
    doc = yaml.safe_load((REPO / "ops" / "pins" / "fleet-pins.yaml").read_text()) or {}
    for section in doc.values():
        if isinstance(section, dict) and isinstance(section.get("netbird-mgmt"), dict):
            fleet = str(section["netbird-mgmt"].get("fleet") or "")
            m = re.search(r"@(sha256:[0-9a-f]{64})$", fleet)
            if m:
                return m.group(1)
    raise Refused("GB-438", "ops/pins/fleet-pins.yaml pins no netbird-mgmt fleet image by digest, so the "
                            "running management image cannot be checked")


def first_host(group):
    """user@address of the group's first host, the one that holds the NetBird
    database (as mesh-enroll-host.yml mints there)."""
    out = subprocess.run([sys.executable, str(REPO / "ops" / "scripts" / "inventory-hosts.py"), group, "--ssh-target"],
                         capture_output=True, text=True)
    hosts = [h for h in out.stdout.split() if h]
    if out.returncode != 0 or not hosts:
        raise RuntimeError(f"no host in the inventory for {group}")
    return hosts[0]


class Host:
    """Commands on the management host, over ssh. Every command is built here
    from fixed words and checked names; none carries a token. stdin is the only
    way a token is sent, and stdout the only way one comes back."""

    def __init__(self, target):
        self.target = target

    def run(self, remote, stdin=None):
        return subprocess.run(["ssh", "-o", "BatchMode=yes", "-T", self.target, remote],
                              input=stdin, capture_output=True, text=True, timeout=600)

    def check(self, remote, what):
        r = self.run(remote)
        if r.returncode != 0:
            first = (r.stderr.strip().splitlines() or ["no output"])[0]
            raise RuntimeError(f"{what} on {self.target} failed (exit {r.returncode}): {first}")
        return r.stdout


def inspect(host, fmt):
    return host.check(f"sudo docker inspect {CONTAINER} --format '{fmt}'", f"docker inspect {CONTAINER}").strip()


def check_image(host, want):
    image = inspect(host, "{{.Image}}")
    digests = host.check(f"sudo docker image inspect {image} --format '{{{{json .RepoDigests}}}}'", "docker image inspect").strip()
    try:
        repo_digests = json.loads(digests or "[]") or []
    except ValueError:
        repo_digests = []
    if not any(d.endswith("@" + want) for d in repo_digests):
        raise Refused("GB-438", f"the running {CONTAINER} on {host.target} is not the fleet's netbird-mgmt "
                                f"({want[:19]}...): only that image has the service-user commands. Deploy the "
                                f"pinned image (standup phase 5) and run phase 11 again, or take the manual route "
                                f"(docs/operations/goatnet-standup.md, Phase 11)")


def compose_run(host):
    """The one-shot form of management-a, from the running container's own
    compose labels: the same project, files, image, environment and volumes."""
    workdir = inspect(host, '{{index .Config.Labels "com.docker.compose.project.working_dir"}}')
    files = inspect(host, '{{index .Config.Labels "com.docker.compose.project.config_files"}}')
    if not workdir or not files:
        raise RuntimeError(f"{CONTAINER} on {host.target} carries no compose labels; it was not started by compose")
    for part in [workdir] + files.split(","):
        if not re.match(r"^/[A-Za-z0-9._/-]+$", part):
            raise RuntimeError(f"{CONTAINER}'s compose label holds an unexpected path: {part!r}")
    fs = " ".join(f"-f {f}" for f in files.split(","))
    return (f"sudo docker compose --project-directory {workdir} {fs} run --rm --no-deps -T "
            f"-v {BOOT_DIR}:{BOOT_DIR} {CONTAINER} service-user")


def status_of(stdout):
    lines = [ln for ln in stdout.strip().splitlines() if ln.startswith("{")]
    if not lines:
        raise RuntimeError("the service-user command printed no status")
    return json.loads(lines[-1])


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("site")
    ap.add_argument("--group", help="group_vars directory (default <site>_hosts)")
    ap.add_argument("--host", help="user@address of the host that holds the NetBird database (default: the group's first)")
    a = ap.parse_args(argv)
    ids = load_identities()
    group = a.group or a.site.replace("-", "_") + "_hosts"
    try:
        return mint(ids, a.site, group, a.host)
    except Refused as e:
        print(str(e), file=sys.stderr)
        return 1
    except (RuntimeError, ValueError, subprocess.TimeoutExpired) as e:
        print(f"standup-bootstrap-token: {e}", file=sys.stderr)
        return 1


def mint(ids, site, group, host_target):
    gvars = ids.load_group(group)
    # 1. Founder only, declared: a joiner's users and tokens replicate from the
    # founder, so a mint there would add a second admin identity.
    if gvars.get("goatnet_founder") is not True:
        raise Refused("GB-435", f"group_vars/{group} does not declare goatnet_founder: true "
                                f"(it is {gvars.get('goatnet_founder', 'unset')!r}); only a founder site's standup "
                                f"mints the bootstrap identity, and a joining site takes the founder's")
    # 2. A posture that keeps a store copy.
    store = ids.secrets_store()
    if store not in STORE_COPY:
        raise Refused("GB-436", f"the declared secrets posture is {store}, which keeps no store copy of a value "
                                f"this script could read back; phase 11 stays manual under it "
                                f"(docs/operations/goatnet-standup.md, Phase 11)")
    spec = ids.load_registry()
    key = next(k for k, v in spec["identities"].items() if v.get("bootstrap"))
    ident = spec["identities"][key]
    env = ids.expand(ident["token_env"], site)
    user = ids.service_user_name(key, ident, site, gvars)
    fqdn = gvars.get("public_mgmt_fqdn")
    if not fqdn:
        raise RuntimeError(f"group_vars/{group} has no public_mgmt_fqdn")
    envrc_dir = ids.primary_checkout() / ".envrc.local.d"

    # 3. Already in place? A stored value that authenticates as the user is
    # the end state. One that does not is kept aside: it may be this run's own
    # pending token, stored by an earlier run that stopped before step 7.
    stored = read_back(ids, spec, ident, site, env, store, envrc_dir)
    if stored:
        ok, detail = authenticates(ids, fqdn, site, stored, user)
        if ok:
            say(f"{env} is stored and authenticates as {user}: nothing to do")
            return 0

    if not re.match(r"^[A-Za-z0-9._@-]+$", user):
        raise RuntimeError(f"the service user name {user!r} is not one this script passes to a shell")
    host = Host(host_target or first_host(group))
    # 4. The fleet image, the one with the commands.
    check_image(host, fleet_digest())
    run = compose_run(host)
    stem = re.sub(r"[^a-z0-9_-]+", "-", user.lower())
    pending_file = f"{BOOT_DIR}/{stem}.token"

    if stored:
        # A stored value that does not authenticate is either this run's own
        # pending token (an earlier run stored it, then stopped) or something a
        # standup must not replace. Decided before anything is written.
        got = host.run(f"sudo cat {pending_file}")
        if got.returncode != 0 or got.stdout != stored:
            raise Refused("GB-437", f"{env} is stored but does not authenticate as {user}, and it is not a "
                                    f"pending token of this standup. A standup never replaces a stored token: "
                                    f"renew or recover it with the rotation (ADR 1274)")

    # 5. The service user and a pending token (or the one already pending).
    r = host.run(f"sudo install -d -m 0700 -o root -g root {BOOT_DIR} && {run} bootstrap-user --name {user} --dir {BOOT_DIR}")
    st = status_of(r.stdout) if r.stdout.strip() else {}
    if st.get("state") == "held":
        raise Refused("GB-437", f"the service user {user} already holds its token, and no stored copy that "
                                f"authenticates was found under the {store} posture. A standup never makes a "
                                f"second: renew or recover it with the rotation (ADR 1274)")
    if st.get("state") == "rotating":
        last = (r.stderr.strip().splitlines() or [""])[-1]
        raise Refused("GB-437", f"a rotation of {user} was not finished: {last}")
    if r.returncode != 0 or st.get("state") != "pending":
        last = (r.stderr.strip().splitlines() or ["no output"])[-1]
        raise RuntimeError(f"bootstrap-user on {host.target} failed (exit {r.returncode}): {last}")

    # 6. Into the store, and read back.
    got = host.run(f"sudo cat {pending_file}")
    token = got.stdout
    if got.returncode != 0 or not TOKEN_SHAPE.match(token):
        raise RuntimeError(f"the pending token could not be read on {host.target} (exit {got.returncode})")
    if stored and stored != token:
        raise RuntimeError(f"the pending token on {host.target} changed during this run; run phase 11 again")
    if not stored:
        write(ids, spec, ident, site, env, store, envrc_dir, token, key, user)
    back = read_back(ids, spec, ident, site, env, store, envrc_dir)
    if back != token:
        raise Refused("GB-439", f"{env} was written under the {store} posture but did not read back as written "
                                f"(the store kept another value, or the write did not land). Nothing was switched "
                                f"on; the pending token stays on the host for a re-run")
    del token

    # 7. Switch it on, with the read-back value.
    done = host.run(f"{run} bootstrap-token-install --name {user} --dir {BOOT_DIR}", stdin=back + "\n")
    st = status_of(done.stdout) if done.stdout.strip() else {}
    if done.returncode != 0 or st.get("state") != "installed":
        first = (done.stderr.strip().splitlines() or ["no output"])[-1]
        raise RuntimeError(f"bootstrap-token-install on {host.target} failed (exit {done.returncode}): {first}")
    say(f"minted {user}: {env} stored under the {store} posture and read back, then switched on")
    return 0


def authenticates(ids, fqdn, site, value, user):
    """(ok, detail): value authenticates through the API as the service user."""
    nb = ids.NetBird(f"https://{fqdn}", value, site=site)
    try:
        users = nb.users()
    except RuntimeError as e:
        return False, str(e)
    return ids.verify_owner(nb, value, user, users)


def read_back(ids, spec, ident, site, env, store, envrc_dir):
    """The stored value of env, from where this posture keeps it; '' if none."""
    p = ids.posture()
    if store == "1password":
        return ids.reference_values([env], envrc_dir, ids.onepassword_account(spec)).get(env, "")
    try:
        return p.values(str(REPO)).get(env, "") or ""
    except p.PostureError:
        return ""


def write(ids, spec, ident, site, env, store, envrc_dir, token, key, user):
    p = ids.posture()
    if store == "1password":
        reference = ids.item_reference(spec, ident, site)
        if not reference:
            raise Refused("GB-436", f"the registry declares no 1Password item for {key}, so there is nowhere to "
                                    f"store {env} under the 1password posture")
        ok, note = ids.store_in_1password(spec, ident, site, token)
        if not ok:
            raise Refused("GB-439", f"{env} was not stored: {note}. Nothing was switched on")
        refs = envrc_dir / "30-site-services.refs"
        try:
            ids.append_reference(refs, env, reference, key, user)
        except RuntimeError:
            pass  # declared already; the read-back decides whether it resolves to this value
        return
    try:
        result = p.set_values({env: token}, str(REPO))
    except p.PostureError as e:
        raise Refused("GB-439", f"{env} was not stored under the {store} posture: {e}. Nothing was switched on")
    if (result or {}).get(env) != "written":
        raise Refused("GB-439", f"the {store} store kept the value it already held for {env}; a standup never "
                                f"replaces one. Nothing was switched on")


if __name__ == "__main__":
    sys.exit(main())
