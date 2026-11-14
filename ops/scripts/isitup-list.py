#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""isitup-list.py -- keep a goatnet's status-page row list current with its declaration.

The status page (isitup) probes the rows in services.env and describes them
from data/meta.json. Both are rendered from the goatnet's row declaration
(isitup_services, the off-mesh catalog, the external-service declarations) by
the isitup role. This tool answers three questions and runs one play:

  isitup-list.py changed --base <commit> [--head <commit>] [--repo DIR]
      Which goatnets' row declaration did a change touch? Prints one goatnet
      per line. Reads `git diff --name-only` of DIR (default: this tree) and
      the goatnets this tree carries. See "What counts as a change" below.

  isitup-list.py render <goatnet> [--json]
      What does this tree declare? Renders the row list on this machine, with
      the same task file the deploy uses, and prints its digest and row ids.
      Contacts no host.

  isitup-list.py check <goatnet> --meta <file | URL | ->
      Is a status page behind the declaration? Compares the digest stamped in
      a data/meta.json with the digest of the list this tree declares.
        exit 0  the page carries the declared list
        exit 1  the lists differ; the rows that differ are named
        exit 3  the meta.json has no stamp (written before the stamp existed)
      `-` reads the meta.json from standard input. A URL is fetched with any
      --header 'Name: value' given (the page sits behind the goatnet sign-in).

  isitup-list.py apply [<goatnet> ...] [--base <commit> [--head <commit>] [--repo DIR]]
                       [--verify-only] [--dry-run]
      Runs the list-only play (ops/ansible/isitup-rows.yml) for the goatnets
      named, or for the goatnets `changed` names when --base is given. The play
      needs no secret. --verify-only runs it with isitup_rows_verify_only: it
      reads the stamp on the host, writes nothing, and fails when the host is
      behind. --dry-run prints each command and runs nothing.

