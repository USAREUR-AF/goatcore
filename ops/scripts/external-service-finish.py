#!/usr/bin/env python3
"""external-service-finish.py -- finish an external service the proposer opened.

When a service owner's declaration is approved in the proposal repository
(hello-galaxy-external-service, beside this one), a pull request opens here
carrying it.
A platform engineer runs this, once, from a laptop that can reach the goatnet:

    python3 ops/scripts/external-service-finish.py <goat PR number>

and it walks every step that needs a person's credentials, in order, stopping
with a plain message at anything wrong. Re-running resumes where it stopped.

   1 check     tools, credentials and a route to the goatnet are present
   2 read      the PR is open and approved; which declaration it carries
   3 peer      find the mesh peer at the declared address; YOU confirm it is
               the requester's host and which groups it gives up
   4 prepare   bring the branch up to main if the zone moved, regenerate the
               zone, sign the name snapshot with the goatnet CA key, record the
               tier move in ops/ci/membership-declared-deviations.yaml, push
   5 review    YOU approve the PR as it now stands (goat dismisses an approval
               given before a new commit), then every check must pass
   6 merge     squash-merge the PR
   7 pull      a fresh checkout of main to apply from
   8 tier      move that peer into <SITE>-EXTERNAL-INFRA (and out of the groups
               you agreed to remove). On a scope warrant governs (its
               ops/warrant/membership/<site>/_scope.yaml enforces) nothing is
               written live: step 4 put an `exact: [<P>-EXTERNAL-INFRA]` entry
               for the host in that scope's services.yaml (or in place, in
               whichever fragment already holds the peer) on the PR, and the
               membership converger applies it
               (docs/design/mesh-change-control.md §4.5)
   9 apply     ops/ansible/external-service.yml state=present, over the mesh or
               an env door; a re-declaration that dropped surfaces names them
               to the mesh-DNS drift guard so only those records retire
  10 verify    each name resolves where it should and the service answers; for
               tls: partner, the public certificate-validation records point at
               the requester's delegation zone and no address is published
  11 email     the requester's configuration lines, from the goatnet's onboarding
               address, to the goatnet identity linked to the PR author's GitHub
               login -- or, if none is linked, an address you type

usage:
  external-service-finish.py <PR> [--dry-run] [--from STEP] [--email ADDR]
                             [--root-key PATH] [--age-identity PATH] [--yes-ci]
  --dry-run   do every read, print every write, change nothing
  --from      resume at a step (state is kept in ~/.goat/external-service-finish/)
  --email     skip the lookup and deliver to this address
This repository, the proposal repository beside it, the state directory holding
the goatnet CA key and the AWS profile an env door opens under are the
checkout's own: repo.source, deploy.state_dir and deploy.aws_profile in
ops/goatbuilder/goatbuilder.yaml (GOAT_AWS_PROFILE overrides the profile).
Design: docs/design/external-service-golden-path.md, "Finishing a proposed service".
"""
import argparse
import base64
import datetime
import glob
import json
import os
import re
import shutil
import smtplib
import socket
import ssl
import subprocess
import sys
import tempfile
import urllib.request
from email.message import EmailMessage

try:
    import yaml
except ImportError:
    sys.exit("external-service-finish: PyYAML is required (python3 -m pip install pyyaml)")

REPO = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
sys.path.insert(0, os.path.join(REPO, "ops", "warrant"))
import membership as warrant_membership  # noqa: E402  (ops/warrant/membership.py)


def declared(section, key, default=""):
    """What this checkout declares about itself in ops/goatbuilder/goatbuilder.yaml,
    read as the builder reads it (crates/goatbuilder/src/settings.rs)."""
    try:
        with open(os.path.join(REPO, "ops", "goatbuilder", "goatbuilder.yaml"), encoding="utf-8") as fh:
            value = ((yaml.safe_load(fh) or {}).get(section) or {}).get(key)
    except (OSError, yaml.YAMLError, AttributeError):
        return default
    return str(value).strip() if value else default


def _clone_slug():
    """The current clone's trunk remote as owner/name (ops/scripts/repo-slug.sh), or ""."""
    try:
        return subprocess.run([os.path.join(REPO, "ops", "scripts", "repo-slug.sh"), "primary"],
                              capture_output=True, text=True, timeout=10).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return ""


# The repository the pull request is in (owner/name): GH_REPO, else the
# checkout's declared repo.source, else the clone's trunk remote. The proposal
# repository lives beside it under the same owner.
GH_REPO = os.environ.get("GH_REPO") or declared("repo", "source") or _clone_slug()
SOURCE_REPO = "%s/hello-galaxy-external-service" % GH_REPO.split("/")[0] if GH_REPO else ""
# Where this machine keeps the goatnet CA keys, and the AWS profile a door opens under.
PROJECT_STATE_DIR = declared("deploy", "state_dir", "~/.goat/state")
# The audit log name-snapshot-sign appends to: ca-audit.jsonl beside the state
# directory, named on the call (ops/scripts/project-setting.sh ca-audit-log).
CA_AUDIT_LOG = os.path.join(os.path.dirname(os.path.expanduser(PROJECT_STATE_DIR.rstrip("/"))) or ".", "ca-audit.jsonl")
AWS_PROFILE = os.environ.get("GOAT_AWS_PROFILE") or declared("deploy", "aws_profile", "default")
STEPS = ["check", "read", "peer", "prepare", "review", "merge", "pull", "tier", "apply", "verify", "email"]
STATE_DIR = os.path.expanduser("~/.goat/external-service-finish")
TTY = sys.stdout.isatty() and not os.environ.get("NO_COLOR")
C = {k: (v if TTY else "") for k, v in dict(b="\033[1m", d="\033[2m", r="\033[31m", g="\033[32m", y="\033[33m", c="\033[36m", x="\033[0m").items()}
DRY = False


class Stop(Exception):
    """A step cannot continue; the message says what to do."""


def step(n, title):
    print("\n%s%s[%d/%d] %s%s" % (C["b"], C["c"], STEPS.index(n) + 1, len(STEPS), title, C["x"]))


def ok(msg):
    print("  %s✓%s %s" % (C["g"], C["x"], msg))


def note(msg):
    print("  %s%s%s" % (C["d"], msg, C["x"]))


def warn(msg):
    print("  %s!%s %s" % (C["y"], C["x"], msg))


def alert(lines):
    bar = "!" * 72
    print("\n%s%s%s" % (C["r"], C["b"], bar))
    for l in lines:
        print("!! " + l)
    print(bar + C["x"] + "\n")


def ask(prompt, default=None):
    suffix = " [%s]" % default if default is not None else ""
    ans = input("  %s?%s %s%s: " % (C["y"], C["x"], prompt, suffix)).strip()
    return ans or (default or "")


def confirm(prompt):
    return ask(prompt + " (yes/no)", "no").lower() in ("y", "yes")


