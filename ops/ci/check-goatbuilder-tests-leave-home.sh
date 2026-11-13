#!/usr/bin/env bash
# ops/ci/check-goatbuilder-tests-leave-home.sh: the goatbuilder test suite
# writes nothing into the real state directory (~/.goat/state).
#
# A verb that drives hosts writes a run record and its logs under the state
# directory (docs/design/goatbuilder-legibility.md, "Run record"). Every test
# that runs a verb points HOME at a directory of its own
# (crates/goatbuilder/tests/common/test_home.rs, or a temporary root). This gate
# fails if anything under ~/.goat/state is newer than a marker made before the
# tests ran. A real run on the same machine at the same time trips it too, so
# run it where nothing else drives hosts (CI, or a quiet laptop).
#
# Usage:
#   ops/ci/check-goatbuilder-tests-leave-home.sh            run the suite, then check
#   ops/ci/check-goatbuilder-tests-leave-home.sh --after M  check only, against marker file M
#                                                           (CI touches M before its test steps)
set -euo pipefail

state="${HOME:?}/.goat/state"
marker=""
if [ "${1:-}" = "--after" ]; then
  marker="${2:?--after needs the marker file}"
  [ -f "$marker" ] || { echo "no marker file $marker" >&2; exit 2; }
else
  marker="$(mktemp)"
  trap 'rm -f "$marker"' EXIT
  # Whatever is written in the same second as the marker still counts as newer.
  sleep 1
  (cd "$(git rev-parse --show-toplevel)/crates" && cargo test -p goatbuilder --locked)
fi

if [ -d "$state" ]; then
  written="$(find "$state" -newer "$marker" -print 2>/dev/null || true)"
  if [ -n "$written" ]; then
    echo "refused: the goatbuilder tests wrote into the real state directory $state:" >&2
    printf '%s\n' "$written" | sed 's/^/  /' >&2
    echo "Point the test's HOME at a directory of its own (crates/goatbuilder/tests/common/test_home.rs)." >&2
    exit 1
  fi
fi
echo "ok: the goatbuilder tests wrote nothing under $state"
