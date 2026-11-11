#!/usr/bin/env bash
# test-outbound-listen.sh — goat-peering-spire takes the outbound forward's listen
# address from this edge's route to its SPIRE server when none is set, keeps one
# that is set, and refuses, naming both values, when there is neither.
#
# Usage:  ops/ansible/roles/goat-peering-spire/tests/test-outbound-listen.sh
# Exit:   0 = pass (or SKIP when ansible-playbook or ip is missing); 1 = fail.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
for tool in ansible-playbook ip; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "goat-peering-spire test-outbound-listen: SKIP ($tool not on PATH)"
    exit 0
  fi
done
run() { ANSIBLE_STDOUT_CALLBACK=default ansible-playbook -i localhost, "$HERE/outbound-listen.yml" "$@" 2>&1; }
fail=0
check() { # $1 name, $2 expected fragment, $3 output
  if printf '%s' "$3" | grep -qF -- "$2"; then echo "ok: $1"; else echo "FAIL: $1 (wanted: $2)"; printf '%s\n' "$3" | tail -20; fail=1; fi
}

out="$(run -e server=127.0.0.1 -e listen= || true)"
check "derived from the route to the server" "LISTEN=127.0.0.1" "$out"

out="$(run --check -e server=127.0.0.1 -e listen= || true)"
check "derived in a --check run too" "LISTEN=127.0.0.1" "$out"

out="$(run -e server=127.0.0.1 -e listen=192.0.2.20 || true)"
check "a set value is kept" "LISTEN=192.0.2.20" "$out"

out="$(run -e server= -e listen= || true)"
check "neither: refused" "cannot tell where to listen for our SPIRE server" "$out"

[ "$fail" = 0 ] && echo "goat-peering-spire test-outbound-listen: PASS"
exit "$fail"
