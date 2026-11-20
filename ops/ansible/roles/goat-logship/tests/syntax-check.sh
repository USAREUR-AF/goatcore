#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
if ! command -v ansible-playbook >/dev/null 2>&1; then
  echo "[goat-logship/syntax-check] SKIP (ansible-playbook not on PATH)"
  exit 0
fi
echo "[goat-logship/syntax-check] ansible-playbook --syntax-check"
ansible-playbook -i 'localhost,' -c local playbook.yml --syntax-check

# GHSA-x44g: no Docker socket for Vector, a non-root user, the metrics port
# bound to loopback + mesh; read access by group; fail-closed bind lookup.
bash render.sh
bash guard.sh
bash bind.sh
bash access.sh
bash datadir.sh
# Opt-in container check of the Docker API proxy (SKIPs without docker or the
# pinned nginx image; pulls nothing).
bash docker-proxy.sh
echo "[goat-logship/syntax-check] PASS"
