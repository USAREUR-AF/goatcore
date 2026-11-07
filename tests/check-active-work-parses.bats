#!/usr/bin/env bats

# The active-work gate has to refuse a row the work CLI cannot load.
#
# #1145: three rows were valid YAML whose SHAPE disagreed with the structs in
# ops/work/main.go (a string where Track.Scope is a list, a list where the
# queued GatedOn is a string). The gate checked YAML validity only, so all three
# merged green and `work update` then refused every write on main. These cases
# pin the gate to the CLI's own parser.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
GATE="$REPO_ROOT/ops/ci/check-active-work-parses.sh"

# This tree may not carry the code a test reads (a public goatcore checkout
# does not carry a goatnet's own scripts); such a test is skipped by name,
# and runs wherever that code is present.
setup() {
  [ -f "$GATE" ] || skip "ops/ci/check-active-work-parses.sh is not in this tree"
  FIXTURE="$BATS_TEST_TMPDIR/repo"
  mkdir -p "$FIXTURE/docs/project/active-work/tracks" "$FIXTURE/docs/project/active-work/queued"
  git -C "$FIXTURE" init -q
}

stage() {
  git -C "$FIXTURE" add -A
}

@test "active-work gate: a well-shaped track passes" {
  printf 'name: good\nowner: someone\nscope:\n  - one thing\nstatus: active\n' \
    > "$FIXTURE/docs/project/active-work/tracks/good.yaml"
  stage
  cd "$FIXTURE"
  run "$GATE"
  [ "$status" -eq 0 ]
}

@test "active-work gate: a track whose scope is a string is refused by name" {
  printf 'name: bad-scope\nowner: someone\nscope: >\n  a folded string, not a list\n' \
    > "$FIXTURE/docs/project/active-work/tracks/bad-scope.yaml"
  stage
  cd "$FIXTURE"
  run "$GATE"
  [ "$status" -eq 1 ]
  [[ "$output" == *"tracks/bad-scope.yaml"* ]]
}

@test "active-work gate: a queued item whose gated_on is a list is refused by name" {
  printf 'name: bad-gate\ngated_on:\n  - first\n  - second\n' \
    > "$FIXTURE/docs/project/active-work/queued/bad-gate.yaml"
  stage
  cd "$FIXTURE"
  run "$GATE"
  [ "$status" -eq 1 ]
  [[ "$output" == *"queued/bad-gate.yaml"* ]]
}
