#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
if ! command -v ansible-playbook >/dev/null 2>&1; then
  echo "[release-file/syntax-check] SKIP (ansible-playbook not on PATH)"
  exit 0
fi
echo "[release-file/syntax-check] ansible-playbook --syntax-check"
ANSIBLE_ROLES_PATH="$(cd ../.. && pwd)" ansible-playbook -i 'localhost,' -c local playbook.yml --syntax-check
# Not only parse: install, reinstall, roll back and each refusal, run on
# localhost against scratch directories.
bash ./local-run.sh
echo "[release-file/syntax-check] PASS"
