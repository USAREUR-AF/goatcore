#!/usr/bin/env bash
# The portal role installs the example release-set registry only on the example
# goatnet or on request; anywhere else an unset portal_release_sets fails the
# play and names the variable (tasks/release-sets-guard.yml).
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fails=0

run() {  # run <want: pass|fail> <label> <extra-vars json>
  local want="$1" label="$2" vars="$3" out rc
  set +e
  out="$(ansible-playbook -i localhost, --connection local "$SCRIPT_DIR/release-sets-guard.yml" \
    -e portal_data_dir=/nonexistent -e "$vars" 2>&1)"
  rc=$?
  set -e
  if [ "$want" = pass ] && [ "$rc" -eq 0 ]; then echo "  ok   $label"
  elif [ "$want" = fail ] && [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'portal_release_sets is unset'; then echo "  ok   $label"
  else echo "  FAIL $label (rc=$rc)"; printf '%s\n' "$out" | tail -5; fails=$((fails + 1)); fi
}

run fail "another goatnet with no portal_release_sets fails, naming the variable" '{"site_id":"alpha"}'
run pass "the example goatnet keeps the committed registry" '{"site_id":"zagros"}'
run pass "an example goatnet tier keeps it too" '{"site_id":"zagros-prod"}'
run fail "a name that only starts with the example's is not the example" '{"site_id":"zagrosx"}'
run pass "another goatnet that asks for the example registry" '{"site_id":"alpha","portal_release_sets_use_example":true}'
run pass "a goatnet that declares its own sets" '{"site_id":"alpha","portal_release_sets":[{"set":"REL-ALL"}]}'
run pass "a goatnet with the release reveal off" '{"site_id":"alpha","portal_release_sets_path":""}'

[ "$fails" -eq 0 ] || { echo "goat-zenoh-portal: release-sets guard FAILED ($fails)"; exit 1; }
echo "goat-zenoh-portal: release-sets guard OK"
