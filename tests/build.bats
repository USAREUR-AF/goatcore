#!/usr/bin/env bats
# Tests for provisioning/build.sh — argument parsing, template rendering, output

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
BUILD_SH="$REPO_ROOT/provisioning/build.sh"

# This tree may not carry the code a test reads (a public goatcore checkout
# does not carry a goatnet's own scripts); such a test is skipped by name,
# and runs wherever that code is present.
setup() {
  case "$BATS_TEST_DESCRIPTION" in
    build.sh:*|cloud-init:*|"security: setup-key"*|"security: cf-api-token"*|"security: ssh password"*)
      [ -f "$BUILD_SH" ] || skip "provisioning/ is not in this tree" ;;
  esac
  # Create a temporary .env for tests
  export TEST_ENV="$BATS_TEST_TMPDIR/.env"
  cat > "$TEST_ENV" <<'EOF'
NETHOST_HOST=100.127.1.23
NETHOST_USER=goat
NETHOST_PASS=TestPass123
NETBIRD_DOMAIN=test.example.com
CF_API_TOKEN=test_token_abc123
EOF
}

# ── Argument validation ─────────────────────────────────

@test "build.sh: fails without --role" {
  run "$BUILD_SH" --platform x86_64 --hostname test-node
  [ "$status" -ne 0 ]
  [[ "$output" == *"--role required"* ]]
}

@test "build.sh: fails without --platform" {
  run "$BUILD_SH" --role client --hostname test-node
  [ "$status" -ne 0 ]
  [[ "$output" == *"--platform required"* ]]
}

@test "build.sh: fails without --hostname" {
  run "$BUILD_SH" --role client --platform x86_64
  [ "$status" -ne 0 ]
  [[ "$output" == *"--hostname required"* ]]
}

@test "build.sh: rejects invalid role" {
  run "$BUILD_SH" --role invalid --platform x86_64 --hostname test-node
  [ "$status" -ne 0 ]
  [[ "$output" == *"must be infrastructure or client"* ]]
}

@test "build.sh: rejects invalid platform" {
  run "$BUILD_SH" --role client --platform mips --hostname test-node
  [ "$status" -ne 0 ]
  [[ "$output" == *"must be x86_64 or arm64"* ]]
}

@test "build.sh: --help prints usage" {
  run "$BUILD_SH" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"--role"* ]]
  [[ "$output" == *"--platform"* ]]
}

# ── Template rendering ──────────────────────────────────

@test "cloud-init: user-data renders with all variables substituted" {
  rendered=$(
    NODE_ROLE=client \
    NODE_HOSTNAME=sensor-01 \
    NETBIRD_DOMAIN=test.example.com \
    NETBIRD_SETUP_KEY=nbsk_test123 \
    CF_API_TOKEN=cf_test_token \
    SSH_PUBLIC_KEY="ssh-ed25519 AAAA test@test" \
    HASHED_PASSWORD='$6$test$hash' \
      envsubst < "$REPO_ROOT/provisioning/cloud-init/user-data.yaml.tpl"
  )

  # No unsubstituted ${VAR} patterns should remain
  if echo "$rendered" | grep -qE '\$\{[A-Z_]+\}'; then
    echo "Unsubstituted variables found:"
    echo "$rendered" | grep -oE '\$\{[A-Z_]+\}'
    return 1
  fi

  # Key values should be present
  [[ "$rendered" == *"sensor-01"* ]]
  [[ "$rendered" == *"test.example.com"* ]]
  [[ "$rendered" == *"nbsk_test123"* ]]
  [[ "$rendered" == *"ssh-ed25519"* ]]
}

@test "cloud-init: user-data contains #cloud-config header" {
  head -1 "$REPO_ROOT/provisioning/cloud-init/user-data.yaml.tpl" | grep -q '#cloud-config'
}

@test "cloud-init: infrastructure role opens ports 443, 3478" {
  grep -q '443/tcp' "$REPO_ROOT/provisioning/cloud-init/user-data.yaml.tpl"
  grep -q '3478/udp' "$REPO_ROOT/provisioning/cloud-init/user-data.yaml.tpl"
}

@test "cloud-init: client role does NOT open server ports" {
  # The UFW section should only open 443/3478 for infrastructure role
  # Extract the firewall section and verify it's conditional
  grep -B2 '443/tcp' "$REPO_ROOT/provisioning/cloud-init/user-data.yaml.tpl" | grep -q 'infrastructure'
}

@test "cloud-init: provision.sh is idempotent (has guards)" {
  tpl="$REPO_ROOT/provisioning/cloud-init/user-data.yaml.tpl"
  # Docker install has a guard
  grep -q 'command -v docker' "$tpl"
  # Netbird infrastructure has a guard
  grep -q 'docker-compose.yml exists' "$tpl" || grep -q 'docker-compose.yml' "$tpl"
  # Netbird client has a guard
  grep -q 'already connected' "$tpl" || grep -q 'Connected' "$tpl"
}

