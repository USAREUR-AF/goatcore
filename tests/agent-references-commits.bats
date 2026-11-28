#!/usr/bin/env bats
# ops/ci/check-no-agent-references-in-commits.sh on a throwaway repository: the
# commit-message scan of .github/workflows/agent-references.yml reads only the
# pull request's own commits, never a commit that main gained after the pull
# request's base.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  SCAN="$REPO_ROOT/ops/ci/check-no-agent-references-in-commits.sh"
  T="$(mktemp -d)"
  git -C "$T" init -q -b main
  # Built in two halves so this file does not itself hold the pattern.
  SITE="claude"".ai"
  commit main-base "the base the pull request was cut from"
  git -C "$T" checkout -q -b pr
  commit pr-change "a change of the pull request's own"
  git -C "$T" checkout -q main
  # main moves on with a commit that quotes the pattern while describing it.
  commit main-later "the guard refuses $SITE and anthropic"".com in a message"
}

teardown() {
  rm -rf "$T"
}

commit() {
  printf '%s\n' "$1" > "$T/$1"
  git -C "$T" add -A
  git -C "$T" -c user.name=t -c user.email=t@example.org -c commit.gpgsign=false commit -q -m "$2"
}

@test "a pull request on an old base is not refused for a commit main gained since" {
  # pull_request_target: the base is main's tip, the head is the pull request's.
  run bash -c "cd '$T' && bash '$SCAN' main pr"
  [ "$status" -eq 0 ]
  [[ "$output" == *"ok: no agent-tooling references in commit messages"* ]]
}

@test "a pull request commit that quotes the pattern is refused, on an old base too" {
  git -C "$T" checkout -q pr
  commit pr-quote "see https://$SITE/x"
  run bash -c "cd '$T' && bash '$SCAN' main pr"
  [ "$status" -eq 1 ]
  [[ "$output" == *"see https://$SITE/x"* ]]
}

@test "a push to main scans the commits it pushed" {
  run bash -c "cd '$T' && bash '$SCAN' main~1 main"
  [ "$status" -eq 1 ]
  [[ "$output" == *"the guard refuses $SITE"* ]]
}

@test "a force push does not scan the commits it dropped" {
  # The pushed range's 'before' is a commit the new tip does not contain.
  git -C "$T" checkout -q -b dropped main~1
  commit dropped "a dropped commit quoting $SITE"
  git -C "$T" checkout -q -B main main~1
  commit main-new "a replacement commit"
  run bash -c "cd '$T' && bash '$SCAN' dropped main"
  [ "$status" -eq 0 ]
}

@test "a usage error exits 2" {
  run bash -c "cd '$T' && bash '$SCAN' main"
  [ "$status" -eq 2 ]
}

@test "the workflow scans commit messages when the recorded base predates the scanner" {
  local scanner_path=ops/ci/check-no-agent-references-in-commits.sh
  mkdir -p "$T/ops/ci"
  cp "$SCAN" "$T/$scanner_path"
  git -C "$T" add "$scanner_path"
  git -C "$T" -c user.name=t -c user.email=t@example.org -c commit.gpgsign=false commit -q -m 'add the scanner'
  GITHUB_WORKFLOW_SHA="$(git -C "$T" rev-parse HEAD)"
  RUNNER_TEMP="$BATS_TEST_TMPDIR"
  RANGE_BASE="$(git -C "$T" rev-parse main~2)"
  RANGE_HEAD=pr
  export GITHUB_WORKFLOW_SHA RUNNER_TEMP RANGE_BASE RANGE_HEAD

  local step
  step="$(python3 - "$REPO_ROOT/.github/workflows/agent-references.yml" <<'PY'
import sys
from pathlib import Path

import yaml

workflow = yaml.safe_load(Path(sys.argv[1]).read_text())
steps = workflow["jobs"]["agent-references"]["steps"]
step = next(item for item in steps if item.get("name") == "Check the commit messages too")
print(step["run"].replace("${{ github.event_name }}", "pull_request_target"))
PY
)"

  git -C "$T" checkout -q --detach "$RANGE_BASE"
  [ ! -e "$T/$scanner_path" ]
  run bash -c 'cd "$1" && bash -c "$2"' -- "$T" "$step"
  [ "$status" -eq 0 ]
  [[ "$output" == *"ok: no agent-tooling references in commit messages"* ]]

  git -C "$T" checkout -q pr
  commit pr-quote "see https://$SITE/x"
  git -C "$T" checkout -q --detach "$RANGE_BASE"
  run bash -c 'cd "$1" && bash -c "$2"' -- "$T" "$step"
  [ "$status" -eq 1 ]
  [[ "$output" == *"see https://$SITE/x"* ]]
}