def run(cmd, cwd=None, check=True, capture=True, env=None, write=False):
    """Run a command. A write is printed and skipped under --dry-run."""
    shown = " ".join(cmd) if isinstance(cmd, list) else cmd
    if write and DRY:
        note("DRY: " + shown)
        return subprocess.CompletedProcess(cmd, 0, "", "")
    r = subprocess.run(cmd, cwd=cwd, capture_output=capture, text=True, env=env)
    if check and r.returncode != 0:
        raise Stop("`%s` failed (exit %d):\n%s" % (shown, r.returncode, ((r.stderr or "") + (r.stdout or "")).strip()[-2000:]))
    return r


def gh_json(args):
    return json.loads(run(["gh"] + args).stdout or "null")


def http_json(url, headers, method="GET", body=None, timeout=20):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method, headers=dict(headers, **({"Content-Type": "application/json"} if data else {})))
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        raw = resp.read()
        return json.loads(raw) if raw else None


# ---- the goatnet's facts, read from its group_vars ---------------------------
ENV_LOOKUP = re.compile(r"lookup\(\s*'env'\s*,\s*'([A-Z0-9_]+)'\s*\)")


def site_facts(repo, group):
    """site_id, zones, edge IP, SMTP settings for <group> -- from group_vars, not guessed."""
    merged = {}
    for f in sorted(glob.glob(os.path.join(repo, "ops/ansible/group_vars", group, "*.yml"))):
        try:
            with open(f, encoding="utf-8") as fh:
                d = yaml.safe_load(fh) or {}
        except yaml.YAMLError:
            continue
        if isinstance(d, dict):
            merged.update(d)
    if "site_id" not in merged:
        raise Stop("group_vars/%s has no site_id -- is %s a goatnet group?" % (group, group))
    sid = str(merged["site_id"])
    pref = sid.upper().replace("-", "_")
    def env_of(key):
        m = ENV_LOOKUP.search(str(merged.get(key, "")))
        return m.group(1) if m else None
    facts = {
        "group": group, "site_id": sid, "prefix": pref,
        "zone": merged.get("site_dns_zone"), "mgmt_fqdn": merged.get("public_mgmt_fqdn") or merged.get("site_dns_zone"),
        "edge_ip": str(merged.get("mesh_ip_a", "")), "zone_dir": merged.get("mesh_dns_zone_dir_name") or sid,
        "snapshot": bool(merged.get("name_snapshot_enabled", False)),
        # external_infra is set by governance() from the taxonomy's site_prefix:
        # `pref` is an env-var prefix (site-a -> SITE_A), not a group name.
        "nb_token_env": "%s_NB_API_TOKEN" % pref,
        "portal_key_env": "%s_PORTAL_OPERATOR_API_KEY" % pref,
        "smtp_host": merged.get("id_admin_console_smtp_host"), "smtp_port": int(str(merged.get("id_admin_console_smtp_port") or 587)),
        "smtp_from": merged.get("id_admin_console_smtp_from"),
        "smtp_user_env": env_of("id_admin_console_smtp_username"), "smtp_pass_env": env_of("id_admin_console_smtp_password"),
        # what external-service.yml and the plays it imports read from the environment
        "play_envs": [e for e in (env_of("cloudflare_api_token"), env_of("isitup_oidc_client_secret"), env_of("isitup_cookie_secret")) if e],
    }
    return governance(repo, facts)


def governance(repo, facts):
    """Mark a scope warrant governs, and name its group from the taxonomy.

    A scope whose ops/warrant/membership/<site_id>/_scope.yaml enforces is
    governed: membership changes by a reviewed entry in that directory, never
    a live write.

    On every scope the external-infra group name is <site_prefix>-EXTERNAL-INFRA,
    with site_prefix read from ops/ansible/netbird_acl_taxonomy.yml -- the file
    netbird-acl-bootstrap.yml creates the groups from. It is never derived from
    the site_id: the env-var prefix of site-a is SITE_A, its group prefix
    is SITE-A (#1451).
    """
    mdir = os.path.join(repo, "ops", "warrant", "membership")
    try:
        path = warrant_membership.governed_dir(facts["site_id"], mdir)
    except warrant_membership.SourceError as exc:
        raise Stop(str(exc))
    facts["governed"] = bool(path)
    scopes = warrant_membership.taxonomy_scopes(os.path.join(repo, "ops", "ansible", "netbird_acl_taxonomy.yml"))
    if facts["site_id"] not in scopes:
        if path:
            raise Stop("%s has a membership declaration but is not a taxonomy scope." % facts["site_id"])
        raise Stop("%s is not a scope in ops/ansible/netbird_acl_taxonomy.yml, so it has no "
                   "EXTERNAL-INFRA group to move a host into." % facts["site_id"])
    facts["site_prefix"] = scopes[facts["site_id"]][0]
    facts["external_infra"] = "%s-EXTERNAL-INFRA" % facts["site_prefix"]
    return facts


def external_infra_entry(st, facts, today=None):
    """The membership entry that makes the host an external-service host.

    `exact`, alone: an ADR 1178 host holds EXTERNAL-INFRA and nothing else, and
    `exact` replaces the key baseline rather than adding to it. It expires on
    the declaration's `retire_after`, or in 180 days when the service declares
    none — EXTERNAL-INFRA always carries an expiry (mesh-change-control.md §4.3).
    """
    today = today or datetime.date.today()
    expires = warrant_membership.parse_date(st.get("retire_after")) if st.get("retire_after") else None
    if expires is None or expires < today:
        expires = today + datetime.timedelta(days=180)
    reason = "external service %s host (ADR 1178), goat #%s" % (st.get("name") or "?", st.get("pr") or "?")
    owner = str(st.get("owner") or "").strip()
    if not owner:
        # ops/ansible/external-service.yml refuses a declaration without one.
        raise Stop("The declaration names no `owner` (who to call about the service), so the membership "
                   "entry has none to carry. Add it to the declaration and re-run.")
    return {
        "peer": st["peer_name"],
        "exact": [facts["external_infra"]],
        "owner": owner,
        "reason": reason[:warrant_membership.REASON_MAX],
        "expires": expires.isoformat(),
    }


def write_membership_entry(tree, st, facts, today=None):
    """Write/replace the host's entry in <tree>'s membership scope; validate it.

    The entry goes into services.yaml, or in place into whichever fragment of
    ops/warrant/membership/<site>/ already holds the peer. The whole scope is
    re-validated (one entry per peer across fragments), and on a failure the
    fragment is put back as it was.

    Returns (fragment path relative to tree, "added"|"replaced"|"unchanged")."""
    rel_dir = os.path.join("ops", "warrant", "membership", facts["site_id"])
    sdir = os.path.join(tree, rel_dir)
    if not os.path.isfile(os.path.join(sdir, warrant_membership.SCOPE_FILE)):
        raise Stop("%s is governed but %s/%s is missing from the branch. Merge main into the PR and re-run."
                   % (facts["site_id"], rel_dir, warrant_membership.SCOPE_FILE))
    entry = external_infra_entry(st, facts, today)
    target = warrant_membership.find_entry(sdir, entry["peer"]) or os.path.join(sdir, warrant_membership.FRAGMENT_SERVICES)
    before = None
    if os.path.isfile(target):
        with open(target, encoding="utf-8") as fh:
            before = fh.read()
    path, how = warrant_membership.upsert_scope_entry(sdir, entry, warrant_membership.FRAGMENT_SERVICES)
    errs = warrant_membership.check_scope(sdir, today=today)
    if errs:
        if before is None:
            os.remove(str(path))
        else:
            with open(str(path), "w", encoding="utf-8") as fh:
                fh.write(before)
        raise Stop("the membership entry for %s does not validate (scope left unchanged):\n    - %s"
                   % (st["peer_name"], "\n    - ".join(errs)))
    with open(str(path), encoding="utf-8") as fh:
        if fh.read() == before:
            how = "unchanged"
    return os.path.relpath(str(path), tree), how


