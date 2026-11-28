#!/usr/bin/env python3
"""
warrant registry gate — ADR 1129. Five checks, all fail-closed.

The registry (ops/warrant/port-sets.yaml + ops/warrant/rule-intents/) is the
source of ops/ansible/netbird_acl_matrix.yml. This guard is what makes that
sentence true rather than aspirational.

WHY THE BYTE-IDENTICAL CHECK IS THE LOAD-BEARING ONE. The cutover from a
hand-written ACL to a generated one is only safe if the generator provably
changes nothing. Not "equivalent" — identical. An equivalence argument is a
judgement call made once by whoever wrote the generator; byte-identity is a
fact re-established on every push. It is also what lets steps 1-3 of the plan
run against a live fleet: if the bytes do not move, no goatnet moves.

WHY AN ADVISORY REGISTRY WOULD BE WORSE THAN NONE. F-324 — three files
described a topology no host carried. A registry that describes what the rules
should contain, without generating them, is that finding by construction: it
accumulates authority in operators' heads while drifting from the deployed
state, and the drift is silent because nothing compares them. It generates, or
it is not worth building.

Checks:

  1. BYTE-IDENTICAL — the generated matrix equals the committed one, exactly.
  2. FAITHFUL EXTRACTION — re-deriving the intents from the matrix is a no-op,
     so the registry is a mechanical transform of the declaration rather than a
     hand transcription a reviewer would have to trust.
  3. VOCABULARY INTEGRITY — every set has a purpose and an owner, no set
     declares two arms for one protocol, and no (protocol, ports) shape is
     claimed by two names. Two names for one shape IS the re-fragmentation this
     registry exists to prevent.
  4. required_by COVERAGE — every pair a set declares it is required by is
     actually granted that set somewhere. This is what makes a narrowing fail
     loudly instead of quietly severing a class. A pair is checked only when
     the tree can hold it: some one goatnet (or rule template) whose rules
     name both its source and its destination group. Otherwise it is printed
     as "not applicable here", naming the two groups. This repository's
     intents hold the hello-galaxy example and a one-box zagros, neither of
     which carries a deployment's full set of classes; a deployment's tree
     does, and there the pair fails as before.
  5. EXPIRY — a rule intent carrying `expires:` must carry an `owner:`, must
     parse as a date, and must not be past due.

Scope limit, stated rather than implied: this is FILE-level. Every check here
can pass while a goatnet runs flat on NetBird's Default All->All, because
nothing here reads a live mesh. Live posture is the cutover play's mode=propose
report. Same boundary as check-netbird-acl-parity.py; see F-283.

Usage:  python3 ops/ci/check-warrant-registry.py [--policy-dir DIR]

--policy-dir moves only the registry (port-sets.yaml + rule-intents/). The
generator, extractor and loader always come from this repository's
ops/warrant/, so a policy directory needs no copy of the code.
Run by: the ops-ci-guards job in .github/workflows/ci.yml, after ops/ci/test-warrant-registry.py.
"""

from __future__ import annotations

import argparse
import datetime as dt
import subprocess
import sys
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover
    sys.exit("warrant: PyYAML is required (pip install pyyaml)")

REPO_ROOT = Path(__file__).resolve().parents[2]
WARRANT_DIR = REPO_ROOT / "ops" / "warrant"
MATRIX = REPO_ROOT / "ops" / "ansible" / "netbird_acl_matrix.yml"

# --policy-dir relocates the POLICY, never the code or the target: generate.py
# and extract.py run from WARRANT_DIR beside this script. The generated matrix
# always lives in this repo. Passing both explicitly matters because
# generate.py resolves its own defaults from its own path: run a copy of it
# out of a temp directory and it would look for the matrix beside the copy,
# fail to find it, and report a mismatch that has nothing to do with the
# registry. The negative tests were passing on exactly that false signal until
# this was pinned.

