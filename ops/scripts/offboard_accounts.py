#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""External accounts: the places outside the goatnets where a person holds
access, declared per deployment and handled by plug-ins (ADR 1272 section 3).

A deployment declares its accounts in ops/offboard/accounts.yaml:

    schema: goat-offboard-accounts/v1
    accounts:
      - id: paging                 # the row's name in the table and the record
        type: pagerduty            # the plug-in: ops/offboard/plugins/<type>
        match: email               # email | github | declared
        credential_env: [PAGING_API_TOKEN]   # NAMES of variables, never values
        options: {}                # passed to the plug-in unchanged
      - id: site-router-logins
        type: manual               # no plug-in: listed as work to do by hand
        what: remove the person's login from each site router

A plug-in is an executable taking one verb, `check` or `remove`, and one JSON
object on standard input:

    {"account": <the declaration>, "identities": ["<login or handle>", ...]}

and answering with one JSON object on standard output:

    {"held": true|false, "items": ["<what is held>", ...], "detail": "<one line>"}

`check` reads only. `remove` removes and then reports what is still held. A
plug-in that exits non-zero, or answers anything else, could not run: the row
is NOT RUN or FAILED and the run fails. Its credentials arrive in the
environment under the names the declaration lists.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
from pathlib import Path

import yaml

SCHEMA = "goat-offboard-accounts/v1"
DECLARATION = "ops/offboard/accounts.yaml"
PLUGINS = "ops/offboard/plugins"
IDENT = re.compile(r"[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?")
ENV_NAME = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
MATCHES = ("email", "github", "declared")
NONE_HELD = "-"
TIMEOUT = 120


class Refusal(ValueError):
    """The declaration cannot be used, or a person cannot be matched safely."""


def load(repo: Path) -> list[dict]:
    """The declared accounts, in order. No file means the deployment declares
    none; a file that cannot be read exactly is refused, never read in part."""
    path = repo / DECLARATION
    if not path.exists():
        return []
    try:
        doc = yaml.safe_load(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, yaml.YAMLError) as exc:
        raise Refusal(f"{DECLARATION} is unreadable") from exc
    if not isinstance(doc, dict) or doc.get("schema") != SCHEMA:
        raise Refusal(f"{DECLARATION} is not a {SCHEMA} document")
    rows = doc.get("accounts")
    if not isinstance(rows, list):
        raise Refusal(f"{DECLARATION}: accounts must be a list")
    seen, accounts = set(), []
    for row in rows:
        if not isinstance(row, dict):
            raise Refusal(f"{DECLARATION}: every account is a mapping")
        identifier, kind = row.get("id"), row.get("type")
        if not isinstance(identifier, str) or not IDENT.fullmatch(identifier):
            raise Refusal(f"{DECLARATION}: an account id must be a lowercase slug")
        if identifier in seen:
            raise Refusal(f"{DECLARATION}: account id {identifier} is declared twice")
        seen.add(identifier)
        if not isinstance(kind, str) or not IDENT.fullmatch(kind):
            raise Refusal(f"{DECLARATION}: account {identifier} needs a type")
        if kind == "manual":
            what = row.get("what")
            if not isinstance(what, str) or not what.strip():
                raise Refusal(f"{DECLARATION}: manual account {identifier} needs `what`: "
                              "the work to do by hand")
            accounts.append({"id": identifier, "type": "manual", "what": what.strip()})
            continue
        match = row.get("match", "email")
        if match not in MATCHES:
            raise Refusal(f"{DECLARATION}: account {identifier} match must be one of "
                          + ", ".join(MATCHES))
        names = row.get("credential_env", [])
        if (not isinstance(names, list)
                or any(not isinstance(n, str) or not ENV_NAME.fullmatch(n) for n in names)):
            raise Refusal(f"{DECLARATION}: account {identifier} credential_env lists "
                          "variable names only")
        options = row.get("options", {})
        if not isinstance(options, dict):
            raise Refusal(f"{DECLARATION}: account {identifier} options must be a mapping")
        accounts.append({"id": identifier, "type": kind, "match": match,
                         "credential_env": names, "options": options})
    return accounts


def manual(accounts: list[dict]) -> list[dict]:
    return [account for account in accounts if account["type"] == "manual"]


def automated(accounts: list[dict]) -> list[dict]:
    return [account for account in accounts if account["type"] != "manual"]


