#!/usr/bin/env python3
"""check-deploy-plays-reached.py: every deploy play is reached by a goatbuilder verb, or listed.

Gate row J28 (docs/design/goatbuilder-v1-gate.md; docs/design/goatbuilder-build-targets.md
section 3.9): a play that only a person can reach is a gap. Every ops/ansible/*-deploy.yml
has one of two answers:

  reached  a goatbuilder verb runs it:
             - `goatbuilder deploy <component>`: a row of ops/pins/fleet-pins.yaml (or the
               lite sidecar) whose play is its `deploy_playbook:`, else
               <ledger_component>-deploy.yml, else <component>-deploy.yml (push.rs find_play);
             - `join-node` and `standup`: a play a type file names (ops/goatbuilder/targets/,
               overlay/targets/), <component>-deploy.yml of a release component a type
               file's phase names, or a play goatbuilder's own source names
               (crates/goatbuilder/src, outside its test modules and comments);
             - a play one of those imports (import_playbook), except through a play listed
               in `runs_in_part:`, which a verb runs only some tags of.
  listed   ops/goatbuilder/deploy-plays.yaml names it under `unreached:` with its reason.
  owed     the list names it under `owed:`: a verb should run it and none does yet. Its
           reason names the gate row that will route it (J<n>). An owed play passes
           --strict, and the report counts it apart from the listed ones: the gap is
           tracked, not answered.

A listed play that a verb now reaches, or that no longer exists, is reported too: the
list only holds real gaps. A reason that names an ops/ansible play (such as "its role
runs in site-bootstrap.yml") must name one a verb reaches, so a reason that points to
another route goes stale loudly when that route goes.

The other public repositories. The list's `repositories:` names each one that has
deploy plays, by its URL and a full commit, with its own `unreached:` and `owed:`. Its
ops/ansible/*-deploy.yml plays are read from a checkout at that commit, given with
--sibling NAME=DIR or fetched with --fetch DIR. A play there is reached by a component
of goatcore's own pins whose play, found as `deploy` finds it in a tree that layers that
repository over goatcore, is that repository's; by a component whose `source:` names the
repository and the play; by goatbuilder's own sources naming it where goatcore has no
play of that name (a verb runs it from the tree's ops/ansible/, so in a layered tree it
is that repository's); or by an import of one of those. A declared repository that was
not read, or a checkout at another commit, is a problem.

Usage:
  check-deploy-plays-reached.py [--repo DIR] [--sibling NAME=DIR]... [--fetch DIR]
                                       report; exit 0
  ... --strict                         exit 1 on a play neither reached nor listed, a stale
                                       list entry, or a declared repository not read
  check-deploy-plays-reached.py --self-test            prove red, then green

Exit 2 = usage, or a list or pins file that cannot be read.
"""
import argparse
import glob
import os
import re
import subprocess
import sys
import tempfile

import yaml

LIST = "ops/goatbuilder/deploy-plays.yaml"
SCHEMA = "goatbuilder-deploy-plays/v1"
ANSIBLE = "ops/ansible"
PLAY_WORD = re.compile(r"(?<![A-Za-z0-9_./-])([a-z0-9][a-z0-9_-]*\.ya?ml)(?![A-Za-z0-9_-])")
IMPORT = re.compile(r"^\s*-?\s*(?:ansible\.builtin\.)?import_playbook:\s*['\"]?([A-Za-z0-9_./-]+\.ya?ml)", re.M)


GATE_ROW = re.compile(r"\bJ[0-9]+\b")
REPO_NAME = re.compile(r"^[a-z0-9][a-z0-9._-]*$")
REPO_URL = re.compile(r"^https://github\.com/[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9._-]+$")
FULL_COMMIT = re.compile(r"^[0-9a-f]{40}$")


class Unreadable(Exception):
    pass


def load_yaml(path, default):
    if not os.path.exists(path):
        return default
    try:
        with open(path, encoding="utf-8") as f:
            v = yaml.safe_load(f)
    except (OSError, yaml.YAMLError) as e:
        raise Unreadable("%s: %s" % (path, e))
    return default if v is None else v


def deploy_plays(repo):
    return sorted(os.path.basename(p) for p in glob.glob(os.path.join(repo, ANSIBLE, "*-deploy.yml")))


def play_exists(repo, name):
    return os.path.isfile(os.path.join(repo, ANSIBLE, name))


