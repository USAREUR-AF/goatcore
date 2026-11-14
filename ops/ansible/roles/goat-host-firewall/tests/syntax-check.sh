#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
if command -v python3 >/dev/null 2>&1 && python3 -c 'import pytest' >/dev/null 2>&1; then
  echo "[goat-host-firewall/syntax-check] render tests"
  python3 -m pytest -q test_render.py
fi
if ! command -v ansible-playbook >/dev/null 2>&1; then
  echo "[goat-host-firewall/syntax-check] SKIP (ansible-playbook not on PATH)"
  exit 0
fi
echo "[goat-host-firewall/syntax-check] ansible-playbook --syntax-check"
ansible-playbook -i 'localhost,' -c local playbook.yml --syntax-check
echo "[goat-host-firewall/syntax-check] PASS"
