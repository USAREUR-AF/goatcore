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
A row whose code lives in a sibling repository names it under `home:` (goatstuff,
goatrodeo). In a tree without that code (goatcore alone) the row is printed as SKIP and
not judged; in an assembled tree it is checked like any other.
A guard is a line that reads the flag as a decision: a `when:` (inline or as a list item
under one), an `assert` `that:` item, or a Jinja `if` — not a comment, and not a mention in
a message. A service whose code reads no flag at all is a refusal naming the file to add
the guard to and the one-line shape to add.

usage: check-role-honours-presence-flag.py [--repo DIR] [--self-test]
exit 0 every flag is read · 1 a flag no code reads · 2 cannot run
"""
import argparse
import re
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


def check(repo):
    cat = yaml.safe_load((repo / "ops/goatbuilder/endstate/catalog.yaml").read_text()) or {}
    errs, ok, skipped = [], 0, []
    for name, spec in (cat.get("services") or {}).items():
        var = (spec.get("present") or {}).get("var")
        if not var:
            continue
        home = spec.get("home")
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
    for e in skipped:
        print(f"SKIP {e}")
    for e in errs:
        print(f"FAIL {e}")
    if not errs:
        print(f"ok: {ok} catalog services; every presence flag is read by the code that deploys it"
              + (f"; {len(skipped)} deployed from a sibling repository, not in this tree" if skipped else ""))
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
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(r)
        out = buf.getvalue()
        assert rc == 0, out
        assert "SKIP sib: deployed from goatstuff, not in this tree" in out, out
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
        (r / ALLOWLIST).parent.mkdir(parents=True, exist_ok=True)
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
    print("self-test ok: an unread flag is refused; when:, assert that: and an import guard pass; "
          "a row deployed from a sibling repository is skipped only while its code is absent")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--repo", default=str(REPO))
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    return self_test() if a.self_test else check(Path(a.repo))


if __name__ == "__main__":
    sys.exit(main())
