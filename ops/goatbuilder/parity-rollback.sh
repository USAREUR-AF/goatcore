#!/usr/bin/env bash
# parity-rollback.sh — the parity gate for `goatbuilder rollback` against
# ops/scripts/rollback.sh (goatbuilder-conformance.md §The parity gate).
#
# The two read-only modes are compared line for line for every component that
# carries a deploy-history.yml: `--list` (the on-host ledger where a host is
# reachable, else the in-repo history) and `--status` (the running ref per host,
# `<unreachable>` where it is not). Where no host is reachable — CI — both sides
# take the in-repo path, which is the fallback a real rollback would take there
# too. Where a host is reachable — the deployment e2e harness's container hosts
# (ops/e2e/hermetic.sh) or an operator's laptop — the host path is compared, and
# the summary counts how many invocations took it. The script reads the in-repo
# ledger through jq whichever yq is installed (mikefarah's or the Python one);
# CI runs this gate under both. The flip itself is the script's own remote
# shell, carried verbatim; the unit tests pin its three shapes.
#
# `--site <s>` narrows a component to the hosts of one goatnet. For every
# component whose hosts span more than one site, `--status --site` is compared
# for its first site, and an unknown site must be refused in the same words.
#
# A goatnet reached through its env door: on a fixture tree with a door
# registry (doors.yaml) and one component on one site, `--list --site` and
# `--status --site` are compared with a door that opens and with one that does
# not. Both avenues try the host directly first, then the door, and fall back to
# the in-repo history when neither answers; neither names the door on a read.
#
# usage: parity-rollback.sh <goatbuilder binary> [component ...]
#        parity-rollback.sh --self-test
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
# goatbuilder prints each refusal's catalog code on a line of its own after it,
# `  (GB-NNN — goatbuilder explain GB-NNN)`; rollback.sh has no codes. That one
# line is taken off goatbuilder's side, and everything else is compared as it is.
binary_filter() { sed -E 's/^goatbuilder: //; s/^(ok  |FAIL)  rollback  //; /^rollback: (ok|refused|not converged)/d; /^[[:space:]]*\(GB-[0-9]+ — goatbuilder explain GB-[0-9]+\)$/d' | sed -E 's/[[:space:]]+$//'; }
# The self-test: the code line is the only thing taken off goatbuilder's side, so
# a real difference still fails. Each case is script output, goatbuilder output,
# and whether the two must compare equal.
code_line='  (GB-260 — goatbuilder explain GB-260)'
self_test() {
  local bad=0 a b want got
  check() {
    a="$(printf '%s\n' "$2" | sed -E 's/[[:space:]]+$//')"; b="$(printf '%s\n' "$3" | binary_filter)"
    want="$4"; if [ "$a" = "$b" ]; then got=same; else got=differ; fi
    if [ "$got" != "$want" ]; then bad=$((bad+1)); echo "SELF-TEST FAILED: $1: expected $want, got $got"; fi
  }
  check "the refusal with its code line" "deploy-history.yml has no hosts entries" \
    "goatbuilder: deploy-history.yml has no hosts entries
$code_line" same
  check "a listing row that differs, code line or not" "  1  2026-01-02  v1.2.3  a deploy" \
    "  1  2026-01-02  v1.2.4  a deploy
$code_line" differ
  check "a refusal in other words" "deploy-history.yml has no hosts entries" \
    "goatbuilder: deploy-history.yml has no host entries
$code_line" differ
  check "a parenthesised line that is not the code line" "deploy-history.yml has no hosts entries" \
    "goatbuilder: deploy-history.yml has no hosts entries
  (GB-260 — see the ledger)" differ
  check "the code line's words inside a row" "deploy-history.yml has no hosts entries" \
    "goatbuilder: deploy-history.yml has no hosts entries (GB-260 — goatbuilder explain GB-260)" differ
  if [ "$bad" = 0 ]; then echo "parity-rollback: self-test ok, 5 cases"; else echo "parity-rollback: self-test failed, $bad of 5 cases"; return 1; fi
}
if [ "${1:-}" = "--self-test" ]; then self_test; exit $?; fi
BIN="${1:?usage: parity-rollback.sh <goatbuilder binary> [component ...]}"; shift || true
COMPONENTS=("$@")
if [ ${#COMPONENTS[@]} -eq 0 ]; then
  for f in "$REPO"/ops/ansible/roles/*/deploy-history.yml; do COMPONENTS+=("$(basename "$(dirname "$f")")"); done
fi
command -v yq >/dev/null || { echo "parity-rollback: yq required by the script side (brew install yq / apt-get install yq)"; exit 2; }

ROOT="$REPO"
script_side() { "$ROOT/ops/scripts/rollback.sh" "$@" 2>&1 | sed -E 's/^rollback\.sh: //' | sed -E 's/[[:space:]]+$//'; }
binary_side() { "$BIN" -C "$ROOT" rollback "$@" 2>&1 | binary_filter; }

# The distinct sites of a component's hosts, whichever yq is installed.
sites_of() {
  if yq --version 2>&1 | grep -qi mikefarah; then yq -o=json '.' "$1"; else yq '.' "$1"; fi \
    | jq -r '[.hosts[]?.site // empty] | unique | .[]'
}

fail=0; n=0; host=0
compare() {
  n=$((n+1))
  a="$(script_side "$@" || true)"; b="$(binary_side "$@" || true)"
  case "$a" in *"(on-host ledger @"*|*"(from host-ledger("*) host=$((host+1)) ;; esac
  if [ "$a" != "$b" ]; then
    fail=$((fail+1)); echo "PARITY FAIL: rollback $*"
    { diff <(printf '%s\n' "$a") <(printf '%s\n' "$b") || true; } | sed 's/^/    /' | head -12
  fi
}
for c in "${COMPONENTS[@]}"; do
  for mode in --list --status; do compare "$c" "$mode"; done
  sites="$(sites_of "$REPO/ops/ansible/roles/$c/deploy-history.yml" 2>/dev/null || true)"
  if [ "$(printf '%s\n' "$sites" | grep -c .)" -gt 1 ]; then
    compare "$c" --status --site "$(printf '%s\n' "$sites" | head -1)"
    compare "$c" --list --site no-such-site
  fi
done

# The door fixture. Its host does not answer here unless something on this
# machine answers on 100.127.0.9; either way both avenues must agree.
DOORS="$(mktemp -d "${TMPDIR:-/tmp}/parity-rollback-door.XXXXXX")"
trap 'rm -rf "$DOORS"' EXIT
mkdir -p "$DOORS/ops/scripts" "$DOORS/ops/ansible/inventory" "$DOORS/ops/ansible/roles/door-fixture"
cp "$REPO/ops/scripts/rollback.sh" "$REPO/ops/scripts/env-door.sh" "$REPO/ops/scripts/generate-door-inventory.py" "$DOORS/ops/scripts/"
cat > "$DOORS/ops/ansible/inventory/doors.yaml" <<'YAML'
doors:
  zagros-dev:
    type: ssm
    instance: i-0000
    region: zz-test-1
    port: 12203
    user: doorkeeper
    mgmt:
      - {name: mgmt-zagros-dev-01, mesh_ip: 100.127.0.9, gv_group: zagros_dev_hosts, landed: true}
YAML
cat > "$DOORS/ops/ansible/roles/door-fixture/deploy-history.yml" <<'YAML'
hosts:
  - host: 100.127.0.9
    ssh_user: ops
    container_name: door-fixture
    compose_path: /opt/door-fixture/docker-compose.yml
    site: zagros-dev
history:
  - sha: sha-b
    image: ghcr.io/example/door-fixture:sha-b
    pr: 2
    merged_at: "2026-10-02T00:00:00Z"
YAML
ROOT="$DOORS"
for opens in 0 1; do
  printf '#!/bin/sh\nexit %s\n' "$opens" > "$DOORS/ops/scripts/door-open.sh"
  chmod +x "$DOORS/ops/scripts/"*.sh
  for mode in --list --status; do compare door-fixture "$mode" --site zagros-dev; done
done
ROOT="$REPO"
[ "${n:-0}" -gt 0 ] || { echo "parity FAIL: nothing was compared (no inputs read — is PyYAML importable by python3?)"; exit 1; }
if [ "$fail" = 0 ]; then echo "parity ok: $n rollback invocations identical, $host on the host path (script vs goatbuilder rollback --list/--status over ${#COMPONENTS[@]} components)"; else echo "parity FAIL: $fail of $n differ"; exit 1; fi
