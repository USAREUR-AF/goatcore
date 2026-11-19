#!/usr/bin/env python3
"""check-role-honours-presence-flag.py — a service a site declares off is never deployed (ADR 1206).

The declared end state reads a service's presence from its flag (`<name>_enabled` in the
site's group_vars, named by `present.var` in ops/goatbuilder/endstate/catalog.yaml). A flag
nothing reads is a declaration with no effect: a play run on a site that declares the
service off would deploy it anyway, and the end state would then report the container as
left over. This gate refuses a catalog service whose flag no deploying code reads as a
guard.

For each catalog service with `present.var`, the guard must appear in one of:
  - the service's role (catalog `role:`, else a role named after the service or its
    ledger), any file under its tasks/;
  - the play that deploys it (catalog `play:`, else `<service>-deploy.yml` /
    `<ledger>-deploy.yml`);
  - the files the catalog row names under `guard:` — then EVERY one of them must read it
    (a service two plays can deploy is guarded only when both do).
ops/ci/presence-flag-allowlist.txt names rows refused today for a known reason (shrink-only:
a listed row that stops failing makes its line stale, and the check refuses it).
A row whose code lives in a sibling repository names it under `home:` (goatstuff,
goatrodeo). In a tree without that code (goatcore alone) the row is printed as SKIP and
not judged; in an assembled tree it is checked like any other. Since a SKIP is an escape
from the check, every `home:` row must also be listed in the allowlist as
`home: <service> <repository>` under a reason; an unlisted one, or one listed with another
repository, is refused, and a listed one whose catalog row has no such `home:` is stale.
Every allowlist entry needs a reason (the comment lines above it). With --base <ref>, the
allowlist is also held to <ref>'s: an entry <ref> does not list is refused (shrink-only).
A guard is a line that reads the flag as a decision: a `when:` (inline or as a list item
under one), an `assert` `that:` item, or a Jinja `if` — not a comment, and not a mention in
a message. A service whose code reads no flag at all is a refusal naming the file to add
the guard to and the one-line shape to add.

usage: check-role-honours-presence-flag.py [--repo DIR] [--base REF] [--self-test]
exit 0 every flag is read · 1 a flag no code reads, or the allowlist is wrong or grew · 2 cannot run
"""
import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    import yaml
except ImportError:
    sys.exit("X-PY-001: check-role-honours-presence-flag.py needs PyYAML (pip install pyyaml)")

REPO = Path(__file__).resolve().parents[2]
ALLOWLIST = "ops/ci/presence-flag-allowlist.txt"
SHAPE = ("- ansible.builtin.import_tasks: deploy.yml\n"
         "  when: {var} | default({default}) | bool")


def candidates(repo, name, spec):
    ans = repo / "ops/ansible"
    files = []
    roles = [spec.get("role")] if spec.get("role") else [name, spec.get("ledger") or ""]
    for r in roles:
        if r and (ans / "roles" / r / "tasks").is_dir():
            files += sorted((ans / "roles" / r / "tasks").glob("*.yml"))
    plays = [spec.get("play")] if spec.get("play") else [f"{name}-deploy.yml", f"{spec.get('ledger')}-deploy.yml"]
    for p in plays:
        if p and (ans / p).is_file():
            files.append(ans / p)
    return files


def guarded(text, var):
    """True when `var` is read as a decision somewhere in the text."""
    v = re.escape(var)
    in_when = False
    for raw in text.splitlines():
        line = raw.split(" #")[0] if not raw.lstrip().startswith("#") else ""
        s = line.strip()
        if not s:
            continue
        if re.match(r"^-?\s*when:\s*$", s) or re.match(r"^-?\s*that:\s*$", s):
            in_when = True
            continue
        if in_when and s.startswith("- "):
            if re.search(rf"\b{v}\b", s):
                return True
            continue
        in_when = False
        if re.match(rf"^-?\s*when:.*\b{v}\b", s):
            return True
        if re.match(rf"^-?\s*that:.*\b{v}\b", s):
            return True
        if re.search(rf"\{{%-?\s*(el)?if\b[^%]*\b{v}\b", s):
            return True
        if re.search(rf"\bif\s+\(?\s*{v}\b", s):   # a Jinja conditional expression
            return True
    return False


def parse_allowlist(text):
    """{key: (service, home, reason)} from the allowlist's text. An entry is `<service>` (a
    known gap) or `home: <service> <repository>` (a reviewed sibling-repository row); its
    reason is the comment lines above it, up to the last blank line. A reason of "" means
    none was given. The key is the entry as written, so a changed repository is a new key."""
    out, reason = {}, []
    for ln in text.splitlines():
        s = ln.strip()
        if not s:
            reason = []
        elif s.startswith("#"):
            reason.append(s.lstrip("#").strip())
        else:
            m = re.match(r"^home:\s+(\S+)\s+(\S+)$", s)
            key = f"home: {m.group(1)} {m.group(2)}" if m else s
            out[key] = (m.group(1) if m else s, m.group(2) if m else None, " ".join(reason))
    return out


