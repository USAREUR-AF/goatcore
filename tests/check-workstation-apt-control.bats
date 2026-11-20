#!/usr/bin/env bats

# The apt negative control of ops/tutorials/check-workstation.sh
# (ops/tutorials/lib/apt-fresh-control.sh), against a stand-in apt-get that
# installs only when the package lists are present, as apt does.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"

setup() {
  TMP="$(mktemp -d)"
  LISTS="$TMP/lists"
  CACHE="$TMP/cache"
  mkdir -p "$LISTS/partial" "$CACHE" "$TMP/bin"
  : > "$LISTS/lock"
  # The stand-in installs when a list file other than lock and partial/ is present.
  cat > "$TMP/bin/apt-get" <<STUB
#!/bin/sh
for f in "$LISTS"/*; do
  case "\${f##*/}" in lock|partial|'*') ;; *) exit 0 ;; esac
done
echo "E: Unable to locate package git" >&2
exit 100
STUB
  chmod 0755 "$TMP/bin/apt-get"
  PATH="$TMP/bin:$PATH"
  unset APT_FAILOVER
  # shellcheck source=ops/tutorials/lib/apt-fresh-control.sh
  . "$REPO_ROOT/ops/tutorials/lib/apt-fresh-control.sh"
  # shellcheck disable=SC2329  # called by apt_fresh_control
  fail() { echo "FAIL  $*"; exit 1; }  # as check-workstation.sh does
  # shellcheck disable=SC2329
  pass() { echo "ok    $*"; }
}

teardown() {
  rm -rf "$TMP"
}

with_lists() {
  : > "$LISTS/archive.ubuntu.com_ubuntu_dists_noble_InRelease"
  : > "$CACHE/pkgcache.bin"
}

@test "a fresh image (no package lists) passes the control" {
  run apt_fresh_control "$LISTS" "$CACHE"
  [ "$status" -eq 0 ]
  [[ "$output" == *"negative control: apt install before apt update fails"* ]]
}

@test "without failover, an image that already has package lists fails the control" {
  with_lists
  run apt_fresh_control "$LISTS" "$CACHE"
  [ "$status" -ne 0 ]
  [[ "$output" == *"this image is not fresh"* ]]
  [ -e "$LISTS/archive.ubuntu.com_ubuntu_dists_noble_InRelease" ]
}

@test "after apt-failover's update (APT_FAILOVER=1), the lists are emptied and the control passes" {
  with_lists
  APT_FAILOVER=1 run apt_fresh_control "$LISTS" "$CACHE"
  [ "$status" -eq 0 ]
  [[ "$output" == *"APT_FAILOVER=1"* ]]
  [[ "$output" == *"negative control: apt install before apt update fails"* ]]
  [ ! -e "$LISTS/archive.ubuntu.com_ubuntu_dists_noble_InRelease" ]
  [ ! -e "$CACHE/pkgcache.bin" ]
}

@test "check-workstation.sh runs the control from the library" {
  grep -q 'apt_fresh_control' "$REPO_ROOT/ops/tutorials/check-workstation.sh"
  if grep -q 'apt-get install -y -qq git' "$REPO_ROOT/ops/tutorials/check-workstation.sh"; then false; fi
}
