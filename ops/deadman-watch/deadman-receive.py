#!/usr/bin/env python3
"""deadman-receive — the heartbeat sink half of the dead-man's switch.

Receives the Alertmanager `Watchdog` webhook and does exactly one thing: writes
the current time to a file. It deliberately does NOT decide anything. The
deciding half is deadman-check.sh, run by a separate systemd timer, so that this
process dying is itself detectable — the file simply goes stale and the checker
fires. A receiver that also did the checking would take its own failure with it.

Why this exists: every alert rule assumes Prometheus is alive to
evaluate it. When the obs host dies, all of them go quiet, and quiet is what a
healthy fleet looks like too. The Watchdog alert fires forever so that its
ABSENCE is the signal — but absence can only be noticed from somewhere else.
This is the somewhere else.

ONE FILE PER SOURCE. Each obs host posts to POST /<its-hostname> and
gets its own timestamp file, <state-dir>/<source>.last-heartbeat, so the checker
can watch each one separately. With a single shared file, any live obs host kept
the file fresh for all of them, so a dead host sharing a watcher with a live
one never paged. A bare POST /
still writes the legacy file so an un-migrated sender keeps working.

Deliberately stdlib-only and deliberately not containerised: it must have fewer
moving parts than the thing it watches. A watcher that needs the docker daemon
inherits the docker daemon's failure modes.
"""
import hmac
import http.server
import os
import re
import socketserver
import sys
import tempfile
import time
import urllib.parse

STATE_FILE = os.environ.get("DEADMAN_STATE_FILE", "/var/lib/deadman-watch/last-heartbeat")
LISTEN_ADDR = os.environ.get("DEADMAN_LISTEN_ADDR", "127.0.0.1")
LISTEN_PORT = int(os.environ.get("DEADMAN_LISTEN_PORT", "9099"))
TOKEN = os.environ.get("DEADMAN_TOKEN", "")

# A source name becomes a file name, so it is held to a hostname shape: no dots,
# no slashes, nothing that could climb out of the state directory.
SOURCE_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,62}$")


def state_path_for(source: str) -> str:
    """Timestamp file for a source; "" is the legacy single file.

    The source is already held to SOURCE_RE; the containment check below is the
    second wall, so a future change to the pattern cannot write outside the
    state directory.
    """
    if not source:
        return STATE_FILE
    base = os.path.realpath(os.path.dirname(STATE_FILE))
    target = os.path.realpath(os.path.join(base, f"{source}.last-heartbeat"))
    if not target.startswith(base + os.sep):
        raise ValueError(f"source {source!r} resolves outside the state directory")
    return target


def source_from_path(path: str):
    """Return the source named by the request path, "" for "/", None if invalid."""
    name = urllib.parse.urlparse(path).path.strip("/")
    if name == "":
        return ""
    return name if SOURCE_RE.fullmatch(name) else None


def record_heartbeat(source: str = "") -> None:
    """Write the timestamp atomically.

    Atomic because a torn write during a crash would leave a file the checker
    cannot parse, and an unparseable state file must never read as 'recent'.
    """
    target = state_path_for(source)
    os.makedirs(os.path.dirname(target), exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=os.path.dirname(target))
    try:
        with os.fdopen(fd, "w") as fh:
            fh.write(str(int(time.time())))
        os.replace(tmp, target)
    except Exception:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise


class Handler(http.server.BaseHTTPRequestHandler):
    def _token_ok(self) -> bool:
        """Accept the shared secret in any form Alertmanager can actually send."""
        if self.headers.get("X-Deadman-Token", "") == TOKEN:
            return True
        auth = self.headers.get("Authorization", "")
        if auth.startswith("Bearer ") and auth[7:] == TOKEN:
            return True
        parsed = urllib.parse.urlparse(self.path)
        supplied = urllib.parse.parse_qs(parsed.query).get("token", [""])[0]
        # hmac.compare_digest to keep the comparison constant-time; the others
        # above are cheap identity checks on already-equal-length strings.
        return hmac.compare_digest(supplied, TOKEN)

    def _reply(self, code: int, body: str = "") -> None:
        payload = body.encode()
        self.send_response(code)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_POST(self) -> None:  # noqa: N802 - stdlib naming
        # Shared-secret check. The listener is mesh-only, so this is defence in
        # depth rather than the primary control — but a heartbeat sink that any
        # peer can spoof would let a compromised host mask the obs host dying,
        # which is precisely the state this is built to detect.
        # Three accepted forms, because Alertmanager 0.27's webhook_config cannot
        # set arbitrary headers: its http_config can send `Authorization: Bearer`,
        # and a query parameter always works. X-Deadman-Token is for curl and
        # anything else hand-driven.
        if TOKEN and not self._token_ok():
            self._reply(403, "forbidden\n")
            return
        source = source_from_path(self.path)
        if source is None:
            self._reply(404, "unknown source path\n")
            return
        # Drain the body; Alertmanager sends JSON we do not need to parse. The
        # arrival is the signal, not the content.
        length = int(self.headers.get("Content-Length", "0") or 0)
        if length:
            self.rfile.read(length)
        try:
            record_heartbeat(source)
        except Exception as exc:  # pragma: no cover - operational path
            print(f"deadman-receive: failed to record heartbeat: {exc}", file=sys.stderr, flush=True)
            self._reply(500, "state write failed\n")
            return
        self._reply(200, "ok\n")

    def do_GET(self) -> None:  # noqa: N802 - stdlib naming
        # Compare the PATH only; a query string must not turn the health probe
        # into a 404 (the ansible verify step would then loop until it fails).
        if urllib.parse.urlparse(self.path).path == "/healthz":
            self._reply(200, "ok\n")
            return
        self._reply(404, "not found\n")

    def log_message(self, fmt, *args):
        # journald already timestamps; keep one terse line per request.
        #
        # REDACT THE QUERY STRING. The token can arrive as ?token=... because
        # Alertmanager's webhook_config cannot set arbitrary headers, and the
        # default BaseHTTPRequestHandler log line echoes the full request path.
        # That wrote the shared secret into the journal — and the journal is now
        # persistent fleet-wide, so it would sit on disk indefinitely.
        # A credential in a log is a credential you have to rotate.
        line = fmt % args
        line = re.sub(r"([?&](?:token|Authorization)=)[^\s\"&]+", r"\1<redacted>", line)
        # The request line already carries the source path; the peer address is
        # added because without it nobody could tell WHICH host was keeping a
        # heartbeat fresh — the question F-529 needed a packet capture to answer.
        print(f"deadman-receive: {self.client_address[0]} {line}", file=sys.stderr, flush=True)


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def main() -> int:
    print(
        f"deadman-receive: listening on {LISTEN_ADDR}:{LISTEN_PORT}, state={STATE_FILE}",
        file=sys.stderr,
        flush=True,
    )
    with Server((LISTEN_ADDR, LISTEN_PORT), Handler) as srv:
        srv.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
