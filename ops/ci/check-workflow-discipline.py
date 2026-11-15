#!/usr/bin/env python3
"""Every workflow under .github/workflows/ keeps the four CI-discipline rules.

Rules (docs/design/ci-discipline.md):

  R1  every job that runs steps declares `timeout-minutes` (a hung job
      otherwise holds a runner for six hours);
  R2  every workflow triggered by pull_request, pull_request_target or push
      declares a `concurrency` group, on the workflow or on every job (a
      superseded push is cancelled, not finished);
  R3  no schedule fires more than once an hour (the minute field of every
      cron is one literal minute: `17 6 * * *`, never `*/5` or `2-59/5`);
  R4  a pull_request-triggered workflow with no `paths:` / `paths-ignore:`
      filter is listed in ops/ci/unfiltered-pr-workflows.txt with its reason,
      and every listed file still exists (the list shrinks, it never rots).
  R5  every GitHub Actions cache export (`cache-to: type=gha...`, or a
      `--cache-to type=gha...` argument) carries `ignore-error=true`, so a
      cache blob the service lost (404 BlobNotFound) cannot fail the build.
      A workflow not yet changed is listed in ops/ci/gha-cache-export-
      exceptions.txt with its reason; a listed file that complies, or that
      no longer exists, fails (the list only shrinks).

Usage:
  ops/ci/check-workflow-discipline.py [ROOT]     # exit 1 on any violation
  ops/ci/check-workflow-discipline.py --self-test
"""

from __future__ import annotations

import os
import re
import sys
import tempfile
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover
    print("needs PyYAML: pip install pyyaml", file=sys.stderr)
    sys.exit(2)

WORKFLOW_DIR = ".github/workflows"
ALLOWLIST = "ops/ci/unfiltered-pr-workflows.txt"
CACHE_EXCEPTIONS = "ops/ci/gha-cache-export-exceptions.txt"
# a cache export to the GitHub Actions cache: the with: key or the buildx argument, then type=gha
CACHE_TO_GHA = re.compile(r"(?:^|\s|-)cache-to(?::|=|\s)\s*['\"]?type=gha\b")
PR_EVENTS = ("pull_request", "pull_request_target")
CONCURRENCY_EVENTS = PR_EVENTS + ("push",)
MINUTE = re.compile(r"^(?:[0-9]|[1-5][0-9])$")


def triggers(doc: dict) -> dict:
    on = doc.get("on")
    if on is None:
        on = doc.get(True)  # YAML 1.1 reads a bare `on:` key as boolean True
    if on is None:
        return {}
    if isinstance(on, str):
        return {on: None}
    if isinstance(on, list):
        return {k: None for k in on}
    return dict(on)


def read_allowlist(path: Path) -> dict[str, str]:
    entries: dict[str, str] = {}
    if not path.exists():
        return entries
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        name, _, reason = line.partition("#")
        entries[name.strip()] = reason.strip()
    return entries


def check(root: Path) -> list[str]:
    problems: list[str] = []
    wf_dir = root / WORKFLOW_DIR
    allow = read_allowlist(root / ALLOWLIST)
    cache_exc = read_allowlist(root / CACHE_EXCEPTIONS)
    seen: set[str] = set()

    for path in sorted(wf_dir.glob("*.yml")) + sorted(wf_dir.glob("*.yaml")):
        name = path.name
        seen.add(name)
        try:
            doc = yaml.safe_load(path.read_text()) or {}
        except yaml.YAMLError as exc:
            problems.append(f"{name}: does not parse as YAML ({exc})")
            continue
        on = triggers(doc)
        jobs = doc.get("jobs") or {}

        # R1 -- timeouts
        for job_name, job in jobs.items():
            if not isinstance(job, dict) or "uses" in job:
                continue
            if "timeout-minutes" not in job:
                problems.append(f"{name}: job '{job_name}' has no timeout-minutes (R1)")

        # R2 -- concurrency on PR/push-triggered workflows
        if any(ev in on for ev in CONCURRENCY_EVENTS) and "concurrency" not in doc:
            # A group declared on every job (a workflow whose jobs must cancel
            # independently) satisfies the rule too.
            per_job = [isinstance(j, dict) and "concurrency" in j for j in jobs.values()]
            if not (per_job and all(per_job)):
                problems.append(f"{name}: triggered by pull_request/push but declares no concurrency group (R2)")

        # R3 -- cron floor
        sched = on.get("schedule") or []
        for entry in sched:
            cron = (entry or {}).get("cron", "") if isinstance(entry, dict) else ""
            fields = cron.split()
            if len(fields) != 5 or not MINUTE.match(fields[0]):
                problems.append(f"{name}: cron '{cron}' fires more than once an hour (R3)")

        # R4 -- unfiltered PR workflows must be listed, with a reason
        for ev in PR_EVENTS:
            if ev not in on:
                continue
            cfg = on.get(ev) or {}
            filtered = isinstance(cfg, dict) and ("paths" in cfg or "paths-ignore" in cfg)
            if filtered:
                continue
            if name not in allow:
                problems.append(
                    f"{name}: runs on every {ev} with no paths filter and is not listed in {ALLOWLIST} (R4)"
                )
            elif not allow[name]:
                problems.append(f"{name}: listed in {ALLOWLIST} without a reason (R4)")

        # R5 -- a lost cache blob must not fail the build
        exposed = [n for n, line in enumerate(path.read_text().splitlines(), 1)
                   if not line.lstrip().startswith("#") and CACHE_TO_GHA.search(line)
                   and "ignore-error=true" not in line]
        if name in cache_exc:
            if not cache_exc[name]:
                problems.append(f"{name}: listed in {CACHE_EXCEPTIONS} without a reason (R5)")
            elif not exposed:
                problems.append(f"{CACHE_EXCEPTIONS}: '{name}' sets ignore-error=true on every cache export now; remove the line (R5)")
        else:
            for n in exposed:
                problems.append(f"{name}:{n}: cache-to type=gha without ignore-error=true, so a lost cache blob fails the build (R5)")

    for listed in cache_exc:
        if listed not in seen:
            problems.append(f"{CACHE_EXCEPTIONS}: '{listed}' names no workflow file; remove the line (R5)")

    for listed in allow:
        if listed not in seen:
            problems.append(f"{ALLOWLIST}: '{listed}' names no workflow file; remove the line (R4)")
    return problems


