#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""The departure record: who left, why, who confirmed it, and what was swept.

One YAML file per departure in the deployment's checkout, ops/departures/
(ADR 1272, docs/design/offboarding-one-driver.md). The offboarding driver
writes it before the first removal and adds one run to it at the finish. It
names the person by handle only and holds no login email and no credential.

  offboard_departure.py list                      every record and its state
  offboard_departure.py show  <handle>            one person's latest record
  offboard_departure.py close <handle> <row> --by <handle> [--note TEXT]
                                                  mark a by-hand row done
"""

from __future__ import annotations

import argparse
import os
import re
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

import yaml

SCHEMA = "goat-departure/v1"
DIRECTORY = "ops/departures"
REASONS = ("new-position", "left-service", "other")
HANDLE = re.compile(r"[a-z0-9](?:[a-z0-9_-]{0,62}[a-z0-9])?")
DAY = re.compile(r"\d{4}-\d{2}-\d{2}")

# What a reason class changes. Every class removes all access; the class
# decides how the person may come back and which follow-on work the record
# keeps open until someone closes it by name.
RETURN_RULE = {
    "new-position": ("a new sponsorship through onboarding, at the tier the new "
                     "position warrants; the old login's groups are never restored"),
    "left-service": "none; a later request is a new onboarding from the start",
    "other": "a new sponsorship through onboarding",
}
FOLLOW_ON = {
    "new-position": [(
        "handover",
        "hand over what the person held for others: people they sponsored, an "
        "organisation's sharing authority, memberships that move with the "
        "position (goat admin members move)")],
    "left-service": [(
        "purge-decision",
        "decide whether the retained, inactive login is purged (goat admin "
        "onboard --revoke --purge); it stays reversible until then")],
    "other": [],
}


class Refusal(ValueError):
    """The record cannot be written or read safely."""


def utcnow() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def today() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%d")


def check_reason(reason: str, text: str) -> None:
    if reason not in REASONS:
        raise Refusal("a departure needs --reason: " + " | ".join(REASONS))
    if reason == "other" and not text.strip():
        raise Refusal("--reason other needs --reason-text saying why the person left")


def check_confirmer(confirmer: str, declared: str) -> None:
    """The person who confirmed the departure, by handle. Where the checkout
    declares goat_offboard_confirmers, the handle must be one of them."""
    if not HANDLE.fullmatch(confirmer or ""):
        raise Refusal("a departure needs --confirmed-by <handle>: who confirmed "
                      "that the person left")
    allowed = [item.strip() for item in declared.split(",") if item.strip()]
    if allowed and confirmer not in allowed:
        raise Refusal(f"--confirmed-by {confirmer} is not one of the declared "
                      "goat_offboard_confirmers")


def check_effective(day: str) -> str:
    if not DAY.fullmatch(day or ""):
        raise Refusal("--effective is a date, YYYY-MM-DD")
    try:
        datetime.strptime(day, "%Y-%m-%d")
    except ValueError as exc:
        raise Refusal("--effective is a date, YYYY-MM-DD") from exc
    return day


def read(path: Path) -> dict:
    try:
        value = yaml.safe_load(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, yaml.YAMLError) as exc:
        raise Refusal(f"departure record {path.name} is unreadable") from exc
    if not isinstance(value, dict) or value.get("schema") != SCHEMA:
        raise Refusal(f"departure record {path.name} is not a {SCHEMA} document")
    return value


def write(path: Path, value: dict) -> None:
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        fd, name = tempfile.mkstemp(prefix=path.name + ".", dir=path.parent)
    except OSError as exc:
        raise Refusal(f"departure record {path} cannot be written: {exc}") from exc
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            yaml.safe_dump(value, stream, sort_keys=False, allow_unicode=True, width=100)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(name, path)
    except OSError as exc:
        raise Refusal(f"departure record {path} cannot be written: {exc}") from exc
    finally:
        try:
            os.unlink(name)
        except FileNotFoundError:
            # A successful os.replace moved the temporary file into place.
            pass


def records(directory: Path, handle: str | None = None) -> list[tuple[Path, dict]]:
    """Every record, oldest effective date first; one person's when named."""
    if not directory.is_dir():
        return []
    found = []
    for path in sorted(directory.glob("*.yml")):
        value = read(path)
        if handle is None or value.get("handle") == handle:
            found.append((path, value))
    found.sort(key=lambda item: (str(item[1].get("effective", "")), item[0].name))
    return found


def state_of(value: dict) -> str:
    """complete only when the latest run verified every row and no by-hand
    row is still open."""
    runs = value.get("runs") or []
    swept = bool(runs) and runs[-1].get("outcome") == "success"
    by_hand = all(row.get("state") == "done" for row in value.get("by_hand") or [])
    return "complete" if swept and by_hand else "open"


