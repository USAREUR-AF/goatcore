#!/usr/bin/env python3
"""
Is this ACL change a NARROWING? — ADR 1129 C4.

A narrowing grants nothing that was not already granted. It can merge without
waiting for its owner, because the only thing it can do is take access away —
and taking access away is recoverable by `git revert`, while granting it is
not recoverable by anything.

A widening waits for the owner named in CODEOWNERS. That asymmetry is the
whole reason the design is fast, and it is one CI check rather than a queue, a
state machine or an approval service.

THE PROOF, AND ITS DIRECTION OF ERROR. For every permit in the proposed ACL,
there must be a permit in the base ACL that already covered it. If one does
not exist, the change grants something new and is NOT a narrowing. The check
is conservative: a genuine narrowing expressed in a way it cannot follow
grades as "not proven", which sends it to a human. Being wrong in that
direction costs a review; being wrong in the other direction auto-merges an
over-grant.

`protocol: all` covers tcp/udp/icmp and an EMPTY ports list means EVERY port —
that is NetBird's own reading, and inverting either would silently treat a
wide rule as narrow, which is the direction a security surface must never be
wrong in.

Usage:
    check-warrant-narrowing.py BASE_MATRIX HEAD_MATRIX
    check-warrant-narrowing.py --json BASE HEAD
    check-warrant-narrowing.py --membership BASE_DIR HEAD_DIR

MEMBERSHIP (docs/design/mesh-change-control.md §4.3). With `--membership` the
two paths are one scope's directory, ops/warrant/membership/<scope>/, at base
and head (BASE may also be the retired single file <scope>.yaml, so the change
that converts a scope still grades). The prover compares the MERGED views —
every fragment's entries by peer, plus _scope.yaml and the generated baseline —
so an entry moved between fragments unchanged is no change at all. A missing
or empty BASE is a scope with no entries yet; a missing HEAD is the scope
being deleted, which ungoverns it and is never a narrowing. The diff is a
narrowing when every changed entry only:

  - removes `grant` groups, or adds `absent` groups;
  - adds a `hold` (or replaces an entry with one);
  - shrinks an `exact`;
  - is a NEW `exact` naming only hold lanes (`<P>-PROBATION-*`,
    `<P>-QUARANTINED`) — what the lifecycle sweep writes, and the least reach
    a peer can hold short of none;
  - is deleted, when it only granted.

Editing `owner` or `reason` changes no access. An earlier `expires`, or one
added, is a narrowing; a later or removed one extends a grant and is not.
Changing `scope` or `converge` is never proven: `report -> enforce` is what
makes every entry real.

THE DECLARED BASELINE (_baseline.generated.yaml) is graded too, because it is
every person-device's starting set: a person's `device`/`handset` list losing
groups narrows; gaining one, a person added, a person removed (their peers
fall back to the minting key's auto_groups) or the baseline appearing for the
first time is never proven.

Exit: 0 = proven narrowing (or no change), 1 = widens / not proven, 2 = error.

In CI the base file comes from `git show <base-sha>:ops/ansible/...`.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover
    sys.exit("warrant: PyYAML is required (pip install pyyaml)")

RULE_SECTIONS = (
    "netbird_exercise_rules_template",
    "netbird_exercise_gear_rules_template",
)


class Permit:
    __slots__ = ("scope", "src", "dst", "proto", "ports", "rule")

    def __init__(self, scope, src, dst, proto, ports, rule):
        self.scope = scope
        self.src = src
        self.dst = dst
        self.proto = (proto or "all").lower()
        self.ports = tuple(str(p) for p in (ports or []))
        self.rule = rule

    @property
    def key(self):
        return (self.scope, self.src, self.dst)

    def covers(self, other: "Permit") -> bool:
        """Does self already grant everything other grants?"""
        if self.proto != "all" and self.proto != other.proto:
            return False
        if not self.ports:
            return True  # every port
        if not other.ports:
            return False  # other is every-port, self is not
        return set(other.ports) <= set(self.ports)

    def describe(self) -> str:
        ports = ", ".join(self.ports) if self.ports else "ANY PORT"
        return (
            f"[{self.scope}] {self.src} -> {self.dst}  {self.proto} {ports}"
            f"   (rule: {self.rule})"
        )


def permits(path: Path) -> list[Permit]:
    doc = yaml.safe_load(path.read_text()) or {}
    out: list[Permit] = []

    def add(scope: str, rules: list) -> None:
        for r in rules or []:
            action = (r.get("action") or "accept").lower()
            # `action: drop` is not a permit. Folding a drop into the permit
            # set would be the worst possible direction to be wrong in.
            if action != "accept":
                continue
            name = r.get("name", "<unnamed>")
            for s in r.get("sources") or []:
                for d in r.get("destinations") or []:
                    out.append(
                        Permit(scope, s, d, r.get("protocol"), r.get("ports"), name)
                    )
                    if r.get("bidirectional"):
                        out.append(
                            Permit(scope, d, s, r.get("protocol"), r.get("ports"), name)
                        )

    for site, cfg in (doc.get("netbird_acl_matrix") or {}).items():
        add(site, cfg.get("rules"))
    for section in RULE_SECTIONS:
        add(section, (doc.get(section) or {}).get("rules"))
    return out


# ---- membership ---------------------------------------------------------------

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "warrant"))


def _load_membership(path: Path) -> dict | None:
    """The merged view of a scope: {} for an empty base, None when missing.

    A directory is read as the layout membership.py defines (fragments merged,
    `baseline` from _baseline.generated.yaml); a file is the retired
    single-file layout, read as one document with no declared baseline.
    """
    import membership as mem

    path = Path(path)
    if path.is_dir():
        try:
            view = mem.load_scope(path)
        except mem.SourceError as exc:
            raise yaml.YAMLError(str(exc)) from exc
        doc = mem.merged_doc(view)
        doc["baseline"] = view["baseline"]
        return doc
    if not path.exists():
        return None
    doc = yaml.safe_load(path.read_text())
    return doc if isinstance(doc, dict) else {}


def baseline_changes(base: dict | None, head: dict | None) -> tuple[list[str], list[str]]:
    """(widenings, narrowings) in the declared baseline between two views."""
    wide: list[str] = []
    narrow: list[str] = []
    bb = (base or {}).get("baseline")
    hb = (head or {}).get("baseline")
    bp = (bb or {}).get("people") or {}
    hp = (hb or {}).get("people") or {}
    if bb is None and hb is not None:
        if hp:
            wide.append(f"+ declared baseline introduced for {len(hp)} people — it replaces their "
                        "key baselines, which is never proven a narrowing")
        return wide, narrow
    if bb is not None and hb is None:
        wide.append("+ declared baseline removed — every person-device falls back to its key baseline")
        return wide, narrow
    for handle in sorted(set(bp) | set(hp), key=str):
        b, h = bp.get(handle), hp.get(handle)
        if b == h:
            continue
        if b is None:
            if any(h.get(k) for k in ("device", "handset")):
                wide.append(f"+ baseline {handle}: person added ({', '.join(h.get('device') or [])})")
            else:
                narrow.append(f"- baseline {handle}: person added with no groups")
            continue
        if h is None:
            wide.append(f"+ baseline {handle}: person removed — their peers fall back to the key baseline")
            continue
        reasons, took = [], []
        for kind in ("device", "handset"):
            added = set(h.get(kind) or []) - set(b.get(kind) or [])
            dropped = set(b.get(kind) or []) - set(h.get(kind) or [])
            if added:
                reasons.append(f"{kind} gains {', '.join(sorted(added))}")
            if dropped:
                took.append(f"{kind} loses {', '.join(sorted(dropped))}")
        if reasons:
            wide.append(f"+ baseline {handle}: {'; '.join(reasons)}")
        elif took:
            narrow.append(f"- baseline {handle}: {'; '.join(took)}")
    return wide, narrow


def _is_hold_lane(group: str) -> bool:
    return "-PROBATION-" in group or group.endswith("-QUARANTINED")


def _grant_only(e: dict) -> bool:
    return "grant" in e and not any(k in e for k in ("absent", "exact", "hold"))


def membership_changes(base: dict | None, head: dict | None) -> tuple[list[str], list[str]]:
    """(widenings, narrowings) — each a human line naming the peer and why."""
    import membership as mem

    wide: list[str] = []
    narrow: list[str] = []
    if head is None:
        if base is not None:
            wide.append("the scope's declaration is deleted — that ungoverns the scope")
        return wide, narrow
    base = base or {}
    bw, bn = baseline_changes(base if base else None, head)
    wide.extend(bw)
    narrow.extend(bn)
    for key in ("scope", "converge"):
        if base and base.get(key) != head.get(key):
            wide.append(f"{key}: {base.get(key)!r} -> {head.get(key)!r} is never proven a narrowing")

    def by_peer(doc):
        return {str(e.get("peer")): e for e in (doc.get("entries") or []) if isinstance(e, dict)}

    b, h = by_peer(base), by_peer(head)
    for peer in sorted(set(b) | set(h)):
        be, he = b.get(peer), h.get(peer)
        if be == he:
            continue
        if be is not None and be.get("peer_id") != (he or {}).get("peer_id"):
            wide.append(f"+ {peer}: peer ID binding changed")
            continue
        if he is None:
            if _grant_only(be):
                narrow.append(f"- {peer}: grant-only entry deleted")
            else:
                what = "hold released" if "hold" in be else "entry deleted (the peer returns to its baseline)"
                wide.append(f"+ {peer}: {what}")
            continue
        hm = mem.entry_mode(he)
        if hm == "hold":
            if be is None or "hold" not in be:
                narrow.append(f"- {peer}: hold added")
                continue
        if be is None:
            if hm == "delta" and not he.get("grant"):
                narrow.append(f"- {peer}: new entry only removes {', '.join(he.get('absent') or [])}")
            elif hm == "exact" and he.get("exact") and all(_is_hold_lane(str(g)) for g in he["exact"]):
                narrow.append(f"- {peer}: new entry holds only {', '.join(he['exact'])}")
            else:
                wide.append(f"+ {peer}: new entry ({hm or 'malformed'})")
            continue
        bm = mem.entry_mode(be)
        reasons: list[str] = []
        hold_lanes_only = hm == "exact" and bool(he.get("exact")) and all(
            _is_hold_lane(str(g)) for g in he["exact"])
        if bm != hm:
            # Into a hold lane from a grant/absent or exact entry is the sweep's
            # dormancy hold on a peer that already had an entry. From a hold it
            # is a release, and is not.
            if not (hold_lanes_only and bm != "hold"):
                reasons.append(f"mode {bm} -> {hm}")
        elif hm == "delta":
            added = set(he.get("grant") or []) - set(be.get("grant") or [])
            dropped_absent = set(be.get("absent") or []) - set(he.get("absent") or [])
            if added:
                reasons.append(f"grants {', '.join(sorted(added))}")
            if dropped_absent:
                reasons.append(f"no longer keeps {', '.join(sorted(dropped_absent))} absent")
        elif hm == "exact":
            added = set(he.get("exact") or []) - set(be.get("exact") or [])
            if added:
                reasons.append(f"exact gains {', '.join(sorted(added))}")
        elif hm == "hold" and be.get("hold") != he.get("hold"):
            reasons.append(f"hold {be.get('hold')!r} -> {he.get('hold')!r}")
        if hm != "hold" and not hold_lanes_only:
            bx, hx = mem.parse_date(be.get("expires")) if be.get("expires") else None, \
                mem.parse_date(he.get("expires")) if he.get("expires") else None
            if bx and not hx:
                reasons.append("expires removed")
            elif bx and hx and hx > bx:
                reasons.append(f"expires extended {bx} -> {hx}")
        if reasons:
            wide.append(f"+ {peer}: {'; '.join(reasons)}")
        else:
            narrow.append(f"- {peer}: changed, grants nothing new")
    return wide, narrow


def main_membership(base_path: Path, head_path: Path, as_json: bool) -> int:
    try:
        base = _load_membership(base_path)
        head = _load_membership(head_path)
    except yaml.YAMLError as exc:
        print(f"warrant: unparseable membership file:\n{exc}", file=sys.stderr)
        return 2
    wide, narrow = membership_changes(base, head)
    scope = (head or base or {}).get("scope") or Path(head_path).stem
    if as_json:
        print(json.dumps({"kind": "membership", "scope": scope,
                          "verdict": "widening" if wide else "narrowing",
                          "auto_approvable": not wide, "widenings": wide, "narrowings": narrow}, indent=2))
        return 1 if wide else 0
    if wide:
        print(f"warrant: this membership change on {scope} WIDENS — it is not auto-approvable.\n")
        for line in wide:
            print(f"    {line}")
        for line in narrow:
            print(f"    {line}")
        print("\n  A widening waits for the owner CODEOWNERS names for the path.\n"
              "  That is the gate working, not a failure.")
        return 1
    if narrow:
        print(f"warrant: proven NARROWING of {scope} membership — {len(narrow)} entr"
              f"{'y' if len(narrow) == 1 else 'ies'} changed, none grants.\n")
        for line in narrow:
            print(f"    {line}")
        print("\n  Nothing here grants membership the base did not already allow,\n"
              "  so it can merge without waiting; `git revert` puts it back.")
    else:
        print(f"warrant: no change to {scope} membership.")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("base", type=Path)
    ap.add_argument("head", type=Path)
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--membership", action="store_true",
                    help="BASE/HEAD are ops/warrant/membership/<scope>/ directories, not matrices")
    args = ap.parse_args()

    if args.membership:
        return main_membership(args.base, args.head, args.json)

    try:
        base = permits(args.base)
        head = permits(args.head)
    except yaml.YAMLError as exc:
        print(f"warrant: unparseable matrix:\n{exc}", file=sys.stderr)
        return 2

    index: dict[tuple, list[Permit]] = {}
    for p in base:
        index.setdefault(p.key, []).append(p)

    new_grants = [h for h in head if not any(b.covers(h) for b in index.get(h.key, []))]

    # Reported for the operator's benefit, never for the verdict: a narrowing
    # is defined by granting nothing new, not by removing something.
    head_index: dict[tuple, list[Permit]] = {}
    for p in head:
        head_index.setdefault(p.key, []).append(p)
    removed = [b for b in base if not any(h.covers(b) for h in head_index.get(b.key, []))]

    verdict = "narrowing" if not new_grants else "widening"

    if args.json:
        print(
            json.dumps(
                {
                    "verdict": verdict,
                    "auto_approvable": not new_grants,
                    "new_grants": [p.describe() for p in new_grants],
                    "removed_grants": [p.describe() for p in removed],
                    "base_permits": len(base),
                    "head_permits": len(head),
                },
                indent=2,
            )
        )
        return 0 if not new_grants else 1

    if new_grants:
        print("warrant: this change WIDENS the ACL — it is not auto-approvable.\n")
        print(f"  {len(new_grants)} permit(s) are not covered by the base ACL:\n")
        for p in new_grants[:40]:
            print(f"    + {p.describe()}")
        if len(new_grants) > 40:
            print(f"    ... and {len(new_grants) - 40} more")
        print(
            "\n  A widening waits for the owner CODEOWNERS names for the path.\n"
            "  That is the gate working, not a failure."
        )
        return 1

    if removed:
        print(f"warrant: proven NARROWING — {len(removed)} permit(s) removed, none added.\n")
        for p in removed[:40]:
            print(f"    - {p.describe()}")
        if len(removed) > 40:
            print(f"    ... and {len(removed) - 40} more")
        print(
            "\n  Nothing here grants access that the base ACL did not already grant,\n"
            "  so it can merge without waiting: the only thing it can do is take\n"
            "  access away, and `git revert` puts it back."
        )
    else:
        print("warrant: no change to the permit set.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
