#!/usr/bin/env bash
# Runs the onboard site-config test playbook on localhost; needs ansible-playbook.
set -euo pipefail
cd "$(dirname "$0")"
command -v ansible-playbook >/dev/null 2>&1 || { echo "[onboard-site-config] SKIP (no ansible-playbook)"; exit 0; }
out=$(mktemp)
trap 'rm -f "$out"' EXIT
fail() { cat "$out"; echo "[onboard-site-config] FAIL: $1"; exit 1; }
run() { ANSIBLE_NOCOLOR=1 ansible-playbook -i 'localhost,' playbook.yml -e test_site=testnet "$@" >"$out" 2>&1; }

run -e test_dir="$PWD/fixtures/good" || fail "a good site file did not merge"
run -e test_dir="$PWD/fixtures/none" -e test_expect_merge=false || fail "no site file changed the maps"
# The refusals: each must fail, and for the reason given.
run -e test_dir="$PWD/fixtures/unknown-key" && fail "an unknown key was accepted"
grep -q 'not per-site maps of' "$out" || fail "unknown key refused for the wrong reason"
run -e test_dir="$PWD/fixtures/thin-kratos" && fail "a Kratos entry without its fields was accepted"
grep -q 'needs' "$out" || fail "thin Kratos entry refused for the wrong reason"
run -e test_dir="$PWD/fixtures/empty-value" && fail "an empty value was accepted"
grep -q 'must be a non-empty string' "$out" || fail "empty value refused for the wrong reason"
echo "[onboard-site-config] PASS"
