#!/usr/bin/env bats
# ops/ci/check-killed-subshells-clear-exit-trap.py: every background subshell a script
# kills by its pid clears the EXIT trap first. The reason and the measurement are in the
# check's header.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
CHECK="$REPO_ROOT/ops/ci/check-killed-subshells-clear-exit-trap.py"

@test "killed-subshells: the check's own cases" {
  run python3 "$CHECK" --self-test
  [ "$status" -eq 0 ]
  [[ "$output" == *"self-test: PASS"* ]]
}

@test "killed-subshells: every tracked shell script clears the trap in a subshell it kills" {
  run python3 "$CHECK"
  printf '%s\n' "$output" >&2
  [ "$status" -eq 0 ]
}

@test "killed-subshells: the watchdog as it was before the fix is named with its line and why" {
  f="$BATS_TEST_TMPDIR/reach.sh"
  cat > "$f" <<'SH'
trap 'rm -f "$kh" "$errf"' EXIT
ssh "$target" true 2>"$errf" &
pid=$!
( sleep "$limit"; : > "$bound"; kill -TERM "$pid" ) </dev/null >/dev/null 2>&1 &
dog=$!
wait "$pid"; rc=$?
kill -TERM "$dog" 2>/dev/null
SH
  run python3 "$CHECK" "$f"
  [ "$status" -eq 1 ]
  [[ "$output" == *"reach.sh:4: a killed background subshell inherits the EXIT trap; begin it with trap - EXIT"* ]]
}