def begin(directory: Path, handle: str, reason: str, text: str, effective: str | None,
          confirmer: str, operator: str, goatnets: list[str],
          by_hand: list[tuple[str, str]] = (),
          accounts: list[str] = (), also: str = "") -> tuple[Path, dict]:
    """Open this person's record before anything is removed, or continue the
    one already open. A different reason or date for the same open record is
    refused: a record states one departure."""
    existing = records(directory, handle)
    latest = existing[-1] if existing else None
    if latest and (effective is None or latest[1].get("effective") == effective):
        path, value = latest
        held = (value.get("reason") or {}).get("class")
        if held != reason:
            raise Refusal(f"departure record {path.name} gives the reason {held}; this run "
                          f"says {reason}. Pass the recorded reason, or --effective with "
                          "the date of a new departure")
    else:
        effective = effective or today()
        path = directory / f"{handle}-{effective}.yml"
        value = {
            "schema": SCHEMA, "handle": handle,
            "reason": {"class": reason, "text": text.strip()},
            "effective": effective, "confirmed_by": confirmer,
            "recorded_by": operator, "recorded_at": utcnow(),
            "return": RETURN_RULE[reason], "state": "open",
            "goatnets": [], "accounts": [], "runs": [], "by_hand": [],
        }
    if also and also != handle:
        value["other_handles"] = sorted(set(value.get("other_handles") or []) | {also})
    value["goatnets"] = sorted(set(value.get("goatnets") or []) | set(goatnets))
    value["accounts"] = sorted(set(value.get("accounts") or []) | set(accounts))
    known = {row.get("id") for row in value.get("by_hand") or []}
    for identifier, what in [*FOLLOW_ON[reason], *by_hand]:
        if identifier not in known:
            value.setdefault("by_hand", []).append(
                {"id": identifier, "what": what, "state": "open"})
    value["state"] = state_of(value)
    write(path, value)
    return path, value


def check_record_as(directory: Path, handle: str) -> None:
    """--record-as joins a run to a departure already on record; it never opens one."""
    if not HANDLE.fullmatch(handle or ""):
        raise Refusal(f"--record-as is a handle, not {handle!r}")
    if not records(directory, handle):
        raise Refusal(f"--record-as {handle}: no departure record for {handle} in {directory}; "
                      "offboard that handle first, then name it here")


def finish(path: Path, value: dict, operator: str, outcome: str, rows: list[dict]) -> dict:
    value.setdefault("runs", []).append(
        {"at": utcnow(), "operator": operator, "outcome": outcome, "rows": rows})
    value["state"] = state_of(value)
    write(path, value)
    return value


def close(directory: Path, handle: str, row: str, by: str, note: str = "") -> tuple[Path, dict]:
    if not HANDLE.fullmatch(by or ""):
        raise Refusal("close needs --by <handle>: who did the work")
    existing = records(directory, handle)
    if not existing:
        raise Refusal(f"no departure record for {handle} in {directory}")
    path, value = existing[-1]
    matches = [item for item in value.get("by_hand") or [] if item.get("id") == row]
    if not matches:
        raise Refusal(f"departure record {path.name} has no by-hand row {row}")
    matches[0].update({"state": "done", "closed_by": by, "closed_at": utcnow()})
    if note.strip():
        matches[0]["note"] = note.strip()
    value["state"] = state_of(value)
    write(path, value)
    return path, value


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dir", type=Path, default=Path(DIRECTORY),
                        help=f"the records directory (default: {DIRECTORY})")
    sub = parser.add_subparsers(dest="verb", required=True)
    sub.add_parser("list")
    show = sub.add_parser("show")
    show.add_argument("handle")
    closer = sub.add_parser("close")
    closer.add_argument("handle")
    closer.add_argument("row")
    closer.add_argument("--by", required=True)
    closer.add_argument("--note", default="")
    args = parser.parse_args()
    try:
        if args.verb == "list":
            print("handle\teffective\treason\tstate\topen by hand")
            for _, value in records(args.dir):
                waiting = [row["id"] for row in value.get("by_hand") or []
                           if row.get("state") != "done"]
                print("\t".join([str(value.get("handle")), str(value.get("effective")),
                                 str((value.get("reason") or {}).get("class")),
                                 state_of(value), ",".join(waiting) or "-"]))
        elif args.verb == "show":
            existing = records(args.dir, args.handle)
            if not existing:
                raise Refusal(f"no departure record for {args.handle} in {args.dir}")
            yaml.safe_dump(existing[-1][1], sys.stdout, sort_keys=False, width=100)
        else:
            path, value = close(args.dir, args.handle, args.row, args.by, args.note)
            print(f"{path}: {args.row} done; record is {value['state']}")
    except Refusal as exc:
        print(f"departure: REFUSE — {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
