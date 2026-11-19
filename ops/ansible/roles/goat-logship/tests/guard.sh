#!/usr/bin/env bash
# GHSA-x44g: the exposure guard admits the mesh-bound metrics port only from
# the mesh interface and loopback, and the bridge-gateway binding only from
# the goat-obs Prometheus's bridge and loopback. Renders the guard, then runs
# guard-lab.sh in a private user + network namespace (no host change), once
# for Docker's DNAT path and once for the docker-proxy (input) path, with
# stand-in peers on wt0 (mesh), the scraper's bridge, another container
# bridge and a LAN interface. SKIPs without nft, ip, curl or user namespaces.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
for t in nft ip nsenter curl python3; do command -v $t >/dev/null || { echo "[goat-logship/guard] SKIP (no $t)"; exit 0; }; done
unshare -rn true 2>/dev/null || { echo "[goat-logship/guard] SKIP (no user namespaces)"; exit 0; }
work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
ansible-playbook -i localhost, --connection local "$SCRIPT_DIR/render.yml" -e out_dir="$work" >/dev/null 2>&1
fails=0
# Who must get through (200); everyone else must be dropped (000).
want() {
  case "$1" in
    "GW host"|"GW SCRAPE"|"MESH host"|"MESH MESH") echo 200 ;;
    *) echo 000 ;;
  esac
}
for mode in dnat input; do
  out="$(timeout 120 unshare -rn bash "$SCRIPT_DIR/guard-lab.sh" "$work/logship-expose-guard.nft" "$mode" 2>&1)" \
    || { echo "  FAIL $mode: lab failed"; printf '%s\n' "$out" | tail -5; fails=$((fails + 1)); continue; }
  while read -r dst who got; do
    w="$(want "$dst $who")"
    if [ "$got" = "$w" ]; then echo "  ok   $mode: $who -> $dst:9598 $got"
    else echo "  FAIL $mode: $who -> $dst:9598 got $got, want $w"; fails=$((fails + 1)); fi
  done <<<"$(printf '%s\n' "$out" | grep -E '^(GW|MESH) ')"
  [ "$(printf '%s\n' "$out" | grep -cE '^(GW|MESH) ')" = 10 ] || { echo "  FAIL $mode: lab printed $(printf '%s\n' "$out" | grep -cE '^(GW|MESH) ') results, want 10"; fails=$((fails + 1)); }
done
# Control: the same lab with no guard reaches the port from everywhere, so the
# drops above are the guard's and not the lab's routing.
: > "$work/empty.nft"
for mode in dnat input; do
  out="$(timeout 120 unshare -rn bash "$SCRIPT_DIR/guard-lab.sh" "$work/empty.nft" "$mode" 2>&1 || true)"
  n="$(printf '%s\n' "$out" | grep -cE '^(GW|MESH) [A-Za-z]+ 200$' || true)"
  [ "$n" = 10 ] && echo "  ok   control ($mode, no guard): all 10 paths reach the port" \
    || { echo "  FAIL control ($mode): $n of 10 paths reached the port without a guard"; fails=$((fails + 1)); }
done
[ "$fails" -eq 0 ] || { echo "goat-logship: guard FAILED ($fails)"; exit 1; }
echo "goat-logship: guard OK"