# Rule-bearing sections of the matrix. The two exercise templates are rules
# too — an earlier read of this file that scanned only netbird_acl_matrix saw
# 38 of the 87 declared rules and drew a wrong conclusion from the shortfall.
RULE_SECTIONS = (
    "netbird_exercise_rules_template",
    "netbird_exercise_gear_rules_template",
)


def host_group(name):
    """`HOST:<GROUP>` is the host site's <GROUP>; any other name is itself."""
    return name[5:] if isinstance(name, str) and name.startswith("HOST:") else name


def collect_rules(intents_doc: dict) -> list[tuple[str, dict]]:
    """Every rule in the intents file, tagged with the scope that declares it."""
    found: list[tuple[str, dict]] = []
    for site, cfg in (intents_doc.get("netbird_acl_matrix") or {}).items():
        for rule in cfg.get("rules") or []:
            found.append((f"netbird_acl_matrix/{site}", rule))
    for section in RULE_SECTIONS:
        for rule in (intents_doc.get(section) or {}).get("rules") or []:
            found.append((section, rule))
    return found


def run_script(script: Path, *args: str) -> tuple[int, str]:
    proc = subprocess.run(
        [sys.executable, str(script), *args],
        capture_output=True,
        text=True,
        cwd=str(REPO_ROOT),
    )
    return proc.returncode, (proc.stdout + proc.stderr).strip()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--policy-dir", type=Path, default=WARRANT_DIR)
    ap.add_argument(
        "--today",
        default=None,
        help="override today's date (YYYY-MM-DD) — for the negative tests",
    )
    args = ap.parse_args()
    policy_dir = args.policy_dir
    violations: list[str] = []

    # 1. byte-identical
    rc, out = run_script(
        WARRANT_DIR / "generate.py",
        "--check",
        "--policy-dir", str(policy_dir),
        "--target", str(MATRIX),
    )
    if rc != 0:
        violations.append(
            "BYTE-IDENTICAL: the registry does not reproduce "
            "ops/ansible/netbird_acl_matrix.yml.\n"
            "  The generated matrix is the deployed one — a difference here means the\n"
            "  registry and the fleet disagree about what is permitted.\n"
            "  Regenerate with: ops/warrant/generate.py\n" + indent(out)
        )

    # 2. faithful extraction
    rc, out = run_script(
        WARRANT_DIR / "extract.py",
        "--check",
        "--policy-dir", str(policy_dir),
        "--source", str(MATRIX),
    )
    if rc != 0:
        violations.append(
            "EXTRACTION: re-deriving the intents from the matrix is not a no-op.\n"
            "  The registry is supposed to be a mechanical transform of the\n"
            "  declaration. Re-run: ops/warrant/extract.py\n" + indent(out)
        )

    # 3 + 4 + 5 need the registry parsed.
    sets_path = policy_dir / "port-sets.yaml"
    intents_path = policy_dir / "rule-intents" / "netbird-acl.yaml"
    if not sets_path.exists() or not intents_path.exists():
        print(f"warrant: registry incomplete under {policy_dir}", file=sys.stderr)
        return 1

    # A malformed registry is a refusal with a filename and a line, not a
    # traceback. An operator reading CI output should not have to parse Python.
    try:
        sets_doc = yaml.safe_load(sets_path.read_text()) or {}
        intents_doc = yaml.safe_load(intents_path.read_text()) or {}
    except yaml.YAMLError as exc:
        print(f"warrant: the registry is not valid YAML:\n{exc}", file=sys.stderr)
        return 1
    rules = collect_rules(intents_doc)

    # 3. vocabulary integrity — generate.py's loader is the single implementation
    #    of these rules; calling it here means the guard and the generator can
    #    never disagree about what a valid vocabulary is.
    sys.path.insert(0, str(WARRANT_DIR))
    from generate import GenerateError, load_port_sets  # noqa: E402

    try:
        load_port_sets(policy_dir)
    except GenerateError as exc:
        violations.append(f"VOCABULARY: {exc}")

    # 4. required_by coverage
    granted: dict[str, list[tuple[str, str, str]]] = {}
    # Groups each goatnet (or rule template) names. A pair applies where one
    # scope names both of its groups; groups named by two different goatnets
    # are two different groups.
    named: dict[str, set[str]] = {}
    for scope, rule in rules:
        named.setdefault(scope, set()).update(host_group(g) for g in rule.get("sources") or [])
        named[scope].update(host_group(g) for g in rule.get("destinations") or [])
    not_applicable: list[str] = []
    for scope, rule in rules:
        name = rule.get("port_set")
        if not name:
            continue
        # A rule of a subordinate scope may name a group of its host site as
        # `HOST:<GROUP>` (ADR 1124). That is the host's <GROUP>: a grant sourced
        # from HOST:INFRA is a grant from INFRA, and a required_by pair written
        # with the plain name is met by it.
        srcs = [host_group(g) for g in rule.get("sources") or []]
        dsts = [host_group(g) for g in rule.get("destinations") or []]
        pairs = [(s, d) for s in srcs for d in dsts]
        if rule.get("bidirectional"):
            pairs += [(d, s) for s in srcs for d in dsts]
        granted.setdefault(name, []).extend((s, d, scope) for s, d in pairs)

    for entry in sets_doc.get("port_sets") or []:
        name = entry.get("name")
        for req in entry.get("required_by") or []:
            src, dst = req.get("src"), req.get("dst")
            if not any(src in groups and dst in groups for groups in named.values()):
                not_applicable.append(
                    f"port-set '{name}': {src} -> {dst} — no goatnet in this tree declares "
                    f"both {src} and {dst} (no one goatnet's rules name the two)"
                )
                continue
            hits = [g for g in granted.get(name, []) if g[0] == src and g[1] == dst]
            if not hits:
                violations.append(
                    f"required_by: port-set '{name}' declares it is required by "
                    f"{src} -> {dst}, but no rule grants that set to that pair.\n"
                    f"  Either a narrowing severed it, or the assertion was written "
                    f"for a pair that never existed. Both are worth failing on."
                )

    # 5. expiry
    today = (
        dt.date.fromisoformat(args.today)
        if args.today
        else dt.date.today()
    )
    for scope, rule in rules:
        expires = rule.get("expires")
        label = f"{scope}: \"{rule.get('name', '<unnamed>')}\""
        if expires is None:
            continue
        if not rule.get("owner"):
            violations.append(
                f"EXPIRY: {label} carries `expires:` with no `owner:`.\n"
                "  An expiring grant with nobody to ask is a grant that will be\n"
                "  renewed by whoever is on shift when CI goes red."
            )
        if isinstance(expires, dt.datetime):
            expires = expires.date()
        if not isinstance(expires, dt.date):
            try:
                expires = dt.date.fromisoformat(str(expires))
            except ValueError:
                violations.append(
                    f"EXPIRY: {label} has `expires: {expires}`, which is not a "
                    "YYYY-MM-DD date."
                )
                continue
        if expires < today:
            violations.append(
                f"EXPIRY: {label} expired on {expires.isoformat()} "
                f"(owner: {rule.get('owner', '<none>')}).\n"
                "  Remove the rule, or move its expiry with a reason in the PR.\n"
                "  Recertification is this check going red, not a review meeting."
            )

    if not_applicable:
        print(f"required_by: {len(not_applicable)} pair(s) not applicable here:")
        for line in not_applicable:
            print(f"  - {line}")

    if violations:
        print("warrant registry gate: FAIL\n", file=sys.stderr)
        for v in violations:
            print(f"  - {v}\n", file=sys.stderr)
        return 1

    print(
        f"warrant registry gate: OK — {len(rules)} rule intents, "
        f"{len(sets_doc.get('port_sets') or [])} port-sets, matrix byte-identical"
    )
    return 0


def indent(text: str, prefix: str = "    ") -> str:
    return "\n".join(prefix + line for line in text.split("\n") if line.strip())


if __name__ == "__main__":
    raise SystemExit(main())
