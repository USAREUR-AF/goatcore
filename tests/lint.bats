#!/usr/bin/env bats
# Linting tests — verify all scripts and configs pass static analysis

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"

# This tree may not carry the code a test reads (a public goatcore checkout
# does not carry a goatnet's own scripts); such a test is skipped by name,
# and runs wherever that code is present.
setup() {
  case "$BATS_TEST_DESCRIPTION" in
    *deploy.sh*) [ -f "$REPO_ROOT/deploy.sh" ] || skip "deploy.sh is not in this tree" ;;
    *) [ -d "$REPO_ROOT/provisioning" ] || skip "provisioning/ is not in this tree" ;;
  esac
  # Lint tools are optional outside CI but required when CI is set.
  if ! command -v shellcheck >/dev/null 2>&1; then
    if [ -n "${CI:-}" ]; then
      echo "missing required lint tool: shellcheck (required in CI)" >&2
      return 1
    else
      skip "shellcheck not installed"
    fi
  fi

  if ! command -v yamllint >/dev/null 2>&1; then
    if [ -n "${CI:-}" ]; then
      echo "missing required lint tool: yamllint (required in CI)" >&2
      return 1
    else
      skip "yamllint not installed"
    fi
  fi
}

# ── ShellCheck ──────────────────────────────────────────

@test "shellcheck: deploy.sh" {
  # SC1091: can't follow .env (not present at lint time)
  # SC2016: single-quoted echo of template vars is intentional
  shellcheck -x -e SC1091,SC2016 "$REPO_ROOT/deploy.sh"
}

@test "shellcheck: provisioning/build.sh" {
  # SC1091: can't follow .env (not present at lint time)
  shellcheck -x -e SC1091 "$REPO_ROOT/provisioning/build.sh"
}

@test "shellcheck: embedded provision.sh in user-data template" {
  # Extract the embedded shell script from user-data.yaml.tpl and lint it
  sed -n '/^      #!/,/^runcmd:/p' "$REPO_ROOT/provisioning/cloud-init/user-data.yaml.tpl" \
    | sed 's/^      //' \
    | sed '$d' \
    > "$BATS_TEST_TMPDIR/provision.sh"
  # SC2154: referenced but not assigned (template vars are fine)
  # SC2086: word splitting (template vars again)
  # SC2016: single-quoted sed patterns are intentional (literal replacement)
  shellcheck -e SC2154,SC2086,SC2016 "$BATS_TEST_TMPDIR/provision.sh"
}

# ── YAML lint ───────────────────────────────────────────

@test "yamllint: network-config.yaml" {
  yamllint -c "$REPO_ROOT/.yamllint.yml" "$REPO_ROOT/provisioning/cloud-init/network-config.yaml"
}

@test "yamllint: user-data template is valid YAML after rendering" {
  # Render with dummy values and validate
  NODE_ROLE=client \
  NODE_HOSTNAME=test-node \
  NETBIRD_DOMAIN=test.example.com \
  NETBIRD_SETUP_KEY=testkey123 \
  CF_API_TOKEN=testtoken123 \
  SSH_PUBLIC_KEY="ssh-ed25519 AAAA test@test" \
  HASHED_PASSWORD='$6$rounds=4096$salt$hash' \
    envsubst < "$REPO_ROOT/provisioning/cloud-init/user-data.yaml.tpl" \
    > "$BATS_TEST_TMPDIR/user-data.yaml"
  yamllint -c "$REPO_ROOT/.yamllint.yml" "$BATS_TEST_TMPDIR/user-data.yaml"
}

@test "yamllint: meta-data template is valid YAML after rendering" {
  NODE_HOSTNAME=test-node \
    envsubst < "$REPO_ROOT/provisioning/cloud-init/meta-data.yaml.tpl" \
    > "$BATS_TEST_TMPDIR/meta-data.yaml"
  yamllint -c "$REPO_ROOT/.yamllint.yml" "$BATS_TEST_TMPDIR/meta-data.yaml"
}
