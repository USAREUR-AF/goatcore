#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# mixed-version.sh — Zenoh candidate row: old and new clients through one
# candidate router, both directions (ADR 1262, upstream-upgrade-process.md §4).
#
# Boots ROUTER_IMAGE as a plain-TCP router on loopback, then for each ordered
# pair (A serves, B drives) of the client versions given, runs client.py at
# those eclipse-zenoh versions in python:3.12-slim on the host network:
# pub/sub (20 samples), query/reply, liveliness, Advanced Pub/Sub history.
# Every pair must pass. Needs docker and network access to PyPI.
#
#   mixed-version.sh <router-image> <client-version> [<client-version> ...]
#   e.g. mixed-version.sh ghcr.io/<org>/zenoh-pqc@sha256:... 1.9.0 1.10.2
set -uo pipefail

IMG="${1:?usage: mixed-version.sh <router-image> <client-version>...}"; shift
[ "$#" -ge 1 ] || { echo "give at least one client version" >&2; exit 2; }
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DOCKER="${DOCKER:-docker}"
PORT="${PORT:-17460}"
NAME="zenoh-cand-mixed-$$"
WORK="$(mktemp -d)"
FAILS=0

cleanup() { $DOCKER rm -f "$NAME" >/dev/null 2>&1 || true; rm -rf "$WORK"; }
trap cleanup EXIT

cat > "$WORK/router.json5" <<JSON5
{
  mode: "router",
  listen: { endpoints: ["tcp/127.0.0.1:${PORT}"] },
  scouting: { multicast: { enabled: false } },
  timestamping: { enabled: { router: true, peer: true, client: true } },
}
JSON5

echo "== mixed-version: router $IMG, clients $*"
$DOCKER run -d --name "$NAME" --network host -v "$WORK":/cfg:ro "$IMG" -c /cfg/router.json5 >/dev/null
for _ in $(seq 1 30); do
  (exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null && break
  sleep 1
done
if ! (exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null; then
  echo "[ERROR] router did not open tcp/127.0.0.1:$PORT"; $DOCKER logs "$NAME" 2>&1 | tail -20; exit 1
fi
$DOCKER exec "$NAME" /zenohd --version 2>/dev/null | head -1 || true

client() {  # client <version> <args...>
  local v="$1"; shift
  $DOCKER run --rm --network host -v "$HERE":/cand:ro \
    -e ZENOH_ENDPOINT="tcp/127.0.0.1:${PORT}" -e ZENOH_MODE=client \
    python:3.12-slim sh -c "pip install -q --disable-pip-version-check eclipse-zenoh==$v >/dev/null 2>&1 && python /cand/client.py $*"
}

for a in "$@"; do
  for b in "$@"; do
    client "$a" serve 40 > "$WORK/serve-$a-$b.txt" 2>&1 &
    spid=$!
    sleep 8   # pip install in the serving container
    drive_out="$(client "$b" drive 2>&1)"; drc=$?
    wait "$spid"; src=$?
    serve_line="$(grep SERVE_RESULT "$WORK/serve-$a-$b.txt" || true)"
    drive_line="$(printf '%s\n' "$drive_out" | grep DRIVE_RESULT || true)"
    if [ "$drc" -eq 0 ] && [ "$src" -eq 0 ]; then
      echo "  PASS serve $a / drive $b"
    else
      echo "  FAIL serve $a / drive $b (drive rc=$drc serve rc=$src)"
      echo "    ${serve_line:-$(tail -3 "$WORK/serve-$a-$b.txt")}"
      echo "    ${drive_line:-$(printf '%s\n' "$drive_out" | tail -3)}"
      FAILS=$((FAILS + 1))
    fi
  done
done

if [ "$FAILS" -ne 0 ]; then
  echo "[FAIL] $FAILS pair(s) failed"; $DOCKER logs "$NAME" 2>&1 | tail -20; exit 1
fi
echo "[PASS] every client pair works through the router"
