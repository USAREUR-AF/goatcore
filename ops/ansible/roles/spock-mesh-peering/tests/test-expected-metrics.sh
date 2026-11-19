#!/usr/bin/env bash
# test-expected-metrics.sh: the expected-subscriptions textfiles that
# spock-mesh-peering and ory-narrow-repset write (tasks/expected-metrics.yml in
# each), rendered on localhost into a temporary directory.
#
# Cases:
#   - four declared peers and the mesh's databases netbird and netbird_events:
#     spock-mesh-expected.prom holds spock_mesh_expected_subscriptions 4 for each;
#   - the Ory role with three peer sites: spock-ory-expected.prom holds 3 for
#     kratos and for hydra;
#   - no peer (a founder alone): 0 for each database, so the alert reads
#     nothing missing;
#   - each file has one HELP and one TYPE line, the same in both files, so
#     node-exporter's textfile collector serves them as one metric.
#
# Usage:  ops/ansible/roles/spock-mesh-peering/tests/test-expected-metrics.sh
# Exit:   0 = pass (or SKIP when ansible-playbook is missing); 1 = fail.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROLES="$(cd "$HERE/../.." && pwd)"
if ! command -v ansible-playbook >/dev/null 2>&1; then
  echo "spock-mesh-peering test-expected-metrics: SKIP (ansible-playbook not on PATH)"
  exit 0
fi

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
FAILS=0
pass() { echo "  PASS  $1"; }
fail() { echo "  FAIL  $1"; FAILS=$((FAILS + 1)); }

# play <dir> <mesh peers> <ory peers>: both task files, with the given peer counts.
play() {
  local dir="$1" peers sites
  peers="$(python3 -c 'import json, sys; print(json.dumps([{"site_id": "zagros-p%d" % i} for i in range(1, int(sys.argv[1]) + 1)]))' "$2")"
  sites="$(python3 -c 'import json, sys; print(json.dumps(["zagros-p%d" % i for i in range(1, int(sys.argv[1]) + 1)]))' "$3")"
  mkdir -p "$dir"
  cat > "$T/play.yml" <<EOF
- hosts: localhost
  connection: local
  gather_facts: false
  become: false
  vars:
    spock_mesh_metrics_dir: "$dir"
    spock_mesh_metrics_become: false
    ory_spock_metrics_dir: "$dir"
    ory_spock_metrics_become: false
    spock_mesh_databases: [{name: netbird}, {name: netbird_events}]
    spock_mesh_target_peers: $peers
    ory_spock_databases: [kratos, hydra]
    ory_spock_peer_sites: $sites
  tasks:
    - ansible.builtin.include_tasks: "$ROLES/spock-mesh-peering/tasks/expected-metrics.yml"
    - ansible.builtin.include_tasks: "$ROLES/ory-narrow-repset/tasks/expected-metrics.yml"
EOF
  (cd "$T" && ANSIBLE_BECOME=False ANSIBLE_NOCOLOR=1 ANSIBLE_LOCAL_TEMP="$T/local" ANSIBLE_REMOTE_TEMP="$T/remote" \
    ansible-playbook -i localhost, "$T/play.yml" > "$T/play.log" 2>&1) || { cat "$T/play.log"; return 1; }
}

expect() { # expect <label> <got> <want>
  if [[ "$2" == "$3" ]]; then pass "$1"; else fail "$1: got '$2', want $3"; fi
}
value() { # value <file> <datname>
  awk -v d="spock_mesh_expected_subscriptions{datname=\"$2\"}" '$1 == d {print $2}' "$1"
}

play "$T/four" 4 3
for db in netbird netbird_events; do
  expect "mesh: $db expects 4" "$(value "$T/four/spock-mesh-expected.prom" "$db")" 4
done
for db in kratos hydra; do
  expect "ory: $db expects 3" "$(value "$T/four/spock-ory-expected.prom" "$db")" 3
done
help_a="$(grep -c '^# HELP spock_mesh_expected_subscriptions ' "$T/four/spock-mesh-expected.prom")"
help_b="$(grep -c '^# HELP spock_mesh_expected_subscriptions ' "$T/four/spock-ory-expected.prom")"
if [[ "$help_a" == 1 && "$help_b" == 1 ]] \
   && [[ "$(grep '^# ' "$T/four/spock-mesh-expected.prom")" == "$(grep '^# ' "$T/four/spock-ory-expected.prom")" ]]; then
  pass "one HELP and TYPE in each file, the same in both"
else
  fail "HELP/TYPE lines differ or repeat"
fi
if [[ "$(grep -vc -E '^(# (HELP|TYPE) |spock_mesh_expected_subscriptions\{datname="[a-z_]+"\} [0-9]+$)' "$T/four/spock-mesh-expected.prom")" == 0 ]]; then
  pass "every line is a HELP, a TYPE or a sample"
else
  fail "a line in spock-mesh-expected.prom is not a HELP, TYPE or sample"
fi

play "$T/alone" 0 0
for db in netbird netbird_events; do
  expect "no peer: $db expects 0" "$(value "$T/alone/spock-mesh-expected.prom" "$db")" 0
done

if [[ "$FAILS" -eq 0 ]]; then
  echo "spock-mesh-peering test-expected-metrics: PASS"
else
  echo "spock-mesh-peering test-expected-metrics: FAIL ($FAILS)"
  exit 1
fi
