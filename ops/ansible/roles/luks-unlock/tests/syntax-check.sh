#!/usr/bin/env bash
# Parse-only YAML + Jinja2 check for the luks-unlock role. Cheap; safe in CI.
#
# Coverage level: structural (catches YAML/Jinja2 errors and the F-075 class
# of register-name collision when paired with the workspace lint at
# ops/ansible/tests/check-register-uniqueness.sh). Does NOT exercise the
# actual LUKS keyslot operations — those need a real block device + TPM /
# FIDO2 hardware and are covered by ops/scripts/cold-boot-drill.sh.

set -euo pipefail

cd "$(dirname "$0")"

if ! command -v ansible-playbook >/dev/null 2>&1; then
  echo "[luks-unlock/syntax-check] SKIP (ansible-playbook not on PATH)"
  exit 0
fi

echo "[luks-unlock/syntax-check] ansible-playbook --syntax-check on parse-only fixture"
ansible-playbook -i 'localhost,' -c local playbook.yml --syntax-check

echo "[luks-unlock/syntax-check] offline test: clevis-bind-one-line.yml"
ansible-playbook -i 'localhost,' -c local clevis-bind-one-line.yml

echo "[luks-unlock/syntax-check] offline test: crypttab-keep-installer.yml"
ansible-playbook -i 'localhost,' -c local crypttab-keep-installer.yml

echo "[luks-unlock/syntax-check] register-uniqueness lint (workspace)"
bash ../../../tests/check-register-uniqueness.sh

echo "[luks-unlock/syntax-check] PASS"
