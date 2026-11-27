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
     (GB-436 otherwise; phase 11 stays manual there), and the store can be read;
  3. a stored token that authenticates as the service user: nothing to do. A
     stored one that does not, and is not this standup's pending token, or a
     token row with no stored copy, or an unfinished rotation: GB-437;
  4. management-a, both the running one and the one a one-shot `docker compose
     run` would start, is the fleet pin, the image that has the commands
     (GB-438);
  5. `service-user bootstrap-user` makes the service user and a PENDING token in
     /run/goat-bootstrap on the host (a one-shot container of management-a: the
     same image, environment and data volume as the server);
  6. the token is read over ssh into this process, stored under the posture
     and read back from where it was written; anything but an equal read-back
     stops here with nothing switched on (GB-439), and a re-run resumes;
  7. `service-user bootstrap-token-install` switches it on, given the read-back
     value on stdin; then the token is checked through the API.

The token is held only in this process's memory and in the pipes to and from
ssh. It is never printed, never an argument or an environment variable, and
never in a file this script writes (under a value store it is in the posture's
own file; under 1password only a reference is on disk, ADR 1186).

Exit: 0 minted or already in place; 1 refused (the message starts with its GB
code) or a step failed; 2 usage.
"""
import argparse
import contextlib
import fcntl
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
SERVICE = "management-a"
STORE_COPY = ("env-file", "sops", "git-crypt", "1password")
TOKEN_SHAPE = re.compile(r"nbp_[A-Za-z0-9]{36}")
IMAGE_ID = re.compile(r"sha256:[0-9a-f]{64}")
SAFE_PATH = re.compile(r"/[A-Za-z0-9._/-]+")
SAFE_WORD = re.compile(r"[A-Za-z0-9][A-Za-z0-9._@-]*")


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


def state_dir():
    """deploy.state_dir in ops/goatbuilder/goatbuilder.yaml, else ~/.goat/state."""
    try:
        cfg = yaml.safe_load((REPO / "ops/goatbuilder/goatbuilder.yaml").read_text()) or {}
    except (OSError, yaml.YAMLError):
        cfg = {}
    return Path(str((cfg.get("deploy") or {}).get("state_dir") or "~/.goat/state")).expanduser()


@contextlib.contextmanager
def run_lock(site):
    """One run per site from this machine: two at once could both create the
    1Password item."""
    d = state_dir()
    d.mkdir(parents=True, exist_ok=True)
    with open(d / f"standup-bootstrap-token-{site}.lock", "w") as f:
        try:
            fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError(f"another phase 11 run for {site} holds {f.name}; wait for it") from None
        yield


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
                            "management image cannot be checked")


def group_hosts(group):
    """user@address of every host of the group, in inventory order."""
    out = subprocess.run([sys.executable, str(REPO / "ops" / "scripts" / "inventory-hosts.py"), group, "--ssh-target"],
                         capture_output=True, text=True)
    if out.returncode != 0:
        raise RuntimeError(f"the inventory of {group} cannot be read")
    return [h for h in out.stdout.split() if h]


class Host:
    """Commands on the management host, over ssh. Every command is built here
    from fixed words and checked names; none carries a token. stdin is the only
    way a token is sent, and stdout the only way one comes back."""

    def __init__(self, target):
        self.target = target

    def run(self, remote, stdin=None):
        return subprocess.run(["ssh", "-o", "BatchMode=yes", "-T", "--", self.target, remote],
                              input=stdin, capture_output=True, text=True, timeout=600)

    def check(self, remote, what):
        r = self.run(remote)
        if r.returncode != 0:
            first = (r.stderr.strip().splitlines() or ["no output"])[0]
            raise RuntimeError(f"{what} on {self.target} failed (exit {r.returncode}): {first}")
        return r.stdout


def last_line(text, default="no output"):
    return (text.strip().splitlines() or [default])[-1]


class Compose:
    """management-a as compose started it: its project, files and directory,
    read from the running container's own labels."""

    def __init__(self, host):
        self.host = host
        labels = {}
        for key in ("project", "project.working_dir", "project.config_files"):
            labels[key] = host.check(f"sudo docker inspect {SERVICE} --format "
                                     f"'{{{{index .Config.Labels \"com.docker.compose.{key}\"}}}}'",
                                     f"docker inspect {SERVICE}").strip()
        if not all(labels.values()):
            raise RuntimeError(f"{SERVICE} on {host.target} carries no compose labels; it was not started by compose")
        if not SAFE_WORD.fullmatch(labels["project"]):
            raise RuntimeError(f"{SERVICE}'s compose project name is unexpected: {labels['project']!r}")
        files = labels["project.config_files"].split(",")
        for part in [labels["project.working_dir"]] + files:
            if not SAFE_PATH.fullmatch(part):
                raise RuntimeError(f"{SERVICE}'s compose label holds an unexpected path: {part!r}")
        self.base = (f"sudo docker compose -p {labels['project']} --project-directory {labels['project.working_dir']} "
                     + " ".join(f"-f {f}" for f in files))

    def check_image(self, want):
        """The running management-a and the image a one-shot would start are the
        same image, and it is the fleet pin."""
        host = self.host
        running = host.check(f"sudo docker inspect {SERVICE} --format '{{{{.Image}}}}'", f"docker inspect {SERVICE}").strip()
        if not IMAGE_ID.fullmatch(running):
            raise RuntimeError(f"{SERVICE} on {host.target} reports an unexpected image id")
        cfg = host.check(f"{self.base} config --format json", "docker compose config")
        try:
            declared = json.loads(cfg)["services"][SERVICE]["image"]
        except (ValueError, KeyError, TypeError):
            raise RuntimeError(f"docker compose config on {host.target} names no image for {SERVICE}") from None
        if not re.fullmatch(r"[A-Za-z0-9._/:@-]+", str(declared)):
            raise RuntimeError(f"the image compose declares for {SERVICE} is unexpected: {declared!r}")
        local = host.run(f"sudo docker image inspect {declared} --format '{{{{.Id}}}}'")
        same = local.returncode == 0 and local.stdout.strip() == running
        digests = host.check(f"sudo docker image inspect {running} --format '{{{{json .RepoDigests}}}}'",
                             "docker image inspect").strip()
        try:
            repo_digests = json.loads(digests or "[]") or []
        except ValueError:
            repo_digests = []
        pinned = any(str(d).endswith("@" + want) for d in repo_digests)
        if not (same and pinned):
            why = ("is not the fleet's netbird-mgmt" if not pinned else
                   "differs from the image compose would start for a one-shot (its .env names another)")
            raise Refused("GB-438", f"{SERVICE} on {host.target} {why} ({want[:19]}...): only the fleet image has "
                                    f"the service-user commands, and the one-shot must be that image. Deploy the "
                                    f"pinned image (standup phase 5) and run phase 11 again, or take the manual "
                                    f"route (docs/operations/goatnet-standup.md, Phase 11)")

    def service_user(self, verb):
        return (f"{self.base} run --rm --no-deps --pull never -T -v {BOOT_DIR}:{BOOT_DIR} "
                f"{SERVICE} service-user {verb}")