def pins_components(repo):
    """(name, spec) for each component of the pins file and the lite sidecar."""
    out = []
    for rel in ("ops/pins/fleet-pins.yaml", "ops/pins/lite-pins.yaml"):
        doc = load_yaml(os.path.join(repo, rel), {})
        if not isinstance(doc, dict):
            raise Unreadable("%s: not a mapping" % rel)
        for name, spec in (doc.get("components") or {}).items():
            out.append((name, spec or {}))
    return out


def by_pins(repo):
    """play -> why, for every pins component whose play is found as `deploy` finds it."""
    out = {}
    for name, spec in pins_components(repo):
        declared = spec.get("deploy_playbook") or ""
        if declared:
            candidates = [declared]
        else:
            ledger = spec.get("ledger_component") or name
            candidates = ["%s-deploy.yml" % ledger, "%s-deploy.yml" % name]
        for c in candidates:
            if play_exists(repo, c):
                out.setdefault(c, "goatbuilder deploy %s" % name)
                break
    return out


def pinned_components(repo):
    names = set()
    for rel in ("ops/pins/fleet-pins.yaml", "ops/pins/lite-pins.yaml"):
        doc = load_yaml(os.path.join(repo, rel), {})
        if isinstance(doc, dict):
            names.update((doc.get("components") or {}).keys())
    return names


def by_types(repo):
    out = {}
    pinned = pinned_components(repo)
    for d in ("ops/goatbuilder/targets", "ops/goatbuilder/overlay/targets"):
        for path in sorted(glob.glob(os.path.join(repo, d, "*.yaml"))):
            doc = load_yaml(path, {}) or {}
            for entry in doc.get("phases") or []:
                for p in (entry or {}).get("plays") or []:
                    play = (p or {}).get("play")
                    if play:
                        out.setdefault(play, "join-node and standup (type %s)" % doc.get("type"))
                # A release component's entry: join-node phase 13 runs the component's
                # play where a deployment's pins have a pin of it, found as `deploy`
                # finds it. A row in these pins is by_pins's to judge; with none, a
                # deployment's plain row finds <component>-deploy.yml.
                comp = (entry or {}).get("component")
                if comp and comp not in pinned and play_exists(repo, "%s-deploy.yml" % comp):
                    out.setdefault("%s-deploy.yml" % comp, "join-node phase 13 (type %s, release component %s)" % (doc.get("type"), comp))
    return out


TEST_MODULE = re.compile(r"#\[cfg\(test\)\]\s*\n\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+\w+\s*\{")


def code_text(text):
    """A Rust file without its test module (from `#[cfg(test)] mod ... {` on, where
    the crate keeps it, at the end) and its comment lines."""
    m = TEST_MODULE.search(text)
    if m:
        text = text[:m.start()]
    return "\n".join(line for line in text.splitlines() if not line.lstrip().startswith("//"))


def source_words(repo):
    """(play word, file) for each play name goatbuilder's sources use outside their
    test modules and comments, in file order."""
    out = []
    for path in sorted(glob.glob(os.path.join(repo, "crates/goatbuilder/src/**/*.rs"), recursive=True)):
        with open(path, encoding="utf-8") as f:
            text = code_text(f.read())
        rel = os.path.relpath(path, repo)
        out += [(m.group(1), rel) for m in PLAY_WORD.finditer(text)]
    return out


def by_source(repo):
    out = {}
    for name, rel in source_words(repo):
        if play_exists(repo, name):
            out.setdefault(name, "goatbuilder (%s)" % rel)
    return out


def imports(repo, play):
    try:
        with open(os.path.join(repo, ANSIBLE, play), encoding="utf-8") as f:
            text = f.read()
    except OSError:
        return []
    return [os.path.basename(m) for m in IMPORT.findall(text)]


def reached(repo, in_part):
    direct = {}
    for source in (by_pins(repo), by_types(repo), by_source(repo)):
        for play, why in source.items():
            direct.setdefault(play, why)
    out = dict(direct)
    todo = sorted(direct)
    while todo:
        parent = todo.pop(0)
        if parent in in_part:
            continue
        for child in imports(repo, parent):
            if child not in out and play_exists(repo, child):
                out[child] = "imported by %s (%s)" % (parent, out[parent])
                todo.append(child)
    return out


