#!/usr/bin/env bash
# Runs the taxonomy-default test playbook on localhost; needs ansible-playbook.
set -euo pipefail
cd "$(dirname "$0")"
command -v ansible-playbook >/dev/null 2>&1 || { echo "[taxonomy-default] SKIP (no ansible-playbook)"; exit 0; }
ANSIBLE_NOCOLOR=1 ansible-playbook -i 'localhost,' playbook.yml > /tmp/.taxonomy-default.$$ 2>&1 \
  && { echo "[taxonomy-default] PASS"; rm -f /tmp/.taxonomy-default.$$; } \
  || { cat /tmp/.taxonomy-default.$$; rm -f /tmp/.taxonomy-default.$$; echo "[taxonomy-default] FAIL"; exit 1; }