# ---- 1 check -----------------------------------------------------------------
def transport(facts):
    """mesh when this laptop is on the goatnet's NetBird profile, else an env door."""
    st = run(["netbird", "status"], check=False)
    prof = re.search(r"^Profile:\s*(\S+)", st.stdout or "", re.M)
    mgmt = re.search(r"^Management:\s*(\S+)", st.stdout or "", re.M)
    if prof and mgmt and prof.group(1) == facts["site_id"] and mgmt.group(1) == "Connected":
        return "mesh", ["-i", "inventory/mgmt-hosts.ini", "-i", "inventory/mgmt-hosts-mesh.ini"]
    door = run([os.path.join(REPO, "ops/scripts/env-door.sh"), facts["site_id"], "--print"], check=False)
    if door.returncode == 0:
        return "door", ["-i", "inventory/mgmt-hosts.ini", "-i", "inventory/doors-groupvars.ini"]
    return None, None


def do_check(a, st):
    step("check", "tools, credentials and a route to the goatnet")
    if not GH_REPO:
        raise Stop("ops/goatbuilder/goatbuilder.yaml declares no repo.source (owner/name), so the repository "
                   "the pull request is in is unknown. Nothing has changed.")
    missing = []
    # aws-vault is NOT here: over the mesh the apply is plain SSH. Only an env
    # door needs AWS, and check_site asks for it once it knows the route.
    for tool in ["gh", "git", "go", "ansible-playbook", "netbird", "openssl"]:
        if not shutil.which(tool):
            missing.append("the `%s` command" % tool)
    if run(["gh", "auth", "status"], check=False).returncode != 0:
        missing.append("a GitHub login: run `gh auth login`")
    else:
        ok("GitHub login: @%s" % gh_json(["api", "user"])["login"])
    if missing:
        raise Stop("Missing before anything can run:\n    - " + "\n    - ".join(missing))
    ok("tools and GitHub login present (credentials for the goatnet are checked once the PR names it)")


def check_site(a, facts):
    need = [facts["nb_token_env"]] + facts["play_envs"]
    if facts["smtp_user_env"]:
        need += [facts["smtp_user_env"], facts["smtp_pass_env"]]
    missing = [v for v in need if not os.environ.get(v)]
    if facts["snapshot"]:
        rk = os.path.expanduser(a.root_key or "%s/%s-ca/ca.key.age" % (PROJECT_STATE_DIR, facts["site_id"]))
        ai = os.path.expanduser(a.age_identity)
        if not os.path.exists(rk):
            missing.append("the %s CA key at %s (or pass --root-key)" % (facts["site_id"], rk))
        if not os.path.exists(ai):
            missing.append("an age identity at %s (or pass --age-identity)" % ai)
        a.root_key, a.age_identity = rk, ai
    if missing:
        raise Stop("For %s this run needs:\n    - %s\nLoad them (direnv / 1Password) and re-run; nothing has changed yet."
                   % (facts["site_id"], "\n    - ".join(missing)))
    how, _ = transport(facts)
    if not how:
        raise Stop("No route to %s: connect this laptop to the %s NetBird profile (no AWS needed), or register an "
                   "env door (ops/scripts/env-door.sh, docs/design/env-doors.md)." % (facts["site_id"], facts["site_id"]))
    if how == "door" and not shutil.which("aws-vault"):
        raise Stop("The route to %s is an env door, which opens through AWS: install aws-vault and add the `%s` "
                   "profile -- or connect this laptop to the %s NetBird profile instead, which needs no AWS."
                   % (facts["site_id"], AWS_PROFILE, facts["site_id"]))
    ok("credentials for %s present; route: %s%s" % (facts["site_id"], how, "" if how == "door" else " (no AWS needed)"))


# ---- 2 read ------------------------------------------------------------------
DECL_PATH = re.compile(r"^ops/ansible/group_vars/([a-z0-9_]+)/external-service-([a-z0-9-]+)\.yml$")
SOURCE_URL = re.compile(r"https://github\.com/%s/pull/(\d+)" % re.escape(SOURCE_REPO))


def do_read(a, st):
    step("read", "the pull request and the declaration it carries")
    pr = gh_json(["pr", "view", str(a.pr), "--repo", GH_REPO, "--json",
                  "number,state,title,headRefName,reviewDecision,files,body,url,mergeCommit"])
    if pr["state"] == "MERGED":
        st["merged"] = True
        note("already merged; continuing from its merged state")
    elif pr["state"] != "OPEN":
        raise Stop("PR #%s is %s." % (a.pr, pr["state"]))
    decls = [(m.group(1), m.group(2)) for m in (DECL_PATH.match(f["path"]) for f in pr["files"]) if m]
    if len(decls) != 1:
        raise Stop("PR #%s carries %d external-service declarations; this script finishes exactly one." % (a.pr, len(decls)))
    group, stem = decls[0]
    src = SOURCE_URL.search(pr["body"] or "")
    st.update(pr=pr["number"], url=pr["url"], branch=pr["headRefName"], group=group, stem=stem,
              source_pr=int(src.group(1)) if src else None)
    ok("%s -- %s" % (pr["url"], pr["title"]))
    ok("declaration: group_vars/%s/external-service-%s.yml; requested in %s"
       % (group, stem, ("hello-galaxy-external-service#%s" % st["source_pr"]) if src else "an unrecorded source"))


def declaration(repo_dir, st):
    p = os.path.join(repo_dir, "ops/ansible/group_vars", st["group"], "external-service-%s.yml" % st["stem"])
    with open(p, encoding="utf-8") as fh:
        d = yaml.safe_load(fh)
    return next(v for k, v in d.items() if k.startswith("external_service_"))


def branch_file(st, path):
    """A file as it is on the PR branch, without a checkout."""
    r = run(["git", "-C", REPO, "fetch", "--quiet", "origin", st["branch"]], check=False)
    if r.returncode != 0:
        raise Stop("could not fetch branch %s -- is this run inside a goat clone with `origin` = %s?" % (st["branch"], GH_REPO))
    return run(["git", "-C", REPO, "show", "FETCH_HEAD:%s" % path]).stdout


# ---- 3 peer ------------------------------------------------------------------
def nb(facts, path, method="GET", body=None):
    return http_json("https://%s/api%s" % (facts["zone"], path),
                     {"Authorization": "Token %s" % os.environ[facts["nb_token_env"]], "Accept": "application/json"},
                     method=method, body=body)


