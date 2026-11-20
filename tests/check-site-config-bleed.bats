#!/usr/bin/env bats
# ops/ci/check-site-config-bleed.sh against an environment repository's
# group_vars, the way .github/workflows/_site-config-bleed.yml runs it: this
# repository's shared trees, with GV_ROOT naming the overlay's group_vars.
#
# The fixture environment is the example goatnet zagros, with a routable zone
# and a documentation mesh address, so both kinds of protected literal are
# derived.

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  SH="${BASH_UNDER_TEST:-/bin/bash}"
  T="$(mktemp -d)"
  GV="$T/overlay/ops/ansible/group_vars"
  mkdir -p "$GV/zagros_hosts" "$GV/_examples"
  cat > "$GV/zagros_hosts/vars.yml" <<'EOF'
site_dns_zone: zagros.example.org
lan_dns_zone: zagros.lan
mesh_ip_a: "192.0.2.10"
EOF
  # _examples/ is documentation: its values are never protected.
  printf 'site_dns_zone: other.example.com\n' > "$GV/_examples/vars.yml"
}

teardown() {
  rm -rf "$T"
}

# A throwaway repository holding one shared template, so a refusal names a
# file this test wrote.
make_tree() {
  local tree="$T/tree"
  mkdir -p "$tree/ops/ci" "$tree/ops/ansible/roles/demo/templates" "$tree/ops/ansible/roles/demo/defaults"
  cp "$REPO_ROOT/ops/ci/check-site-config-bleed.sh" "$tree/ops/ci/"
  printf '%s\n' "$1" > "$tree/ops/ansible/roles/demo/templates/demo.conf.j2"
  printf 'demo_port: 8080\n' > "$tree/ops/ansible/roles/demo/defaults/main.yml"
  git -C "$tree" init -q
  git -C "$tree" add -A
  git -C "$tree" -c user.name=t -c user.email=t@example.org -c commit.gpgsign=false commit -q -m fixture
  printf '%s' "$tree"
}

@test "derives the overlay's zone apex and mesh address, not _examples/ or a .lan zone" {
  run bash -c "cd '$REPO_ROOT' && GV_ROOT='$GV' '$SH' ops/ci/check-site-config-bleed.sh --list"
  [ "$status" -eq 0 ]
  [[ "$output" == *"derived site literals (2)"* ]]
  [[ "$output" == *"  example.org"* ]]
  [[ "$output" == *"  192.0.2.10"* ]]
  [[ "$output" != *"example.com"* ]]
  [[ "$output" != *"zagros.lan"* ]]
}

@test "a shared template that hard-codes the overlay's zone is refused, naming the line" {
  tree="$(make_tree 'issuer = https://oauth2.zagros.example.org/')"
  run bash -c "cd '$tree' && GV_ROOT='$GV' '$SH' ops/ci/check-site-config-bleed.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"ops/ansible/roles/demo/templates/demo.conf.j2:1:"* ]]
  [[ "$output" == *"site-config bleed"* ]]
}

@test "a shared template that hard-codes the overlay's mesh address is refused" {
  tree="$(make_tree 'peer = 192.0.2.10')"
  run bash -c "cd '$tree' && GV_ROOT='$GV' '$SH' ops/ci/check-site-config-bleed.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"demo.conf.j2:1: peer = 192.0.2.10"* ]]
}

@test "the same value taken from a variable passes" {
  tree="$(make_tree 'issuer = https://oauth2.{{ site_dns_zone }}/')"
  run bash -c "cd '$tree' && GV_ROOT='$GV' '$SH' ops/ci/check-site-config-bleed.sh"
  [ "$status" -eq 0 ]
  [[ "$output" == *"OK: no site-config bleed"* ]]
}

@test "this repository's shared trees pass against the fixture environment" {
  run bash -c "cd '$REPO_ROOT' && GV_ROOT='$GV' '$SH' ops/ci/check-site-config-bleed.sh"
  [ "$status" -eq 0 ]
  [[ "$output" == *"OK: no site-config bleed in shared deploy artifacts (2 derived site literals)."* ]]
}

@test "a group_vars path with no environment refuses as vacuous, naming it" {
  rm -rf "$GV/zagros_hosts"
  run bash -c "cd '$REPO_ROOT' && GV_ROOT='$GV' '$SH' ops/ci/check-site-config-bleed.sh"
  [ "$status" -eq 2 ]
  [[ "$output" == *"derived zero site literals from $GV"* ]]
}