def check_lists(where, d):
    """The unreached: and owed: mappings of `d`, checked; `where` names them in a refusal."""
    lists = {}
    for key in ("unreached", "owed"):
        v = d.get(key) or {}
        if not isinstance(v, dict):
            raise Unreadable("%s: %s is a mapping of play to reason" % (where, key))
        for play, why in v.items():
            if not isinstance(why, str) or not why.strip():
                raise Unreadable("%s: %s.%s has no reason" % (where, key, play))
        lists[key] = v
    for play, why in lists["owed"].items():
        if not GATE_ROW.search(why):
            raise Unreadable("%s: owed.%s names no gate row (J<n>) that will route it" % (where, play))
    both = sorted(set(lists["owed"]) & set(lists["unreached"]))
    if both:
        raise Unreadable("%s: %s is under both unreached: and owed:" % (where, both[0]))
    return lists["unreached"], lists["owed"]


def read_list(repo):
    """(runs_in_part, unreached, owed, repositories) of the list."""
    doc = load_yaml(os.path.join(repo, LIST), None)
    if doc is None:
        raise Unreadable("%s is missing" % LIST)
    if not isinstance(doc, dict) or doc.get("schema") != SCHEMA:
        raise Unreadable("%s: schema is not %s" % (LIST, SCHEMA))
    unknown = sorted(set(doc) - {"schema", "runs_in_part", "unreached", "owed", "repositories"})
    if unknown:
        raise Unreadable("%s: unknown key %r" % (LIST, unknown[0]))
    in_part = doc.get("runs_in_part") or {}
    if not isinstance(in_part, dict) or any(not isinstance(w, str) or not w.strip() for w in in_part.values()):
        raise Unreadable("%s: runs_in_part is a mapping of play to reason" % LIST)
    unreached, owed = check_lists(LIST, doc)
    repos = doc.get("repositories") or {}
    if not isinstance(repos, dict):
        raise Unreadable("%s: repositories is a mapping of name to url, commit, unreached and owed" % LIST)
    out = {}
    for name, d in repos.items():
        where = "%s: repositories.%s" % (LIST, name)
        if not isinstance(name, str) or not REPO_NAME.match(name) or not isinstance(d, dict):
            raise Unreadable("%s: a mapping of url, commit, unreached and owed under a plain name" % where)
        extra = sorted(set(d) - {"url", "commit", "unreached", "owed"})
        if extra:
            raise Unreadable("%s: unknown key %r" % (where, extra[0]))
        if not isinstance(d.get("url"), str) or not REPO_URL.match(d["url"]):
            raise Unreadable("%s: url is https://github.com/<owner>/<repository>" % where)
        if not isinstance(d.get("commit"), str) or not FULL_COMMIT.match(d["commit"]):
            raise Unreadable("%s: commit is a full commit of 40 hexadecimal characters" % where)
        u, o = check_lists(where, d)
        out[name] = {"url": d["url"], "commit": d["commit"], "unreached": u, "owed": o}
    return in_part, unreached, owed, out


def norm_url(u):
    u = (u or "").strip().lower()
    return u[:-4] if u.endswith(".git") else u


def sibling_reached(repo, url, sdir):
    """play -> why, for each of a sibling repository's ops/ansible plays a goatbuilder
    verb reaches: a pins component (goatcore's own pins) whose play, found as `deploy`
    finds it in a tree that layers this repository, is the sibling's; a component whose
    `source:` names this repository and the play; a play goatbuilder's own sources
    run by name (a verb runs it from the tree's ops/ansible/, so in a tree that layers
    this repository it is the sibling's) where goatcore has no play of that name; or a
    play one of those imports."""
    direct = {}
    for name, rel in source_words(repo):
        if not play_exists(repo, name) and play_exists(sdir, name):
            direct.setdefault(name, "goatbuilder (%s), in a tree that layers this repository" % rel)
    for cname, spec in pins_components(repo):
        src = spec.get("source")
        if isinstance(src, dict):
            play = str(src.get("play") or "")
            if norm_url(src.get("repository")) == norm_url(url) and os.path.dirname(play) == ANSIBLE and play_exists(sdir, os.path.basename(play)):
                direct.setdefault(os.path.basename(play), "goatbuilder deploy %s (its source: at %s)" % (cname, str(src.get("commit"))[:12]))
            continue
        declared = spec.get("deploy_playbook") or ""
        if declared:
            candidates = [declared]
        else:
            ledger = spec.get("ledger_component") or cname
            candidates = ["%s-deploy.yml" % ledger, "%s-deploy.yml" % cname]
        for c in candidates:
            if play_exists(repo, c):
                break
            if play_exists(sdir, c):
                direct.setdefault(c, "goatbuilder deploy %s, in a tree that layers this repository" % cname)
                break
    out = dict(direct)
    todo = sorted(direct)
    while todo:
        parent = todo.pop(0)
        for child in imports(sdir, parent):
            if child not in out and play_exists(sdir, child):
                out[child] = "imported by %s (%s)" % (parent, out[parent])
                todo.append(child)
    return out


