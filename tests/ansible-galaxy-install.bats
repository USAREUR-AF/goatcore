#!/usr/bin/env bats

# Tests for ops/ci/ansible-galaxy-install.sh, with a stand-in ansible-galaxy
# on PATH that records its calls.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
SUBJECT="$REPO_ROOT/ops/ci/ansible-galaxy-install.sh"

setup() {
  bin="$BATS_TEST_TMPDIR/bin"
  mkdir -p "$bin"
  export CALLS="$BATS_TEST_TMPDIR/calls" STATE="$BATS_TEST_TMPDIR/state"
  : >"$CALLS"
  echo 0 >"$STATE.install"
  echo 0 >"$STATE.verify"
  # INSTALL_HANG: the first install call hangs (an attempt cut off by its bound).
  # VERIFY_FAILS: how many verify calls fail before one passes.
  cat >"$bin/ansible-galaxy" <<'SH'
#!/bin/sh
echo "$*" >>"$CALLS"
case "$2" in
install)
  n=$(cat "$STATE.install"); echo $((n + 1)) >"$STATE.install"
  if [ "$n" -eq 0 ] && [ -n "${INSTALL_HANG:-}" ]; then sleep 30; fi
  echo "installed"
  ;;
verify)
  n=$(cat "$STATE.verify"); echo $((n + 1)) >"$STATE.verify"
  if [ "$n" -lt "${VERIFY_FAILS:-0}" ]; then echo "plugins/modules/modprobe.py is missing"; exit 1; fi
  echo "verified"
  ;;
esac
SH
  chmod 755 "$bin/ansible-galaxy"
  export PATH="$bin:$PATH" GALAXY_ATTEMPT_SECONDS=1
}

@test "a clean install is verified and passes" {
  run "${BASH_UNDER_TEST:-/bin/bash}" "$SUBJECT" -r requirements.yml
  [ "$status" -eq 0 ]
  [[ "$output" == *"every collection matches its manifest"* ]]
  [ "$(grep -c 'collection install' "$CALLS")" -eq 1 ]
  grep -q 'collection verify --offline -r requirements.yml' "$CALLS"
}

@test "an attempt cut off by its bound is followed by a forced install" {
  INSTALL_HANG=1 run "${BASH_UNDER_TEST:-/bin/bash}" "$SUBJECT" -r requirements.yml
  [ "$status" -eq 0 ]
  [ "$(sed -n 1p "$CALLS")" = "collection install --timeout 60 -r requirements.yml" ]
  [ "$(sed -n 2p "$CALLS")" = "collection install --timeout 60 --force-with-deps -r requirements.yml" ]
}

@test "an install that does not match its manifest is installed again, forced" {
  VERIFY_FAILS=1 run "${BASH_UNDER_TEST:-/bin/bash}" "$SUBJECT" -r requirements.yml
  [ "$status" -eq 0 ]
  [[ "$output" == *"does not match its manifest"* ]]
  grep -q -- '--force-with-deps' "$CALLS"
}

@test "an install that never verifies fails and says so" {
  VERIFY_FAILS=9 run "${BASH_UNDER_TEST:-/bin/bash}" "$SUBJECT" -r requirements.yml
  [ "$status" -eq 1 ]
  [[ "$output" == *"does not match its manifest (ansible-galaxy collection verify --offline"* ]]
}
