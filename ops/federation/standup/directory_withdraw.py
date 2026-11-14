#!/usr/bin/env python3
"""directory_withdraw — withdraw the package we host for ONE counterparty from the
peering directory, when a peering is backed out.

A directory that hosts one package per counterparty (saltlick
`/v1/parties/<us>/packages/<them>`, docs/design/peering-directory-service.md) keeps the
package we signed for `them` in its own slot. Backing the peering out withdraws that
slot with the signed DELETE saltlick defines for it: the statement is
`DELETE`, `<us>/packages/<them>`, the empty body's hash and `-`, signed by our bound key.

What it never does:
  * touch another party's entry: every request names our own sovereign id first;
  * delete our single entry `/v1/parties/<us>` or its pointer: that one slot serves
    every counterparty on an older release, so withdrawing it is a decision about all
    of them, not about this peering. It is reported, never changed;
  * fail the back-out because the directory is unreachable: the directory is
    advisory (being listed grants nothing), so the outcome is reported and the exit
    status says whether the slot is known to be gone.

  directory_withdraw.py --url URL --us US --them THEM --key FILE [--dry-run]

Prints one JSON line: {"outcome": "withdrawn"|"absent"|"not-applicable"|"would-withdraw"|
"failed", "detail": ..., "resource": ...}. Exit 0 for withdrawn, absent, not-applicable
and would-withdraw; 1 for failed; 2 for usage.
"""
from __future__ import annotations

import argparse
import http.client
import importlib.util
import json
import re
import sys
import urllib.error
import urllib.parse
from pathlib import Path

ID_RE = re.compile(r"^[a-z0-9][a-z0-9-]{1,15}$")


def _directory():
    spec = importlib.util.spec_from_file_location("directory", Path(__file__).with_name("directory.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def slot_url(base: str, us: str, them: str) -> str:
    return (base.rstrip("/") + "/v1/parties/" + urllib.parse.quote(us, safe="")
            + "/packages/" + urllib.parse.quote(them, safe=""))


def withdraw(url: str, us: str, them: str, key_path: str | None, dry_run: bool, timeout: float = 10.0,
             d=None) -> dict:
    resource = f"{us}/packages/{them}"
    target = slot_url(url, us, them)
    out = {"resource": resource, "url": target}
    if not ID_RE.match(us) or not ID_RE.match(them) or us == them:
        return {**out, "outcome": "failed", "detail": f"'{us}' and '{them}' must be two different sovereign ids"}
    if dry_run:
        return {**out, "outcome": "would-withdraw",
                "detail": f"DELETE {target}, signed by {us}'s bound key; our single entry is left as it is"}
    if not key_path:
        return {**out, "outcome": "failed",
                "detail": "no signing key: set FED_SIGNING_KEY (the key the directory binds to us)"}
    d = d or _directory()
    try:
        seed = d.load_seed(key_path)
        statement = d.request_statement("DELETE", resource, b"", None)
        headers = {"X-Saltlick-Signature": d.b64(d.sign(seed, statement))}
        status, _, body = d._http("DELETE", target, None, headers, timeout)
    except (OSError, ValueError, RuntimeError, urllib.error.URLError, http.client.HTTPException) as e:
        return {**out, "outcome": "failed", "detail": f"the directory could not be asked: {e}"}
    said = body.decode(errors="replace").strip()[:300]
    if status in (200, 204):
        return {**out, "outcome": "withdrawn", "detail": f"HTTP {status}: the slot for {them} is gone"}
    if status == 404:
        return {**out, "outcome": "absent", "detail": "HTTP 404: no package for this counterparty is hosted"}
    if status == 405:
        return {**out, "outcome": "not-applicable",
                "detail": "HTTP 405: this directory hosts one package per party, not one per counterparty"}
    return {**out, "outcome": "failed", "detail": f"HTTP {status}: {said}"}


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--url", required=True)
    ap.add_argument("--us", required=True)
    ap.add_argument("--them", required=True)
    ap.add_argument("--key")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args(argv)
    r = withdraw(a.url, a.us, a.them, a.key, a.dry_run)
    print(json.dumps(r, sort_keys=True))
    return 1 if r["outcome"] == "failed" else 0


if __name__ == "__main__":
    sys.exit(main())
