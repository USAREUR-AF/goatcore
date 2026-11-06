#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
if ! command -v ansible-playbook >/dev/null 2>&1; then
  echo "[goat-get/syntax-check] SKIP"; exit 0
fi
ansible-playbook -i 'localhost,' -c local github-release-access.yml --syntax-check
bash ../../../tests/check-register-uniqueness.sh
echo "[goat-get/syntax-check] PASS"
