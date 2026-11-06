#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
if ! command -v ansible-playbook >/dev/null 2>&1; then
  echo "[goat-peering-forwarder/syntax-check] SKIP (ansible-playbook not on PATH)"
  exit 0
fi
echo "[goat-peering-forwarder/syntax-check] ansible-playbook --syntax-check"
ansible-playbook -i 'localhost,' -c local playbook.yml --syntax-check
echo "[goat-peering-forwarder/syntax-check] PASS"
