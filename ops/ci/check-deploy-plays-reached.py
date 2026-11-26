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

Usage:
  check-deploy-plays-reached.py [--repo DIR]           report; exit 0 (the first step of J28)
  check-deploy-plays-reached.py [--repo DIR] --strict  exit 1 on a play neither reached nor
                                                       listed, or a stale list entry
  check-deploy-plays-reached.py --self-test            prove red, then green

Exit 2 = usage, or a list or pins file that cannot be read.
"""
import argparse
import glob
import os
import re
import sys
import tempfile

import yaml

LIST = "ops/goatbuilder/deploy-plays.yaml"
SCHEMA = "goatbuilder-deploy-plays/v1"
ANSIBLE = "ops/ansible"
PLAY_WORD = re.compile(r"(?<![A-Za-z0-9_./-])([a-z0-9][a-z0-9_-]*\.ya?ml)(?![A-Za-z0-9_-])")
IMPORT = re.compile(r"^\s*-?\s*(?:ansible\.builtin\.)?import_playbook:\s*['\"]?([A-Za-z0-9_./-]+\.ya?ml)", re.M)


GATE_ROW = re.compile(r"\bJ[0-9]+\b")


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


def by_pins(repo):
    """play -> why, for every pins component whose play is found as `deploy` finds it."""
    out = {}
    for rel in ("ops/pins/fleet-pins.yaml", "ops/pins/lite-pins.yaml"):
        doc = load_yaml(os.path.join(repo, rel), {})
        if not isinstance(doc, dict):
            raise Unreadable("%s: not a mapping" % rel)
        for name, spec in (doc.get("components") or {}).items():
            spec = spec or {}
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


def by_source(repo):
    out = {}
    for path in sorted(glob.glob(os.path.join(repo, "crates/goatbuilder/src/**/*.rs"), recursive=True)):
        with open(path, encoding="utf-8") as f:
            text = code_text(f.read())
        rel = os.path.relpath(path, repo)
        for m in PLAY_WORD.finditer(text):
            if play_exists(repo, m.group(1)):
                out.setdefault(m.group(1), "goatbuilder (%s)" % rel)
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


def read_list(repo):
    doc = load_yaml(os.path.join(repo, LIST), None)
    if doc is None:
        raise Unreadable("%s is missing" % LIST)
    if not isinstance(doc, dict) or doc.get("schema") != SCHEMA:
        raise Unreadable("%s: schema is not %s" % (LIST, SCHEMA))
    unknown = sorted(set(doc) - {"schema", "runs_in_part", "unreached", "owed"})
    if unknown:
        raise Unreadable("%s: unknown key %r" % (LIST, unknown[0]))
    lists = {}
    for key in ("runs_in_part", "unreached", "owed"):
        v = doc.get(key) or {}
        if not isinstance(v, dict):
            raise Unreadable("%s: %s is a mapping of play to reason" % (LIST, key))
        for play, why in v.items():
            if not isinstance(why, str) or not why.strip():
                raise Unreadable("%s: %s.%s has no reason" % (LIST, key, play))
        lists[key] = v
    for play, why in lists["owed"].items():
        if not GATE_ROW.search(why):
            raise Unreadable("%s: owed.%s names no gate row (J<n>) that will route it" % (LIST, play))
    both = sorted(set(lists["owed"]) & set(lists["unreached"]))
    if both:
        raise Unreadable("%s: %s is under both unreached: and owed:" % (LIST, both[0]))
    return lists["runs_in_part"], lists["unreached"], lists["owed"]


def judge(repo):
    """(rows, problems): one row per deploy play, and each problem a strict run fails on."""
    in_part, unreached, owed = read_list(repo)
    listed = dict(unreached, **owed)
    reach = reached(repo, in_part)
    plays = deploy_plays(repo)
    rows, problems = [], []
    for play in plays:
        if play in reach:
            rows.append(("reached", play, reach[play]))
            if play in listed:
                problems.append("%s is listed in %s, and %s reaches it: take it off the list" % (play, LIST, reach[play]))
        elif play in owed:
            rows.append(("owed", play, owed[play]))
        elif play in listed:
            rows.append(("listed", play, listed[play]))
        else:
            rows.append(("GAP", play, "no verb reaches it and %s does not list it" % LIST))
            problems.append("%s: no goatbuilder verb reaches it. Give it a component, a phase or a type, or list it in %s: under unreached: with its reason, or under owed: with the gate row that will route it" % (play, LIST))
    for play, why in sorted(listed.items()):
        for m in PLAY_WORD.finditer(why):
            named = m.group(1)
            if not play_exists(repo, named):
                problems.append("%s: the reason for %s names %s, which is not in %s/" % (LIST, play, named, ANSIBLE))
            elif named not in reach:
                problems.append("%s: the reason for %s names %s, which no goatbuilder verb reaches" % (LIST, play, named))
    for play in sorted(set(listed) - set(plays)):
        problems.append("%s lists %s, which is not an ops/ansible/*-deploy.yml play" % (LIST, play))
    for play in sorted(set(in_part)):
        if not play_exists(repo, play):
            problems.append("%s: runs_in_part names %s, which is not in ops/ansible/" % (LIST, play))
    return rows, problems


def report(repo, strict, out=sys.stdout):
    rows, problems = judge(repo)
    counts = {}
    for state, play, why in rows:
        counts[state] = counts.get(state, 0) + 1
        out.write("%-8s %s: %s\n" % (state, play, why))
    out.write("deploy plays: %d; reached %d, listed %d, owed %d, gaps %d\n" % (len(rows), counts.get("reached", 0), counts.get("listed", 0), counts.get("owed", 0), counts.get("GAP", 0)))
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
        "ops/goatbuilder/targets/t.yaml": "type: t\nphases:\n  - plays:\n      - {play: typed-deploy.yml, args: x}\n  - {label: r, component: rel, skip: none}\n  - {label: b, component: beta, skip: none}\n",
        "ops/ansible/beta-deploy.yml": "- hosts: all\n",
        "ops/ansible/rel-deploy.yml": "- hosts: all\n",
        "crates/goatbuilder/src/a.rs": "#[cfg(test)]\nfn helper() {}\nfn f() { run(\"coded-deploy.yml site.yml\"); }\n// tested-deploy.yml\n#[cfg(test)]\nmod tests { const P: &str = \"part-deploy.yml\"; }\n",
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
    check("a type's component whose pins row names another play does not reach <component>-deploy.yml", state.get("beta-deploy.yml") == "GAP", repr(state))
    check("a gap is a problem a strict run fails on", report(d, True, io.StringIO()) == 1 and any("part-deploy.yml" in p for p in problems), repr(problems))
    check("a report run exits 0", report(d, False, io.StringIO()) == 0)
    d = tree(dict(base, **{LIST: "schema: %s\nruns_in_part:\n  site.yml: some tags\nunreached:\n  part-deploy.yml: run by hand for a test\n  beta-deploy.yml: run by hand for a test\n" % SCHEMA}))
    check("a listed gap passes strict", report(d, True, io.StringIO()) == 0)
    d = tree(dict(base, **{LIST: "schema: %s\nruns_in_part:\n  site.yml: some tags\nunreached:\n  part-deploy.yml: x\n  alpha-deploy.yml: x\n  gone-deploy.yml: x\n" % SCHEMA}))
    _, problems = judge(d)
    check("a listed play a verb reaches, and a listed play that is gone, are problems",
          any("alpha-deploy.yml is listed" in p for p in problems) and any("gone-deploy.yml" in p for p in problems), repr(problems))
    d = tree(dict(base, **{LIST: "schema: %s\nruns_in_part:\n  site.yml: some tags\nunreached:\n  part-deploy.yml: its role runs in typed-deploy.yml\n  beta-deploy.yml: run by hand for a test\n" % SCHEMA}))
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
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    try:
        return report(a.repo, a.strict)
    except Unreadable as e:
        sys.stderr.write("check-deploy-plays-reached: %s\n" % e)
        return 2


if __name__ == "__main__":
    sys.exit(main())