What counts as a change (changed, apply --base)
    ops/ansible/group_vars/<site>_hosts/**            that goatnet
    ops/ansible/group_vars/all.yml, all/**            every goatnet (the catalog)
    ops/ansible/roles/isitup/templates/isitup-rows.json.j2,
      services.env.j2, meta.json.j2                   every goatnet
    goatcore.ref, goatstuff.ref                       every goatnet (an overlay
                                                      moved the public tree)
    Only goatnets whose group_vars set isitup_enabled: true are named. The rule
    is by path, so it also names a goatnet whose change left the rows as they
    were; the play is idempotent and such a run changes nothing.

A goatnet is written either way (zagros-prod or zagros_prod); its mgmt group is
<goatnet>_hosts. Design: docs/design/isitup-status-page.md, "Keeping the list
current".
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.request

REPO = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
ME = "isitup-list"

GROUP_FILE = re.compile(r"^ops/ansible/group_vars/([a-z0-9_]+)_hosts(?:/|\.ya?ml$)")
EVERY_GOATNET = re.compile(
    r"^(?:ops/ansible/group_vars/all(?:\.ya?ml$|/)"
    r"|ops/ansible/roles/isitup/templates/(?:isitup-rows\.json|services\.env|meta\.json)\.j2$"
    r"|(?:goatcore|goatstuff)\.ref$)"
)


class Stop(Exception):
    """The command cannot continue; the message says why."""


def group_of(goatnet):
    return goatnet.replace("-", "_") + "_hosts"


def name_of(group):
    return group[: -len("_hosts")].replace("_", "-")


# ---- which goatnets this tree carries ----------------------------------------
def isitup_goatnets(repo):
    """Groups (<site>_hosts) whose group_vars set isitup_enabled true, sorted."""
    try:
        import yaml
    except ImportError:
        raise Stop("PyYAML is required to read group_vars (python3 -m pip install pyyaml)")
    base = os.path.join(repo, "ops", "ansible", "group_vars")
    out = []
    for entry in sorted(os.listdir(base)) if os.path.isdir(base) else []:
        stem = re.sub(r"\.ya?ml$", "", entry)
        if not stem.endswith("_hosts"):
            continue
        path = os.path.join(base, entry)
        files = [path] if os.path.isfile(path) else sorted(
            os.path.join(path, f) for f in os.listdir(path) if re.search(r"\.ya?ml$", f))
        enabled = False
        for f in files:
            try:
                with open(f, encoding="utf-8") as fh:
                    data = yaml.safe_load(fh) or {}
            except (OSError, yaml.YAMLError):
                continue
            if isinstance(data, dict) and "isitup_enabled" in data:
                enabled = str(data["isitup_enabled"]).strip().lower() in ("true", "yes", "1", "on")
        if enabled and stem not in out:
            out.append(stem)
    return out


# ---- changed ------------------------------------------------------------------
def groups_touched(paths, known):
    """The groups among `known` whose row declaration the changed paths can reach."""
    hit = set()
    for p in paths:
        if EVERY_GOATNET.match(p):
            return list(known)
        m = GROUP_FILE.match(p)
        if m and (m.group(1) + "_hosts") in known:
            hit.add(m.group(1) + "_hosts")
    return [g for g in known if g in hit]


def changed_paths(repo, base, head):
    r = subprocess.run(["git", "-C", repo, "diff", "--name-only", base, head],
                       capture_output=True, text=True)
    if r.returncode != 0:
        raise Stop("git diff %s %s failed in %s: %s" % (base, head, repo, r.stderr.strip()))
    return [line for line in r.stdout.splitlines() if line]


def changed(a):
    paths = changed_paths(os.path.abspath(a.repo or REPO), a.base, a.head)
    return groups_touched(paths, isitup_goatnets(REPO))


# ---- render -------------------------------------------------------------------
def render(group, inventories=None):
    """Render the group's row list on this machine. Returns {sha256, source, rows}."""
    if not shutil.which("ansible-playbook"):
        raise Stop("ansible-playbook is not on PATH; the row list is rendered by the isitup role")
    work = tempfile.mkdtemp(prefix="isitup-list-")
    try:
        inv = []
        for i in inventories or []:
            inv += ["-i", os.path.abspath(i)]
        if not inv:
            # A one-host inventory of our own: the group's group_vars beside the
            # playbooks apply to it, and there is no remote host to reach.
            path = os.path.join(work, "inventory.ini")
            with open(path, "w", encoding="utf-8") as fh:
                fh.write("[%s]\nisitup-list-render ansible_connection=local\n" % group)
            inv = ["-i", path]
        out = os.path.join(work, "out")
        os.mkdir(out)
        cmd = ["ansible-playbook"] + inv + [
            "-e", "target=%s" % group, "-e", "isitup_rows_out=%s" % out,
            "-e", "ansible_connection=local", "isitup-rows-render.yml"]
        env = dict(os.environ, ANSIBLE_NOCOLOR="1", OBJC_DISABLE_INITIALIZE_FORK_SAFETY="YES")
        r = subprocess.run(cmd, cwd=os.path.join(REPO, "ops", "ansible"),
                           capture_output=True, text=True, env=env)
        files = sorted(os.listdir(out))
        if r.returncode != 0 or not files:
            tail = "\n".join((r.stdout + r.stderr).strip().splitlines()[-25:])
            raise Stop("could not render the row list for %s (is it a group with isitup_enabled: true "
                       "in ops/ansible/group_vars?):\n%s" % (group, tail))
        renders = []
        for f in files:
            with open(os.path.join(out, f), encoding="utf-8") as fh:
                renders.append(json.load(fh))
        digests = sorted({x["sha256"] for x in renders})
        if len(digests) > 1:
            raise Stop("the hosts of %s render different row lists (%s); name one inventory host per run"
                       % (group, ", ".join("%s=%s" % (x["host"], x["sha256"][:12]) for x in renders)))
        return renders[0]
    finally:
        shutil.rmtree(work, ignore_errors=True)


# ---- check --------------------------------------------------------------------
def load_meta(where, headers=()):
    if where == "-":
        text = sys.stdin.read()
    elif re.match(r"^https?://", where):
        req = urllib.request.Request(where)
        for h in headers:
            if ":" not in h:
                raise Stop("--header takes 'Name: value', got %r" % h)
            k, v = h.split(":", 1)
            req.add_header(k.strip(), v.strip())
        try:
            with urllib.request.urlopen(req, timeout=20) as resp:
                text = resp.read().decode("utf-8")
        except OSError as e:
            raise Stop("could not fetch %s: %s" % (where, e))
    else:
        try:
            with open(where, encoding="utf-8") as fh:
                text = fh.read()
        except OSError as e:
            raise Stop("could not read %s: %s" % (where, e))
    try:
        meta = json.loads(text)
    except ValueError:
        raise Stop("%s is not JSON (a sign-in page, if the URL sits behind the goatnet sign-in)" % where)
    if not isinstance(meta, dict):
        raise Stop("%s is not a meta.json object" % where)
    return meta


def source_commit(source):
    """The commit that holds the row declaration: the overlay's for an assembled tree."""
    if not isinstance(source, dict):
        return ""
    return str(source.get("overlay") or source.get("commit") or source.get("goatcore") or "")


def compare(goatnet, declared, meta):
    """(exit code, [lines]) for a rendered declaration against a page's meta.json."""
    stamp = meta.get("rendered") if isinstance(meta.get("rendered"), dict) else {}
    want = declared["sha256"]
    ids = [r["id"] for r in declared["rows"]]
    on_page = sorted((meta.get("rows") or {}).keys()) if isinstance(meta.get("rows"), dict) else []
    missing = [i for i in ids if i not in on_page]
    extra = [i for i in on_page if i not in ids]
    lines = []
    if not stamp.get("sha256"):
        lines.append("%s: UNKNOWN. The meta.json carries no list stamp: it was written before the stamp "
                     "existed, so its list cannot be compared." % goatnet)
        code = 3
    elif stamp["sha256"] == want:
        return 0, ["%s: current. The page carries the declared list (%s, %d rows), rendered from %s at %s."
                   % (goatnet, want[:12], len(ids), source_commit(stamp.get("source"))[:12] or "an unrecorded commit",
                      stamp.get("at", "an unrecorded time"))]
    else:
        lines.append("%s: BEHIND. The page's row list is not the list this tree declares." % goatnet)
        lines.append("  page:     %s, rendered from %s at %s"
                     % (stamp["sha256"][:12], source_commit(stamp.get("source"))[:12] or "an unrecorded commit",
                        stamp.get("at", "an unrecorded time")))
        code = 1
    lines.append("  declared: %s, %d rows, from %s"
                 % (want[:12], len(ids), source_commit(declared.get("source"))[:12] or "this tree"))
    if extra:
        lines.append("  on the page, not declared (probed, and shown as down once the service is gone): %s"
                     % ", ".join(extra))
    if missing:
        lines.append("  declared, not on the page: %s" % ", ".join(missing))
    if code == 1 and not extra and not missing:
        lines.append("  the same row ids; a field of one or more rows differs")
    lines.append("  refresh it: ops/scripts/isitup-list.py apply %s" % goatnet)
    return code, lines


# ---- apply --------------------------------------------------------------------
def declared_setting(section, key):
    r = subprocess.run([os.path.join(REPO, "ops", "scripts", "project-setting.sh"), "--repo", REPO, section, key],
                       capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else ""


def transport(group):
    """(kind, inventory flags) that reach the goatnet from this machine: the mesh
    when this machine is on the goatnet's NetBird profile, else its env door
    (docs/design/env-doors.md), else the base inventory. mgmt-hosts.ini loads
    first in every case, as in ops/scripts/deploy-component.sh."""
    name = name_of(group)
    base = ["-i", "inventory/mgmt-hosts.ini"]
    st = subprocess.run(["netbird", "status"], capture_output=True, text=True) if shutil.which("netbird") else None
    out = st.stdout if st else ""
    prof = re.search(r"^Profile:\s*(\S+)", out, re.M)
    mgmt = re.search(r"^Management:\s*(\S+)", out, re.M)
    if prof and mgmt and prof.group(1) == name and mgmt.group(1) == "Connected":
        return "mesh", base + ["-i", "inventory/mgmt-hosts-mesh.ini"]
    door = subprocess.run([os.path.join(REPO, "ops", "scripts", "env-door.sh"), name, "--print"],
                          capture_output=True, text=True)
    if door.returncode == 0:
        return "door", base + ["-i", "inventory/doors-groupvars.ini"]
    return "direct", base


def apply_one(group, verify_only, dry):
    name = name_of(group)
    kind, inv = transport(group)
    cmd = ["../scripts/ansible-preflight.sh"] + inv + ["-e", "target=%s" % group]
    if verify_only:
        cmd += ["-e", "isitup_rows_verify_only=true"]
    cmd.append("isitup-rows.yml")
    if kind == "door":
        # A door runs over AWS Session Manager, so the play needs the AWS
        # profile; mesh and direct reach the host over plain SSH and need none.
        profile = os.environ.get("GOAT_AWS_PROFILE") or declared_setting("deploy", "aws_profile") or "default"
        cmd = ["../scripts/with-aws", profile, "--"] + cmd
    print("%s: %s via %s: (cd ops/ansible && %s)" % (ME, name, kind, " ".join(cmd)))
    if dry:
        return 0
    if kind == "door":
        opened = subprocess.run([os.path.join(REPO, "ops", "scripts", "door-open.sh"), name])
        if opened.returncode != 0:
            print("%s: %s: the door did not open" % (ME, name), file=sys.stderr)
            return 1
    env = dict(os.environ, OBJC_DISABLE_INITIALIZE_FORK_SAFETY="YES")
    return subprocess.run(cmd, cwd=os.path.join(REPO, "ops", "ansible"), env=env).returncode


# ---- main ---------------------------------------------------------------------
def main(argv=None):
    ap = argparse.ArgumentParser(prog="isitup-list.py", description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    def diff_args(p, required):
        p.add_argument("--base", required=required, help="the commit before the change")
        p.add_argument("--head", default="HEAD", help="the commit after it (default HEAD)")
        p.add_argument("--repo", help="the git checkout to diff (default: this tree); the overlay's own "
                                      "clone when this tree is an assembled one")

    p = sub.add_parser("changed", help="goatnets whose row declaration a change touched")
    diff_args(p, True)

    p = sub.add_parser("render", help="the row list this tree declares for a goatnet")
    p.add_argument("goatnet")
    p.add_argument("--inventory", "-i", action="append", help="an inventory to read the group's hosts and "
                   "variables from (repeatable); default is a one-host local inventory")
    p.add_argument("--json", action="store_true", help="print the whole render as JSON")

    p = sub.add_parser("check", help="compare a page's meta.json with the declared list")
    p.add_argument("goatnet")
    p.add_argument("--meta", required=True, help="data/meta.json as a file path, a URL, or - for stdin")
    p.add_argument("--header", action="append", default=[], help="'Name: value' sent with a URL fetch (repeatable)")
    p.add_argument("--inventory", "-i", action="append")

    p = sub.add_parser("apply", help="run the list-only play for goatnets")
    p.add_argument("goatnets", nargs="*")
    diff_args(p, False)
    p.add_argument("--verify-only", action="store_true")
    p.add_argument("--dry-run", action="store_true")

    a = ap.parse_args(argv)
    try:
        if a.cmd == "changed":
            for g in changed(a):
                print(name_of(g))
            return 0
        if a.cmd == "render":
            d = render(group_of(a.goatnet), a.inventory)
            if a.json:
                print(json.dumps(d, indent=2, sort_keys=True))
            else:
                print("%s %s" % (d["sha256"], " ".join(r["id"] for r in d["rows"])))
            return 0
        if a.cmd == "check":
            meta = load_meta(a.meta, a.header)
            code, lines = compare(a.goatnet, render(group_of(a.goatnet), a.inventory), meta)
            print("\n".join(lines), file=sys.stdout if code == 0 else sys.stderr)
            return code
        if a.cmd == "apply":
            groups = [group_of(g) for g in a.goatnets]
            if a.base:
                groups += [g for g in changed(a) if g not in groups]
            if not groups:
                if a.base:
                    print("%s: no goatnet's row declaration changed between %s and %s" % (ME, a.base, a.head))
                    return 0
                raise Stop("name a goatnet, or give --base <commit> to take the goatnets a change touched")
            rc = 0
            for g in groups:
                if apply_one(g, a.verify_only, a.dry_run) != 0:
                    print("%s: %s: the list-only play failed" % (ME, name_of(g)), file=sys.stderr)
                    rc = 1
            return rc
    except Stop as e:
        print("%s: %s" % (ME, e), file=sys.stderr)
        return 2
    return 2


if __name__ == "__main__":
    sys.exit(main())
