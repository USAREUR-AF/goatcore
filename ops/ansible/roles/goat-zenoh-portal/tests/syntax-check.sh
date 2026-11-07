#!/usr/bin/env bash
# Parse-only smoke for the goat-zenoh-portal role.
#
# Behavioral coverage lives in ops/goat-zenoh-portal/ (`go test ./...`)
# — this script guarantees the role parses + templates render.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ANSIBLE_DIR="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

cd "$ANSIBLE_DIR"

# Point Ansible at the project roles/ directory so the test fixture
# can reference the role by short name.
ANSIBLE_ROLES_PATH="$ANSIBLE_DIR/roles" \
ansible-playbook \
  -i localhost, \
  --connection local \
  --syntax-check \
  "$SCRIPT_DIR/playbook.yml"

# Behavioural: the --org support detection on fixture usage texts (localhost
# only, no host, no docker).
ansible-playbook \
  -i localhost, \
  --connection local \
  "$SCRIPT_DIR/cert-sign-org-support.yml"

echo "goat-zenoh-portal: syntax-check OK"

# The release-set registry a goatnet declares in group_vars must render to the
# golden file the portal's Go test parses.
bash "$SCRIPT_DIR/render-release-sets.sh"

# An unset portal_release_sets installs the example registry only on the example
# goatnet or on request; anywhere else the play fails.
bash "$SCRIPT_DIR/release-sets-guard.sh"
