#!/usr/bin/env bash
# Syntax-check the e2e-canary plays and the role's parse-only fixture.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ANSIBLE_DIR="$(cd "$HERE/../../.." && pwd)"
cd "$ANSIBLE_DIR"
ansible-playbook --syntax-check e2e-canary-deploy.yml -e target=localhost >/dev/null
ansible-playbook --syntax-check e2e-canary-secret-deploy.yml -e target=localhost >/dev/null
ansible-playbook --syntax-check e2e-canary-secret.yml -e target=localhost >/dev/null
ansible-playbook --syntax-check "$HERE/playbook.yml" >/dev/null
echo "e2e-canary: syntax ok"

# The reference-only fixture executes production tasks without host writes.
ansible-playbook -i localhost, -c local "$HERE/pull-ref.yml"
