#!/usr/bin/env bash
# Runs tests/carriage-signer.yml and checks that the key never reaches the
# play's output, even at -vvv.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
out="$work/play.log"
CARRIAGE_TEST_DIR="$work" ansible-playbook -vvv -i localhost, --connection local "$SCRIPT_DIR/carriage-signer.yml" >"$out" 2>&1 || {
  tail -40 "$out"; echo "goat-zenoh-portal: carriage signer play failed" >&2; exit 1; }
body="$(sed -n '2p' "$work/k.pem")"
if grep -qF "$body" "$out"; then
  echo "goat-zenoh-portal: the signer key appears in the play output" >&2
  exit 1
fi
grep -q "carriage signer not configured" "$out" || { echo "goat-zenoh-portal: no 'not configured' line for an unset signer" >&2; exit 1; }
echo "goat-zenoh-portal: carriage signer wiring OK (key never in -vvv output)"