@test "cloud-init: meta-data renders hostname" {
  rendered=$(NODE_HOSTNAME=nethost-01 envsubst < "$REPO_ROOT/provisioning/cloud-init/meta-data.yaml.tpl")
  [[ "$rendered" == *"nethost-01"* ]]
}

@test "cloud-init: network-config uses DHCP" {
  grep -q 'dhcp4: true' "$REPO_ROOT/provisioning/cloud-init/network-config.yaml"
}

# ── Security checks ────────────────────────────────────

@test "security: .env is in .gitignore" {
  grep -q '^\.env$' "$REPO_ROOT/.gitignore"
}

@test "security: .build/ is in .gitignore" {
  grep -q '\.build' "$REPO_ROOT/.gitignore"
}

@test "security: no hardcoded passwords in tracked shell scripts" {
  # Check that no actual passwords appear in scripts (only template vars)
  for f in "$REPO_ROOT/deploy.sh" "$REPO_ROOT/provisioning/build.sh"; do
    if grep -qiE "(password|passwd)\s*=\s*['\"][^$]" "$f" 2>/dev/null; then
      echo "Potential hardcoded password in $f"
      return 1
    fi
  done
}

# Read repository-relative, NUL-delimited paths. Return failure on any match.
scan_for_credentials() {
  local f found=0
  while IFS= read -r -d '' f; do
    [ -f "$REPO_ROOT/$f" ] || continue
    # Skip .env.example (has placeholder values)
    [[ "$f" == *".env.example" ]] && continue
    # Skip test files (they reference patterns but don't contain real secrets)
    [[ "$f" == tests/* ]] && continue
    [[ "$f" == *_test.go ]] && continue

    # Check for real credential values (actual passwords, API tokens, private keys)
    if grep -n "$@" -E '(Ma1tFurnac3|cfut_[A-Za-z0-9]|AKIA[A-Z0-9]{16}|-----BEGIN (RSA |EC )?PRIVATE KEY)' "$REPO_ROOT/$f" 2>/dev/null; then
      echo "CREDENTIAL LEAK in $f"
      found=1
    fi
  done
  return "$found"
}

@test "security: no credentials in any tracked file" {
  local found=0
  # Keep binary matching for tracked files.
  scan_for_credentials < <(cd "$REPO_ROOT" && git ls-files -z --cached) || found=1
  # Skip untracked binaries: crypto build artifacts contain PEM header constants.
  # Untracked text files still need scanning before they are committed.
  scan_for_credentials -I < <(cd "$REPO_ROOT" && git ls-files -z --others --exclude-standard) || found=1
  [ "$found" -eq 0 ]
}

credential_scan_fixture() {
  scan_root="$BATS_TEST_TMPDIR/credential-repo"
  mkdir -p "$scan_root/tests"
  cp "$BATS_TEST_DIRNAME/build.bats" "$scan_root/tests/build.bats"
  git init -q "$scan_root"
  # Keep user ignore rules from hiding the credential fixtures.
  git -C "$scan_root" config core.excludesFile /dev/null
}

@test "credential scan detects quoted and newline filenames before and after staging" {
  credential_scan_fixture
  local name=$'fixture-\n"key.pem'
  printf '%s\n' '-----BEGIN PRIVATE KEY-----' > "$scan_root/$name"

  run bats "$scan_root/tests/build.bats" -f 'no credentials'
  [ "$status" -ne 0 ]
  [[ "$output" == *"CREDENTIAL LEAK"* ]]

  git -C "$scan_root" add -- "$name"
  run bats "$scan_root/tests/build.bats" -f 'no credentials'
  [ "$status" -ne 0 ]
  [[ "$output" == *"CREDENTIAL LEAK"* ]]
}

@test "credential scan skips an untracked binary but detects it after staging" {
  credential_scan_fixture
  printf 'x\000-----BEGIN PRIVATE KEY-----\000y' > "$scan_root/fixture.bin"

  run bats "$scan_root/tests/build.bats" -f 'no credentials'
  [ "$status" -eq 0 ]

  git -C "$scan_root" add -- fixture.bin
  run bats "$scan_root/tests/build.bats" -f 'no credentials'
  [ "$status" -ne 0 ]
  [[ "$output" == *"CREDENTIAL LEAK"* ]]
}

@test "security: setup-key file has restricted permissions in cloud-init" {
  grep -A1 'setup-key' "$REPO_ROOT/provisioning/cloud-init/user-data.yaml.tpl" | grep -q '0600'
}

@test "security: cf-api-token file has restricted permissions in cloud-init" {
  grep -A1 'cf-api-token' "$REPO_ROOT/provisioning/cloud-init/user-data.yaml.tpl" | grep -q '0600'
}

@test "security: ssh password auth is disabled" {
  grep -q 'ssh_pwauth: false' "$REPO_ROOT/provisioning/cloud-init/user-data.yaml.tpl"
}
