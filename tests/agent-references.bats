#!/usr/bin/env bats
# ops/ci/check-no-agent-references.sh in whole-tree mode, on a throwaway repo:
# ADR 1018, the decision record for authoring design in an AI design tool, names
# that tool's site and may (ADRs are never rewritten); the same site named in any
# other document is still refused.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  T="$(mktemp -d)"
  mkdir -p "$T/ops/ci" "$T/docs/adr" "$T/docs/operations"
  cp "$REPO_ROOT/ops/ci/check-no-agent-references.sh" "$T/ops/ci/"
  git -C "$T" init -q
  site="claude"".ai/design"
  printf 'authored in %s\n' "$site" > "$T/docs/adr/1018-augmented-design-driven-development.md"
  printf 'plain text\n' > "$T/docs/operations/checklist.md"
}

teardown() {
  rm -rf "$T"
}

commit_all() {
  git -C "$T" add -A
  git -C "$T" -c user.name=t -c user.email=t@example.org -c commit.gpgsign=false commit -q -m fixture
}

@test "ADR 1018 naming the design tool's site passes the whole-tree scan" {
  commit_all
  run bash -c "cd '$T' && bash ops/ci/check-no-agent-references.sh"
  [ "$status" -eq 0 ]
  [[ "$output" != *"REFUSED"* ]]
}

@test "the same site in another document is refused" {
  printf 'see %s\n' "claude"".ai/design" > "$T/docs/operations/checklist.md"
  commit_all
  run bash -c "cd '$T' && bash ops/ci/check-no-agent-references.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"REFUSED  docs/operations/checklist.md"* ]]
  [[ "$output" != *"REFUSED  docs/adr/1018"* ]]
}

@test "another ADR naming the site is refused: the allow entry is this one record" {
  printf 'see %s\n' "claude"".ai/design" > "$T/docs/adr/1019-something-else.md"
  commit_all
  run bash -c "cd '$T' && bash ops/ci/check-no-agent-references.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"REFUSED  docs/adr/1019-something-else.md"* ]]
}
