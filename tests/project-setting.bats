#!/usr/bin/env bats
# Behavior tests for ops/scripts/project-setting.sh: the shell drivers read their
# project's names (AWS profile, state directory, source repository, image
# namespace) from ops/goatbuilder/goatbuilder.yaml through it.
#
# The tests pin:
#   - a declared value is printed as written, quotes and trailing comment removed,
#   - an absent file, section or key prints nothing and exits 0 (the caller
#     writes its own fallback),
#   - a key of the same name in another section is not read,
#   - trunk-remote names the remote whose URL is repo.source, whatever the
#     remote is called and however the URL is spelled.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
SETTING="$REPO_ROOT/ops/scripts/project-setting.sh"

setup() {
  T="$BATS_TEST_TMPDIR/checkout"
  mkdir -p "$T/ops/goatbuilder"
  cat > "$T/ops/goatbuilder/goatbuilder.yaml" <<'YAML'
# a comment
schema: goatbuilder/v1
repo:
  source: Example-Org/example-repo
  image_namespace: "ghcr.io/example-org"   # quoted, with a comment
deploy:
  aws_profile: example-profile
  state_dir: ~/.example/state
other:
  aws_profile: not-this-one
YAML
}

@test "project-setting: prints a declared value" {
  run "$SETTING" --repo "$T" deploy aws_profile
  [ "$status" -eq 0 ]
  [ "$output" = "example-profile" ]
  run "$SETTING" --repo "$T" deploy state_dir
  [ "$output" = "~/.example/state" ]
  run "$SETTING" --repo "$T" repo source
  [ "$output" = "Example-Org/example-repo" ]
}

@test "project-setting: quotes and a trailing comment are not part of the value" {
  run "$SETTING" --repo "$T" repo image_namespace
  [ "$status" -eq 0 ]
  [ "$output" = "ghcr.io/example-org" ]
}

@test "project-setting: an absent key, section or file prints nothing and exits 0" {
  run "$SETTING" --repo "$T" deploy ssh_user
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  run "$SETTING" --repo "$T" secrets store
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  run "$SETTING" --repo "$BATS_TEST_TMPDIR" deploy aws_profile
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "project-setting: a wrong argument count is a usage error" {
  run "$SETTING" --repo "$T" deploy
  [ "$status" -eq 2 ]
}

@test "project-setting: trunk-remote is the remote whose URL is repo.source" {
  git -C "$T" init -q
  git -C "$T" remote add origin "https://git.example/someone/a-fork.git"
  git -C "$T" remote add upstream "ssh://git@git.example/example-org/example-repo.git"
  run "$SETTING" --repo "$T" trunk-remote
  [ "$status" -eq 0 ]
  [ "$output" = "upstream" ]
}

@test "project-setting: trunk-remote reads the scp form and prints nothing when no remote matches" {
  git -C "$T" init -q
  git -C "$T" remote add origin "git@git.example:Example-Org/example-repo"
  run "$SETTING" --repo "$T" trunk-remote
  [ "$output" = "origin" ]
  git -C "$T" remote set-url origin "https://git.example/someone/example-repo-mirror.git"
  run "$SETTING" --repo "$T" trunk-remote
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "project-setting: ca-audit-log is ca-audit.jsonl beside the state directory" {
  run "$SETTING" --repo "$T" ca-audit-log
  [ "$status" -eq 0 ]
  [ "$output" = "~/.example/ca-audit.jsonl" ]
  run "$SETTING" --repo "$BATS_TEST_TMPDIR/absent" ca-audit-log
  [ "$status" -eq 0 ]
  [ "$output" = "~/.goat/ca-audit.jsonl" ]
}

@test "project-setting: var prints one plain line of group_vars/all.yml and nothing for a template" {
  mkdir -p "$T/ops/ansible/group_vars"
  cat > "$T/ops/ansible/group_vars/all.yml" <<'YAML'
plain_name: "EXAMPLE_VALUE"   # quoted, with a comment
bare_name: two words
templated: "{{ lookup('env', 'HOME') }}/x"
plain_name_longer: other
YAML
  run "$SETTING" --repo "$T" var plain_name
  [ "$status" -eq 0 ]
  [ "$output" = "EXAMPLE_VALUE" ]
  run "$SETTING" --repo "$T" var bare_name
  [ "$output" = "two words" ]
  run "$SETTING" --repo "$T" var templated
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  run "$SETTING" --repo "$T" var absent_name
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  run "$SETTING" --repo "$BATS_TEST_TMPDIR/absent" var plain_name
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}