FIXTURE_GOOD = """\
name: good
on:
  pull_request:
    paths: ['ops/**']
  schedule:
    - cron: '17 6 * * *'
concurrency:
  group: g-${{ github.ref }}
  cancel-in-progress: true
jobs:
  a:
    runs-on: ubuntu-24.04
    timeout-minutes: 5
    steps:
      - uses: docker/build-push-action@v6
        with:
          cache-from: type=gha,scope=good
          cache-to: type=gha,mode=max,scope=good,ignore-error=true
      - run: docker buildx build --cache-to type=gha,mode=max,ignore-error=true .
"""

FIXTURE_BAD = """\
name: bad
on:
  pull_request:
  schedule:
    - cron: '2-59/5 * * * *'
jobs:
  a:
    runs-on: ubuntu-24.04
    steps:
      - uses: docker/build-push-action@v6
        with:
          cache-from: type=gha,scope=bad
          cache-to: type=gha,mode=max,scope=bad
      - run: docker buildx build --cache-to type=gha,mode=max .
"""

# listed as an exception, but it complies: the line must go
FIXTURE_FIXED = FIXTURE_GOOD.replace("name: good", "name: fixed")


def self_test() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / WORKFLOW_DIR).mkdir(parents=True)
        (root / "ops/ci").mkdir(parents=True)
        (root / WORKFLOW_DIR / "good.yml").write_text(FIXTURE_GOOD)
        (root / WORKFLOW_DIR / "bad.yml").write_text(FIXTURE_BAD)
        (root / WORKFLOW_DIR / "fixed.yml").write_text(FIXTURE_FIXED)
        (root / ALLOWLIST).write_text("gone.yml  # a file that no longer exists\n")
        (root / CACHE_EXCEPTIONS).write_text("fixed.yml  # complies now\ngone.yml  # a file that no longer exists\n")
        found = check(root)
        want = {
            "R1": "bad.yml: job 'a' has no timeout-minutes (R1)",
            "R2": "bad.yml: triggered by pull_request/push but declares no concurrency group (R2)",
            "R3": "bad.yml: cron '2-59/5 * * * *' fires more than once an hour (R3)",
            "R4-unlisted": f"bad.yml: runs on every pull_request with no paths filter and is not listed in {ALLOWLIST} (R4)",
            "R4-stale": f"{ALLOWLIST}: 'gone.yml' names no workflow file; remove the line (R4)",
            "R5-with": "bad.yml:13: cache-to type=gha without ignore-error=true, so a lost cache blob fails the build (R5)",
            "R5-argument": "bad.yml:14: cache-to type=gha without ignore-error=true, so a lost cache blob fails the build (R5)",
            "R5-complies": f"{CACHE_EXCEPTIONS}: 'fixed.yml' sets ignore-error=true on every cache export now; remove the line (R5)",
            "R5-stale": f"{CACHE_EXCEPTIONS}: 'gone.yml' names no workflow file; remove the line (R5)",
        }
        ok = True
        for label, msg in want.items():
            if msg in found:
                print(f"  ok   {label} fires")
            else:
                print(f"  FAIL {label} did not fire; got {found}")
                ok = False
        extra = [p for p in found if p not in want.values()]
        if extra:
            print(f"  FAIL unexpected: {extra}")
            ok = False
        if any(p.startswith("good.yml") or p.startswith("fixed.yml") for p in found):
            print("  FAIL the clean fixture was flagged")
            ok = False
        print("self-test: " + ("PASS" if ok else "FAIL"))
        return 0 if ok else 1


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    root = Path(argv[1]) if len(argv) > 1 else Path(os.getcwd())
    problems = check(root)
    if problems:
        print("workflow discipline violations (docs/design/ci-discipline.md):")
        for p in problems:
            print(f"  {p}")
        return 1
    print(f"ok: every workflow under {WORKFLOW_DIR} keeps R1-R5")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