def do_peer(a, st, facts):
    step("peer", "the host the requester declared")
    ext = yaml.safe_load(branch_file(st, "ops/ansible/group_vars/%s/external-service-%s.yml" % (st["group"], st["stem"])))
    ext = next(v for k, v in ext.items() if k.startswith("external_service_"))
    tls = str(ext.get("tls", "edge"))
    addr = ext.get("mesh_ip") if tls == "partner" else re.sub(r"^https?://([^:/]+).*$", r"\1", str(ext.get("backend_url", "")))
    peers = [p for p in nb(facts, "/peers") if p.get("ip") == addr]
    if len(peers) != 1:
        raise Stop("%d mesh peers hold %s on %s. The declaration must name the address of one existing peer, the "
                   "requester's host. Ask them to confirm it with `netbird status` on that host, and to correct the "
                   "declaration in hello-galaxy-external-service if it is wrong." % (len(peers), addr, facts["site_id"]))
    p = peers[0]
    if ext.get("mesh_peer") and p.get("name") != ext["mesh_peer"]:
        raise Stop("%s is held by peer %r, but the declaration names mesh_peer %r. The host was re-enrolled or the "
                   "address is wrong; ask the requester for `netbird status` on their host and to correct the "
                   "declaration." % (addr, p.get("name"), ext["mesh_peer"]))
    groups = [g["name"] for g in p.get("groups", [])]
    print("\n    peer        %s%s%s" % (C["b"], p.get("name"), C["x"]))
    print("    hostname    %s  (%s)" % (p.get("hostname"), p.get("os")))
    print("    address     %s" % addr)
    print("    connected   %s, last seen %s" % (p.get("connected"), p.get("last_seen")))
    print("    groups      %s" % ", ".join(groups))
    print("    declared    %s (%d hostnames, tls: %s%s)\n" % (ext.get("name"), len(ext.get("hostnames") or [ext.get("name")]), tls,
                                                     (", validates at %s" % ext["acme_delegate"]) if ext.get("acme_delegate") else ""))
    if not DRY and not confirm("Is this the requester's host, and should it become an external-service host (%s)?" % facts["external_infra"]):
        raise Stop("Stopped at your answer. Nothing has changed.")
    drop = [g for g in groups if g not in ("All", facts["external_infra"])]
    remove = []
    if drop and facts.get("governed"):
        # `exact` replaces the whole managed set, so there is nothing to choose.
        note("%s is governed by warrant: the host's entry is `exact: [%s]`, so the converger takes it out of %s."
             % (facts["site_id"], facts["external_infra"], ", ".join(drop)))
        remove = drop
    elif drop:
        note("An external-service host holds %s alone: reachable one-way from every role group, initiating only "
             "toward INFRA. Its other groups would let it reach further than that." % facts["external_infra"])
        if DRY or confirm("Remove it from %s? (recommended)" % ", ".join(drop)):
            remove = drop
    st.update(peer_id=p["id"], peer_name=p.get("name"), addr=addr, tls=tls, remove=remove, name=ext.get("name"),
              hostnames=ext.get("hostnames") or [ext.get("name")], health_path=ext.get("health_path", "/"),
              door=ext.get("door"), client=ext.get("client"), owner=ext.get("owner"),
              retire_after=str(ext.get("retire_after") or "") or None,
              acme_delegate=str(ext.get("acme_delegate") or "").lower().rstrip(".") or None)
    ok("confirmed %s; will add %s%s" % (p.get("name"), facts["external_infra"], (", remove " + ", ".join(remove)) if remove else ""))


# ---- 4 prepare ---------------------------------------------------------------
def deviation_row(st, facts, who):
    lines = ["", "  # ---- external service %s on %s (hello-galaxy-external-service#%s, goat #%s) ----"
             % (st["name"], facts["site_id"], st.get("source_pr") or "?", st["pr"]),
             "  - peer: %s" % st["peer_name"], "    group: %s" % facts["external_infra"]]
    lines += ["    absent: %s" % g for g in st["remove"]]
    lines += ["    decided_by: %s" % who,
              "    decided_on: %s" % datetime.date.today().isoformat(),
              "    reason: >-",
              "      Host of the external service %s, declared by its owner and approved in" % st["name"],
              "      hello-galaxy-external-service and in goat #%s. An external-service host holds" % st["pr"],
              "      %s alone (ADR 1178). Moved there by ops/scripts/external-service-finish.py," % facts["external_infra"],
              "      because no node class mints that tier.",
              "    revisit_when: >-",
              "      The service is withdrawn, or a node class mints %s and the host is" % facts["external_infra"],
              "      re-enrolled on it.",
              ""]
    return "\n".join(lines)


def merge_main_into(tmp, st):
    if run(["git", "-C", tmp, "merge-base", "--is-ancestor", "origin/main", "HEAD"], check=False).returncode == 0:
        return
    note("the branch is behind main; merging main in")
    if run(["git", "-C", tmp, "merge", "--no-edit", "--signoff", "origin/main"], check=False).returncode == 0:
        return
    conflicted = run(["git", "-C", tmp, "diff", "--name-only", "--diff-filter=U"]).stdout.split()
    if conflicted and all(f.startswith("ops/dns/") for f in conflicted):
        note("only generated zone files conflict; taking main's copies, they are regenerated next")
        run(["git", "-C", tmp, "checkout", "origin/main", "--"] + conflicted)
        run(["git", "-C", tmp, "add"] + conflicted)
        run(["git", "-C", tmp, "commit", "--no-edit", "--signoff"])
        return
    run(["git", "-C", tmp, "merge", "--abort"], check=False)
    raise Stop("merging main into %s conflicts outside generated files (%s). Resolve that on the PR by hand, then re-run."
               % (st["branch"], ", ".join(conflicted)))


