#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""traefik-compression-metrics.py — make response compression observable.

WHY THIS EXISTS
---------------
Traefik's own Prometheus output cannot see compression. `traefik_service_
responses_bytes_total` counts what the BACKEND produced, not what Traefik put
on the wire, so it moves identically whether or not `goat-compress` is
attached. That was measured on a live goatnet 2026-08-18: over one window the metric
tracked the origin figure while the wire figure was a third of it. Anything
built on that counter will report a healthy-looking number after compression
silently stops.

The access log is the only place both numbers exist, per request:
`OriginContentSize` (what the service produced) and `DownstreamContentSize`
(what Traefik sent). This exporter reads that log and publishes the pair, so
the ratio becomes a graphable, alertable series.

WHAT IT PUBLISHES  (node_exporter textfile collector)
-----------------------------------------------------
  goat_traefik_origin_bytes_total{service,client_class}
  goat_traefik_wire_bytes_total{service,client_class}
  goat_traefik_eligible_origin_bytes_total{service,client_class}
  goat_traefik_eligible_wire_bytes_total{service,client_class}
  goat_traefik_responses_total{service,client_class,compressed}
  goat_traefik_compressible_missed_bytes_total{service,client_class}
  goat_traefik_compression_exporter_*        (self-honesty; see below)

`client_class` is the single most important label here and the reason this is
not just a global ratio. Measured on a live goatnet the day compression shipped:

    mesh peers 100.64/10  (real users)   4,153 req   5.97 -> 1.71 MB   71.4%
    gateway    172.18.0.1 (our probes)  27,769 req  21.53 -> 21.87 MB   -1.6%
    combined                            31,922 req  27.51 -> 23.58 MB   14.3%

A single combined ratio reports 14.3% and hides both facts: that users get
71.4%, and that 87% of requests come from a canary which does not negotiate
compression and never will. Splitting the label keeps the alert honest — a
regression in user traffic cannot be masked by probe volume, and probe volume
cannot be mistaken for a regression.

RAW vs ELIGIBLE, AND WHY THE RATIO NEEDS BOTH
---------------------------------------------
The raw `origin`/`wire` pair is every measurable response. Its ratio is honest
only when the workload is dominated by compressible pages. When a service is
dominated instead by tiny API polls (under the size floor), event streams, or
chunked responses, its raw ratio collapses toward zero for reasons that have
nothing to do with whether compression works. Measured on a live goatnet 2026-09-21: the
portal's raw mesh ratio read 8.5% while its ratio over the responses
goat-compress can actually act on read 36.5%.

`eligible_origin`/`eligible_wire` is the ratio input restricted to what the
middleware is able to compress: a body over the `minResponseBodyBytes` floor, a
compressible content type, and not a type the middleware never compresses. That
ratio is the honest health signal; the raw pair is retained for bandwidth
accounting and for parity with the older dashboards.

`compressible_missed` is the actionable one: bytes of an ELIGIBLE response that
went out uncompressed. On the gateway class that is expected and inert. On the
mesh class it means a real client is leaving bytes on the table. It is scoped to
the eligible population for the same reason as the ratio: a response under the
floor was declined deliberately, not missed.

COUNTERS, AND WHY THEY ARE MONOTONIC ACROSS RUNS
------------------------------------------------
Each run reads only the last `--window` of log and would naturally produce a
gauge. Prometheus `rate()` needs a counter, so the previous values are read
back from the existing .prom file and added to. State lives in the published
file itself rather than a sidecar, so deleting the file resets cleanly and
there is no second thing to keep in sync. A restart therefore shows as a
counter reset, which `rate()` already handles.

SELF-HONESTY
------------
Every failure mode publishes rather than hides: if the log cannot be read the
exporter emits `goat_traefik_compression_exporter_up 0` and leaves the previous
counters untouched, so a broken exporter is visible as a stale-but-flat series
plus an explicit zero, never as "compression stopped". A record the log
records without both size fields, or with an unmeasurable origin size, is
counted in `_skipped_records` rather than being silently folded into the ratio.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from collections import defaultdict

# Content types worth compressing. Anything else that arrives uncompressed is
# not a missed opportunity, so it must not inflate `compressible_missed`.
COMPRESSIBLE = re.compile(
    r"^(?:text/|application/(?:json|javascript|xml|x-javascript|manifest\+json)"
    r"|image/svg\+xml)",
    re.I,
)

# Never counted as a missed opportunity even though the type is textual:
# compressing an event stream delays the events it exists to deliver, and the
# goat-compress middleware excludes it deliberately. Counting it here would
# create a metric that only goes up and can never be acted on.
NEVER_COMPRESS = ("text/event-stream", "application/grpc")

# The goat-compress middleware's `minResponseBodyBytes`. Below this the
# middleware does not compress, by design (the gzip framing would cost more than
# it saves), so a smaller response left uncompressed is correct, not a miss.
MIN_COMPRESS_BYTES = 1024


def eligible(ctype: str, origin: int) -> bool:
    """True when goat-compress is able to act on this response, so it belongs in
    the honest compression ratio.

    A response qualifies only if it clears the size floor, is a compressible
    content type, and is not one the middleware deliberately never compresses
    (event streams, grpc). Tiny, streamed and binary responses are excluded
    because compressing them is impossible or deliberately declined, and
    counting them drags the ratio toward zero for reasons unrelated to health.
    """
    if origin < MIN_COMPRESS_BYTES:
        return False
    if any(ctype.startswith(t) for t in NEVER_COMPRESS):
        return False
    return bool(COMPRESSIBLE.match(ctype))


def client_class(host: str) -> str:
    """mesh | gateway | other.

    100.64/10 is the NetBird CGNAT range: a real peer, which is to say a real
    person's browser or a peer-resident client. Anything on a private bridge
    address reached Traefik from a container on this host — our own probes and
    the relay-ingress path — and shares one source address, so it can be
    classified but never attributed further (peer-demand-surface.md).
    """
    if host.startswith("100."):
        # 100.64/10 spans 100.64.x - 100.127.x; be precise rather than clever.
        try:
            second = int(host.split(".")[1])
            if 64 <= second <= 127:
                return "mesh"
        except (IndexError, ValueError):
            pass  # unexpected host form: not mesh
    if host.startswith(("172.", "10.", "192.168.", "127.")):
        return "gateway"
    return "other"


def parse_records(text: str) -> tuple[list[dict], int]:
    """Turn raw access-log text into records usable for a compression ratio.

    Returns (records, skipped). A line is skipped and counted when it cannot
    contribute to a ratio:
      - it is not a JSON object,
      - it is missing either size field,
      - its OriginContentSize is <= 0. Traefik logs 0 (older builds -1) for a
        chunked or streamed response that sends no Content-Length. That origin
        size is UNKNOWN, not zero, so dividing by it manufactures a false
        figure: a missing field would read as a 100% saving, a chunked response
        as pure expansion (wire bytes over a zero origin). This is the same
        reason a missing field is skipped, applied to a present-but-unusable one.
    """
    records: list[dict] = []
    skipped = 0
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            skipped += 1
            continue
        o = rec.get("OriginContentSize")
        d = rec.get("DownstreamContentSize")
        if o is None or d is None:
            skipped += 1
            continue
        try:
            if int(o) <= 0:
                skipped += 1
                continue
        except (TypeError, ValueError):
            skipped += 1
            continue
        records.append(rec)
    return records, skipped


def read_log(container: str, window_s: int, max_bytes: int) -> tuple[list[dict], int, str | None]:
    """Return (records, skipped, error). Never raises for an unreadable log."""
    try:
        raw = subprocess.run(
            ["docker", "logs", "--since", f"{window_s}s", container],
            capture_output=True,
            timeout=90,
        ).stdout
    except FileNotFoundError:
        return [], 0, "docker not on PATH"
    except subprocess.TimeoutExpired:
        return [], 0, "docker logs timed out"
    except Exception as exc:  # noqa: BLE001 - any failure must publish, not crash
        return [], 0, f"docker logs failed: {type(exc).__name__}"

    if len(raw) > max_bytes:
        raw = raw[-max_bytes:]

    records, skipped = parse_records(raw.decode("utf-8", "replace"))
    return records, skipped, None


def accumulate(
    records: list[dict],
) -> tuple[
    dict[tuple[str, str], int],
    dict[tuple[str, str], int],
    dict[tuple[str, str], int],
    dict[tuple[str, str], int],
    dict[tuple[str, str, str], int],
    dict[tuple[str, str], int],
]:
    """Fold records into the published series.

    Returns (origin, wire, eligible_origin, eligible_wire, responses, missed).
    Callers pass records that already cleared parse_records, so every record
    here has a positive, measurable origin size.
    """
    origin: dict[tuple[str, str], int] = defaultdict(int)
    wire: dict[tuple[str, str], int] = defaultdict(int)
    eligible_origin: dict[tuple[str, str], int] = defaultdict(int)
    eligible_wire: dict[tuple[str, str], int] = defaultdict(int)
    resp: dict[tuple[str, str, str], int] = defaultdict(int)
    missed: dict[tuple[str, str], int] = defaultdict(int)

    for rec in records:
        svc = str(rec.get("ServiceName") or "-")
        cls = client_class(str(rec.get("ClientHost") or ""))
        o = int(rec.get("OriginContentSize") or 0)
        d = int(rec.get("DownstreamContentSize") or 0)
        ctype = str(
            rec.get("downstream_Content-Type") or rec.get("DownstreamContentType") or ""
        ).lower().split(";")[0].strip()

        origin[(svc, cls)] += o
        wire[(svc, cls)] += d
        compressed = "true" if (o > 0 and d < o) else "false"
        resp[(svc, cls, compressed)] += 1

        # The honest ratio and the missed count are both scoped to what the
        # middleware can actually act on. A response under the floor, of a
        # non-compressible or excluded type, is neither a compression success
        # nor a miss, so it belongs in neither.
        if eligible(ctype, o):
            eligible_origin[(svc, cls)] += o
            eligible_wire[(svc, cls)] += d
            if compressed == "false":
                missed[(svc, cls)] += o

    return origin, wire, eligible_origin, eligible_wire, resp, missed


def load_previous(path: str) -> dict[tuple[str, ...], float]:
    """Read counters back out of the published file. Missing file = fresh start."""
    prev: dict[tuple[str, ...], float] = {}
    try:
        with open(path) as fh:
            for line in fh:
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                name, _, rest = line.partition("{")
                if not rest:
                    continue
                labels, _, value = rest.rpartition("}")
                try:
                    prev[(name, labels)] = float(value.strip())
                except ValueError:
                    continue
    except FileNotFoundError:
        pass  # no prior metrics file
    except OSError:
        pass  # unreadable prior metrics file
    return prev


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--container", default="traefik")
    ap.add_argument("--window", type=int, default=300, help="seconds of log to read")
    ap.add_argument("--max-bytes", type=int, default=64 * 1024 * 1024)
    ap.add_argument("--out", required=True, help="textfile collector .prom path")
    args = ap.parse_args()

    started = time.time()
    records, skipped, error = read_log(args.container, args.window, args.max_bytes)
    prev = load_previous(args.out)

    origin, wire, eligible_origin, eligible_wire, resp, missed = accumulate(records)

    lines: list[str] = []

    def emit(name: str, help_text: str, mtype: str, samples: list[tuple[str, float]]) -> None:
        lines.append(f"# HELP {name} {help_text}")
        lines.append(f"# TYPE {name} {mtype}")
        for labels, value in samples:
            key = (name, labels)
            base = prev.get(key, 0.0) if mtype == "counter" else 0.0
            lines.append(f"{name}{{{labels}}} {base + value:.0f}")

    def lbl(**kw: str) -> str:
        return ",".join(f'{k}="{v}"' for k, v in kw.items())

    emit(
        "goat_traefik_origin_bytes_total",
        "Bytes the backend produced, before any compression, from the Traefik access log.",
        "counter",
        [(lbl(service=s, client_class=c), v) for (s, c), v in sorted(origin.items())],
    )
    emit(
        "goat_traefik_wire_bytes_total",
        "Bytes Traefik actually sent downstream. Divided by origin, the raw compression effect.",
        "counter",
        [(lbl(service=s, client_class=c), v) for (s, c), v in sorted(wire.items())],
    )
    emit(
        "goat_traefik_eligible_origin_bytes_total",
        "Origin bytes of responses goat-compress can act on (over the size floor, compressible type, not excluded). Divided by eligible_wire, the honest compression ratio.",
        "counter",
        [(lbl(service=s, client_class=c), v) for (s, c), v in sorted(eligible_origin.items())],
    )
    emit(
        "goat_traefik_eligible_wire_bytes_total",
        "Wire bytes of the eligible responses. See goat_traefik_eligible_origin_bytes_total.",
        "counter",
        [(lbl(service=s, client_class=c), v) for (s, c), v in sorted(eligible_wire.items())],
    )
    emit(
        "goat_traefik_responses_total",
        "Responses seen, split by whether the wire size came in under the origin size.",
        "counter",
        [
            (lbl(service=s, client_class=c, compressed=k), v)
            for (s, c, k), v in sorted(resp.items())
        ],
    )
    emit(
        "goat_traefik_compressible_missed_bytes_total",
        "Origin bytes of an eligible response sent uncompressed because the client did not negotiate.",
        "counter",
        [(lbl(service=s, client_class=c), v) for (s, c), v in sorted(missed.items())],
    )

    # Self-honesty block. `up 0` with untouched counters is the honest shape of
    # a broken exporter; a zeroed ratio would read as "compression stopped".
    lines.append("# HELP goat_traefik_compression_exporter_up Whether this run could read the access log.")
    lines.append("# TYPE goat_traefik_compression_exporter_up gauge")
    lines.append(f"goat_traefik_compression_exporter_up {0 if error else 1}")
    lines.append("# HELP goat_traefik_compression_exporter_records Access-log records parsed on the last run.")
    lines.append("# TYPE goat_traefik_compression_exporter_records gauge")
    lines.append(f"goat_traefik_compression_exporter_records {len(records)}")
    lines.append("# HELP goat_traefik_compression_exporter_skipped_records Records with no usable size pair, excluded from every ratio.")
    lines.append("# TYPE goat_traefik_compression_exporter_skipped_records gauge")
    lines.append(f"goat_traefik_compression_exporter_skipped_records {skipped}")
    lines.append("# HELP goat_traefik_compression_exporter_duration_seconds Wall time of the last run.")
    lines.append("# TYPE goat_traefik_compression_exporter_duration_seconds gauge")
    lines.append(f"goat_traefik_compression_exporter_duration_seconds {time.time() - started:.3f}")

    if error:
        # Carry the previous counters forward untouched so a failed run reads as
        # flat, not as a collapse to zero.
        for (name, labels), value in sorted(prev.items()):
            if name.startswith("goat_traefik_") and not name.endswith(
                ("_up", "_records", "_skipped_records", "_duration_seconds")
            ):
                lines.append(f"{name}{{{labels}}} {value:.0f}")
        print(f"traefik-compression-metrics: {error}", file=sys.stderr)

    body = "\n".join(lines) + "\n"
    out_dir = os.path.dirname(args.out) or "."
    os.makedirs(out_dir, exist_ok=True)
    # Atomic replace: node_exporter reads this directory continuously and a
    # half-written file is a parse error on the scrape, not a retry.
    fd, tmp = tempfile.mkstemp(dir=out_dir, prefix=".compression-", suffix=".tmp")
    try:
        with os.fdopen(fd, "w") as fh:
            fh.write(body)
        os.chmod(tmp, 0o644)
        os.replace(tmp, args.out)
    except Exception:
        try:
            os.unlink(tmp)
        except OSError:
            pass  # best-effort temp cleanup
        raise

    print(
        f"traefik-compression-metrics: {len(records)} records, "
        f"{len(origin)} service/class pairs, skipped={skipped}, up={0 if error else 1}"
    )
    return 1 if error else 0


if __name__ == "__main__":
    sys.exit(main())
