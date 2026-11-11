#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Measure concurrent Kratos creates without conflating conflicts and outages.

Only use disposable test identities. This probe retains accepted identities so
the caller can inspect replication and credentials; it never deletes a loser.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import json
from threading import Barrier
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen
from uuid import UUID


def create_identity(url, body):
    request = Request(
        url.rstrip("/") + "/admin/identities",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urlopen(request, timeout=5) as response:
            status, raw = response.status, response.read()
    except HTTPError as exc:
        status, raw = exc.code, exc.read()
    except (URLError, TimeoutError, OSError):
        return {"http": 0, "identity_id": None, "error": "transport"}

    # Never export credentials, traits, error messages or response bodies.
    result = {"http": status, "identity_id": None, "error": None}
    if status == 201:
        try:
            result["identity_id"] = str(UUID(json.loads(raw)["id"]))
        except (ValueError, TypeError, KeyError, AttributeError):
            result["error"] = "invalid_identity_response"
    return result


def classify(a, b):
    accepted = sum(r["http"] == 201 and r["identity_id"] is not None
                   and r["error"] is None for r in (a, b))
    rejected = sum(r["http"] == 409 and r["error"] is None for r in (a, b))
    if accepted == 1 and rejected == 1:
        outcome = "unique_conflict"
    elif accepted == 2:
        outcome = "both_accepted"
    else:
        outcome = "inconclusive"
    return {
        "f3_http_a": a["http"],
        "f3_http_b": b["http"],
        "f3_identity_id_a": a["identity_id"],
        "f3_identity_id_b": b["identity_id"],
        "f3_error_a": a["error"],
        "f3_error_b": b["error"],
        "f3_ok_count": accepted,
        "f3_outcome": outcome,
        "f3_app_layer_caught": int(outcome == "unique_conflict"),
    }


def probe(url_a, url_b, email, create=create_identity):
    barrier = Barrier(2)

    def submit(url, side):
        body = {
            "schema_id": "default",
            "traits": {"email": email, "name": "Concurrent create fixture"},
            "credentials": {"password": {"config": {
                # Separate fixture passwords let a follow-up login probe tell
                # which accepted identity's credentials remain usable.
                "password": "ConcurrentFixture!" + side + "37",
            }}},
        }
        barrier.wait(timeout=10)
        return create(url, body)

    with ThreadPoolExecutor(max_workers=2) as pool:
        a = pool.submit(submit, url_a, "A")
        b = pool.submit(submit, url_b, "B")
        return classify(a.result(), b.result())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--admin-a", required=True)
    parser.add_argument("--admin-b", required=True)
    parser.add_argument("--email", required=True)
    parser.add_argument("--require-unique", action="store_true",
                        help="exit nonzero unless exactly one 201 and one 409")
    args = parser.parse_args()
    result = probe(args.admin_a, args.admin_b, args.email)
    print(json.dumps(result, sort_keys=True))
    return int(args.require_unique and not result["f3_app_layer_caught"])


if __name__ == "__main__":
    raise SystemExit(main())