def do_prepare(a, st, facts):
    step("prepare", "zone, signature and the tier record, on the PR branch")
    if st.get("merged"):
        ok("PR already merged; nothing to prepare")
        return
    who = gh_json(["api", "user"])["login"]
    tmp = tempfile.mkdtemp(prefix="esf-%s-" % st["pr"])
    os.rmdir(tmp)
    run(["git", "-C", REPO, "fetch", "--quiet", "origin", "main:refs/remotes/origin/main",
         "%s:refs/remotes/origin/%s" % (st["branch"], st["branch"])])
    run(["git", "-C", REPO, "worktree", "add", "--quiet", "-B", "esf-%s" % st["pr"], tmp, "origin/%s" % st["branch"]])
    try:
        merge_main_into(tmp, st)
        run([sys.executable, "ops/dns/generate-mesh-hosts.py"], cwd=tmp)
        ok("zone regenerated from the branch")
        if facts["snapshot"]:
            if run(["bash", "ops/ci/check-name-snapshot-sig.sh"], cwd=tmp, check=False).returncode == 0:
                ok("name snapshot signature already valid")
            else:
                note("signing ops/dns/%s/name-snapshot.json with the %s CA key (age may ask for a passphrase)"
                     % (facts["zone_dir"], facts["site_id"]))
                run(["go", "run", "./cmd/name-snapshot-sign", "--snapshot", "../dns/%s/name-snapshot.json" % facts["zone_dir"],
                     "--root-key", a.root_key, "--age-identity", a.age_identity,
                     "--audit-log", CA_AUDIT_LOG],
                    cwd=os.path.join(tmp, "ops", "enrollment"), capture=False, write=True)
                if not DRY:
                    run(["bash", "ops/ci/check-name-snapshot-sig.sh"], cwd=tmp)
                ok("name snapshot signed and verified")
        dev = os.path.join(tmp, "ops", "ci", "membership-declared-deviations.yaml")
        with open(dev, encoding="utf-8") as fh:
            body = fh.read()
        record_paths = ["ops/ci/membership-declared-deviations.yaml"]
        if facts.get("governed"):
            # The membership scope is the only record on a governed scope; the
            # deviation list is the ungoverned scopes' stand-in for it.
            if DRY:
                note("DRY: would write into ops/warrant/membership/%s/%s:\n%s"
                     % (facts["site_id"], warrant_membership.FRAGMENT_SERVICES,
                        warrant_membership.render_entry(external_infra_entry(st, facts))))
            else:
                rel, how = write_membership_entry(tmp, st, facts)
                ok("membership entry %s in %s" % (how, rel))
            record_paths = ["ops/warrant/membership/%s" % facts["site_id"]]
        elif re.search(r"^\s*- peer: %s\s*$\n\s*group: %s\s*$" % (re.escape(st["peer_name"]), re.escape(facts["external_infra"])), body, re.M):
            ok("tier move already recorded")
        elif DRY:
            note("DRY: would append to ops/ci/membership-declared-deviations.yaml:" + deviation_row(st, facts, who).replace("\n", "\n        "))
        else:
            with open(dev, "a", encoding="utf-8", newline="\n") as fh:
                fh.write(deviation_row(st, facts, who))
            ok("tier move recorded in ops/ci/membership-declared-deviations.yaml")
        run(["git", "-C", tmp, "add", "ops/dns"] + record_paths)
        if run(["git", "-C", tmp, "diff", "--cached", "--quiet"], check=False).returncode != 0:
            run(["git", "-C", tmp, "commit", "--signoff", "-m",
                 "chore(external-service): sign the %s zone and record %s for %s"
                 % (facts["site_id"], facts["external_infra"], st["peer_name"])], write=True)
        ahead = run(["git", "-C", tmp, "rev-list", "--count", "origin/%s..HEAD" % st["branch"]]).stdout.strip()
        if ahead != "0":
            run(["git", "-C", tmp, "push", "origin", "HEAD:%s" % st["branch"]], write=True)
            ok("pushed %s commit(s) to %s" % (ahead, st["branch"]))
        else:
            ok("the branch already carries everything")
    finally:
        run(["git", "-C", REPO, "worktree", "remove", "--force", tmp], check=False)
        run(["git", "-C", REPO, "branch", "-D", "esf-%s" % st["pr"]], check=False)


# ---- 6 merge -----------------------------------------------------------------
def do_merge(a, st, facts):
    step("merge", "squash-merge the PR")
    if DRY:
        note("DRY: gh pr merge %s --repo %s --squash --delete-branch" % (st["pr"], GH_REPO))
        return
    pr = gh_json(["pr", "view", str(st["pr"]), "--repo", GH_REPO, "--json", "state,reviewDecision"])
    if pr["state"] == "MERGED":
        st["merged"] = True
        ok("already merged")
        return
    if pr["reviewDecision"] != "APPROVED":
        raise Stop("the new commits dismissed the approval (review decision: %s). Re-approve %s, then re-run with --from merge."
                   % (pr["reviewDecision"] or "none", st["url"]))
    checked_head = st.get("checked_head")
    if not checked_head:
        raise Stop("No checked proposal head; restart from read before merging.")
    run(["gh", "pr", "merge", str(st["pr"]), "--repo", GH_REPO, "--squash", "--delete-branch",
         "--match-head-commit", checked_head], write=True)
    st["merged"] = not DRY
    ok("merged")


# ---- 7 pull / 8 tier ---------------------------------------------------------
def do_pull(a, st, facts):
    step("pull", "a fresh checkout of main to apply from")
    tmp = os.path.join(tempfile.gettempdir(), "esf-apply-%s" % st["pr"])
    if os.path.isdir(tmp):
        run(["git", "-C", REPO, "worktree", "remove", "--force", tmp], check=False)
    run(["git", "-C", REPO, "fetch", "--quiet", "origin", "main:refs/remotes/origin/main"])
    run(["git", "-C", REPO, "worktree", "add", "--quiet", "-B", "esf-apply-%s" % st["pr"], tmp, "origin/main"])
    decl = os.path.join(tmp, "ops/ansible/group_vars", st["group"], "external-service-%s.yml" % st["stem"])
    if not DRY and not os.path.exists(decl):
        raise Stop("main does not carry %s yet; the merge has not landed." % decl)
    st["apply_tree"] = tmp
    ok("checked out origin/main at %s" % run(["git", "-C", tmp, "rev-parse", "--short", "HEAD"]).stdout.strip())
    st["retired_hostnames"] = retired_hostnames(tmp, st)
    if st["retired_hostnames"]:
        note("this is a RE-declaration that drops %d surface(s) the live zone still serves: %s"
             % (len(st["retired_hostnames"]), ", ".join(st["retired_hostnames"])))
        note("apply will name exactly those to the mesh-DNS drift guard; every other record stays guarded")


def retired_hostnames(tree, st):
    """Hostnames the previous declaration of this service had and the merged one lacks.

    A partner re-declares by opening a new proposal; when the new list has fewer
    surfaces, the merged zone correctly omits them, but the live resolver still
    serves them from the last apply, and the mesh-coredns drift guard
    refuses to push a zone that drops a live record -- it cannot tell a
    deliberate retirement from a stale checkout. The previous declaration is
    the file as it stood on main before this PR's squash merge, i.e. at the
    merge commit's parent; a first declaration has no such file and retires
    nothing. Returned as bare labels; apply qualifies them with the zone.
    """
    rel = "ops/ansible/group_vars/%s/external-service-%s.yml" % (st["group"], st["stem"])
    merge = gh_json(["pr", "view", str(st["pr"]), "--repo", GH_REPO, "--json", "mergeCommit"]).get("mergeCommit") or {}
    sha = merge.get("oid")
    if not sha:
        return []
    r = run(["git", "-C", tree, "show", "%s^:%s" % (sha, rel)], check=False)
    if r.returncode != 0:
        return []
    prev = next((v for k, v in yaml.safe_load(r.stdout).items() if k.startswith("external_service_")), {})
    before = [h for h in (prev.get("hostnames") or [])]
    after = set(declaration(tree, st).get("hostnames") or [])
    return [h for h in before if h not in after]


