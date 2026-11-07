#!/usr/bin/env python3
"""cf-acme-challenge-clean — delete leftover `_acme-challenge.<name>` TXT records
from Cloudflare so a Let's Encrypt DNS-01 order for <name> can be retried.

Traefik's DNS-01 client creates `_acme-challenge.<name>` TXT, waits, and deletes
it. An order that fails part-way can leave the record behind; the next attempt
is then refused by Cloudflare ("An identical record already exists") and
Traefik keeps serving its default certificate for <name>. traefik-acme.yml runs
this for each declared name still on the default certificate, restarts Traefik
once, and re-checks.

    CF_DNS_API_TOKEN=... ops/scripts/cf-acme-challenge-clean.py <name> [<name> ...]
        [--token-env VAR] [--api URL]

The token needs Zone:Read and DNS:Edit on the name's zone (the same scopes the
DNS-01 client uses). The zone is the longest suffix of <name> that the token
can see. Only TXT records named exactly `_acme-challenge.<name>` are deleted.

Prints one line per name. Exit 0 when every name's stale records are gone (or
there were none), 1 when a zone cannot be found or the API refuses, 2 on usage.
The token and record contents are never printed.
"""
import argparse
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request

API = "https://api.cloudflare.com/client/v4"


class CfError(Exception):
    pass


def call(api, token, method, path, query=None):
    url = api.rstrip("/") + path
    if query:
        url += "?" + urllib.parse.urlencode(query)
    req = urllib.request.Request(url, method=method, headers={
        "Authorization": f"Bearer {token}",
        "Content-Type": "application/json",
    })
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            body = json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        try:
            body = json.loads(e.read() or b"{}")
        except ValueError:
            body = {}
        errs = "; ".join(f"{x.get('code')}: {x.get('message')}" for x in body.get("errors") or [])
        raise CfError(f"{method} {path} -> HTTP {e.code}{' (' + errs + ')' if errs else ''}") from None
    except (urllib.error.URLError, TimeoutError) as e:
        raise CfError(f"{method} {path} -> {getattr(e, 'reason', e)}") from None
    if not body.get("success", False):
        errs = "; ".join(f"{x.get('code')}: {x.get('message')}" for x in body.get("errors") or [])
        raise CfError(f"{method} {path} -> refused ({errs or 'no error detail'})")
    return body


def zone_id(api, token, name):
    labels = name.rstrip(".").split(".")
    for i in range(len(labels) - 1):
        suffix = ".".join(labels[i:])
        res = call(api, token, "GET", "/zones", {"name": suffix}).get("result") or []
        if res:
            return res[0]["id"], suffix
    raise CfError(f"no Cloudflare zone this token can read holds {name}")


def clean(api, token, name):
    zid, zone = zone_id(api, token, name)
    record = f"_acme-challenge.{name.rstrip('.')}"
    recs = call(api, token, "GET", f"/zones/{zid}/dns_records",
                {"type": "TXT", "name": record, "per_page": 100}).get("result") or []
    recs = [r for r in recs if r.get("type") == "TXT" and r.get("name") == record]
    for r in recs:
        call(api, token, "DELETE", f"/zones/{zid}/dns_records/{r['id']}")
    return zone, record, len(recs)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("names", nargs="+")
    ap.add_argument("--token-env", default="CF_DNS_API_TOKEN")
    ap.add_argument("--api", default=os.environ.get("CF_API_BASE", API))
    a = ap.parse_args(argv)
    token = os.environ.get(a.token_env, "")
    if not token:
        print(f"cf-acme-challenge-clean: {a.token_env} is empty; it must hold the goatnet's Cloudflare DNS-01 token", file=sys.stderr)
        return 2
    rc = 0
    for n in a.names:
        try:
            zone, record, k = clean(a.api, token, n)
            print(f"{n}: deleted {k} stale {record} TXT record(s) in zone {zone}")
        except CfError as e:
            print(f"{n}: FAILED: {e}")
            rc = 1
    return rc


if __name__ == "__main__":
    sys.exit(main())
