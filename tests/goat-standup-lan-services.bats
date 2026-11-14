#!/usr/bin/env bats
# Phase 10 (LAN service surface) behavior tests for goat-standup.sh (#392).
# (Phase number per the renumbering, #2152 — this was phase 12
# before that; the file/tag bug this tests is unaffected by the renumber.)
#
# Every operational command is stubbed; these tests touch no real host and
# make no network or AWS call. goat-standup.sh is copied fresh from the repo
# into an isolated fixture tree per test, so a change to the real script is
# what gets exercised, not a frozen copy of it.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
SUBJECT="$REPO_ROOT/ops/scripts/goat-standup.sh"
SITE=testlan
GROUP="${SITE}_hosts"

setup() {
  FIXTURE="$BATS_TEST_TMPDIR/repo"
  BIN="$BATS_TEST_TMPDIR/bin"
  # goat-standup.sh keeps its state (timings, logs) under ~/.goat/state unless the
  # checkout declares another; give the run a home of its own so a test run never
  # writes into the real one. Python packages installed for the user stay found.
  export PYTHONUSERBASE="${PYTHONUSERBASE:-$HOME/.local}"
  export HOME="$BATS_TEST_TMPDIR/home"
  mkdir -p "$HOME"
  mkdir -p "$FIXTURE/ops/scripts" "$FIXTURE/ops/ansible" "$BIN"
  cp "$SUBJECT" "$FIXTURE/ops/scripts/goat-standup.sh"
  chmod +x "$FIXTURE/ops/scripts/goat-standup.sh"
  # Every phase runs through the shared runner, which refuses an empty
  # $(printenv NAME); it is real here, not stubbed, so that refusal is tested too.
  cp "$REPO_ROOT/ops/scripts/standup-phase-run" "$FIXTURE/ops/scripts/standup-phase-run"
  chmod +x "$FIXTURE/ops/scripts/standup-phase-run"
  # goat-standup.sh reads deploy.state_dir and deploy.aws_profile through
  # project-setting.sh and stops the run when that read fails, so it is real here.
  cp "$REPO_ROOT/ops/scripts/project-setting.sh" "$FIXTURE/ops/scripts/project-setting.sh"
  chmod +x "$FIXTURE/ops/scripts/project-setting.sh"

  # fresh-source.sh (contract C3): stubbed. This test is about phase 12's own
  # command and exit-code handling, not trunk staleness.
  cat > "$FIXTURE/ops/scripts/fresh-source.sh" <<'SH'
#!/usr/bin/env bash
echo ""
SH
  chmod +x "$FIXTURE/ops/scripts/fresh-source.sh"

  # with-aws: skip the real AWS profile wiring, just run the wrapped command.
  cat > "$FIXTURE/ops/scripts/with-aws" <<'SH'
#!/usr/bin/env bash
shift 2  # drop "<profile> --"
exec "$@"
SH
  chmod +x "$FIXTURE/ops/scripts/with-aws"

  # ansible-preflight.sh: forward straight to whatever ansible-playbook is on
  # PATH. The wrapper's own staleness/arg guards are a different file's tests;
  # this one is about goat-standup.sh step 12 itself.
  cat > "$FIXTURE/ops/scripts/ansible-preflight.sh" <<'SH'
#!/usr/bin/env bash
exec ansible-playbook "$@"
SH
  chmod +x "$FIXTURE/ops/scripts/ansible-preflight.sh"

  # The per-site file step 12 used to require (#392) — present, so a run of
  # the OLD code reaches the ansible call instead of tripping the "missing
  # file" echo branch. Without this, the old code's exit-0 would just be that
  # branch, not the `|| echo` masking an ansible failure this test is after.
  : > "$FIXTURE/ops/ansible/$SITE-lan-services.yml"
  : > "$FIXTURE/ops/ansible/site-services.yml"
}

# A stub ansible-playbook that always fails, recording every invocation.
stub_ansible_playbook_fails() {
  CALLS="$BATS_TEST_TMPDIR/ansible-playbook.calls"
  : > "$CALLS"
  cat > "$BIN/ansible-playbook" <<SH
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$CALLS"
exit 7
SH
  chmod +x "$BIN/ansible-playbook"
}

# Synthetic values for the secrets phase 10 reads; a test that wants one empty
# overrides it after these.
run_standup() {
  cd "$FIXTURE" || return
  run env PATH="$BIN:$PATH" GOAT_AWS_MODE=none GOAT_SECRET_STORE=env \
    TESTLAN_PG_PASS=synthetic-pg TESTLAN_NB_ENCKEY=synthetic-key ${STANDUP_ENV:-} \
    "${BASH_UNDER_TEST:-/bin/bash}" ./ops/scripts/goat-standup.sh "$SITE" --from 10 --to 10 "$@"
}

@test "phase 10 (lan): a failed ansible-playbook run makes the script exit non-zero" {
  stub_ansible_playbook_fails
  run_standup

  printf '%s\n' "$output" >&2
  [ -s "$CALLS" ]        # ansible-playbook was actually invoked, not short-circuited
  [ "$status" -ne 0 ]    # a failed play must fail the step, not be swallowed by `|| echo`
}

@test "phase 10 (lan): an empty secret refuses the phase, names it, and never calls ansible" {
  stub_ansible_playbook_fails
  STANDUP_ENV="TESTLAN_PG_PASS=" run_standup

  printf '%s\n' "$output" >&2
  [ ! -s "$CALLS" ]                                # the play never ran
  [ "$status" -ne 0 ]
  [[ "$output" == *"TESTLAN_PG_PASS"* ]]           # the empty name is named
  [[ "$output" == *"refusing to run this phase"* ]]
}

@test "phase 10 (lan): --plan invokes site-services.yml directly, not a per-site filename" {
  run_standup --plan

  [ "$status" -eq 0 ]
  [[ "$output" == *"site-services.yml"* ]]
  [[ "$output" != *"$SITE-lan-services.yml"* ]]
}