def do_tier(a, st, facts):
    step("tier", "move %s into %s" % (st["peer_name"], facts["external_infra"]))
    if facts.get("governed"):
        # mesh-change-control.md §4.5: on a governed scope nothing writes
        # membership live. The merged entry is the change; the converger on the
        # management host applies it (or reports it, under converge: report).
        ok("%s is governed by warrant: no live write. The merged entry in ops/warrant/membership/%s/ "
           "is applied by the membership converger." % (facts["site_id"], facts["site_id"]))
        return
    groups = nb(facts, "/groups")
    by_name = {g["name"]: g for g in groups}
    if facts["external_infra"] not in by_name:
        raise Stop("%s has no group named %s." % (facts["site_id"], facts["external_infra"]))
    def put(name, add=None, drop=None):
        g = nb(facts, "/groups/%s" % by_name[name]["id"])
        ids = [p["id"] for p in g.get("peers") or []]
        new = [i for i in ids if i != drop] + ([add] if add and add not in ids else [])
        if new == ids:
            return False
        body = {"name": g["name"], "peers": new}
        if g.get("resources"):
            body["resources"] = g["resources"]
        if DRY:
            note("DRY: PUT /api/groups/%s  %s %s" % (g["id"], "add" if add else "remove", st["peer_name"]))
        else:
            nb(facts, "/groups/%s" % g["id"], method="PUT", body=body)
        return True
    verb = ("would be added to %s", "would be removed from %s") if DRY else ("added to %s", "removed from %s")
    ok((verb[0] if put(facts["external_infra"], add=st["peer_id"]) else "already in %s") % facts["external_infra"])
    for name in st["remove"]:
        if name in by_name:
            ok((verb[1] if put(name, drop=st["peer_id"]) else "already out of %s") % name)


# ---- 9 apply -----------------------------------------------------------------
def do_apply(a, st, facts):
    step("apply", "publish the service on %s" % facts["site_id"])
    secret_env = (st.get("client") or {}).get("secret_env")
    if secret_env and not os.environ.get(secret_env) and not DRY:
        raise Stop("the declaration registers an OAuth2 client, so the apply needs $%s: the same value the requester's "
                   "fence is configured with. Set it, then --from apply." % secret_env)
    how, inv = transport(facts)
    if not how:
        raise Stop("lost the route to %s; reconnect the NetBird profile or open the env door, then --from apply." % facts["site_id"])
    if how == "door":
        run([os.path.join(REPO, "ops/scripts/door-open.sh"), facts["site_id"]], write=True)
    # No aws-vault wrapper: the mgmt host is reached over SSH (mesh, or the door's
    # local port forward). door-open.sh does its own `aws-vault exec`, and refuses
    # to run inside one.
    cmd = ["../scripts/ansible-preflight.sh"] + inv + [
        "-e", "target=%s" % st["group"], "-e", "service=%s" % st["stem"].replace("-", "_"), "-e", "state=present"]
    retired = st.get("retired_hostnames") or []
    if retired:
        # The mesh-DNS drift guard (roles/mesh-coredns, F-215) refuses any push
        # that drops a live record. Name the retired surfaces, and only those,
        # so the guard lets them go and stays armed for everything else. The
        # blanket allow_zone_record_removal is never passed from here.
        fq = ["%s.%s.%s" % (h, facts["site_id"], facts["zone"]) for h in retired]
        cmd += ["-e", json.dumps({"zone_record_removal_allow": fq})]
        note("retiring from the mesh zone: %s" % ", ".join(fq))
    cmd.append("external-service.yml")
    env = dict(os.environ, OBJC_DISABLE_INITIALIZE_FORK_SAFETY="YES")
    note("via %s: %s" % (how, " ".join(cmd)))
    run(cmd, cwd=os.path.join(st.get("apply_tree") or REPO, "ops", "ansible"), capture=False, env=env, write=True)
    if not DRY:
        ok("applied")


# ---- 10 verify ---------------------------------------------------------------
def do_verify(a, st, facts):
    step("verify", "the names resolve and the service answers")
    if DRY:
        note("DRY: would resolve %d names and probe the service; nothing is published in a dry run" % len(st["hostnames"]))
        return
    expect = st["addr"] if st["tls"] == "partner" else facts["edge_ip"]
    bad = 0
    for h in st["hostnames"]:
        fq = "%s.%s.%s" % (h, facts["site_id"], facts["zone"])
        try:
            got = socket.gethostbyname(fq)
        except OSError:
            warn("%s does not resolve from this laptop (it may not use mesh DNS)" % fq)
            bad += 1
            continue
        (ok if got == expect else warn)("%s -> %s%s" % (fq, got, "" if got == expect else " (expected %s)" % expect))
        bad += got != expect
    front = "%s.%s.%s" % (st["hostnames"][0], facts["site_id"], facts["zone"])
    if st["tls"] == "edge":
        try:
            urllib.request.urlopen("https://%s%s" % (front, st["health_path"]), timeout=15, context=ssl.create_default_context())
            ok("https://%s%s answers with a certificate this laptop trusts" % (front, st["health_path"]))
        except Exception as e:
            warn("https://%s%s: %s" % (front, st["health_path"], e))
            bad += 1
    else:
        # Not counted: a partner gets its certificate through the delegation this
        # run just published, so it usually cannot serve TLS until after the email.
        try:
            socket.create_connection((st["addr"], 443), timeout=10).close()
            ok("%s:443 accepts connections" % st["addr"])
        except OSError as e:
            note("%s:443 does not answer yet (%s). Expected until the requester installs the certificate "
                 "the delegation lets them obtain; the email tells them how." % (st["addr"], e))
        if st.get("acme_delegate"):
            bad += verify_delegation(st, facts)
    if bad:
        warn("%d check(s) did not pass from here. The apply itself succeeded; confirm from a mesh peer before emailing." % bad)
        if not DRY and not confirm("Send the requester their configuration anyway?"):
            raise Stop("Stopped before emailing. Re-run with --from email once the service answers.")


def challenge_names(st, facts):
    """The public _acme-challenge names a partner validates with: one per first label under the site."""
    labels = sorted({h.split(".")[-1] for h in st["hostnames"]})
    return ["_acme-challenge.%s.%s.%s" % (l, facts["site_id"], facts["zone"]) for l in labels]


def cert_names(st, facts):
    """What the partner's certificate carries: *.<name> for nested names, each flat name as itself."""
    sfx = "%s.%s" % (facts["site_id"], facts["zone"])
    out = []
    for h in st["hostnames"]:
        n = ("*." + h.split(".", 1)[1]) if "." in h else h
        if n not in out:
            out.append(n)
    return ["%s.%s" % (n, sfx) for n in out]


def public_dns(name, rtype):
    """Answers from public DNS (DNS over HTTPS), not this laptop's resolver, which may be mesh DNS."""
    url = "https://cloudflare-dns.com/dns-query?name=%s&type=%s" % (name, rtype)
    r = http_json(url, {"Accept": "application/dns-json"})
    return [a.get("data", "").rstrip(".").lower() for a in (r.get("Answer") or [])]


