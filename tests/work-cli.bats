#!/usr/bin/env bats
# Regression tests for the ops/work/work wrapper.
#
# The wrapper rebuilds the ops/work Go module from source on every invocation.
# The original `exec go run "$DIR"` resolved the package against the CALLER's
# cwd module, not $DIR's — and there is no go.mod/go.work at the repo root, so
# every caller invoking `work` from the master worktree root (the /iso skill,
# `work release`) died with "go: cannot find main module". That silently broke
# `work pickup` from iso enter, so tracks never registered and cleanup-on-release
# never fired. These tests pin the wrapper to resolve its module regardless of
# the caller's cwd.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
WORK="$REPO_ROOT/ops/work/work"

# This tree may not carry the code a test reads (a public goatcore checkout
# does not carry a goatnet's own scripts); such a test is skipped by name,
# and runs wherever that code is present.
setup() {
  [ -f "$WORK" ] || skip "ops/work is not in this tree"
}

@test "work wrapper: runs from repo root (a non-module cwd — the bug's location)" {
  cd "$REPO_ROOT"
  run "$WORK" list
  [ "$status" -eq 0 ]
  [[ "$output" != *"cannot find main module"* ]]
  [[ "$output" != *"is not in std"* ]]
}

@test "work wrapper: builds + runs from an unrelated cwd (not the go-module error)" {
  cd "$BATS_TEST_TMPDIR"
  run "$WORK" list
  # The CLI itself may error (not inside a git repo) — that's the binary
  # RUNNING, which is the point. What must NOT appear is the build-time
  # module-resolution failure.
  [[ "$output" != *"cannot find main module"* ]]
  [[ "$output" != *"is not in std"* ]]
}

@test "work wrapper: does not leave a .work-bin temp behind" {
  cd "$REPO_ROOT"
  run "$WORK" list
  [ "$status" -eq 0 ]
  run bash -c "ls '$REPO_ROOT'/ops/work/.work-bin.* 2>/dev/null"
  [ -z "$output" ]
}
