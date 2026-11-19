#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""door-admit: the admission check behind an external service's sign-in door (ADR 1254).

It sits between the service's oauth2-proxy door and the service:

    browser -> Traefik (mesh-only) -> oauth2-proxy (sign-in) -> door-admit (who may enter) -> the service

oauth2-proxy has already signed the person in and passes their ID token as
`Authorization: Bearer <id token>`. door-admit is reachable only from that door (it
listens on the door's private compose network and publishes no port), so it reads the
token's claims without verifying the signature again. It admits the person when any
one of these holds:

  - the token's `aff.id` is in the rule's admitted organization ids
    (the rule is expanded at deploy time from the org registry: named organizations,
    their descendants, and every organization of a named nation);
  - one of the token's `groups` is in the rule's admitted groups;
  - the token's `email` is on the rule's e-mail list.

Admitted requests are forwarded to the service without the Authorization header.
Refused requests get a 403 page that says which rule applies and who to ask.
Every decision is logged as one JSON line (who, which rule matched, path); tokens never are.

Python standard library only.
  door_admit.py --listen 0.0.0.0:8080 --upstream http://<mesh ip>:<port> --rules /etc/door-admit/rules.json
"""
import argparse
import base64
import http.client
import json
import sys
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HOP = {"connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "te", "trailers",
       "transfer-encoding", "upgrade", "host", "authorization", "content-length"}


def claims_from(auth_header):
    """The payload of a Bearer JWT, or {} when there is none or it does not parse."""
    if not auth_header or not auth_header.lower().startswith("bearer "):
        return {}
    parts = auth_header.split(None, 1)[1].strip().split(".")
    if len(parts) != 3:
        return {}
    pad = "=" * (-len(parts[1]) % 4)
    try:
        return json.loads(base64.urlsafe_b64decode(parts[1] + pad))
    except (ValueError, json.JSONDecodeError):
        return {}


def decide(claims, admit):
    """(admitted, reason). admit: {"aff_ids": [...], "groups": [...], "emails": [...]}."""
    aff = claims.get("aff") or {}
    aff_id = aff.get("id") if isinstance(aff, dict) else None
    if aff_id and aff_id in set(admit.get("aff_ids", [])):
        return True, f"organization {aff_id}"
    groups = claims.get("groups") or []
    if isinstance(groups, str):
        groups = [groups]
    hit = sorted(set(groups) & set(admit.get("groups", [])))
    if hit:
        return True, f"group {hit[0]}"
    email = (claims.get("email") or "").strip().lower()
    if email and email in {e.strip().lower() for e in admit.get("emails", [])}:
        return True, "e-mail allowlist"
    if not claims:
        return False, "no sign-in token reached the door"
    return False, f"organization {aff_id or '(none recorded)'} is not admitted"


REFUSED = """<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Not admitted</title><style>body{{font:15px/1.5 system-ui,sans-serif;margin:0;background:#f6f7f9;color:#16202a}}
main{{max-width:620px;margin:64px auto;padding:0 16px}}h1{{font-size:20px}}code{{background:#e9edf0;padding:1px 5px;border-radius:4px}}
@media (prefers-color-scheme:dark){{body{{background:#0f1418;color:#e6edf3}}code{{background:#24262b}}}}</style></head>
<body><main><h1>{service}: you are signed in but not admitted</h1>
<p>This page is open to: {rule}.</p>
<p>You are signed in as <code>{who}</code>. Reason: {reason}.</p>
<p>If you belong to an admitted organization, ask a goatnet administrator to record your organization on your account. {contact}</p>
</main></body></html>"""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--listen", default="0.0.0.0:8080")
    ap.add_argument("--upstream", required=True, help="http://host:port of the service")
    ap.add_argument("--rules", required=True, help="JSON: {service, rule_text, contact, admit:{aff_ids,groups,emails}}")
    ap.add_argument("--timeout", type=float, default=30)
    a = ap.parse_args()

    with open(a.rules) as f:
        rules = json.load(f)
    admit = rules.get("admit", {})
    up = urllib.parse.urlsplit(a.upstream)
    if up.scheme != "http" or not up.hostname:
        sys.exit("--upstream must be http://host:port")
    up_host, up_port = up.hostname, up.port or 80

    class H(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *args):
            pass

        def _log(self, admitted, who, reason):
            print(json.dumps({"ts": round(time.time(), 3), "service": rules.get("service"), "admitted": admitted,
                              "who": who, "reason": reason, "method": self.command, "path": self.path.split("?")[0]}),
                  flush=True)

        def _refuse(self, who, reason):
            body = REFUSED.format(service=rules.get("service", "This service"), rule=rules.get("rule_text", "listed people"),
                                  who=who or "unknown", reason=reason, contact=rules.get("contact", "")).encode()
            self.send_response(403)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(body)

        def _handle(self):
            if self.path == "/door-admit/healthz":
                body = b"ok\n"
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            claims = claims_from(self.headers.get("Authorization"))
            who = claims.get("email") or self.headers.get("X-Forwarded-Email") or ""
            ok, reason = decide(claims, admit)
            self._log(ok, who, reason)
            if not ok:
                return self._refuse(who, reason)
            length = int(self.headers.get("Content-Length") or 0)
            body = self.rfile.read(length) if length else None
            headers = {k: v for k, v in self.headers.items() if k.lower() not in HOP}
            headers["Host"] = f"{up_host}:{up_port}"
            conn = http.client.HTTPConnection(up_host, up_port, timeout=a.timeout)
            try:
                conn.request(self.command, self.path, body=body, headers=headers)
                resp = conn.getresponse()
                data = resp.read()
            except OSError as e:
                msg = f"the service did not answer: {e}\n".encode()
                self.send_response(502)
                self.send_header("Content-Type", "text/plain")
                self.send_header("Content-Length", str(len(msg)))
                self.end_headers()
                self.wfile.write(msg)
                return
            finally:
                conn.close()
            self.send_response(resp.status, resp.reason)
            for k, v in resp.getheaders():
                if k.lower() not in HOP:
                    self.send_header(k, v)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(data)

        do_GET = do_POST = do_PUT = do_DELETE = do_HEAD = do_PATCH = do_OPTIONS = _handle

    host, port = a.listen.rsplit(":", 1)
    print(json.dumps({"event": "start", "service": rules.get("service"), "rule": rules.get("rule_text"),
                      "admitted_orgs": len(admit.get("aff_ids", [])), "admitted_groups": admit.get("groups", []),
                      "allowlisted_emails": len(admit.get("emails", []))}), flush=True)
    ThreadingHTTPServer((host, int(port)), H).serve_forever()


if __name__ == "__main__":
    main()