def classify(label, plays, reach, unreached, owed, where, named_ok):
    """Rows and problems for one repository's plays. `named_ok(name)` says why a play a
    reason names is not acceptable, or None."""
    listed = dict(unreached, **owed)
    rows, problems = [], []
    for play in plays:
        shown = label + play
        if play in reach:
            rows.append(("reached", shown, reach[play]))
            if play in listed:
                problems.append("%s is listed in %s, and %s reaches it: take it off the list" % (shown, where, reach[play]))
        elif play in owed:
            rows.append(("owed", shown, owed[play]))
        elif play in listed:
            rows.append(("listed", shown, listed[play]))
        else:
            rows.append(("GAP", shown, "no verb reaches it and %s does not list it" % where))
            problems.append("%s: no goatbuilder verb reaches it. Give it a component, a phase or a type, or list it in %s: under unreached: with its reason, or under owed: with the gate row that will route it" % (shown, where))
    for play, why in sorted(listed.items()):
        for m in PLAY_WORD.finditer(why):
            bad = named_ok(m.group(1))
            if bad:
                problems.append("%s: the reason for %s names %s, %s" % (where, label + play, m.group(1), bad))
    for play in sorted(set(listed) - set(plays)):
        problems.append("%s lists %s, which is not an ops/ansible/*-deploy.yml play" % (where, label + play))
    return rows, problems


