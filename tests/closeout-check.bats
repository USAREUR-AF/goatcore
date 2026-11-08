#!/usr/bin/env bats
# closeout-check.sh must finish and say so when a network call stalls, and
# its key scan must not read build trees (#1356).

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
SCRIPT="$REPO_ROOT/ops/scripts/closeout-check.sh"
REAL_GIT="$(command -v git)"
# Assembled so the literal header never sits in the tree for the secret scans.
PEM_HEAD="-----BEGIN PRIV""ATE KEY-----"

setup() {
  export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
  W="$BATS_TEST_TMPDIR"
  git init -q --bare "$W/origin.git"
  git init -q -b main "$W/repo"
  git -C "$W/repo" -c user.name=t -c user.email=t@t commit -q --allow-empty -m init
  git -C "$W/repo" remote add origin "$W/origin.git"
  git -C "$W/repo" push -q origin main
  mkdir -p "$W/bin" "$W/scratch"
  export TMPDIR="$W/scratch" CLOSEOUT_NET_TIMEOUT=1
  export PATH="$W/bin:$PATH"
}

stub_gh() {
  printf '#!/usr/bin/env bash\n%s\n' "$1" > "$W/bin/gh"
  chmod +x "$W/bin/gh"
}

run_check() {
  cd "$W/repo" && run "$SCRIPT" "$@"
}

# A PATH with the script's tools but no timeout or gtimeout, as on a stock
# macOS. The gh and git stubs in $W/bin are copied in; git otherwise links to
# the real binary.
bare_path() {
  mkdir -p "$W/bare"
  for t in bash awk grep wc tr tail head sed find xargs sleep pkill; do
    p=$(command -v "$t") && ln -s "$p" "$W/bare/$t"
  done
  if [ -e "$W/bin/git" ]; then cp "$W/bin/git" "$W/bare/git"; else ln -s "$REAL_GIT" "$W/bare/git"; fi
  cp "$W/bin/gh" "$W/bare/gh"
}

run_check_bare() {
  cd "$W/repo" && run /usr/bin/env PATH="$W/bare" "$SCRIPT"
}

@test "a stalled gh is bounded and reported, not a pass" {
  stub_gh 'sleep 30'
  SECONDS=0
  run_check
  [ "$SECONDS" -lt 15 ]
  [ "$status" -eq 1 ]
  [[ "$output" == *"gh timed out after 1s"* ]]
}

@test "a failing gh is reported, not read as no open PRs" {
  stub_gh 'exit 4'
  run_check
  [ "$status" -eq 1 ]
  [[ "$output" == *"gh exited 4"* ]]
  [[ "$output" != *"none open under"* ]]
}

@test "a stalled ls-remote is bounded and reported" {
  stub_gh 'exit 0'
  printf '#!/usr/bin/env bash\nfor a in "$@"; do [ "$a" = ls-remote ] && exec sleep 30; done\nexec %s "$@"\n' \
    "$REAL_GIT" > "$W/bin/git"
  chmod +x "$W/bin/git"
  SECONDS=0
  run_check
  [ "$SECONDS" -lt 15 ]
  [ "$status" -eq 1 ]
  [[ "$output" == *"git ls-remote timed out after 1s"* ]]
}

@test "a failing ls-remote is reported, not read as no track branches" {
  stub_gh 'exit 0'
  printf '#!/usr/bin/env bash\nfor a in "$@"; do [ "$a" = ls-remote ] && exit 128; done\nexec %s "$@"\n' \
    "$REAL_GIT" > "$W/bin/git"
  chmod +x "$W/bin/git"
  run_check
  [ "$status" -eq 1 ]
  [[ "$output" == *"git ls-remote exited 128"* ]]
}

@test "a failing gh for the track's PR is reported, not read as no PR opened" {
  stub_gh 'case "$*" in *--head*) exit 4;; "api user"*) echo me;; esac; exit 0'
  run_check --track demo
  [ "$status" -eq 1 ]
  [[ "$output" == *"gh exited 4 — track/demo PR state NOT checked"* ]]
  [[ "$output" != *"no PR was opened"* ]]
}

@test "without timeout or gtimeout, the watchdog bounds a stalled gh" {
  stub_gh 'sleep 30'
  bare_path
  [ ! -e "$W/bare/timeout" ] && [ ! -e "$W/bare/gtimeout" ]
  SECONDS=0
  run_check_bare
  [ "$SECONDS" -lt 15 ]
  [ "$status" -eq 1 ]
  [[ "$output" == *"a shell watchdog bounds the network calls"* ]]
  [[ "$output" == *"gh timed out after 1s"* ]]
}

@test "without timeout or gtimeout, the watchdog bounds a stalled ls-remote" {
  stub_gh 'exit 0'
  printf '#!/usr/bin/env bash\nfor a in "$@"; do [ "$a" = ls-remote ] && exec sleep 30; done\nexec %s "$@"\n' \
    "$REAL_GIT" > "$W/bin/git"
  chmod +x "$W/bin/git"
  bare_path
  SECONDS=0
  run_check_bare
  [ "$SECONDS" -lt 15 ]
  [ "$status" -eq 1 ]
  [[ "$output" == *"git ls-remote timed out after 1s"* ]]
}

@test "the watchdog reports the timeout on every run, not intermittently (race guard, #1529)" {
  # The pre-#1529 watchdog decided timeout-vs-not on a kill/wait exit-code race
  # that flaked under load, misreporting a real timeout as the call's SIGTERM
  # exit (143). Repeat the stalled-gh bound: each run must report the timeout,
  # or the race is back.
  stub_gh 'sleep 30'
  bare_path
  for _ in 1 2 3; do
    run_check_bare
    [ "$status" -eq 1 ]
    [[ "$output" == *"gh timed out after 1s"* ]]
  done
}

@test "without timeout or gtimeout, a fast call is not reported as timed out" {
  stub_gh 'case "$*" in "api user"*) echo me;; esac; exit 0'
  bare_path
  run_check_bare
  [[ "$output" == *"none open under me"* ]]
  [[ "$output" != *"timed out"* ]]
}

@test "the key scan finds a key and skips build trees" {
  stub_gh 'exit 0'
  mkdir -p "$W/scratch/s1" "$W/scratch/s2/target/debug"
  printf '%s\nx\n' "$PEM_HEAD" > "$W/scratch/s1/k.pem"
  printf '%s\nx\n' "$PEM_HEAD" > "$W/scratch/s2/target/debug/fixture.pem"
  run_check
  [[ "$output" == *"s1/k.pem"* ]]
  [[ "$output" != *"fixture.pem"* ]]
  [[ "$output" == *"private key material in the scratchpad"* ]]
}

@test "a clean scratch dir reports no key material" {
  stub_gh 'exit 0'
  run_check
  [[ "$output" == *"none found under $W/scratch"* ]]
}