def status_of(stdout):
    lines = [ln for ln in stdout.strip().splitlines() if ln.startswith("{")]
    if not lines:
        return {}
    try:
        return json.loads(lines[-1])
    except ValueError:
        return {}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("site")
    ap.add_argument("--group", help="group_vars directory and inventory group (default <site>_hosts)")
    ap.add_argument("--host", help="user@address of the group's host that holds the NetBird database "
                                   "(default: the group's first); it must be in the group's inventory")
    a = ap.parse_args(argv)
    if not SAFE_WORD.fullmatch(a.site):
        ap.error(f"{a.site!r} is not a site name")
    group = a.group or a.site.replace("-", "_") + "_hosts"
    try:
        ids = load_identities()
        with run_lock(a.site):
            return mint(ids, a.site, group, a.host)
    except Refused as e:
        print(str(e), file=sys.stderr)
        return 1
    except (RuntimeError, ValueError, OSError, subprocess.TimeoutExpired) as e:
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
    p = ids.posture()
    try:
        store = p.store(str(REPO))
    except p.PostureError as e:
        raise RuntimeError(f"the declared secrets posture cannot be read: {e}") from None
    if store not in STORE_COPY:
        raise Refused("GB-436", f"the declared secrets posture is {store}, which keeps no store copy of a value "
                                f"this script could read back; phase 11 stays manual under it "
                                f"(docs/operations/goatnet-standup.md, Phase 11)")
    spec = ids.load_registry()
    key = next(k for k, v in spec["identities"].items() if v.get("bootstrap"))
    ident = spec["identities"][key]
    env = ids.expand(ident["token_env"], site)
    user = ids.service_user_name(key, ident, site, gvars)
    if not SAFE_WORD.fullmatch(user):
        raise RuntimeError(f"the service user name {user!r} is not one this script passes to a shell")
    fqdn = gvars.get("public_mgmt_fqdn")
    if not fqdn:
        raise RuntimeError(f"group_vars/{group} has no public_mgmt_fqdn")
    envrc_dir = ids.primary_checkout() / ".envrc.local.d"

    # 3. Already in place? A stored value that authenticates as the user is
    # the end state. One that does not may be this run's own pending token,
    # stored by an earlier run that stopped before step 7.
    stored = read_back(ids, spec, env, store, envrc_dir)
    detail = ""
    if stored:
        ok, detail = authenticates(ids, fqdn, site, stored, user)
        if ok:
            say(f"{env} is stored and authenticates as {user} ({detail}): nothing to do")
            return 0

    hosts = group_hosts(group)
    target = host_target or (hosts[0] if hosts else "")
    if target not in hosts:
        raise RuntimeError(f"{target or 'no host'} is not a host of {group} in the inventory")
    host = Host(target)
    compose = Compose(host)
    # 4. The fleet image, running and for the one-shot.
    compose.check_image(fleet_digest())
    stem = re.sub(r"[^a-z0-9_-]+", "-", user.lower())
    pending_file = f"{BOOT_DIR}/{stem}.token"

    if stored:
        # Decided before anything is written: the stored value is this run's
        # pending token, or a standup does not touch it.
        got = host.run(f"sudo cat {pending_file}")
        if got.returncode != 0 or got.stdout != stored:
            raise Refused("GB-437", f"{env} is stored but does not authenticate as {user} ({detail}), and it is not "
                                    f"a pending token of this standup. A standup never replaces a stored value. If "
                                    f"the service user holds no token (a reboot cleared the pending one from "
                                    f"{BOOT_DIR}), remove {env} from the store and run phase 11 again; otherwise "
                                    f"renew or recover it with the rotation (ADR 1274)")

    # 5. The service user and a pending token (or the one already pending).
    r = host.run(f"sudo install -d -m 0700 -o root -g root {BOOT_DIR} && "
                 f"{compose.service_user('bootstrap-user')} --name {user} --dir {BOOT_DIR}")
    st = status_of(r.stdout)
    if st.get("state") == "held":
        raise Refused("GB-437", f"the service user {user} already holds its token, and no stored copy that "
                                f"authenticates was found under the {store} posture. A standup never makes a "
                                f"second: renew or recover it with the rotation (ADR 1274)")
    if st.get("state") == "rotating":
        raise Refused("GB-437", f"a rotation of {user} was not finished: {last_line(r.stderr, '')}")
    if r.returncode != 0 or st.get("state") != "pending":
        raise RuntimeError(f"bootstrap-user on {host.target} failed (exit {r.returncode}): {last_line(r.stderr)}")

    # 6. Into the store, and read back.
    got = host.run(f"sudo cat {pending_file}")
    token = got.stdout
    if got.returncode != 0 or not TOKEN_SHAPE.fullmatch(token):
        raise RuntimeError(f"the pending token could not be read on {host.target} (exit {got.returncode})")
    if stored and stored != token:
        raise RuntimeError(f"the pending token on {host.target} changed during this run; run phase 11 again")
    if not stored:
        write(ids, spec, ident, site, env, store, envrc_dir, token, key, user)
    back = read_back(ids, spec, env, store, envrc_dir)
    if back != token:
        raise Refused("GB-439", f"{env} was written under the {store} posture but did not read back as written "
                                f"(the store kept another value, or the write did not land). Nothing was switched "
                                f"on; the pending token stays on the host for a re-run")

    # 7. Switch it on, with the read-back value, then check it through the API.
    done = host.run(f"{compose.service_user('bootstrap-token-install')} --name {user} --dir {BOOT_DIR}", stdin=back + "\n")
    st = status_of(done.stdout)
    if st.get("state") != "installed":
        raise RuntimeError(f"bootstrap-token-install on {host.target} failed (exit {done.returncode}): {last_line(done.stderr)}")
    ok, detail = authenticates(ids, fqdn, site, back, user)
    if not ok:
        raise RuntimeError(f"{env} was switched on but does not authenticate as {user} through the API ({detail})")
    if done.returncode != 0:
        say(f"the token is on, and the pending files were not removed: {last_line(done.stderr)}; "
            f"remove {BOOT_DIR}/{stem}.* on {host.target}")
    say(f"minted {user}: {env} stored under the {store} posture and read back, switched on, and checked ({detail})")
    return 0


def authenticates(ids, fqdn, site, value, user):
    """(ok, detail): value authenticates through the API as the service user.
    A token that works with under 30 days left counts as working; the detail
    says so. An API that cannot be reached stops the run."""
    nb = ids.NetBird(f"https://{fqdn}", value, site=site)
    status, _ = nb.call("GET", "/api/users?service_user=true")
    if status in (401, 403, 404):
        return False, f"HTTP {status}"
    users = nb.users()
    ok, detail = ids.verify_owner(nb, value, user, users)
    if not ok and "rotate it" in detail and detail.startswith("belongs to "):
        return True, detail
    return ok, detail


def read_back(ids, spec, env, store, envrc_dir):
    """The stored value of env, from where this posture keeps it; '' if none. A
    store that cannot be read stops the run rather than reading as empty."""
    p = ids.posture()
    if store == "1password":
        return ids.reference_values([env], envrc_dir, ids.onepassword_account(spec)).get(env, "")
    try:
        return p.values(str(REPO)).get(env, "") or ""
    except p.PostureError as e:
        if not Path(p.store_file(str(REPO))).exists():
            return ""
        raise RuntimeError(f"the {store} store cannot be read ({e}); unlock it and run phase 11 again") from None


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
