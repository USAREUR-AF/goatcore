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
