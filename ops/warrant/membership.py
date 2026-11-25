#!/usr/bin/env python3
"""warrant membership — validate the declared peer membership of governed scopes.

    ops/warrant/membership/<scope>/ ──→ membership.py --check ──→ exit 0 / 1

Design: docs/design/mesh-change-control.md §4.1 (the declaration) and §4.3
(validation). ADR 1189.

A SCOPE IS A DIRECTORY. `ops/warrant/membership/<scope>/` holds:

  _scope.yaml                `version: 1`, `scope`, `converge: report|enforce`.
                             Required. The ONLY file that carries scope or
                             converge — it is the switch.
  **/*.yaml                  fragments: `version: 1` + `entries: [...]`, any
                             depth (e.g. `orgs/<org>.yaml`). Writers target
                             holds.yaml, services.yaml, people.yaml and
                             substrate.yaml, and edit an existing entry in
                             place in whichever fragment holds it.
  _baseline.generated.yaml   GENERATED from the operator manifests
                             (--generate-baseline): a top-level handle→email
                             hash map plus each person's `device` and `handset`
                             group sets. The map's placement keeps the file
                             readable by the previous CLI during rollout. The
                             converger uses the group sets before falling back
                             to the minting key.

Any other `_*.yaml`, a `.yml`, and the old single-file `<scope>.yaml` are
refused. One entry per peer ACROSS fragments: the merged view is what the
converger reads, so it is what every rule below is checked against.

THE `_scope.yaml` SAYING `converge: enforce` IS THE SWITCH. A scope whose
declaration enforces is governed by warrant: its peer membership changes
through a reviewed diff here, and every tool that used to write membership
live writes an entry instead. A `converge: report` scope is an adoption draft:
the converger reports against it and the older audited tools still write
live. An unreadable `_scope.yaml` counts as governed. That makes this
validator the gate on the only record of who holds which group, so it is
strict in the permissive direction — an entry it cannot prove is well-formed
is a failure, never a warning.

WHAT IT CHECKS, per scope directory (§4.3):

  - layout: `_scope.yaml` present with only version/scope/converge, `scope`
    equal to the directory name; fragments carry only version/entries;
    no stray `_*.yaml` or `.yml`;
  - `scope` is a taxonomy scope (a key under `netbird_acl_taxonomy` or
    `netbird_exercises` in ops/ansible/netbird_acl_taxonomy.yml);
  - one entry per peer across every fragment; `owner` and `reason`
    (<=200 chars) present;
  - exactly one of `grant`/`absent` (either or both), `exact`,
    `hold: quarantine`;
  - every group is a full name `<site_prefix>-<suffix>` whose suffix the
    scope's taxonomy declares (the baseline's groups too);
  - `expires` required on a hold and on any `grant`/`exact` naming
    EXTERNAL-INFRA, PLATFORM-ADMIN, BREAKGLASS or P2P; a past `expires` fails
    (`expire.py` opens the removal PR, as for rule intents);
  - EXTERNAL-INFRA only as `exact: [<P>-EXTERNAL-INFRA]` — an ADR 1178 service
    host holds it alone, never beside a person role (#1286);
  - PLATFORM-ADMIN only on a person device (`peer-…`), never a machine;
  - a quarantine entry cannot target a person whose declared baseline carries
    PLATFORM-ADMIN; live Ory and mesh facts are runtime checks, not CI guesses;
  - `_baseline.generated.yaml` is present, well-formed, and — when the
    operator manifests are in this checkout — byte-identical to what
    --generate-baseline would write now (stale = fail, like generate-pins).
    The manifests are gitignored (they carry identity material), so a
    checkout without them (CI) says the freshness is unverified rather than
    passing it silently.

It also validates the network read-back declarations
(`ops/warrant/network/<scope>.yaml`, §5), because the same workflow gates both.

Importable: `load`, `load_scope`, `validate`, `validate_scope`, `check_scope`,
`taxonomy_scopes`, `expired_entries`, `render_entry`, `upsert_entry`,
`remove_entries`, `upsert_scope_entry`, `remove_scope_entries`, `find_entry`,
`governed_dir`, `generate_baseline`, `render_baseline` — the finish script,
the lifecycle sweep, expire.py and the narrowing prover reuse these rather than
re-reading the layout their own way.

Usage:
    membership.py --check [--scope S]              # exit 1 on any violation
    membership.py --generate-baseline [--scope S]  # write _baseline.generated.yaml
    membership.py --list-expired [--scope S]       # what is past due, exit 0 always
    ... --operators-dir DIR                        # where the manifests are
    ... --today YYYY-MM-DD                         # pretend it is that day (tests)

Exit: 0 clean, 1 violation, 2 a source could not be read.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import re
import sys
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover
    sys.exit("warrant: PyYAML is required (pip install pyyaml)")

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[1]
MEMBERSHIP_DIR = HERE / "membership"
NETWORK_DIR = HERE / "network"
TAXONOMY = REPO_ROOT / "ops" / "ansible" / "netbird_acl_taxonomy.yml"

TOP_KEYS = {"version", "scope", "converge", "entries"}
SCOPE_FILE = "_scope.yaml"
BASELINE_FILE = "_baseline.generated.yaml"
SCOPE_KEYS = {"version", "scope", "converge"}
FRAGMENT_KEYS = {"version", "entries"}
BASELINE_KEYS = {"version", "scope", "generated_from", "identity_hashes", "people"}
BASELINE_PERSON_KEYS = {"device", "handset"}
OPERATORS_DIR = REPO_ROOT / "ops" / "operators"

# Where each writer puts a NEW entry. An existing entry for the peer is edited
# in place in whichever fragment already holds it.
FRAGMENT_HOLDS = "holds.yaml"          # lifecycle sweep, quarantine holds
FRAGMENT_SERVICES = "services.yaml"    # ADR 1178 external-service hosts
FRAGMENT_PEOPLE = "people.yaml"        # members move, platform-admin grant/absent
FRAGMENT_SUBSTRATE = "substrate.yaml"  # hand-kept INFRA hosts
FRAGMENT_PURPOSE = {
    FRAGMENT_HOLDS: "holds: dormancy and quarantine holds (lifecycle-sweep.py, §4.6)",
    FRAGMENT_SERVICES: "external-service hosts holding EXTERNAL-INFRA alone (ADR 1178)",
    FRAGMENT_PEOPLE: "person devices that differ from their declared baseline",
    FRAGMENT_SUBSTRATE: "substrate hosts (INFRA), kept by hand",
}

# Manifest auto_groups tokens retired by the grammar, and the group each one
# minted as. VENDOR -> DEVELOPER is ops/operators/validate.py
# _DEPRECATED_GROUPS (ADR 1043); ADMIN is the pre-rename PLATFORM-ADMIN token
# (goat-id-console maps grant_ADMINS -> grant_PLATFORM-ADMIN), and every live
# zagros key minted from an ADMIN manifest carries <P>-PLATFORM-ADMIN.
LEGACY_SUFFIX = {"VENDOR": "DEVELOPER", "ADMIN": "PLATFORM-ADMIN"}
ENTRY_KEYS = {"peer", "peer_id", "owner", "reason", "expires", "grant", "absent", "exact", "hold"}
CONVERGE_MODES = ("report", "enforce")
HOLD_KINDS = ("quarantine",)
REASON_MAX = 200
# A grant of any of these suffixes widens reach enough that it must lapse.
EXPIRY_REQUIRED_SUFFIXES = {"EXTERNAL-INFRA", "PLATFORM-ADMIN", "BREAKGLASS", "P2P"}
PERSON_PEER_PREFIX = "peer-"

NETWORK_TOP_KEYS = {"version", "scope", "nameserver_groups", "routes"}


class SourceError(Exception):
    """A file that could not be read or parsed. Exit 2, never a pass."""


# ---- the taxonomy ------------------------------------------------------------


def taxonomy_scopes(path: Path = TAXONOMY) -> dict[str, tuple[str, set[str]]]:
    """Every governed-able scope -> (site_prefix, set of group suffixes).

    Goatnet scopes are the keys of `netbird_acl_taxonomy` (except the
    `_default` template). Exercise and site scopes are the keys of
    `netbird_exercises`; their groups come from `netbird_exercise_template`,
    plus `netbird_exercise_gear_template` when the scope carries `gear: true`.
    """
    try:
        doc = yaml.safe_load(Path(path).read_text()) or {}
    except (OSError, yaml.YAMLError) as exc:
        raise SourceError(f"cannot read taxonomy {path}: {exc}") from exc

    def suffixes(groups) -> set[str]:
        return {str(g["suffix"]) for g in (groups or []) if isinstance(g, dict) and g.get("suffix")}

    out: dict[str, tuple[str, set[str]]] = {}
    for scope, cfg in (doc.get("netbird_acl_taxonomy") or {}).items():
        if scope.startswith("_") or not isinstance(cfg, dict):
            continue
        out[scope] = (str(cfg.get("site_prefix") or scope.upper()), suffixes(cfg.get("groups")))
    ex_groups = suffixes((doc.get("netbird_exercise_template") or {}).get("groups"))
    gear_groups = suffixes((doc.get("netbird_exercise_gear_template") or {}).get("groups"))
    for scope, cfg in (doc.get("netbird_exercises") or {}).items():
        if not isinstance(cfg, dict):
            continue
        groups = set(ex_groups) | (gear_groups if cfg.get("gear") else set())
        out[scope] = (str(cfg.get("site_prefix") or scope.upper()), groups)
    return out


# ---- reading -----------------------------------------------------------------


def load(path: Path) -> dict:
    try:
        doc = yaml.safe_load(Path(path).read_text())
    except (OSError, yaml.YAMLError) as exc:
        raise SourceError(f"cannot read {path}: {exc}") from exc
    if doc is None:
        doc = {}
    if not isinstance(doc, dict):
        raise SourceError(f"{path}: top level must be a mapping, got {type(doc).__name__}")
    return doc


def parse_date(raw) -> dt.date | None:
    if isinstance(raw, dt.datetime):
        return raw.date()
    if isinstance(raw, dt.date):
        return raw
    try:
        return dt.date.fromisoformat(str(raw).strip().strip('"').strip("'"))
    except ValueError:
        return None


def entry_mode(entry: dict) -> str | None:
    """`hold`, `exact` or `delta` (grant/absent); None when zero or several."""
    modes = []
    if "hold" in entry:
        modes.append("hold")
    if "exact" in entry:
        modes.append("exact")
    if "grant" in entry or "absent" in entry:
        modes.append("delta")
    return modes[0] if len(modes) == 1 else None


def groups_of(entry: dict, *fields: str) -> list[str]:
    out: list[str] = []
    for f in fields or ("grant", "exact"):
        v = entry.get(f)
        if isinstance(v, list):
            out.extend(str(g) for g in v)
    return out


def suffix_of(group: str, prefix: str) -> str | None:
    lead = prefix + "-"
    return group[len(lead):] if group.startswith(lead) else None


# ---- validation --------------------------------------------------------------


def validate(
    doc: dict,
    *,
    stem: str | None = None,
    scopes: dict[str, tuple[str, set[str]]] | None = None,
    today: dt.date | None = None,
    labels: list[str] | None = None,
) -> list[str]:
    """Every violation in one membership view. Empty list = valid.

    *doc* is the merged view of a scope (`version`, `scope`, `converge`,
    `entries`); *stem* the scope directory's name. *labels*, parallel to
    `entries`, names where each entry came from (`people.yaml entries[3]`),
    so a violation points at the fragment to edit.
    """
    today = today or dt.date.today()
    scopes = scopes if scopes is not None else taxonomy_scopes()
    errs: list[str] = []

    unknown = set(doc) - TOP_KEYS
    if unknown:
        errs.append(f"unknown top-level key(s): {', '.join(sorted(unknown))}")
    if doc.get("version") != 1:
        errs.append(f"version must be 1, got {doc.get('version')!r}")
    scope = doc.get("scope")
    if not isinstance(scope, str) or not scope:
        errs.append("scope is required")
        return errs
    if stem is not None and scope != stem:
        errs.append(f"scope {scope!r} does not match the directory name {stem!r}")
    if scope not in scopes:
        errs.append(
            f"scope {scope!r} is not a taxonomy scope in {TAXONOMY.relative_to(REPO_ROOT)} "
            "(a key under netbird_acl_taxonomy or netbird_exercises)"
        )
        return errs
    prefix, suffixes = scopes[scope]
    if doc.get("converge") not in CONVERGE_MODES:
        errs.append(f"converge must be one of {'|'.join(CONVERGE_MODES)}, got {doc.get('converge')!r}")
    entries = doc.get("entries")
    if entries is None:
        entries = []
    if not isinstance(entries, list):
        errs.append("entries must be a list")
        return errs

    seen: dict[str, str] = {}
    for i, e in enumerate(entries):
        where = labels[i] if labels and i < len(labels) else f"entries[{i}]"
        if not isinstance(e, dict):
            errs.append(f"{where}: must be a mapping")
            continue
        peer = e.get("peer")
        if not isinstance(peer, str) or not peer.strip():
            errs.append(f"{where}: peer is required (the exact NetBird peer name)")
            peer = None
        else:
            first = where
            where = f"{where} ({peer})"
            if peer in seen:
                errs.append(f"{where}: duplicate peer — already declared at {seen[peer]}; one entry per peer "
                            "across every fragment of the scope")
            else:
                seen[peer] = first
        unknown = set(e) - ENTRY_KEYS
        if unknown:
            errs.append(f"{where}: unknown key(s): {', '.join(sorted(unknown))}")
        if "peer_id" in e:
            peer_id = e["peer_id"]
            if not isinstance(peer_id, str) or not peer_id.strip() or peer_id != peer_id.strip():
                errs.append(f"{where}: peer_id must be a non-empty string without surrounding whitespace")
        owner = e.get("owner")
        if not isinstance(owner, str) or not owner.strip():
            errs.append(f"{where}: owner is required")
        reason = e.get("reason")
        if not isinstance(reason, str) or not reason.strip():
            errs.append(f"{where}: reason is required")
        elif len(reason) > REASON_MAX:
            errs.append(f"{where}: reason is {len(reason)} chars; the limit is {REASON_MAX}")

        mode = entry_mode(e)
        if mode is None:
            errs.append(f"{where}: exactly one of grant/absent, exact, hold: quarantine is required")
            continue

        # Every group list: a non-empty list of full names the scope declares.
        for field in ("grant", "absent", "exact"):
            if field not in e:
                continue
            v = e[field]
            if not isinstance(v, list) or not v:
                errs.append(f"{where}: {field} must be a non-empty list of group names")
                continue
            if len(set(map(str, v))) != len(v):
                errs.append(f"{where}: {field} names a group twice")
            for g in v:
                g = str(g)
                sfx = suffix_of(g, prefix)
                if sfx is None:
                    errs.append(f"{where}: {field} group {g!r} does not carry the scope prefix {prefix}-")
                elif sfx not in suffixes:
                    errs.append(f"{where}: {field} group {g!r} — suffix {sfx} is not in the {scope} taxonomy")
        if mode == "delta":
            both = set(groups_of(e, "grant")) & set(groups_of(e, "absent"))
            if both:
                errs.append(f"{where}: {', '.join(sorted(both))} is both granted and absent")
        if mode == "hold" and e.get("hold") not in HOLD_KINDS:
            errs.append(f"{where}: hold must be one of {'|'.join(HOLD_KINDS)}, got {e.get('hold')!r}")

        # expires: required where the entry holds something that must lapse.
        held = groups_of(e, "grant", "exact")
        held_sfx = {suffix_of(g, prefix) for g in held}
        needs_expiry = mode == "hold" or bool(held_sfx & EXPIRY_REQUIRED_SUFFIXES)
        if "expires" in e and e["expires"] is not None:
            when = parse_date(e["expires"])
            if when is None:
                errs.append(f"{where}: expires {e['expires']!r} is not a YYYY-MM-DD date")
            elif when < today:
                errs.append(
                    f"{where}: expired {when.isoformat()} — past due; "
                    "`ops/warrant/expire.py --apply` removes it"
                )
        elif needs_expiry:
            why = "a hold" if mode == "hold" else (
                "it grants " + ", ".join(sorted(s for s in held_sfx & EXPIRY_REQUIRED_SUFFIXES))
            )
            errs.append(f"{where}: expires is required because {why}")

        # EXTERNAL-INFRA: held alone, by exact, or not at all.
        ext = f"{prefix}-EXTERNAL-INFRA"
        if ext in groups_of(e, "grant"):
            errs.append(
                f"{where}: {ext} may only be held as `exact: [{ext}]` — an external-service "
                "host holds it alone, never on top of a key baseline (#1286)"
            )
        if ext in groups_of(e, "exact") and groups_of(e, "exact") != [ext]:
            errs.append(
                f"{where}: {ext} may only be held alone — `exact: [{ext}]`, never beside "
                "another group (#1286)"
            )

        # PLATFORM-ADMIN: a person's device, never a machine.
        pa = f"{prefix}-PLATFORM-ADMIN"
        if pa in held and peer and not peer.startswith(PERSON_PEER_PREFIX):
            errs.append(
                f"{where}: {pa} is for a person device (peer name `{PERSON_PEER_PREFIX}…`), "
                "never a machine"
            )
    return errs


def validate_network(doc: dict, *, stem: str | None = None, scopes=None) -> list[str]:
    """ops/warrant/network/<scope>.yaml: version 1, scope, two lists of strings."""
    scopes = scopes if scopes is not None else taxonomy_scopes()
    errs: list[str] = []
    unknown = set(doc) - NETWORK_TOP_KEYS
    if unknown:
        errs.append(f"unknown top-level key(s): {', '.join(sorted(unknown))}")
    if doc.get("version") != 1:
        errs.append(f"version must be 1, got {doc.get('version')!r}")
    scope = doc.get("scope")
    if not isinstance(scope, str) or not scope:
        errs.append("scope is required")
    else:
        if stem is not None and scope != stem:
            errs.append(f"scope {scope!r} does not match the file name {stem!r}.yaml")
        if scope not in scopes:
            errs.append(f"scope {scope!r} is not a taxonomy scope")
    for field in ("nameserver_groups", "routes"):
        v = doc.get(field)
        if v is None:
            continue
        if not isinstance(v, list) or not all(isinstance(x, str) and x for x in v):
            errs.append(f"{field} must be a list of non-empty strings")
        elif len(set(v)) != len(v):
            errs.append(f"{field} names an item twice")
    return errs


def expired_entries(doc: dict, today: dt.date | None = None) -> list[tuple[dict, dt.date]]:
    today = today or dt.date.today()
    out = []
    for e in doc.get("entries") or []:
        if isinstance(e, dict) and e.get("expires") is not None:
            when = parse_date(e["expires"])
            if when is not None and when < today:
                out.append((e, when))
    return out


# ---- the scope directory -----------------------------------------------------


def scope_dir(scope: str, directory: Path = MEMBERSHIP_DIR) -> Path:
    return Path(directory) / scope


def scope_dirs(directory: Path = MEMBERSHIP_DIR, scope: str | None = None) -> list[Path]:
    """Every scope directory under *directory* (or just *scope*'s)."""
    directory = Path(directory)
    if scope:
        return [directory / scope]
    if not directory.is_dir():
        return []
    return sorted(p for p in directory.iterdir() if p.is_dir() and not p.name.startswith("."))


def legacy_files(directory: Path = MEMBERSHIP_DIR) -> list[Path]:
    """Old single-file declarations (`<scope>.yaml`/`.yml` beside the dirs) — refused."""
    directory = Path(directory)
    if not directory.is_dir():
        return []
    return sorted(p for p in directory.iterdir() if p.is_file() and p.suffix in (".yaml", ".yml"))


def scope_files(sdir: Path) -> tuple[Path, Path, list[Path], list[str]]:
    """(_scope.yaml, _baseline.generated.yaml, fragments, refusals) of one scope.

    Fragments are every `*.yaml` at any depth that does not start with `_`.
    A `.yml`, or a `_*.yaml` other than the two reserved names at the top, is
    a refusal: the converger would read one layout and a person another.
    """
    sdir = Path(sdir)
    fragments: list[Path] = []
    refused: list[str] = []
    for p in sorted(sdir.rglob("*")):
        if not p.is_file() or any(part.startswith(".") for part in p.relative_to(sdir).parts):
            continue
        rel = p.relative_to(sdir).as_posix()
        if p.suffix == ".yml":
            refused.append(f"{rel}: fragments are `.yaml`; a `.yml` would be read by nothing")
        elif p.suffix != ".yaml":
            continue
        elif p.name.startswith("_"):
            if rel not in (SCOPE_FILE, BASELINE_FILE):
                refused.append(
                    f"{rel}: `_` names are reserved — only {SCOPE_FILE} and {BASELINE_FILE}, "
                    "at the top of the scope directory")
        else:
            fragments.append(p)
    return sdir / SCOPE_FILE, sdir / BASELINE_FILE, fragments, refused


def load_scope(sdir: Path) -> dict:
    """The merged view of one scope directory.

    Returns {`version`, `scope`, `converge`, `entries`, `labels`, `entry_files`,
    `baseline`, `dir`}: `entries` is every fragment's entries in path order,
    `labels`/`entry_files` parallel to it. `baseline` is the parsed
    _baseline.generated.yaml or None. Raises SourceError on a file that is not
    YAML or not a mapping; a missing _scope.yaml leaves scope/converge None
    (validate_scope reports it).
    """
    sdir = Path(sdir)
    if not sdir.is_dir():
        raise SourceError(f"{sdir} is not a scope directory")
    sf, bf, fragments, _ = scope_files(sdir)
    head = load(sf) if sf.is_file() else {}
    entries: list = []
    labels: list[str] = []
    files: list[Path] = []
    for f in fragments:
        doc = load(f)
        rel = f.relative_to(sdir).as_posix()
        got = doc.get("entries")
        if isinstance(got, list):
            for i, e in enumerate(got):
                entries.append(e)
                labels.append(f"{rel} entries[{i}]")
                files.append(f)
    return {
        "version": head.get("version"),
        "scope": head.get("scope"),
        "converge": head.get("converge"),
        "entries": entries,
        "labels": labels,
        "entry_files": files,
        "baseline": load(bf) if bf.is_file() else None,
        "dir": sdir,
    }


def merged_doc(view: dict) -> dict:
    """A load_scope view as the plain document validate()/the prover read."""
    return {k: view.get(k) for k in ("version", "scope", "converge", "entries")}


def validate_baseline(doc: dict, *, scope: str, prefix: str, suffixes: set[str]) -> list[str]:
    errs: list[str] = []
    where = BASELINE_FILE
    unknown = set(doc) - BASELINE_KEYS
    if unknown:
        errs.append(f"{where}: unknown key(s): {', '.join(sorted(unknown))}")
    if doc.get("version") != 1:
        errs.append(f"{where}: version must be 1, got {doc.get('version')!r}")
    if doc.get("scope") != scope:
        errs.append(f"{where}: scope {doc.get('scope')!r} is not {scope!r}")
    people = doc.get("people")
    if people is None:
        people = {}
    if not isinstance(people, dict):
        return errs + [f"{where}: people must be a mapping of handle -> {{device, handset}}"]
    identity_hashes = doc.get("identity_hashes")
    if not isinstance(identity_hashes, dict):
        errs.append(f"{where}: identity_hashes must be a mapping of handle -> SHA-256 hash")
        identity_hashes = {}
    for handle, value in identity_hashes.items():
        if handle not in people:
            errs.append(f"{where} identity_hashes.{handle}: has no matching people entry")
        if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
            errs.append(f"{where} identity_hashes.{handle}: must be a 64-character lowercase SHA-256 hash")
    for handle, sets in people.items():
        w = f"{where} people.{handle}"
        if not isinstance(handle, str) or not handle:
            errs.append(f"{w}: a handle must be a non-empty string")
        if not isinstance(sets, dict):
            errs.append(f"{w}: must be a mapping with device and handset")
            continue
        bad = set(sets) - BASELINE_PERSON_KEYS
        if bad or "device" not in sets:
            errs.append(f"{w}: needs device (and handset unless the person mints no handset key), "
                        f"got {', '.join(sorted(map(str, sets)))}")
        if handle not in identity_hashes:
            errs.append(f"{where} identity_hashes.{handle}: is required for every person")
        for kind in sorted(BASELINE_PERSON_KEYS & set(sets)):
            v = sets[kind]
            if not isinstance(v, list):
                errs.append(f"{w}.{kind}: must be a list of group names")
                continue
            for g in v:
                sfx = suffix_of(str(g), prefix)
                if sfx is None or sfx not in suffixes:
                    errs.append(f"{w}.{kind}: {g!r} is not a {scope} taxonomy group")
    return errs


def validate_privileged_quarantine(view: dict, *, scope: str, prefix: str) -> list[str]:
    """Reject quarantine transitions that conflict with a declared admin baseline.

    CI deliberately proves only repository facts. Live Ory roles and current
    NetBird membership are checked by the authoring command and converger.
    Ambiguous handle matches are safe here only when every declared candidate
    is ordinary.
    """
    baseline = view.get("baseline") or {}
    people = baseline.get("people") or {}
    if not isinstance(people, dict):
        return []
    errs = []
    labels = view.get("labels") or []
    quarantine = f"{prefix}-QUARANTINED"
    for i, entry in enumerate(view.get("entries") or []):
        if not isinstance(entry, dict):
            continue
        changes_quarantine = (
            entry.get("hold") == "quarantine"
            or quarantine in groups_of(entry, "grant", "absent", "exact")
        )
        if not changes_quarantine:
            continue
        peer = entry.get("peer")
        if not isinstance(peer, str):
            continue
        matches = resolve_handle_candidates(peer, people, scope)
        where = labels[i] if i < len(labels) else f"entries[{i}]"
        if not matches:
            continue
        privileged_matches = []
        for handle in matches:
            person = people.get(handle) or {}
            if not isinstance(person, dict):
                continue
            groups = list(person.get("device") or []) + list(person.get("handset") or [])
            if f"{prefix}-PLATFORM-ADMIN" in groups:
                privileged_matches.append(handle)
        if len(matches) > 1 and privileged_matches:
            errs.append(
                f"{where} ({peer}): ownership is ambiguous and one possible owner has a declared "
                "PLATFORM-ADMIN baseline; quarantine is refused"
            )
        elif len(matches) == 1 and privileged_matches:
            errs.append(
                f"{where} ({peer}): declared PLATFORM-ADMIN baseline cannot use a QUARANTINED transition; "
                "use the ADR 1091 severance and credential-rotation path"
            )
    return errs


def validate_scope(
    sdir: Path,
    *,
    scopes: dict[str, tuple[str, set[str]]] | None = None,
    today: dt.date | None = None,
) -> list[str]:
    """Every violation in one scope directory. Raises SourceError when a file
    cannot be read — exit 2, never a pass."""
    scopes = scopes if scopes is not None else taxonomy_scopes()
    sdir = Path(sdir)
    sf, bf, fragments, refused = scope_files(sdir)
    errs = list(refused)
    if not sf.is_file():
        errs.append(f"{SCOPE_FILE} is missing — it carries scope and converge, the switch")
    else:
        head = load(sf)
        unknown = set(head) - SCOPE_KEYS
        if unknown:
            errs.append(f"{SCOPE_FILE}: unknown key(s): {', '.join(sorted(unknown))} "
                        "(entries live in fragments, not here)")
    for f in fragments:
        doc = load(f)
        rel = f.relative_to(sdir).as_posix()
        extra = set(doc) - FRAGMENT_KEYS
        if extra & {"scope", "converge"}:
            errs.append(f"{rel}: only {SCOPE_FILE} carries scope/converge")
        if extra - {"scope", "converge"}:
            errs.append(f"{rel}: unknown top-level key(s): {', '.join(sorted(extra - {'scope', 'converge'}))}")
        if doc.get("version") != 1:
            errs.append(f"{rel}: version must be 1, got {doc.get('version')!r}")
        if doc.get("entries") is not None and not isinstance(doc.get("entries"), list):
            errs.append(f"{rel}: entries must be a list")
    view = load_scope(sdir)
    if sf.is_file():
        errs.extend(validate(merged_doc(view), stem=sdir.name, scopes=scopes, today=today,
                             labels=view["labels"]))
    scope = view.get("scope")
    if isinstance(scope, str) and scope in scopes:
        prefix, suffixes = scopes[scope]
        if view["baseline"] is None:
            errs.append(f"{BASELINE_FILE} is missing — run `ops/warrant/membership.py "
                        f"--generate-baseline --scope {scope}`")
        else:
            errs.extend(validate_baseline(view["baseline"], scope=scope, prefix=prefix, suffixes=suffixes))
            errs.extend(validate_privileged_quarantine(view, scope=scope, prefix=prefix))
    return errs


def check_scope(sdir: Path, *, today: dt.date | None = None, scopes=None) -> list[str]:
    return validate_scope(sdir, scopes=scopes, today=today)


def converge_of(path: Path) -> str | None:
    """The `converge:` of a scope (its directory or its _scope.yaml), read
    without a YAML parser. None when it cannot be read."""
    path = Path(path)
    if path.is_dir():
        path = path / SCOPE_FILE
    try:
        text = path.read_text()
    except OSError:
        return None
    for line in text.splitlines():
        m = re.match(r"^converge:\s*([a-z]+)", line)
        if m:
            return m.group(1)
    return None


def governed_dir(scope: str, directory: Path = MEMBERSHIP_DIR) -> Path | None:
    """The scope directory that makes *scope* governed, or None.

    Governed means the directory exists AND its _scope.yaml does not say
    `converge: report`. A `report` scope is an adoption draft: the converger
    reports against it while the older audited tools still write the live
    account. Only an explicit `report` is a draft; a missing or unreadable
    _scope.yaml fails toward governed, so no tool writes live on a declaration
    it cannot read. The old single-file layout is refused outright.
    """
    directory = Path(directory)
    legacy = directory / f"{scope}.yaml"
    if legacy.is_file():
        raise SourceError(
            f"{legacy}: the single-file layout is retired — a scope is the directory "
            f"{directory / scope}/ (_scope.yaml + fragments)")
    p = directory / scope
    if not p.is_dir():
        return None
    return None if converge_of(p) == "report" else p


# Kept for callers written against the single-file layout; it now answers with
# the governed scope DIRECTORY.
governed_file = governed_dir


def find_entry(sdir: Path, peer: str) -> Path | None:
    """The fragment that holds *peer*'s entry, or None."""
    for f in scope_files(sdir)[2]:
        for p, _s, _e in _blocks(Path(f).read_text().split("\n")):
            if p == peer:
                return f
    return None


# ---- the declared baseline ---------------------------------------------------


def load_manifests(operators_dir: Path = OPERATORS_DIR) -> list[tuple[Path, dict]]:
    """Every per-goatnet operator manifest (`<operators_dir>/<env>/*.yml`).

    The manifests are gitignored — they carry identity material — so a
    checkout that is not the operator's own clone has none; callers must
    treat an empty answer as "cannot tell", never as "nobody".
    """
    out = []
    root = Path(operators_dir)
    if not root.is_dir():
        return out
    for f in sorted(root.glob("*/*.yml")):
        try:
            doc = yaml.safe_load(f.read_text())
        except (OSError, yaml.YAMLError) as exc:
            raise SourceError(f"cannot read manifest {f}: {exc}") from exc
        if isinstance(doc, dict) and isinstance(doc.get("identity"), dict):
            out.append((f, doc))
    return out


def generate_baseline(
    scope: str,
    *,
    scopes: dict[str, tuple[str, set[str]]] | None = None,
    manifests: list[tuple[Path, dict]] | None = None,
    operators_dir: Path = OPERATORS_DIR,
) -> dict:
    """The declared baseline for *scope* from the operator manifests.

    For every manifest with `identity.status: active`, `netbird.enabled` not
    false and *scope* in `netbird.sites`: `device` = its `netbird.auto_groups`
    as `<site_prefix>-<suffix>` (legacy tokens translated, LEGACY_SUFFIX);
    `handset` = `<P>-OBSERVER` only — a phone or tablet key is OBSERVER-only
    whatever auto_groups says (ADR 1175). `<P>-HANDSET` is left out: no ACL
    rule names it, so the converger could never add it past its coverage check.
    A person with `netbird.handset_limit: 0` mints no handset key, so their
    entry carries `device` only.

    Raises SourceError on a suffix the taxonomy does not declare or on two
    active manifests claiming one handle: a baseline that guessed would move
    real peers.
    """
    scopes = scopes if scopes is not None else taxonomy_scopes()
    if scope not in scopes:
        raise SourceError(f"{scope!r} is not a taxonomy scope")
    prefix, suffixes = scopes[scope]
    if manifests is None:
        manifests = load_manifests(operators_dir)
    people: dict[str, dict[str, list[str]]] = {}
    identity_hashes: dict[str, str] = {}
    source: dict[str, Path] = {}
    problems: list[str] = []
    for f, doc in manifests:
        ident = doc.get("identity") or {}
        nb = doc.get("netbird") or {}
        if not isinstance(nb, dict) or ident.get("status") != "active" or nb.get("enabled") is False:
            continue
        if scope not in [str(s) for s in (nb.get("sites") or [])]:
            continue
        handle = str(ident.get("handle") or "").strip()
        if not handle:
            problems.append(f"{f}: active manifest with no identity.handle")
            continue
        if handle in people:
            problems.append(f"{f}: handle {handle!r} is also claimed by {source[handle]}")
            continue
        device = set()
        for tok in nb.get("auto_groups") or []:
            sfx = LEGACY_SUFFIX.get(str(tok), str(tok))
            if sfx not in suffixes:
                problems.append(f"{f}: auto_groups token {tok!r} is not a {scope} taxonomy group")
                continue
            device.add(f"{prefix}-{sfx}")
        email = str(ident.get("email") or "").strip().lower()
        if not email:
            problems.append(f"{f}: active manifest with no identity.email")
            continue
        identity_hashes[handle] = hashlib.sha256(email.encode()).hexdigest()
        people[handle] = {"device": sorted(device)}
        if nb.get("handset_limit") != 0:
            people[handle]["handset"] = [f"{prefix}-OBSERVER"]
        source[handle] = f
    if problems:
        raise SourceError("cannot generate the baseline for " + scope + ":\n  - " + "\n  - ".join(problems))
    return {
        "version": 1,
        "scope": scope,
        "generated_from": (
            f"ops/operators/<env>/*.yml: active manifests whose netbird.sites includes {scope} "
            "(ops/warrant/membership.py --generate-baseline)"),
        "identity_hashes": dict(sorted(identity_hashes.items())),
        "people": dict(sorted(people.items())),
    }


def resolve_handle(peer_name: str, handles, scope: str) -> str | None:
    """The declared handle a peer name resolves to on *scope*, or None.

    Anchored, as the Rust converger reads it: `peer-<H>` resolves to H only
    when the rest of the name is `(-<org>)?-<scope>-<YYYY-MM-DD>(-...)?`.
    Among the handles that match, the longest wins, so `maintainer` never claims
    `peer-maintainer-smith-zagros-…` when `maintainer-smith` is declared, and a peer of
    another goatnet is never claimed at all.
    """
    matches = resolve_handle_candidates(peer_name, handles, scope)
    return max(matches, key=len) if matches else None


def resolve_handle_candidates(peer_name: str, handles, scope: str) -> list[str]:
    """Every declared handle whose person-peer grammar matches *peer_name*.

    Read-only attribution may choose the longest candidate. A quarantine
    security decision evaluates every syntactically valid candidate and allows
    ambiguity only when all candidates are proven ordinary. This is byte-for-byte
    equivalent to the Rust candidate rule: besides the v1.1.0 grammar, it accepts
    only the pre-v1.1.0 exact-repeat form `peer-H-H-<scope>-<date>...`.
    """
    if not peer_name.startswith(PERSON_PEER_PREFIX):
        return []
    tail = re.compile(
        r"^(-[^-]+)?-" + re.escape(scope) + r"-[0-9]{4}-[0-9]{2}-[0-9]{2}(-[\s\S]*)?\Z"
    )
    matches = []
    for h in handles:
        h = str(h)
        lead = PERSON_PEER_PREFIX + h
        rest = peer_name[len(lead):] if h and peer_name.startswith(lead + "-") else ""
        doubled = rest.removeprefix("-" + h) if rest.startswith("-" + h) else ""
        if rest and (tail.match(rest) or (doubled and tail.match(doubled))):
            matches.append(h)
    return matches


def _key_scalar(s: str) -> str:
    """A mapping key YAML reads back as this exact string (`user290x` is an int)."""
    try:
        same = yaml.safe_load(f"{s}: x") == {s: "x"}
    except yaml.YAMLError:
        same = False
    return s if same and not re.search(r"[:#\[\]{},&*!|>'\"%@`\s]", s) else _yaml_scalar_quoted(s)


def _yaml_scalar_quoted(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def render_baseline(doc: dict) -> str:
    """_baseline.generated.yaml text. Deterministic: sorted, no timestamps, so
    a stale file is a plain byte difference."""
    lines = [
        "# GENERATED by ops/warrant/membership.py --generate-baseline. Do not edit:",
        "# change the operator manifest and regenerate. membership.py --check fails",
        "# when this file differs from what the manifests produce.",
        "#",
        "# Each person's baseline on this goatnet (docs/design/mesh-change-control.md",
        "# §4.2): a peer named peer-<handle>[-<org>]-<scope>-<date>... for a handle",
        "# below starts from `handset` when it enrolled on the person's handset key,",
        "# else `device`; entries in the fragments then grant, remove, pin or hold as",
        "# usual. `identity_hashes` joins each owner to live Ory without deploying email.",
        "# A person who mints no handset key (handset_limit: 0) has `device` only.",
        "version: 1",
        f"scope: {doc['scope']}",
        f"generated_from: {_yaml_scalar(doc['generated_from'])}",
    ]
    identity_hashes = doc.get("identity_hashes") or {}
    if not identity_hashes:
        lines.append("identity_hashes: {}")
    else:
        lines.append("identity_hashes:")
        for handle, value in identity_hashes.items():
            lines.append(f"  {_key_scalar(handle)}: {value}")
    people = doc.get("people") or {}
    if not people:
        lines.append("people: {}")
    else:
        lines.append("people:")
        for handle, sets in people.items():
            lines.append(f"  {_key_scalar(handle)}:")
            for kind in ("device", "handset"):
                if kind in sets:
                    lines.append(f"    {kind}: [{', '.join(sets[kind])}]")
    return "\n".join(lines) + "\n"


def baseline_status(sdir: Path, *, scopes=None, manifests=None, operators_dir: Path = OPERATORS_DIR) -> tuple[str, str]:
    """("fresh"|"stale"|"unverified", message) for one scope's baseline."""
    sdir = Path(sdir)
    if manifests is None:
        manifests = load_manifests(operators_dir)
    if not manifests:
        return "unverified", (
            f"{BASELINE_FILE} freshness unverified: no operator manifests under {operators_dir} "
            "(they are gitignored; run from the clone that has them, or pass --operators-dir)")
    want = render_baseline(generate_baseline(sdir.name, scopes=scopes, manifests=manifests))
    bf = sdir / BASELINE_FILE
    have = bf.read_text() if bf.is_file() else ""
    if have == want:
        return "fresh", f"{BASELINE_FILE} matches the manifests"
    return "stale", (
        f"{BASELINE_FILE} is out of date with the operator manifests — run "
        f"`ops/warrant/membership.py --generate-baseline --scope {sdir.name}` and commit it")


# ---- writing, line-oriented --------------------------------------------------
#
# Line-oriented on purpose, as expire.py is: the files carry hand-written
# comments a YAML round-trip would destroy.

RE_ENTRY_START = re.compile(r"^(?P<indent>\s*)-\s+peer:\s*(?P<peer>\S.*?)\s*$")
RE_TOP_KEY = re.compile(r"^[A-Za-z_][\w-]*\s*:")
RE_ENTRIES_EMPTY = re.compile(r"^entries:\s*\[\s*\]\s*(#.*)?$")
RE_ENTRIES = re.compile(r"^entries:\s*(#.*)?$")


def _unquote(s: str) -> str:
    s = s.split(" #", 1)[0].strip()
    if len(s) >= 2 and s[0] == s[-1] and s[0] in "\"'":
        return s[1:-1]
    return s


def _blocks(lines: list[str]) -> list[tuple[str, int, int]]:
    """(peer, start, end-exclusive) for every entry block."""
    starts = []
    for i, line in enumerate(lines):
        if line.lstrip().startswith("#"):
            continue
        m = RE_ENTRY_START.match(line)
        if m:
            starts.append((i, _unquote(m.group("peer"))))
    out = []
    for idx, (start, peer) in enumerate(starts):
        end = starts[idx + 1][0] if idx + 1 < len(starts) else len(lines)
        for j in range(start + 1, end):
            if RE_TOP_KEY.match(lines[j]):
                end = j
                break
        # Trailing comments/blank lines are the NEXT block's banner.
        while end - 1 > start and (lines[end - 1].strip() == "" or lines[end - 1].lstrip().startswith("#")):
            end -= 1
        out.append((peer, start, end))
    return out


def _yaml_scalar(v) -> str:
    if isinstance(v, (dt.date, dt.datetime)):
        return v.isoformat()[:10]
    s = str(v)
    if s == "" or re.search(r"[:#\[\]{},&*!|>'\"%@`]|^\s|\s$|^-", s):
        return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'
    return s


def render_entry(entry: dict, indent: str = "  ") -> str:
    """One entry as YAML lines, in the field order the design uses."""
    lines = [f"{indent}- peer: {_yaml_scalar(entry['peer'])}"]
    pad = indent + "  "
    if "peer_id" in entry:
        lines.append(f"{pad}peer_id: {json.dumps(entry['peer_id'], ensure_ascii=False)}")
    for field in ("hold", "exact", "grant", "absent"):
        if field in entry:
            v = entry[field]
            if isinstance(v, list):
                lines.append(f"{pad}{field}: [{', '.join(_yaml_scalar(g) for g in v)}]")
            else:
                lines.append(f"{pad}{field}: {_yaml_scalar(v)}")
    for field in ("owner", "reason", "expires"):
        if entry.get(field) is not None:
            lines.append(f"{pad}{field}: {_yaml_scalar(entry[field])}")
    return "\n".join(lines)


def upsert_entry(path: Path, entry: dict) -> str:
    """Write *entry* into the file, replacing any entry for the same peer.

    Returns "added" or "replaced". The file must already exist — creating it
    would govern a scope, which is a decision, not a side effect.
    """
    path = Path(path)
    text = path.read_text()
    lines = text.split("\n")
    for peer, start, end in _blocks(lines):
        if peer == entry["peer"]:
            previous = yaml.safe_load("\n".join(lines[start:end]))[0]
            if "peer_id" in previous:
                if entry.get("peer_id", previous["peer_id"]) != previous["peer_id"]:
                    raise SourceError(f"{path}: {peer} is already bound to a different peer ID")
                entry = {**entry, "peer_id": previous["peer_id"]}
            lines[start:end] = render_entry(entry).split("\n")
            path.write_text("\n".join(lines))
            return "replaced"
    rendered = render_entry(entry).split("\n")
    for i, line in enumerate(lines):
        if RE_ENTRIES_EMPTY.match(line):
            lines[i:i + 1] = ["entries:"] + rendered
            break
        if RE_ENTRIES.match(line):
            blocks = _blocks(lines)
            at = blocks[-1][2] if blocks else i + 1
            lines[at:at] = rendered
            break
    else:
        raise SourceError(f"{path}: no `entries:` key to add to")
    path.write_text("\n".join(lines))
    return "added"


def remove_entries(path: Path, peers: set[str]) -> list[str]:
    """Delete the entries for *peers*; returns the peers actually removed."""
    path = Path(path)
    lines = path.read_text().split("\n")
    drop: set[int] = set()
    removed = []
    for peer, start, end in _blocks(lines):
        if peer in peers:
            drop.update(range(start, end))
            removed.append(peer)
    if not removed:
        return []
    remaining = [line for i, line in enumerate(lines) if i not in drop]
    # A list emptied by removal is written back as `entries: []`, so the file
    # still parses to a list rather than to null.
    if not _blocks(remaining):
        for i, line in enumerate(remaining):
            if RE_ENTRIES.match(line):
                remaining[i] = "entries: []"
                break
    path.write_text("\n".join(remaining))
    return removed


def new_fragment_text(name: str) -> str:
    purpose = FRAGMENT_PURPOSE.get(Path(name).name, "entries")
    return (f"# {purpose}.\n"
            "# Schema and rules: docs/design/mesh-change-control.md §4; the scope's switch\n"
            "# is _scope.yaml beside this file.\n"
            "version: 1\nentries: []\n")


def upsert_scope_entry(sdir: Path, entry: dict, fragment: str) -> tuple[Path, str]:
    """Write *entry* into the scope: in place in whichever fragment already
    holds the peer, else into *fragment* (created if absent — a fragment is
    not the switch; _scope.yaml is, and this never creates it).

    Returns (the file written, "added"|"replaced").
    """
    sdir = Path(sdir)
    if not (sdir / SCOPE_FILE).is_file():
        raise SourceError(f"{sdir}: no {SCOPE_FILE} — creating a scope governs it, which is a decision, "
                          "not a side effect")
    target = find_entry(sdir, entry["peer"]) or (sdir / fragment)
    if not target.is_file():
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(new_fragment_text(fragment))
    return target, upsert_entry(target, entry)


def remove_scope_entries(sdir: Path, peers: set[str]) -> list[tuple[Path, str]]:
    """Delete *peers*' entries from whichever fragments hold them."""
    out: list[tuple[Path, str]] = []
    for f in scope_files(Path(sdir))[2]:
        for peer in remove_entries(f, set(peers)):
            out.append((f, peer))
    return out


# ---- CLI ---------------------------------------------------------------------


def _rel(p: Path) -> str:
    p = Path(p).resolve()
    return str(p.relative_to(REPO_ROOT)) if p.is_relative_to(REPO_ROOT) else str(p)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    mode = ap.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--list-expired", action="store_true")
    mode.add_argument("--generate-baseline", action="store_true")
    ap.add_argument("--scope")
    ap.add_argument("--dir", type=Path, default=MEMBERSHIP_DIR)
    ap.add_argument("--network-dir", type=Path, default=NETWORK_DIR)
    ap.add_argument("--taxonomy", type=Path, default=TAXONOMY)
    ap.add_argument("--operators-dir", type=Path, default=OPERATORS_DIR,
                    help="where the (gitignored) operator manifests are: ops/operators of the "
                         "operator's own clone; a worktree has none")
    ap.add_argument("--today", default=None)
    args = ap.parse_args(argv)

    today = dt.date.fromisoformat(args.today) if args.today else dt.date.today()
    try:
        scopes = taxonomy_scopes(args.taxonomy)
    except SourceError as exc:
        print(f"warrant membership: {exc}", file=sys.stderr)
        return 2

    dirs = scope_dirs(args.dir, args.scope)
    if args.scope and not dirs[0].is_dir():
        print(f"warrant membership: {args.scope} is not governed — no {dirs[0]}/", file=sys.stderr)
        return 2

    if args.generate_baseline:
        try:
            manifests = load_manifests(args.operators_dir)
        except SourceError as exc:
            print(f"warrant membership: {exc}", file=sys.stderr)
            return 2
        if not manifests:
            print(f"warrant membership: no operator manifests under {args.operators_dir} — refusing to "
                  "write an empty baseline that would read as 'nobody'. They are gitignored; pass "
                  "--operators-dir <your clone>/ops/operators.", file=sys.stderr)
            return 2
        for d in dirs:
            try:
                text = render_baseline(generate_baseline(d.name, scopes=scopes, manifests=manifests))
            except SourceError as exc:
                print(f"warrant membership: {exc}", file=sys.stderr)
                return 2
            bf = d / BASELINE_FILE
            old = bf.read_text() if bf.is_file() else None
            if old != text:
                bf.write_text(text)
            n = text.count("\n    device:")
            print(f"{'wrote' if old != text else 'same '} {_rel(bf)}  ({n} {'person' if n == 1 else 'people'})")
        return 0

    if args.list_expired:
        due = 0
        for d in dirs:
            try:
                view = load_scope(d)
            except SourceError as exc:
                print(f"warrant membership: {exc}", file=sys.stderr)
                return 2
            for e, when in expired_entries(view, today):
                due += 1
                print(f"  - {d.name}: {e.get('peer')}  expired {when.isoformat()}  owner: {e.get('owner') or '<none>'}")
        if not due:
            print(f"warrant membership: nothing past due as of {today.isoformat()}")
        return 0

    bad = 0
    if not args.scope:
        for f in legacy_files(args.dir):
            bad += 1
            print(f"FAIL {_rel(f)}")
            print(f"  - the single-file layout is refused: a scope is the directory "
                  f"{f.stem}/ (_scope.yaml + fragments + {BASELINE_FILE})")
    try:
        manifests = load_manifests(args.operators_dir)
    except SourceError as exc:
        print(f"warrant membership: {exc}", file=sys.stderr)
        return 2
    notes: list[str] = []
    for d in dirs:
        try:
            errs = check_scope(d, today=today, scopes=scopes)
            if not errs:
                state, msg = baseline_status(d, scopes=scopes, manifests=manifests,
                                             operators_dir=args.operators_dir)
                if state == "stale":
                    errs.append(msg)
                elif state == "unverified":
                    notes.append(f"{d.name}: {msg}")
        except SourceError as exc:
            print(f"warrant membership: {exc}", file=sys.stderr)
            return 2
        if errs:
            bad += len(errs)
            print(f"FAIL {_rel(d)}/")
            for e in errs:
                print(f"  - {e}")
        else:
            view = load_scope(d)
            n = len(view["entries"])
            nf = len(scope_files(d)[2])
            np_ = len((view["baseline"] or {}).get("people") or {})
            print(f"ok   {_rel(d)}/  ({n} entr{'y' if n == 1 else 'ies'} in {nf} fragment"
                  f"{'' if nf == 1 else 's'}; baseline {np_} {'person' if np_ == 1 else 'people'}; "
                  f"converge {view['converge']})")

    net_files = [args.network_dir / f"{args.scope}.yaml"] if args.scope else sorted(args.network_dir.glob("*.yaml"))
    for f in net_files:
        if not f.is_file():
            continue
        try:
            errs = validate_network(load(f), stem=f.stem, scopes=scopes)
        except SourceError as exc:
            print(f"warrant membership: {exc}", file=sys.stderr)
            return 2
        if errs:
            bad += len(errs)
            print(f"FAIL {_rel(f)}")
            for e in errs:
                print(f"  - {e}")
        else:
            print(f"ok   {_rel(f)}")

    for n in notes:
        print(f"note {n}")
    if bad:
        print(f"\nwarrant membership: {bad} violation(s).")
        return 1
    print("\nwarrant membership: every declaration is valid.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