def read_allowlist(repo):
    p = repo / ALLOWLIST
    return parse_allowlist(p.read_text()) if p.is_file() else {}


def grown(base, head):
    """Entries of `head` that `base` does not list (the allowlist only shrinks)."""
    return [f"{k}: not in the base's {ALLOWLIST}; the list only shrinks, so fix the row "
            f"instead of listing it" for k in sorted(head) if k not in base]


def against_base(repo, base_ref):
    """0 when the allowlist at `repo` lists nothing `base_ref`'s does not; 1 when it grew;
    2 when the base cannot be read. A base that predates the file passes (this change
    introduces it)."""
    def git(*args):
        return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
    if git("cat-file", "-e", f"{base_ref}^{{commit}}").returncode != 0:
        print(f"cannot compare: {base_ref!r} is not a commit here (fetch the base first)", file=sys.stderr)
        return 2
    listed = git("ls-tree", "--name-only", base_ref, "--", ALLOWLIST)
    if listed.returncode != 0:
        print(f"cannot list {ALLOWLIST} at {base_ref}: {listed.stderr.strip()}", file=sys.stderr)
        return 2
    if not listed.stdout.strip():
        print(f"ok: {base_ref} has no {ALLOWLIST}; this change introduces it")
        return 0
    got = git("show", f"{base_ref}:{ALLOWLIST}")
    if got.returncode != 0:
        print(f"cannot read {ALLOWLIST} at {base_ref}: {got.stderr.strip()}", file=sys.stderr)
        return 2
    problems = grown(parse_allowlist(got.stdout), read_allowlist(repo))
    for p in problems:
        print(f"FAIL {p}")
    if problems:
        return 1
    print(f"ok: {ALLOWLIST} lists nothing {base_ref}'s does not")
    return 0


def check(repo):
    cat = yaml.safe_load((repo / "ops/goatbuilder/endstate/catalog.yaml").read_text()) or {}
    entries = read_allowlist(repo)
    allow = {svc: why for svc, home, why in entries.values() if home is None}
    listed_home = {svc: (home, why) for svc, home, why in entries.values() if home is not None}
    errs, ok, skipped = [], 0, []
    services = cat.get("services") or {}
    # A listed home entry is stale when its catalog row does not say that home.
    for svc, (home, _) in listed_home.items():
        if (services.get(svc) or {}).get("home") != home:
            errs.append(f"{svc}: listed in {ALLOWLIST} as `home: {svc} {home}`, and its catalog row does not "
                        f"say home: {home} (or it is not in the catalog): the line is stale, remove it")
    for name, spec in services.items():
        var = (spec.get("present") or {}).get("var")
        if not var:
            continue
        home = spec.get("home")
        if home and name not in listed_home:
            errs.append(f"{name}: its catalog row says home: {home}, which is not listed in {ALLOWLIST}; "
                        f"a home: row is skipped where its code is absent, so it needs a reviewed entry "
                        f"`home: {name} {home}` with its reason")
            continue
        if home and listed_home[name][0] != home:
            errs.append(f"{name}: its catalog row says home: {home}, and {ALLOWLIST} lists it as "
                        f"`home: {name} {listed_home[name][0]}`; make the two agree")
            continue
        explicit = [repo / g for g in (spec.get("guard") or [])]
        missing = [str(f.relative_to(repo)) for f in explicit if not f.is_file()]
        # Deployed from a sibling repository: judged only where its code is (an assembled
        # tree); in a tree without it, named and skipped.
        if home and ((explicit and len(missing) == len(explicit)) or (not explicit and not candidates(repo, name, spec))):
            skipped.append(f"{name}: deployed from {home}, not in this tree")
            continue
        if missing:
            errs.append(f"{name}: its `guard:` names files that do not exist: {', '.join(missing)}")
            continue
        if explicit:
            bad = [f for f in explicit if not guarded(f.read_text(errors="replace"), var)]
            if bad:
                errs.append(f"{name}: {', '.join(str(f.relative_to(repo)) for f in bad)} deploy it without reading {var}; "
                            f"gate the tasks that start it with `when: {var} | default(true) | bool`")
            else:
                ok += 1
            continue
        files = candidates(repo, name, spec)
        if not files:
            errs.append(f"{name}: no role, play or `guard:` file found for its flag {var}; name one in the catalog row (role:, play: or guard:)")
            continue
        if any(guarded(f.read_text(errors="replace"), var) for f in files):
            ok += 1
            continue
        default = "true" if (spec.get("present") or {}).get("default", True) else "false"
        first = next((f for f in files if f.name == "main.yml"), files[0])
        errs.append(f"{name}: no deploying code reads {var}, so a site that declares it off still gets it deployed. "
                    f"Guard {first.relative_to(repo)}: move its tasks to deploy.yml and make main.yml\n    "
                    + SHAPE.format(var=var, default=default).replace("\n", "\n    "))
    # The shrink-only allowlist: a listed row that fails is a known gap; a listed row that
    # does not fail (or is not in the catalog) makes its line stale.
    known = []
    failing = {e.split(":", 1)[0] for e in errs}
    for name, why in allow.items():
        if name in failing:
            known.append(f"{name}: {why}")
            errs = [e for e in errs if e.split(":", 1)[0] != name]
        else:
            errs.append(f"{name}: listed in {ALLOWLIST}, and the check no longer refuses it "
                        f"(or it is not in the catalog): the line is stale, remove it")
    # Every entry carries a reason (after the known gaps are taken out, so a listed row
    # without one is refused even while it fails).
    for key, (_, _, why) in entries.items():
        if not why:
            errs.append(f"{key.split()[-2] if key.startswith('home:') else key}: the entry `{key}` in {ALLOWLIST} "
                        f"has no reason; write one in a comment line above it")
    for e in skipped:
        print(f"SKIP {e}")
    for e in known:
        print(f"KNOWN {e}")
    for e in errs:
        print(f"FAIL {e}")
    if not errs:
        print(f"ok: {ok} catalog services; every presence flag is read by the code that deploys it"
              + (f"; {len(skipped)} deployed from a sibling repository, not in this tree" if skipped else "")
              + (f"; {len(known)} known gap(s) on the shrink-only allowlist ({ALLOWLIST})" if known else ""))
    return 1 if errs else 0


