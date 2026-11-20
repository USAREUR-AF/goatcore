#!/usr/bin/env bats
# ops/goatbuilder/parity-federate-verbs.sh runs each binary from inside the
# vectors directory. The workflow passes the binaries as paths relative to the
# repository root (crates/target/debug/goat), so they must still be found from
# there. When a binary was not found, both sides exited 127 with empty stdout,
# and the rows compared equal without running anything.
#
# Stub binaries record each call; nothing here needs a cargo build.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
PARITY="$REPO_ROOT/ops/goatbuilder/parity-federate-verbs.sh"

setup() {
  W="$BATS_TEST_TMPDIR/w"
  mkdir -p "$W/bin"
  for b in goat goatbuilder; do
    # shellcheck disable=SC2016 # $0 and $* belong to the stub
    printf '#!/bin/sh\necho "$0 $*" >> "%s/calls.%s"\nexit 0\n' "$W" "$b" > "$W/bin/$b"
    chmod +x "$W/bin/$b"
  done
}

@test "parity-federate-verbs: binaries given as relative paths are run from the vectors directory" {
  cd "$W"
  run bash "$PARITY" bin/goat bin/goatbuilder
  # The stubs print nothing, so the far-term rows fail; what matters is that
  # every row reached both binaries.
  [ -f "$W/calls.goat" ]
  [ -f "$W/calls.goatbuilder" ]
  [ "$(grep -c ' federate ' "$W/calls.goat")" -gt 100 ]
  [ "$(grep -c ' federate ' "$W/calls.goatbuilder")" -gt 100 ]
  [[ "$output" != *"exit 127"* ]]
}

@test "parity-federate-verbs: a binary that does not exist is refused before any row runs" {
  cd "$W"
  run bash "$PARITY" bin/no-such-goat bin/goatbuilder
  [ "$status" -eq 1 ]
  [[ "$output" == *"PARITY FAIL: bin/no-such-goat is not an executable file"* ]]
  [ ! -f "$W/calls.goatbuilder" ]
}

@test "parity-federate-verbs: a row neither binary could run is a failure, not parity" {
  for b in goat goatbuilder; do
    printf '#!/bin/sh\nexit 127\n' > "$W/bin/$b"
  done
  run bash "$PARITY" "$W/bin/goat" "$W/bin/goatbuilder"
  [ "$status" -eq 1 ]
  [[ "$output" == *"PARITY FAIL: federate lint "*" — goat exit 127, goatbuilder exit 127; a binary could not be run"* ]]
}
