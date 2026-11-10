#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Rollback-targetability coverage report for one goatnet (ADR 1060 / the
# deploy-rollback SRE strategy). Answers: "for this site, which deploying
# components can `ops/scripts/rollback.sh` actually act on, and which are
# deployed here but not yet wired?"
#
# Two independent facts per deploying role:
#   - deployed-here : the role has an on-host ledger dir
#     (/var/lib/goat/<role>/) — i.e. its meta-dependency has run on this host
#     since the rollback substrate landed (its jsonl is the source of truth).
#   - wired         : the role's in-repo deploy-history.yml lists this host in
#     its `hosts:` block (so rollback.sh knows how to reach + flip it).
#
# Targetable = deployed-here AND wired. The gap to close at standup is
# "deployed-here but NOT wired" — add the site's host to that role's
# deploy-history.yml hosts-map.
#
# Usage: ops/scripts/rollback-coverage.sh <site-label> <host> [ssh_user]
#   e.g. ops/scripts/rollback-coverage.sh zagros <the host's mesh address>
#   ssh_user defaults to the account the project declares for its machines
#   (deploy.ssh_user in ops/goatbuilder/goatbuilder.yaml); with neither, ssh
#   picks the account itself (~/.ssh/config, else the local user name).
set -euo pipefail

SITE="${1:?usage: $0 <site-label> <host> [ssh_user]}"
HOST="${2:?usage: $0 <site-label> <host> [ssh_user]}"

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ROLES_DIR="${REPO_ROOT}/ops/ansible/roles"

SSH_USER="${3:-}"
if [ -z "$SSH_USER" ] && [ -f "$REPO_ROOT/ops/goatbuilder/goatbuilder.yaml" ]; then
  SSH_USER="$(python3 -c 'import sys, yaml; print(str(((yaml.safe_load(open(sys.argv[1])) or {}).get("deploy") or {}).get("ssh_user") or "").strip())' "$REPO_ROOT/ops/goatbuilder/goatbuilder.yaml" 2>/dev/null || true)"
fi
SSH_TARGET="${SSH_USER:+${SSH_USER}@}${HOST}"

# Deploying roles = those carrying the rollback-ledger meta-dependency
# (the universal-coverage invariant; see check-rollback-coverage.sh).
DEPLOYING=()
while IFS= read -r r; do
  DEPLOYING+=("$r")
done < <(
  command grep -rl 'role:[[:space:]]*rollback-ledger' "$ROLES_DIR"/*/meta/main.yml 2>/dev/null \
    | awk -F/ '{for(i=1;i<=NF;i++) if($i=="roles"){print $(i+1); break}}' | sort -u
)

# One round-trip: which roles have an on-host ledger dir here.
LEDGERS="$(ssh -o StrictHostKeyChecking=no -o ConnectTimeout=8 "$SSH_TARGET" \
  'ls -1 /var/lib/goat/ 2>/dev/null' 2>/dev/null || true)"
deployed_here() { echo "$LEDGERS" | grep -qxF "$1"; }

wired() {
  local dh="$ROLES_DIR/$1/deploy-history.yml"
  [ -f "$dh" ] || return 1
  command grep -q "$HOST" "$dh"
}

printf "%-30s %-12s %-7s %s\n" "ROLE" "deployed?" "wired?" "targetable?"
printf -- "----------------------------------------------------------------\n"
n_dep=0 n_wired=0 n_target=0 n_gap=0
gap_roles=()
if (( ${#DEPLOYING[@]} > 0 )); then
for r in "${DEPLOYING[@]}"; do
  d="no"; w="no"; t="no"
  if deployed_here "$r"; then d="yes"; n_dep=$((n_dep+1)); fi
  if wired "$r"; then w="yes"; n_wired=$((n_wired+1)); fi
  if [ "$d" = "yes" ] && [ "$w" = "yes" ]; then t="yes"; n_target=$((n_target+1)); fi
  if [ "$d" = "yes" ] && [ "$w" = "no" ]; then n_gap=$((n_gap+1)); gap_roles+=("$r"); fi
  # Only print rows that are interesting (deployed or wired).
  if [ "$d" = "yes" ] || [ "$w" = "yes" ]; then
    printf "%-30s %-12s %-7s %s\n" "$r" "$d" "$w" "$t"
  fi
done
fi

echo
echo "site=$SITE host=$HOST : ${#DEPLOYING[@]} deploying roles; ${n_dep} deployed here; ${n_target} targetable; ${n_gap} deployed-but-unwired."
if [ "$n_gap" -gt 0 ]; then
  echo "GAP (deployed on $SITE but no hosts-entry — add this host to their deploy-history.yml):"
  printf '  %s\n' "${gap_roles[@]}"
fi