def self_test():
    with tempfile.TemporaryDirectory() as d:
        r = Path(d)
        (r / "ops/goatbuilder/endstate").mkdir(parents=True)
        (r / "ops/ansible/roles/good/tasks").mkdir(parents=True)
        (r / "ops/ansible/roles/bad/tasks").mkdir(parents=True)
        (r / "ops/ansible/roles/listed/tasks").mkdir(parents=True)
        (r / "ops/goatbuilder/endstate/catalog.yaml").write_text(
            "services:\n"
            "  good: {present: {var: good_enabled}, containers: [good]}\n"
            "  listed: {present: {var: listed_enabled}, containers: [listed]}\n"
            "  bad: {present: {var: bad_enabled}, containers: [bad]}\n")
        (r / "ops/ansible/roles/good/tasks/main.yml").write_text(
            "- ansible.builtin.import_tasks: deploy.yml\n  when: good_enabled | default(true) | bool\n")
        (r / "ops/ansible/roles/listed/tasks/main.yml").write_text(
            "- ansible.builtin.assert:\n    that:\n      - listed_enabled | bool\n")
        (r / "ops/ansible/roles/bad/tasks/main.yml").write_text(
            "# bad_enabled: true turns this on (a comment is not a guard)\n"
            "- ansible.builtin.debug:\n    msg: \"bad_enabled is {{ bad_enabled }}\"\n")
        import io, contextlib
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(r)
        out = buf.getvalue()
        assert rc == 1, out
        assert "bad: no deploying code reads bad_enabled" in out, out
        assert "good:" not in out and "listed:" not in out, out
        (r / "ops/ansible/roles/bad/tasks/main.yml").write_text(
            "- ansible.builtin.meta: end_role\n  when: not (bad_enabled | bool)\n")
        with contextlib.redirect_stdout(io.StringIO()):
            assert check(r) == 0
        # A row whose code lives in a sibling repository (`home:`): absent from this tree it
        # is named and skipped; present (an assembled tree) it is checked like any other; a
        # row with no home and no code is still refused.
        (r / "ops/goatbuilder/endstate/catalog.yaml").write_text(
            "services:\n"
            "  good: {present: {var: good_enabled}, containers: [good]}\n"
            "  sib: {present: {var: sib_enabled}, home: goatstuff, containers: [sib]}\n")
        # A `home:` row is an escape from the check, so it must be listed, with its repository
        # and a reason, in the allowlist's reviewed home entries. Unlisted, it is refused.
        (r / ALLOWLIST).parent.mkdir(parents=True, exist_ok=True)
        (r / ALLOWLIST).write_text("")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(r)
        out = buf.getvalue()
        assert rc == 1 and "sib: its catalog row says home: goatstuff" in out and "not listed" in out, out
        # Listed with another repository: refused, naming both.
        (r / ALLOWLIST).write_text("# deployed by goatrodeo's play\nhome: sib goatrodeo\n")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(r)
        out = buf.getvalue()
        assert rc == 1 and "sib: its catalog row says home: goatstuff" in out and "goatrodeo" in out, out
        # Listed with no reason: refused.
        (r / ALLOWLIST).write_text("home: sib goatstuff\n")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(r)
        out = buf.getvalue()
        assert rc == 1 and "home: sib goatstuff" in out and "no reason" in out, out
        # Listed, with its repository and a reason: skipped by name while its code is absent.
        (r / ALLOWLIST).write_text("# deployed by goatstuff's sib role\nhome: sib goatstuff\n")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(r)
        out = buf.getvalue()
        assert rc == 0, out
        assert "SKIP sib: deployed from goatstuff, not in this tree" in out, out
        # A listed home entry whose catalog row has no `home:` (or is gone) is stale.
        (r / ALLOWLIST).write_text("# deployed by goatstuff's sib role\nhome: sib goatstuff\n"
                                   "# moved\nhome: good goatstuff\n")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(r)
        out = buf.getvalue()
        assert rc == 1 and "good: listed in" in out and "stale" in out, out
        (r / ALLOWLIST).write_text("# deployed by goatstuff's sib role\nhome: sib goatstuff\n")
        (r / "ops/ansible/roles/sib/tasks").mkdir(parents=True)
        (r / "ops/ansible/roles/sib/tasks/main.yml").write_text("- ansible.builtin.debug:\n    msg: up\n")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(r)
        out = buf.getvalue()
        assert rc == 1 and "sib: no deploying code reads sib_enabled" in out, out
        (r / "ops/goatbuilder/endstate/catalog.yaml").write_text(
            "services:\n  orphan: {present: {var: orphan_enabled}, containers: [orphan]}\n")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(r)
        assert rc == 1 and "orphan: no role, play" in buf.getvalue(), buf.getvalue()
        # The shrink-only allowlist: a listed row that fails is reported as known, with the
        # reason; a listed row that no longer fails, or is gone, is a stale line and refused.
        (r / ALLOWLIST).write_text("orphan\n")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(r)
        out = buf.getvalue()
        assert rc == 1 and "orphan" in out and "no reason" in out, out
        (r / ALLOWLIST).write_text("# deploying code not found; owner unknown\norphan\n")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(r)
        out = buf.getvalue()
        assert rc == 0 and "KNOWN orphan: deploying code not found; owner unknown" in out, out
        (r / "ops/ansible/roles/orphan/tasks").mkdir(parents=True)
        (r / "ops/ansible/roles/orphan/tasks/main.yml").write_text(
            "- ansible.builtin.import_tasks: deploy.yml\n  when: orphan_enabled | default(true) | bool\n")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(r)
        out = buf.getvalue()
        assert rc == 1 and "orphan: listed in" in out and "stale" in out, out
        (r / ALLOWLIST).write_text("# gone\nnot-in-the-catalog\n")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(r)
        assert rc == 1 and "not-in-the-catalog: listed in" in buf.getvalue(), buf.getvalue()
    # Shrink-only against the base: an entry the base does not list is refused, a removed
    # one passes; a base that is not a commit fails closed.
    base = parse_allowlist("# a\nbrief\n# b\nhome: herd goatstuff\nhome: gym goatstuff\n")
    assert grown(base, parse_allowlist("# a\nbrief\n# b\nhome: herd goatstuff\n")) == [], "removing passes"
    assert len(grown(base, parse_allowlist("# a\nbrief\n# c\nnew\n"))) == 1, "a new known gap fails"
    assert len(grown(base, parse_allowlist("# b\nhome: herd goatstuff\nhome: brief goatstuff\n"))) == 1, \
        "a new home entry fails"
    assert len(grown(base, parse_allowlist("# b\nhome: herd goatrodeo\n"))) == 1, "a home entry's repository changing fails"
    assert against_base(REPO, "no-such-ref") == 2, "a ref that does not exist"
    assert against_base(REPO, "0" * 40) == 2, "an all-zero sha"
    print("self-test ok: an unread flag is refused; when:, assert that: and an import guard pass; "
          "a row deployed from a sibling repository is skipped only while its code is absent and only when the "
          "allowlist lists it with its repository; every entry needs a reason; the allowlist only shrinks against the base")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--repo", default=str(REPO))
    ap.add_argument("--base", help="also refuse an allowlist entry this git ref does not list")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    rc = check(Path(a.repo))
    if a.base:
        rc = max(rc, against_base(Path(a.repo), a.base))
    return rc


if __name__ == "__main__":
    sys.exit(main())
