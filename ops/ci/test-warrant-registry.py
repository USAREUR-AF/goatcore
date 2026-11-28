#!/usr/bin/env python3
"""
Negative tests for the warrant registry gate — every check must refuse something.

A guard that passes but never fails is worthless, and this one guards the
cutover from a hand-written mesh ACL to a generated one. So each of the five
checks in ops/ci/check-warrant-registry.py gets a mutation here that it is
supposed to catch, and the test fails if the guard stays green.

The byte-identical check earns two: one proving a changed PORT is caught (the
obvious case) and one proving a changed RULE SHAPE is caught (the case that
would actually change a goatnet's posture at cutover).

Each test copies the registry to a temp dir, mutates the copy, and runs the
guard against it with --policy-dir. The real registry is never touched.

The rule intents in this repository are the hello-galaxy example (HUB, CREW,
PROBE, OBSERVER, BREAKGLASS, QUARANTINED), so the mutations name its rules. The
vocabulary's required_by pairs name a deployment's classes, which the example
does not have; the guard reports those as not applicable here, and the 4a/4b
tests below pin that boundary from both sides.

Usage:  python3 ops/ci/test-warrant-registry.py
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
WARRANT_DIR = REPO_ROOT / "ops" / "warrant"
GUARD = REPO_ROOT / "ops" / "ci" / "check-warrant-registry.py"

FAILURES: list[str] = []


def run_guard(policy_dir: Path, *extra: str) -> tuple[int, str]:
    proc = subprocess.run(
        [sys.executable, str(GUARD), "--policy-dir", str(policy_dir), *extra],
        capture_output=True,
        text=True,
        cwd=str(REPO_ROOT),
    )
    return proc.returncode, proc.stdout + proc.stderr


def with_registry(fn):
    """Run fn against a throwaway copy of the registry."""
    with tempfile.TemporaryDirectory() as tmp:
        dest = Path(tmp) / "warrant"
        shutil.copytree(WARRANT_DIR, dest)
        return fn(dest)


def expect_refusal(label: str, marker: str, mutate, *extra: str) -> None:
    """The guard must fail, and its output must name the right thing."""

    def run(policy_dir: Path) -> tuple[int, str]:
        mutate(policy_dir)
        return run_guard(policy_dir, *extra)

    rc, out = with_registry(run)
    if rc == 0:
        FAILURES.append(f"{label}: guard PASSED on a registry it should refuse")
    elif marker.lower() not in out.lower():
        FAILURES.append(
            f"{label}: guard failed (good) but never mentioned '{marker}'.\n"
            f"    A refusal an operator cannot act on is half a guard.\n"
            f"    got: {out.strip()[:400]}"
        )
    else:
        print(f"  ok  {label}")


def sub(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    if old not in text:
        raise AssertionError(f"test fixture drifted: {old!r} not in {path}")
    path.write_text(text.replace(old, new, 1))


def main() -> int:
    print("warrant guard — negative tests")

    # 1a. a changed PORT must be caught
    expect_refusal(
        "a port changed in the vocabulary is caught",
        "byte-identical",
        lambda d: sub(d / "port-sets.yaml", '"5353", "7447"', '"5353", "7999"'),
    )

    # 1b. a changed RULE SHAPE must be caught — the case that would actually
    #     move a goatnet's posture. Widening a destination list is the shape of
    #     an accidental over-grant.
    expect_refusal(
        "a widened destination list is caught",
        "byte-identical",
        lambda d: sub(
            d / "rule-intents" / "netbird-acl.yaml",
            "        sources: [QUARANTINED]",
            "        sources: [QUARANTINED, DEVELOPER]",
        ),
    )

    # 2. the vocabulary must stay unambiguous
    expect_refusal(
        "two names for one (protocol, ports) shape is caught",
        "re-fragmented",
        lambda d: (d / "port-sets.yaml").write_text(
            (d / "port-sets.yaml").read_text()
            + '\n  - name: shell-again\n'
            + '    purpose: A duplicate of shell, which must not be allowed to exist.\n'
            + '    owner: platform-admin\n'
            + '    members:\n'
            + '      - { protocol: tcp, ports: ["22"] }\n'
        ),
    )

    # 3. a set with no stated purpose is caught — `purpose` is the thing that
    #    currently exists in drifting copies, so an empty one defeats the point.
    expect_refusal(
        "a port-set with no purpose is caught",
        "purpose",
        # Empty the purpose while keeping the file valid YAML — the guard must
        # refuse on the empty purpose itself, not incidentally on a parse error.
        lambda d: sub(
            d / "port-sets.yaml",
            "    purpose: >\n      Interactive login — tcp/22, and nothing else.",
            '    purpose: ""\n    _former_purpose: >\n      Interactive login — tcp/22, and nothing else.',
        ),
    )

    # 4a. required_by must actually be satisfied when the tree holds the pair:
    #     OBSERVER and HUB are both named by example rules, and no rule grants
    #     `netadmin` from one to the other.
    expect_refusal(
        "a required_by pair whose groups exist but nothing grants is caught",
        "OBSERVER -> HUB, but no rule grants",
        lambda d: sub(
            d / "port-sets.yaml",
            "      - { src: NETWORK, dst: INFRA }",
            "      - { src: NETWORK, dst: INFRA }\n      - { src: OBSERVER, dst: HUB }",
        ),
    )

    # 4b. a pair whose groups no goatnet in the tree declares is not a failure,
    #     and the guard says which pair and which group it lacks.
    def absent_pair(policy_dir: Path):
        sub(
            policy_dir / "port-sets.yaml",
            "      - { src: NETWORK, dst: INFRA }",
            "      - { src: NETWORK, dst: INFRA }\n      - { src: NOBODY, dst: NOWHERE }",
        )
        return run_guard(policy_dir)

    rc, out = with_registry(absent_pair)
    if rc != 0:
        FAILURES.append(f"a pair with undeclared groups was refused.\n    got: {out.strip()[:400]}")
    elif "NOBODY -> NOWHERE — no goatnet in this tree declares both NOBODY and NOWHERE" not in out:
        FAILURES.append(f"a pair with undeclared groups passed without being named.\n    got: {out.strip()[:400]}")
    else:
        print("  ok  a pair whose groups no goatnet declares is named as not applicable")

    # 4b'. groups named only by two DIFFERENT goatnets are two different groups:
    #      CREW is hello-galaxy's, INFRA is zagros's, so CREW -> INFRA is not a
    #      pair this tree holds, and is not refused.
    def cross_goatnet_pair(policy_dir: Path):
        sub(
            policy_dir / "port-sets.yaml",
            "      - { src: NETWORK, dst: INFRA }",
            "      - { src: NETWORK, dst: INFRA }\n      - { src: CREW, dst: INFRA }",
        )
        return run_guard(policy_dir)

    rc, out = with_registry(cross_goatnet_pair)
    if rc != 0:
        FAILURES.append(f"a pair named only across two goatnets was refused.\n    got: {out.strip()[:400]}")
    elif "CREW -> INFRA — no goatnet in this tree declares both CREW and INFRA" not in out:
        FAILURES.append(f"a pair named only across two goatnets passed without being named.\n    got: {out.strip()[:400]}")
    else:
        print("  ok  a pair named only by two different goatnets is not applicable")

    # 4b''. a grant sourced from the host site's group (`HOST:<GROUP>`, ADR 1124)
    #       meets a required_by pair written with the plain group name: a
    #       subordinate scope's rule from HOST:OBSERVER to HUB grants OBSERVER -> HUB.
    with tempfile.TemporaryDirectory() as tmp:
        bare = Path(tmp) / "policy"
        (bare / "rule-intents").mkdir(parents=True)
        shutil.copy2(WARRANT_DIR / "port-sets.yaml", bare / "port-sets.yaml")
        shutil.copy2(WARRANT_DIR / "rule-intents" / "netbird-acl.yaml", bare / "rule-intents" / "netbird-acl.yaml")
        sub(bare / "port-sets.yaml", "      - { src: NETWORK, dst: INFRA }",
            "      - { src: NETWORK, dst: INFRA }\n      - { src: OBSERVER, dst: HUB }")
        rc_plain, out_plain = run_guard(bare)
        intents = bare / "rule-intents" / "netbird-acl.yaml"
        text = intents.read_text()
        anchor = text.index("port_set: web")
        # A netadmin grant from the host site's OBSERVER group to HUB.
        block = ('      - name: "host observer → hub (netadmin)"\n        description: "fixture: a host-scoped grant"\n'
                 '        sources: ["HOST:OBSERVER"]\n        destinations: [HUB]\n        protocol: tcp\n        port_set: netadmin\n'
                 '        action: accept\n        bidirectional: false\n\n')
        start = text.rindex("      - name:", 0, anchor)
        intents.write_text(text[:start] + block + text[start:])
        rc_host, out_host = run_guard(bare)
    if rc_plain == 0 or "OBSERVER -> HUB, but no rule grants" not in out_plain:
        FAILURES.append(f"the pair was not refused before the host-scoped grant.\n    got: {out_plain.strip()[:400]}")
    elif "OBSERVER -> HUB, but no rule grants" in out_host:
        FAILURES.append(f"a grant from HOST:OBSERVER did not meet OBSERVER -> HUB.\n    got: {out_host.strip()[:400]}")
    else:
        print("  ok  a grant sourced from the host site's group meets the plain pair")

    # 4c. --policy-dir moves only the registry: a directory holding just
    #     port-sets.yaml and rule-intents/ is judged with this repository's code.
    with tempfile.TemporaryDirectory() as tmp:
        bare = Path(tmp) / "policy"
        (bare / "rule-intents").mkdir(parents=True)
        shutil.copy2(WARRANT_DIR / "port-sets.yaml", bare / "port-sets.yaml")
        shutil.copy2(WARRANT_DIR / "rule-intents" / "netbird-acl.yaml", bare / "rule-intents" / "netbird-acl.yaml")
        rc, out = run_guard(bare)
    if rc != 0:
        FAILURES.append(f"a policy dir without the generator was refused.\n    got: {out.strip()[:400]}")
    else:
        print("  ok  a policy dir with no copy of the code is judged with the repository's")

    # 5a. an expired grant is caught. The observer rule carries no expiry of its
    #     own (the breakglass one does), so the inserted key is the only one.
    expect_refusal(
        "a past-due expires: is caught",
        "expired on",
        lambda d: sub(
            d / "rule-intents" / "netbird-acl.yaml",
            '      - name: "observer → hub (https only)"',
            '      - name: "observer → hub (https only)"\n'
            "        expires: 2020-01-01\n"
            "        owner: platform-admin",
        ),
    )

    # 5b. an expiring grant with nobody to ask is caught
    expect_refusal(
        "expires: without owner: is caught",
        "no `owner:`",
        lambda d: sub(
            d / "rule-intents" / "netbird-acl.yaml",
            '      - name: "observer → hub (https only)"',
            '      - name: "observer → hub (https only)"\n'
            "        expires: 2099-01-01",
        ),
    )

    # 5c. and the expiry clock must be real: the same grant, not yet due, passes.
    #     Without this, "expired" could be firing on the presence of the field.
    def not_yet_due(policy_dir: Path):
        sub(
            policy_dir / "rule-intents" / "netbird-acl.yaml",
            '      - name: "observer → hub (https only)"',
            '      - name: "observer → hub (https only)"\n'
            "        expires: 2030-06-01\n"
            "        owner: platform-admin",
        )
        return run_guard(policy_dir, "--today", "2030-05-31")

    rc, out = with_registry(not_yet_due)
    if rc != 0:
        FAILURES.append(
            "a not-yet-due expires: was refused — the check is firing on the "
            f"field's presence, not on the date.\n    got: {out.strip()[:400]}"
        )
    else:
        print("  ok  a not-yet-due expires: still passes")

    if FAILURES:
        print("\nwarrant guard negative tests: FAIL", file=sys.stderr)
        for f in FAILURES:
            print(f"  - {f}", file=sys.stderr)
        return 1
    print("\nwarrant guard negative tests: OK — every check refuses something")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
