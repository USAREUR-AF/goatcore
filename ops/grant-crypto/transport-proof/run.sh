#!/usr/bin/env bash
# E0/M2.0 on-fabric transport proof driver.
#
# Builds the gce crypto CLI, ensures a pinned eclipse-zenoh (matching the fleet
# router pin =1.9.0, zenoh-version-pinning.md) in a throwaway venv, and runs the
# harness — a Go-crypto + live-Zenoh end-to-end proof (see harness.py).
#
#   ./run.sh            # self-contained; prints PASS or a FAIL reason
#
# Exit 0 = proof passed. No network beyond the loopback Zenoh link and, on first
# run, the pip install of eclipse-zenoh.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODULE_ROOT="$(cd "$HERE/.." && pwd)"
ZENOH_PIN="${ZENOH_PIN:-1.10.2}"
VENV="${VENV:-$HERE/.venv}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/gce-e0.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

echo "==> building gce"
GCE="$WORK/gce"
( cd "$MODULE_ROOT" && go build -o "$GCE" ./cmd/gce )

echo "==> ensuring eclipse-zenoh==$ZENOH_PIN"
# Prefer a 3.11-3.13 interpreter — eclipse-zenoh wheels lag the newest CPython.
PY=""
for cand in python3.13 python3.12 python3.11 python3; do
  if command -v "$cand" >/dev/null 2>&1; then PY="$cand"; break; fi
done
[ -n "$PY" ] || { echo "FAIL: no python3 found"; exit 1; }
if [ ! -x "$VENV/bin/python" ]; then
  "$PY" -m venv "$VENV"
fi
"$VENV/bin/python" -c "import zenoh" 2>/dev/null || \
  "$VENV/bin/pip" install --quiet --disable-pip-version-check "eclipse-zenoh==$ZENOH_PIN"

echo "==> running transport proof"
# Quiet the router's own logging; the harness prints the verdict.
export RUST_LOG="${RUST_LOG:-error}" GCE="$GCE"
"$VENV/bin/python" "$HERE/harness.py" "$WORK/run"