def verify_delegation(st, facts):
    bad = 0
    for n in challenge_names(st, facts):
        try:
            got = public_dns(n, "CNAME")
        except Exception as e:
            warn("%s: public DNS lookup failed (%s)" % (n, e))
            bad += 1
            continue
        good = got == [st["acme_delegate"]]
        (ok if good else warn)("%s CNAME %s%s" % (n, ", ".join(got) or "(none)", "" if good else " (expected %s)" % st["acme_delegate"]))
        bad += not good
    for h in st["hostnames"]:
        fq = "%s.%s.%s" % (h, facts["site_id"], facts["zone"])
        try:
            leaked = public_dns(fq, "A")
        except Exception:
            continue
        if leaked:
            warn("%s has a PUBLIC address (%s); partner names resolve only in mesh DNS" % (fq, ", ".join(leaked)))
            bad += 1
    return bad


# ---- 11 email ----------------------------------------------------------------
EMAIL_RE = re.compile(r"^[^@\s]+@[^@\s]+\.[^@\s]+$")


def linked_email(st, facts):
    """(login, email or None, why) -- the goatnet identity whose onboarding record names the PR author's GitHub login."""
    if not st.get("source_pr"):
        return None, None, "the goat PR does not record which hello-galaxy-external-service PR requested it"
    login = gh_json(["api", "repos/%s/pulls/%s" % (SOURCE_REPO, st["source_pr"])])["user"]["login"]
    key = os.environ.get(facts["portal_key_env"])
    if not key:
        return login, None, "%s is not set, so the onboarding records could not be read" % facts["portal_key_env"]
    url = "https://onboard.%s.%s/api/onboard/proposals?status=onboarded" % (facts["site_id"], facts["mgmt_fqdn"])
    try:
        rows = http_json(url, {"Authorization": "Bearer %s" % key, "Accept": "application/json"})
    except Exception as e:
        return login, None, "the onboarding records at %s could not be read (%s)" % (url, e)
    hits = []
    for r in rows or []:
        try:
            gh = (json.loads(r.get("manifest_json") or "{}").get("identity") or {}).get("github") or ""
        except ValueError:
            continue
        if gh.lstrip("@").lower() == login.lower() and r.get("email"):
            hits.append(r["email"])
    hits = sorted(set(hits))
    if len(hits) == 1:
        return login, hits[0], "onboarding record on %s" % facts["site_id"]
    if not hits:
        return login, None, "no onboarded identity on %s lists GitHub login @%s" % (facts["site_id"], login)
    return login, None, "%d onboarded identities list GitHub login @%s (%s)" % (len(hits), login, ", ".join(hits))


def config_text(st, facts):
    urls = ["  https://%s.%s.%s" % (h, facts["site_id"], facts["zone"]) for h in st["hostnames"]]
    issuer = "https://oauth2.%s.%s/" % (facts["site_id"], facts["zone"])
    lines = ["Your service %s is published on the %s goatnet." % (st["name"], facts["site_id"]), "",
             "Hostnames (reachable from the mesh):"] + urls + ["",
             "Sign-in -- verify every token your service receives against:",
             "  issuer   %s   (compare exactly, trailing slash included)" % issuer,
             "  jwks     %s.well-known/jwks.json" % issuer,
             "  pin      oauth2.%s.%s -> %s   (only if your host does not use mesh DNS)" % (facts["site_id"], facts["zone"], facts["edge_ip"])]
    if st.get("client"):
        c = st["client"]
        lines += ["  client   %s" % c.get("id"),
                  "  redirect https://%s.%s.%s%s" % (st["hostnames"][0], facts["site_id"], facts["zone"], c.get("redirect_path", "/oauth2/callback")),
                  "  secret   the value of $%s, which the platform team will share with you separately" % c.get("secret_env")]
    lines += ["  door     %s   (what your own fence decides)" % st.get("door"), ""]
    if st["tls"] == "partner" and st.get("acme_delegate"):
        lines += ["TLS: these names resolve straight to your host, %s, and you terminate TLS. Get a publicly" % st["addr"],
                  "trusted certificate from Let's Encrypt with the DNS-01 challenge; nothing needs to be served",
                  "publicly and your host needs no public address. We publish, in public DNS:"]
        lines += ["  %s  CNAME  %s" % (n, st["acme_delegate"]) for n in challenge_names(st, facts)]
        lines += ["so Let's Encrypt reads each challenge TXT record at %s, a name in a zone you control." % st["acme_delegate"],
                  "Point your ACME client's DNS provider at that zone and have it follow the CNAME, e.g.",
                  "  lego:     --dns <your provider>   (follows CNAMEs by default)",
                  "  acme.sh:  --dns <your provider> --domain-alias %s" % st["acme_delegate"],
                  "Request a certificate for %s." % ", ".join(cert_names(st, facts)),
                  "Renewal works the same way, unattended. Keep that TXT zone's credentials on the renewing host only."]
    elif st["tls"] == "partner":
        lines += ["TLS: these names resolve straight to your host, %s. You terminate TLS with your own" % st["addr"],
                  "certificate; browsers warn unless their devices trust its issuer. The goatnet issues none."]
    else:
        lines += ["TLS: the goatnet's edge serves these names with a publicly trusted certificate and forwards",
                  "to your backend over the mesh. Serve plain HTTP on your mesh address; bind it, never 0.0.0.0."]
    lines += ["", "Your host %s is now in %s: every role group can reach it, and it can reach only" % (st["peer_name"], facts["external_infra"]),
              "infrastructure. To change or withdraw the service, edit your declaration in",
              "hello-galaxy-external-service (withdraw with enabled: false, never by deleting it).", "",
              "-- the %s platform team" % facts["site_id"]]
    return "\n".join(lines)


def do_email(a, st, facts):
    step("email", "send the requester their configuration")
    if a.email:
        to, why = a.email, "given with --email"
    else:
        login, to, why = linked_email(st, facts)
        if not to:
            alert(["GITHUB ACCOUNT NOT LINKED TO AN IDENTITY ON %s" % facts["site_id"].upper(),
                   "",
                   "Requester: @%s   (%s)" % (login or "unknown", why),
                   "",
                   "No verified address to deliver to. Enter one you have confirmed with the requester",
                   "out of band. It is used for this email only and is not stored."])
            while True:
                to = ask("Delivery email for @%s" % (login or "the requester"))
                if EMAIL_RE.match(to) and confirm("Send to %s?" % to):
                    break
                warn("not sent; enter a valid address and confirm it")
            why = "entered by the platform engineer"
    ok("deliver to %s (%s)" % (to, why))
    msg = EmailMessage()
    msg["From"], msg["To"] = facts["smtp_from"], to
    msg["Subject"] = "[%s] %s is published" % (facts["site_id"], st["name"])
    msg.set_content(config_text(st, facts))
    if DRY:
        note("DRY: would send from %s via %s:%s:\n" % (facts["smtp_from"], facts["smtp_host"], facts["smtp_port"]))
        print("\n".join("        " + l for l in msg.get_content().splitlines()))
        return
    # Recipient prompts may wait while the owner withdraws the proposal.
    require_present_declaration(st)
    with smtplib.SMTP(facts["smtp_host"], facts["smtp_port"], timeout=30) as s:
        s.starttls(context=ssl.create_default_context())
        s.login(os.environ[facts["smtp_user_env"]], os.environ[facts["smtp_pass_env"]])
        s.send_message(msg)
    ok("sent from %s" % facts["smtp_from"])
    run(["gh", "pr", "comment", str(st["pr"]), "--repo", GH_REPO, "--body",
         "Finished with `ops/scripts/external-service-finish.py`: %s moved to %s, applied to %s, configuration emailed."
         % (st["peer_name"], facts["external_infra"], facts["site_id"])], check=False)