def git_head(d):
    try:
        return subprocess.run(["git", "-C", d, "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return ""


def fetch(name, url, commit, into):
    """A checkout of `url` at `commit` under `into`/<name>, made or reused."""
    d = os.path.join(into, name)
    if git_head(d) == commit:
        return d
    os.makedirs(d, exist_ok=True)
    for cmd in (["git", "-C", d, "init", "-q"],
                ["git", "-C", d, "fetch", "-q", "--depth", "1", url, commit],
                ["git", "-C", d, "-c", "advice.detachedHead=false", "checkout", "-q", "--force", "FETCH_HEAD"]):
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0:
            raise Unreadable("repositories.%s: %s failed: %s" % (name, " ".join(cmd[3:5]), (r.stderr or r.stdout).strip()))
    return d


def judge(repo, siblings=None):
    """(rows, problems): one row per deploy play, and each problem a strict run fails on.
    `siblings` maps a repository the list declares to its checkout."""
    siblings = siblings or {}
    in_part, unreached, owed, repos = read_list(repo)
    reach = reached(repo, in_part)

    def in_goatcore(named):
        if not play_exists(repo, named):
            return "which is not in %s/" % ANSIBLE
        if named not in reach:
            return "which no goatbuilder verb reaches"
        return None

    rows, problems = classify("", deploy_plays(repo), reach, unreached, owed, LIST, in_goatcore)
    for play in sorted(set(in_part)):
        if not play_exists(repo, play):
            problems.append("%s: runs_in_part names %s, which is not in %s/" % (LIST, play, ANSIBLE))
    for name in sorted(set(siblings) - set(repos)):
        problems.append("--sibling %s: %s declares no such repository" % (name, LIST))
    for name, d in sorted(repos.items()):
        where = "%s (repositories.%s)" % (LIST, name)
        sdir = siblings.get(name)
        if sdir is None:
            rows.append(("NOT READ", name + ":", "pass --sibling %s=<checkout at %s>, or --fetch <dir>" % (name, d["commit"][:12])))
            problems.append("%s: %s was not read: pass --sibling %s=<checkout at %s>, or --fetch <dir>" % (where, name, name, d["commit"]))
            continue
        head = git_head(sdir)
        if head != d["commit"]:
            problems.append("%s: the checkout of %s is at %s, and the list pins %s" % (where, name, head or "no commit", d["commit"]))
        sreach = sibling_reached(repo, d["url"], sdir)

        def in_either(named, sreach=sreach, sdir=sdir):
            if play_exists(sdir, named):
                return None if named in sreach else "which no goatbuilder verb reaches"
            return in_goatcore(named)

        r, p = classify(name + ":", deploy_plays(sdir), sreach, d["unreached"], d["owed"], where, in_either)
        rows += r
        problems += p
    return rows, problems


def report(repo, strict, out=sys.stdout, siblings=None):
    rows, problems = judge(repo, siblings)
    counts = {}
    for state, play, why in rows:
        counts[state] = counts.get(state, 0) + 1
        out.write("%-8s %s: %s\n" % (state, play, why))
    out.write("deploy plays: %d; reached %d, listed %d, owed %d, gaps %d\n" % (
        sum(counts.get(k, 0) for k in ("reached", "listed", "owed", "GAP")), counts.get("reached", 0),
        counts.get("listed", 0), counts.get("owed", 0), counts.get("GAP", 0)))
    if counts.get("NOT READ"):
        out.write("repositories not read: %d\n" % counts["NOT READ"])
    for p in problems:
        out.write(("FAIL " if strict else "REPORT ") + p + "\n")
    return 1 if strict and problems else 0


def self_test():
    import io

    def tree(files):
        d = tempfile.mkdtemp(prefix="deploy-plays-")
        for rel, text in files.items():
            p = os.path.join(d, rel)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "w", encoding="utf-8") as f:
                f.write(text)
        return d

    base = {
        LIST: "schema: %s\nruns_in_part:\n  site.yml: a verb runs some of its tags\nunreached: {}\n" % SCHEMA,
        "ops/pins/fleet-pins.yaml": "components:\n  alpha: {var: a}\n  beta: {var: b, deploy_playbook: beta-play-deploy.yml}\n  gamma: {var: c, ledger_component: gee}\n",
        "ops/ansible/alpha-deploy.yml": "- hosts: all\n",
        "ops/ansible/beta-play-deploy.yml": "- import_playbook: inner-deploy.yml\n",
        "ops/ansible/inner-deploy.yml": "- hosts: all\n",
        "ops/ansible/gee-deploy.yml": "- hosts: all\n",
        "ops/ansible/typed-deploy.yml": "- hosts: all\n",
        "ops/ansible/coded-deploy.yml": "- hosts: all\n",
        "ops/ansible/site.yml": "- ansible.builtin.import_playbook: part-deploy.yml\n",
        "ops/ansible/part-deploy.yml": "- hosts: all\n",
        "ops/goatbuilder/targets/t.yaml": "type: t\nphases:\n  - plays:\n      - {play: typed-deploy.yml, args: x}\n  - {label: r, component: rel, skip: none}\n",
        "ops/ansible/rel-deploy.yml": "- hosts: all\n",
        "crates/goatbuilder/src/a.rs": "#[cfg(test)]\nfn helper() {}\nfn f() { run(\"coded-deploy.yml site.yml\"); run(\"staged-deploy.yml\"); }\n// tested-deploy.yml\n#[cfg(test)]\nmod tests { const P: &str = \"part-deploy.yml\"; }\n",
    }
    failed = 0

    def check(name, cond, detail=""):
        nonlocal failed
        print(("ok   " if cond else "FAIL ") + name + ("" if cond else ": " + detail))
        failed += 0 if cond else 1

    d = tree(base)
    rows, problems = judge(d)
    state = {p: s for s, p, _ in rows}
    check("pins, a declared play, a ledger name, a type file, a type's release component, the source and an import reach",
          all(state.get(p) == "reached" for p in ("alpha-deploy.yml", "beta-play-deploy.yml", "gee-deploy.yml", "typed-deploy.yml", "rel-deploy.yml", "coded-deploy.yml", "inner-deploy.yml")), repr(state))
    check("an import through a play run in part does not reach, nor a test module's word", state.get("part-deploy.yml") == "GAP", repr(state))
    other = tree(dict(base, **{"ops/goatbuilder/targets/u.yaml": "type: u\nphases:\n  - {label: b, component: beta, skip: none}\n", "ops/ansible/beta-deploy.yml": "- hosts: all\n"}))
    other_state = {p: s for s, p, _ in judge(other)[0]}
    check("a type's component whose pins row names another play does not reach <component>-deploy.yml", other_state.get("beta-deploy.yml") == "GAP", repr(other_state))
    check("a gap is a problem a strict run fails on", report(d, True, io.StringIO()) == 1 and any("part-deploy.yml" in p for p in problems), repr(problems))
    check("a report run exits 0", report(d, False, io.StringIO()) == 0)
    d = tree(dict(base, **{LIST: "schema: %s\nruns_in_part:\n  site.yml: some tags\nunreached:\n  part-deploy.yml: run by hand for a test\n" % SCHEMA}))
    check("a listed gap passes strict", report(d, True, io.StringIO()) == 0)
    d = tree(dict(base, **{LIST: "schema: %s\nruns_in_part:\n  site.yml: some tags\nunreached:\n  part-deploy.yml: x\n  alpha-deploy.yml: x\n  gone-deploy.yml: x\n" % SCHEMA}))
    _, problems = judge(d)
    check("a listed play a verb reaches, and a listed play that is gone, are problems",
          any("alpha-deploy.yml is listed" in p for p in problems) and any("gone-deploy.yml" in p for p in problems), repr(problems))
    d = tree(dict(base, **{LIST: "schema: %s\nruns_in_part:\n  site.yml: some tags\nunreached:\n  part-deploy.yml: its role runs in typed-deploy.yml\n" % SCHEMA}))
    check("a reason naming a play a verb reaches passes strict", report(d, True, io.StringIO()) == 0)
    for named, says in (("loose.yml", "no goatbuilder verb reaches"), ("gone.yml", "not in ops/ansible")):
        d = tree(dict(base, **{"ops/ansible/loose.yml": "- hosts: all\n", LIST: "schema: %s\nruns_in_part:\n  site.yml: some tags\nunreached:\n  part-deploy.yml: its role runs in %s\n" % (SCHEMA, named)}))
        _, problems = judge(d)
        check("a reason naming %s is a problem (%s)" % (named, says), any(named in p and says in p for p in problems), repr(problems))
    d = tree(dict(base, **{LIST: "schema: %s\nruns_in_part:\n  site.yml: some tags\nunreached: {}\nowed:\n  part-deploy.yml: J99 routes it\n" % SCHEMA}))
    rows, _ = judge(d)
    check("an owed play passes strict and is counted as owed",
          report(d, True, io.StringIO()) == 0 and ("owed", "part-deploy.yml", "J99 routes it") in rows, repr(rows))
    d = tree(dict(base, **{LIST: "schema: %s\nruns_in_part:\n  site.yml: some tags\nowed:\n  part-deploy.yml: J99\n  alpha-deploy.yml: J99\n" % SCHEMA}))
    _, problems = judge(d)
    check("an owed play a verb reaches is a problem", any("alpha-deploy.yml is listed" in p for p in problems), repr(problems))
    for bad, says in (("schema: %s\nowed:\n  part-deploy.yml: some day\n" % SCHEMA, "names no gate row"),
                      ("schema: %s\nunreached:\n  part-deploy.yml: x\nowed:\n  part-deploy.yml: J9\n" % SCHEMA, "under both")):
        d = tree(dict(base, **{LIST: bad}))
        try:
            judge(d)
            check("a malformed owed list is refused (%s)" % says, False, "accepted")
        except Unreadable as e:
            check("a malformed owed list is refused (%s)" % says, says in str(e), str(e))
    # A sibling repository: layered by name, by `source:`, by import; its own lists.
    sib = tree({
        "ops/ansible/layered-deploy.yml": "- import_playbook: inner-deploy.yml\n",
        "ops/ansible/inner-deploy.yml": "- hosts: all\n",
        "ops/ansible/sourced-deploy.yml": "- hosts: all\n",
        "ops/ansible/demo-deploy.yml": "- hosts: all\n",
        "ops/ansible/alpha-deploy.yml": "- hosts: all\n",
        "ops/ansible/staged-deploy.yml": "- hosts: all\n",
        "ops/ansible/tested-deploy.yml": "- hosts: all\n",
    })
    for cmd in (["init", "-q"], ["add", "-A"], ["-c", "user.name=t", "-c", "user.email=t@example.org", "commit", "-q", "-m", "one"]):
        subprocess.run(["git", "-C", sib] + cmd, check=True, capture_output=True)
    head = git_head(sib)
    url = "https://github.com/example/sib"
    pins = base["ops/pins/fleet-pins.yaml"] + "  layered: {var: l}\n  sourced:\n    var: s\n    source: {repository: %s.git, commit: '%s', play: ops/ansible/sourced-deploy.yml}\n" % (url, head)

    def sib_list(commit, unreached="{}", owed="{}"):
        return "schema: %s\nruns_in_part:\n  site.yml: some tags\nunreached:\n  part-deploy.yml: x\nrepositories:\n  sib:\n    url: %s\n    commit: '%s'\n    unreached: %s\n    owed: %s\n" % (SCHEMA, url, commit, unreached, owed)

    d = tree(dict(base, **{"ops/pins/fleet-pins.yaml": pins, LIST: sib_list(head, owed="{demo-deploy.yml: J98 routes it}")}))
    rows, problems = judge(d, {"sib": sib})
    state = {p: s for s, p, _ in rows}
    check("a sibling's play is reached by a layered name, a source: and an import",
          all(state.get("sib:" + p) == "reached" for p in ("layered-deploy.yml", "sourced-deploy.yml", "inner-deploy.yml")), repr(state))
    check("a sibling's play goatbuilder's sources run by name is reached",
          state.get("sib:staged-deploy.yml") == "reached", repr(state))
    check("a sibling's play named only in a comment or a test module is not reached",
          state.get("sib:tested-deploy.yml") == "GAP", repr(state))
    check("a sibling play goatcore's own play of that name shadows is not reached through it, and its owed play is owed",
          state.get("sib:alpha-deploy.yml") == "GAP" and state.get("sib:demo-deploy.yml") == "owed", repr(state))
    check("a sibling's gap is a problem", any(p.startswith("sib:alpha-deploy.yml") for p in problems), repr(problems))
    d = tree(dict(base, **{"ops/pins/fleet-pins.yaml": pins, LIST: sib_list(head, unreached="{alpha-deploy.yml: its role runs in layered-deploy.yml, tested-deploy.yml: x}", owed="{demo-deploy.yml: J98}")}))
    check("a sibling with every play reached or listed passes strict", report(d, True, io.StringIO(), {"sib": sib}) == 0)
    _, problems = judge(d, {})
    check("a declared repository not read is a problem", any("sib was not read" in p for p in problems), repr(problems))
    d = tree(dict(base, **{"ops/pins/fleet-pins.yaml": pins, LIST: sib_list("0" * 40, unreached="{alpha-deploy.yml: x, tested-deploy.yml: x}", owed="{demo-deploy.yml: J98}")}))
    _, problems = judge(d, {"sib": sib})
    check("a checkout at another commit than the list pins is a problem", any("the list pins " + "0" * 40 in p for p in problems), repr(problems))
    for bad, says in ((sib_list("main"), "full commit"), (sib_list(head).replace(url, "https://example.org/x"), "https://github.com")):
        d = tree(dict(base, **{LIST: bad}))
        try:
            judge(d, {})
            check("a malformed repository is refused (%s)" % says, False, "accepted")
        except Unreadable as e:
            check("a malformed repository is refused (%s)" % says, says in str(e), str(e))
    for bad, says in (("schema: other\n", "schema"), ("schema: %s\nunreached:\n  part-deploy.yml: ''\n" % SCHEMA, "no reason"), ("schema: %s\nextra: 1\n" % SCHEMA, "unknown key")):
        d = tree(dict(base, **{LIST: bad}))
        try:
            judge(d)
            check("a malformed list is refused (%s)" % says, False, "accepted")
        except Unreadable as e:
            check("a malformed list is refused (%s)" % says, says in str(e), str(e))
    return 1 if failed else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    ap.add_argument("--repo", default=os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..")))
    ap.add_argument("--strict", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--sibling", action="append", default=[], metavar="NAME=DIR",
                    help="a checkout of a repository the list declares, at its pinned commit")
    ap.add_argument("--fetch", metavar="DIR",
                    help="fetch each repository the list declares, at its pinned commit, under DIR")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    siblings = {}
    for kv in a.sibling:
        name, sep, d = kv.partition("=")
        if not sep or not name or not d:
            sys.stderr.write("check-deploy-plays-reached: --sibling is NAME=DIR\n")
            return 2
        siblings[name] = os.path.abspath(d)
    try:
        if a.fetch:
            for name, d in sorted(read_list(a.repo)[3].items()):
                if name not in siblings:
                    siblings[name] = fetch(name, d["url"], d["commit"], os.path.abspath(a.fetch))
        return report(a.repo, a.strict, siblings=siblings)
    except Unreadable as e:
        sys.stderr.write("check-deploy-plays-reached: %s\n" % e)
        return 2


if __name__ == "__main__":
    sys.exit(main())
