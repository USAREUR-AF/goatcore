#!/usr/bin/env python3
"""
Tests for the narrowing prover — ADR 1129 C4.

This one decides whether a change to the mesh ACL can merge without a human,
so the tests that matter most are the ones proving it REFUSES: an auto-approve
that is wrong in the permissive direction merges an over-grant unattended.

Every case is built from the real matrix so the shapes are the shapes that
actually occur.

Usage: python3 ops/ci/test-warrant-narrowing.py
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
MATRIX = REPO_ROOT / "ops" / "ansible" / "netbird_acl_matrix.yml"
CHECK = REPO_ROOT / "ops" / "ci" / "check-warrant-narrowing.py"

FAILURES: list[str] = []


def _materialize(root: Path, side: str, spec) -> Path:
    """A str is one file (a matrix, or the retired single-file membership
    layout); a dict {relpath: text} is a scope directory; None is absent."""
    if isinstance(spec, dict):
        d = root / side / "zagros-dev"
        for rel, text in spec.items():
            (d / rel).parent.mkdir(parents=True, exist_ok=True)
            (d / rel).write_text(text)
        return d
    f = root / f"{side}.yml"
    if spec is not None:
        f.write_text(spec)
    return f


def run(base, head, membership: bool = False) -> tuple[int, str]:
    """None for a side means the file/directory does not exist at that revision."""
    with tempfile.TemporaryDirectory() as tmp:
        b = _materialize(Path(tmp), "base", base)
        h = _materialize(Path(tmp), "head", head)
        proc = subprocess.run(
            [sys.executable, str(CHECK)] + (["--membership"] if membership else []) + [str(b), str(h)],
            capture_output=True,
            text=True,
        )
        return proc.returncode, proc.stdout + proc.stderr


def case(label: str, base, head, want_narrowing: bool, membership: bool = False) -> None:
    rc, out = run(base, head, membership)
    if rc == 2:
        FAILURES.append(f"{label}: checker errored\n    {out.strip()[:300]}")
        return
    got = rc == 0
    if got != want_narrowing:
        FAILURES.append(
            f"{label}: expected {'NARROWING' if want_narrowing else 'WIDENING'}, "
            f"got {'NARROWING' if got else 'WIDENING'}\n    {out.strip()[:300]}"
        )
    else:
        print(f"  ok  {label}")


def matrix(rules: str, section: str = "netbird_acl_matrix") -> str:
    if section == "netbird_acl_matrix":
        return f"netbird_acl_matrix:\n  zagros:\n    site_prefix: ZAGROS\n    rules:\n{rules}"
    return f"{section}:\n  rules:\n{rules}"


def rule(
    name="r",
    src="DEVELOPER",
    dst="INFRA",
    proto="tcp",
    ports='["443"]',
    bidir=False,
    action="accept",
) -> str:
    ports_line = f"        ports: {ports}\n" if ports else ""
    return (
        f'      - name: "{name}"\n'
        f"        sources: [{src}]\n"
        f"        destinations: [{dst}]\n"
        f"        protocol: {proto}\n"
        f"{ports_line}"
        f"        action: {action}\n"
        f"        bidirectional: {str(bidir).lower()}\n"
    )


def fragment(*entries: str) -> str:
    body = "".join(entries)
    return "version: 1\n" + (f"entries:\n{body}" if body else "entries: []\n")


def baseline(people: dict | None = None, scope: str = "zagros-dev") -> str:
    lines = ["version: 1", f"scope: {scope}", "generated_from: test"]
    if not people:
        lines.append("people: {}")
    else:
        lines.append("people:")
        for h, (dev, hs) in people.items():
            lines += [f"  {h}:", f"    device: [{', '.join(dev)}]", f"    handset: [{', '.join(hs)}]"]
    return "\n".join(lines) + "\n"


def mfile(*entries: str, converge: str = "report", scope: str = "zagros-dev",
          frag: str = "people.yaml", people: dict | None = None) -> dict:
    """One scope directory: _scope.yaml, the entries in *frag*, a baseline."""
    return {
        "_scope.yaml": f"version: 1\nscope: {scope}\nconverge: {converge}\n",
        frag: fragment(*entries),
        "_baseline.generated.yaml": baseline(people, scope),
    }


def legacy(*entries: str, converge: str = "report", scope: str = "zagros-dev") -> str:
    body = "".join(entries)
    return (f"version: 1\nscope: {scope}\nconverge: {converge}\n"
            + (f"entries:\n{body}" if body else "entries: []\n"))


def ment(peer: str = "peer-alice-zagros-dev-2026-09-01-laptop", **fields) -> str:
    lines = [f"  - peer: {peer}"]
    for k, v in fields.items():
        lines.append(f"    {k}: [{', '.join(v)}]" if isinstance(v, list) else f"    {k}: {v}")
    lines += ["    owner: alice", "    reason: test"]
    return "\n".join(lines) + "\n"


def membership_cases() -> None:
    """mesh-change-control.md §4.3 "Narrowing", both directions."""
    P = "ZAGROS-DEV"
    M = dict(membership=True)
    grant = ment(grant=[f"{P}-DEVELOPER", f"{P}-P2P"], expires="2026-12-01")

    case("membership: unchanged file", mfile(grant), mfile(grant), True, **M)
    bound = ment(peer_id="peer-a", grant=[f"{P}-DEVELOPER"])
    case("membership: rebinding a peer ID requires review",
         mfile(bound), mfile(ment(peer_id="peer-b", hold="quarantine", expires="2026-12-01")), False, **M)
    case("membership: dropping a peer ID requires review",
         mfile(bound), mfile(ment(grant=[f"{P}-DEVELOPER"])), False, **M)
    case("membership: deleting a bound entry requires review",
         mfile(bound), mfile(), False, **M)
    case("membership: removing a grant group is a narrowing",
         mfile(grant), mfile(ment(grant=[f"{P}-DEVELOPER"], expires="2026-12-01")), True, **M)
    case("membership: adding a grant group is a widening",
         mfile(ment(grant=[f"{P}-DEVELOPER"])), mfile(ment(grant=[f"{P}-DEVELOPER", f"{P}-MONITOR"])), False, **M)
    case("membership: adding an absent group is a narrowing",
         mfile(grant), mfile(ment(grant=[f"{P}-DEVELOPER", f"{P}-P2P"], absent=[f"{P}-OBSERVER"],
                                   expires="2026-12-01")), True, **M)
    case("membership: dropping an absent group is a widening",
         mfile(ment(absent=[f"{P}-OBSERVER", f"{P}-MONITOR"])), mfile(ment(absent=[f"{P}-OBSERVER"])), False, **M)
    case("membership: adding a hold is a narrowing",
         mfile(grant), mfile(grant, ment(peer="box-1", hold="quarantine", expires="2026-09-30")), True, **M)
    case("membership: replacing a grant with a hold is a narrowing",
         mfile(grant), mfile(ment(hold="quarantine", expires="2026-09-30")), True, **M)
    case("membership: releasing a hold is a widening",
         mfile(ment(hold="quarantine", expires="2026-09-30")), mfile(), False, **M)
    case("membership: shrinking an exact is a narrowing",
         mfile(ment(peer="box-1", exact=[f"{P}-INFRA", f"{P}-NETWORK"])),
         mfile(ment(peer="box-1", exact=[f"{P}-INFRA"])), True, **M)
    case("membership: growing an exact is a widening",
         mfile(ment(peer="box-1", exact=[f"{P}-INFRA"])),
         mfile(ment(peer="box-1", exact=[f"{P}-INFRA", f"{P}-NETWORK"])), False, **M)
    case("membership: deleting a grant-only entry is a narrowing", mfile(grant), mfile(), True, **M)
    case("membership: deleting an absent entry is a widening (the group comes back)",
         mfile(ment(absent=[f"{P}-OBSERVER"])), mfile(), False, **M)
    case("membership: deleting an exact entry is a widening (baseline returns)",
         mfile(ment(peer="box-1", exact=[f"{P}-INFRA"])), mfile(), False, **M)
    case("membership: a new grant entry is a widening", mfile(), mfile(grant), False, **M)
    case("membership: a new absent-only entry is a narrowing",
         mfile(), mfile(ment(absent=[f"{P}-OBSERVER"])), True, **M)
    case("membership: a new EXTERNAL-INFRA exact entry is a widening",
         mfile(), mfile(ment(peer="vendor-registry-01", exact=[f"{P}-EXTERNAL-INFRA"], expires="2026-12-31")),
         False, **M)
    case("membership: a new PROBATION exact entry (the sweep) is a narrowing",
         mfile(), mfile(ment(exact=[f"{P}-PROBATION-DEVELOPER"])), True, **M)
    case("membership: a grant entry moved into PROBATION is a narrowing",
         mfile(grant), mfile(ment(exact=[f"{P}-PROBATION-DEVELOPER"])), True, **M)
    case("membership: a hold moved into PROBATION is a widening (a release)",
         mfile(ment(hold="quarantine", expires="2026-09-30")),
         mfile(ment(exact=[f"{P}-PROBATION-DEVELOPER"])), False, **M)
    case("membership: extending expires on a grant is a widening",
         mfile(grant), mfile(ment(grant=[f"{P}-DEVELOPER", f"{P}-P2P"], expires="2027-06-01")), False, **M)
    case("membership: removing expires on a grant is a widening",
         mfile(grant), mfile(ment(grant=[f"{P}-DEVELOPER", f"{P}-P2P"])), False, **M)
    case("membership: shortening expires is a narrowing",
         mfile(grant), mfile(ment(grant=[f"{P}-DEVELOPER", f"{P}-P2P"], expires="2026-10-01")), True, **M)
    case("membership: report -> enforce is never proven",
         mfile(grant), mfile(grant, converge="enforce"), False, **M)
    case("membership: a new file with no entries is a no-op", None, mfile(), True, **M)
    case("membership: a new file with a grant is a widening", None, mfile(grant), False, **M)
    case("membership: deleting the file ungoverns the scope — a widening", mfile(), None, False, **M)

    # The directory layout: the prover compares MERGED views.
    case("membership: an entry moved to another fragment unchanged is a no-op",
         mfile(grant), {**mfile(), "orgs/acme.yaml": fragment(grant)}, True, **M)
    case("membership: an entry moved AND widened is still a widening",
         mfile(ment(grant=[f"{P}-DEVELOPER"])),
         {**mfile(), "orgs/acme.yaml": fragment(ment(grant=[f"{P}-DEVELOPER", f"{P}-MONITOR"]))}, False, **M)
    case("membership: splitting one fragment into two changes nothing",
         mfile(grant, ment(peer="box-1", exact=[f"{P}-INFRA"])),
         {**mfile(grant), "substrate.yaml": fragment(ment(peer="box-1", exact=[f"{P}-INFRA"]))}, True, **M)
    case("membership: converting the retired single file, entries unchanged, no people",
         legacy(grant), mfile(grant), True, **M)
    case("membership: converting with a baseline of people is never proven",
         legacy(grant), mfile(grant, people={"alice": ([f"{P}-DEVELOPER"], [f"{P}-OBSERVER"])}), False, **M)

    # The declared baseline is every person-device's starting set.
    alice = {"alice": ([f"{P}-DEVELOPER", f"{P}-OBSERVER"], [f"{P}-OBSERVER"])}
    case("membership: a baseline person losing a device group is a narrowing",
         mfile(people=alice), mfile(people={"alice": ([f"{P}-OBSERVER"], [f"{P}-OBSERVER"])}), True, **M)
    case("membership: a baseline person gaining a device group is a widening",
         mfile(people=alice),
         mfile(people={"alice": ([f"{P}-DEVELOPER", f"{P}-OBSERVER", f"{P}-P2P"], [f"{P}-OBSERVER"])}),
         False, **M)
    case("membership: a baseline person gaining a handset group is a widening",
         mfile(people=alice),
         mfile(people={"alice": ([f"{P}-DEVELOPER", f"{P}-OBSERVER"], [f"{P}-OBSERVER", f"{P}-HANDSET"])}),
         False, **M)
    case("membership: a person added to the baseline is a widening",
         mfile(people=alice), mfile(people={**alice, "bob": ([f"{P}-OBSERVER"], [])}), False, **M)
    case("membership: a person removed from the baseline is a widening (key baseline returns)",
         mfile(people=alice), mfile(), False, **M)
    case("membership: a _scope.yaml converge change is never proven",
         mfile(grant), mfile(grant, converge="enforce"), False, **M)

    for scope in ("zagros-dev", "zagros"):
        d = REPO_ROOT / "ops" / "warrant" / "membership" / scope
        real = {f.relative_to(d).as_posix(): f.read_text() for f in d.rglob("*.yaml")}
        case(f"membership: the real {scope} directory against itself", real, real, True, **M)


def main() -> int:
    print("warrant narrowing prover — tests")

    # Removing a port is a narrowing.
    case(
        "dropping a port from a rule is a narrowing",
        matrix(rule(ports='["443", "22"]')),
        matrix(rule(ports='["443"]')),
        True,
    )

    # Adding a port is not.
    case(
        "adding a port is a widening",
        matrix(rule(ports='["443"]')),
        matrix(rule(ports='["443", "22"]')),
        False,
    )

    # Deleting a rule outright is a narrowing.
    case(
        "deleting a rule is a narrowing",
        matrix(rule(name="a") + rule(name="b", dst="NETWORK")),
        matrix(rule(name="a")),
        True,
    )

    # Adding a whole new pair is a widening.
    case(
        "a brand-new pair is a widening",
        matrix(rule(name="a")),
        matrix(rule(name="a") + rule(name="b", dst="NETWORK")),
        False,
    )

    # `all` -> tcp is a narrowing; the reverse is not.
    case(
        "protocol all -> tcp is a narrowing",
        matrix(rule(proto="all", ports="")),
        matrix(rule(proto="tcp", ports='["443"]')),
        True,
    )
    case(
        "protocol tcp -> all is a widening",
        matrix(rule(proto="tcp", ports='["443"]')),
        matrix(rule(proto="all", ports="")),
        False,
    )

    # THE ONE THAT MATTERS MOST: an empty ports list means EVERY port. A
    # checker that read it as "no ports" would call this a narrowing and
    # auto-merge a rule that opens everything.
    case(
        "dropping the ports list entirely is a WIDENING, not a narrowing",
        matrix(rule(ports='["443"]')),
        matrix(rule(ports="")),
        False,
    )

    # Bidirectional adds the reverse direction — that is a new grant.
    case(
        "flipping bidirectional on is a widening",
        matrix(rule(bidir=False)),
        matrix(rule(bidir=True)),
        False,
    )
    case(
        "flipping bidirectional off is a narrowing",
        matrix(rule(bidir=True)),
        matrix(rule(bidir=False)),
        True,
    )

    # A drop rule is not a permit, so turning accept into drop removes a grant.
    case(
        "accept -> drop is a narrowing",
        matrix(rule(action="accept")),
        matrix(rule(action="drop")),
        True,
    )

    # No change at all is trivially safe.
    case("an unchanged matrix is a narrowing (no new grants)", matrix(rule()), matrix(rule()), True)

    # The exercise templates are rules too — a widening hidden in one of them
    # must not slip through because the scanner only looked at the main matrix.
    case(
        "a widening in an exercise template is caught",
        matrix(rule(ports='["443"]'), section="netbird_exercise_rules_template"),
        matrix(rule(ports='["443", "22"]'), section="netbird_exercise_rules_template"),
        False,
    )

    # The real file against itself must be a clean no-op.
    real = MATRIX.read_text()
    case("the live matrix against itself is unchanged", real, real, True)

    membership_cases()

    if FAILURES:
        print("\nwarrant narrowing tests: FAIL", file=sys.stderr)
        for f in FAILURES:
            print(f"  - {f}", file=sys.stderr)
        return 1
    print("\nwarrant narrowing tests: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