# ---- 5 review: approval AFTER the signature, then checks ------------------------
def do_review(a, st, facts):
    step("review", "the PR is approved as it now stands, and every check passes")
    if st.get("merged"):
        ok("already merged")
        return
    if DRY:
        note("DRY: would wait for an approval of %s and for its checks" % st["url"])
        return
    # goat dismisses an approval when a commit lands after it, and the signature
    # commit just did. So the approval that counts is the one given now, on what
    # will actually merge: the requester's file, a signed zone and a recorded tier.
    # A PR merged while this waits (or before a --from review re-run) never
    # reports APPROVED when the merge bypassed review, so read its state too.
    import time
    announced = False
    while True:
        pr = gh_json(["pr", "view", str(st["pr"]), "--repo", GH_REPO, "--json", "state,reviewDecision"])
        if pr["state"] == "MERGED":
            st["merged"] = True
            ok("already merged")
            return
        if pr["state"] != "OPEN":
            raise Stop("PR #%s is %s." % (st["pr"], pr["state"]))
        if pr["reviewDecision"] == "APPROVED":
            break
        if not announced:
            print("\n  %s>>> Review and approve %s now. <<<%s" % (C["b"], st["url"], C["x"]))
            note("It carries the requester's declaration, the signed zone and the recorded tier move.")
            note("Waiting for the approval (Ctrl-C to stop; re-run with --from review).")
            announced = True
        time.sleep(15)
    ok("approved")
    note("watching checks; usually a few minutes")
    if run(["gh", "pr", "checks", str(st["pr"]), "--repo", GH_REPO, "--watch", "--interval", "30"], check=False, capture=False).returncode != 0:
        raise Stop("a check failed on %s. Fix it on the PR, then re-run with --from review." % st["url"])
    ok("all checks passed")


def require_present_declaration(st):
    """Recheck the source before each step, including resumed steps."""
    path = "ops/ansible/group_vars/%s/external-service-%s.yml" % (st["group"], st["stem"])
    pr = gh_json(["pr", "view", str(st["pr"]), "--repo", GH_REPO,
                  "--json", "state,headRefOid"])
    if pr["state"] not in ("OPEN", "MERGED"):
        raise Stop("Cannot finish a closed, unmerged proposal.")
    # The apply path uses main after merge; the PR branch may already be deleted.
    ref = "main" if pr["state"] == "MERGED" else pr["headRefOid"]
    blob = gh_json(["api", "repos/%s/contents/%s?ref=%s" % (GH_REPO, path, ref)])
    try:
        content = base64.b64decode("".join(blob["content"].split()), validate=True)
        data = yaml.safe_load(content)
    except (KeyError, TypeError, ValueError, AttributeError, yaml.YAMLError):
        raise Stop("Cannot read a valid external-service declaration; nothing can continue.") from None
    if not isinstance(data, dict):
        raise Stop("The external-service declaration must be a mapping.")
    entries = [v for k, v in data.items() if isinstance(k, str) and k.startswith("external_service_")]
    if len(entries) != 1 or not isinstance(entries[0], dict) or entries[0].get("enabled", True) is not True:
        raise Stop("This finisher accepts enabled declarations only. For withdrawal, retain the disabled "
                   "declaration, sign the regenerated DNS snapshot and apply external-service.yml state=absent "
                   "with named zone_record_removal_allow entries. No membership or email steps are needed.")
    if pr["state"] == "OPEN":
        st["checked_head"] = ref
    elif st.get("apply_tree") and "pull" in st.get("done", []):
        local_path = os.path.join(st["apply_tree"], path)
        try:
            with open(local_path, "rb") as fh:
                local_content = fh.read()
        except OSError:
            raise Stop("Saved apply worktree has no readable declaration; restart from pull.") from None
        if local_content != content:
            raise Stop("Saved apply declaration differs from current main; restart from pull.")


# ---- runner ------------------------------------------------------------------
def main():
    global DRY
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("pr", type=int, help="the goat pull request the proposer opened")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--from", dest="start", choices=STEPS)
    ap.add_argument("--email")
    ap.add_argument("--root-key")
    ap.add_argument("--age-identity", default="~/.config/age/keys.txt")
    a = ap.parse_args()
    DRY = a.dry_run
    os.makedirs(STATE_DIR, exist_ok=True)
    state_file = os.path.join(STATE_DIR, "%s.json" % a.pr)
    st = {}
    if os.path.exists(state_file):
        with open(state_file) as fh:
            st = json.load(fh)
    st.setdefault("done", [])
    if a.start:
        st["done"] = [s for s in st["done"] if STEPS.index(s) < STEPS.index(a.start)]
    print("%sFinishing goat PR #%s%s%s" % (C["b"], a.pr, "  (dry run: nothing will change)" if DRY else "", C["x"]))
    handlers = dict(check=do_check, read=do_read, peer=do_peer, prepare=do_prepare, review=do_review, merge=do_merge,
                    pull=do_pull, tier=do_tier, apply=do_apply, verify=do_verify, email=do_email)
    facts = None
    s = "read"
    try:
        if "read" in st["done"]:
            require_present_declaration(st)
        for s in STEPS:
            if s in st["done"]:
                note("[%s] done earlier" % s)
                if s == "read" and facts is None:
                    facts = site_facts(REPO, st["group"])
                continue
            if s in ("check", "read"):
                handlers[s](a, st)
            else:
                # Review can wait while the proposal changes. Recheck before
                # each subsequent action, especially merge, tier and apply.
                if facts is not None:
                    require_present_declaration(st)
                handlers[s](a, st, facts)
            if s == "read":
                require_present_declaration(st)
                facts = site_facts(REPO, st["group"])
                check_site(a, facts)
            if not DRY:
                st["done"].append(s)
                with open(state_file, "w") as fh:
                    json.dump(st, fh, indent=2)
    except Stop as e:
        print("\n%sSTOPPED%s at step: %s\n  %s\n" % (C["r"], C["x"], s, str(e).replace("\n", "\n  ")))
        return 1
    except KeyboardInterrupt:
        print("\nInterrupted at step %s. Re-run to resume, or use --from %s." % (s, s))
        return 130
    if st.get("apply_tree"):
        run(["git", "-C", REPO, "worktree", "remove", "--force", st["apply_tree"]], check=False, write=True)
        run(["git", "-C", REPO, "branch", "-d", "esf-apply-%s" % st["pr"]], check=False, write=True)
    if DRY:
        print("\n%sDry run complete.%s Nothing was changed. Run without --dry-run to do it." % (C["b"], C["x"]))
        return 0
    print("\n%s%sDone.%s %s is live on %s." % (C["b"], C["g"], C["x"], st.get("name"), facts["site_id"] if facts else "?"))
    if os.path.exists(state_file):
        os.remove(state_file)
    return 0


if __name__ == "__main__":
    sys.exit(main())