def declared_handles(repo: Path, subjects, account_id: str) -> list[str] | None:
    """What the person's manifests declare as their identity in this account
    (identity.accounts.<id>: one handle or a list). None when no manifest
    says; [] when a manifest says the person holds none ("-")."""
    found, said = [], False
    for goatnet, handle, _, _ in subjects:
        path = repo / "ops/operators" / goatnet / f"{handle}.yml"
        try:
            data = yaml.safe_load(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, yaml.YAMLError) as exc:
            raise Refusal(f"{goatnet}/{handle}.yml: unreadable manifest") from exc
        identity = data.get("identity") if isinstance(data, dict) else None
        table = identity.get("accounts") if isinstance(identity, dict) else None
        if not isinstance(table, dict) or account_id not in table:
            continue
        said = True
        value = table[account_id]
        values = value if isinstance(value, list) else [value]
        if any(not isinstance(v, str) or not v.strip() for v in values):
            raise Refusal(f"{goatnet}/{handle}.yml: identity.accounts.{account_id} "
                          "must be a handle or a list of handles")
        found.extend(v.strip() for v in values if v.strip() != NONE_HELD)
    return list(dict.fromkeys(found)) if said else None


def identities(account: dict, repo: Path, subjects, email: str, github_login: str,
               supplied: dict[str, list[str]]) -> list[str]:
    """Who the person is in this account. Never a name pattern: the login
    email, the manifest-bound GitHub login, or handles declared for this
    account in the manifest or with --account-handle."""
    identifier = account["id"]
    declared = declared_handles(repo, subjects, identifier)
    extra = supplied.get(identifier)
    if account["match"] == "email":
        return list(dict.fromkeys([email, *(declared or []), *(extra or [])]))
    if account["match"] == "github":
        return list(dict.fromkeys([github_login, *(declared or []), *(extra or [])]))
    if declared is None and extra is None:
        raise Refusal(
            f"account {identifier} matches by declared handle and none is declared: add "
            f"identity.accounts.{identifier} to the person's manifest, or pass "
            f"--account-handle {identifier}=<handle> ({identifier}={NONE_HELD} states "
            "that the person holds no account there)")
    return list(dict.fromkeys([*(declared or []), *(extra or [])]))


def parse_supplied(values: list[str]) -> dict[str, list[str]]:
    """--account-handle <id>=<handle>, repeatable; <id>=- means none held."""
    supplied: dict[str, list[str]] = {}
    for value in values:
        identifier, sep, handle = value.partition("=")
        if not sep or not IDENT.fullmatch(identifier) or not handle.strip():
            raise Refusal("--account-handle takes <account id>=<handle>")
        bucket = supplied.setdefault(identifier, [])
        if handle.strip() != NONE_HELD:
            bucket.append(handle.strip())
    return supplied


def plugin_path(repo: Path, kind: str) -> Path:
    return repo / PLUGINS / kind


def cannot_run(account: dict, repo: Path) -> str:
    """Why this account's plug-in cannot run on this host, or ''."""
    path = plugin_path(repo, account["type"])
    if not path.is_file() or not os.access(path, os.X_OK):
        return (f"no executable plug-in {PLUGINS}/{account['type']}; declare the account "
                "with type manual to do it by hand")
    missing = [name for name in account["credential_env"] if not os.getenv(name)]
    if missing:
        return "missing credential variable(s): " + ", ".join(missing)
    return ""


def call(repo: Path, account: dict, verb: str, who: list[str]) -> dict:
    """Run one verb of one plug-in and hold its answer to the contract."""
    payload = json.dumps({"account": account, "identities": who})
    try:
        proc = subprocess.run([str(plugin_path(repo, account["type"])), verb],
                              input=payload, text=True, capture_output=True,
                              timeout=TIMEOUT, check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise RuntimeError(f"plug-in {account['type']} {verb} did not run: {exc}") from exc
    if proc.returncode:
        reason = (proc.stderr.strip() or proc.stdout.strip() or "no message").splitlines()[-1]
        raise RuntimeError(f"plug-in {account['type']} {verb} exited {proc.returncode}: {reason}")
    try:
        answer = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"plug-in {account['type']} {verb} returned unreadable JSON") from exc
    if (not isinstance(answer, dict) or type(answer.get("held")) is not bool
            or not isinstance(answer.get("items", []), list)
            or any(not isinstance(item, str) for item in answer.get("items", []))
            or (answer["held"] is False and answer.get("items"))):
        raise RuntimeError(f"plug-in {account['type']} {verb} broke the answer contract")
    return {"held": answer["held"], "items": answer.get("items", []),
            "detail": str(answer.get("detail", ""))}
