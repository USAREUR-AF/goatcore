#!/usr/bin/env bash
# GHSA-x44g: the rendered compose gives Vector no Docker socket, a non-root
# user and a metrics port bound to loopback + mesh (+ bridge gateway); the
# Docker socket goes only to the read-only API proxy, and only when a
# container-stdout source is on. Renders on localhost; no host, no docker.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fails=0
run() {  # run <case> <extra -e args...>
  local c=$1; shift
  mkdir -p "$work/$c"
  if ! out="$(ansible-playbook -i localhost, --connection local "$SCRIPT_DIR/render.yml" -e out_dir="$work/$c" "$@" 2>&1)"; then
    echo "  FAIL $c: render failed"; printf '%s\n' "$out" | tail -8; fails=$((fails + 1)); return
  fi
  echo " case $c"
  python3 "$SCRIPT_DIR/check_render.py" "$work/$c" "$c" || fails=$((fails + 1))
}
run default
# The guard ruleset loads (in a private network namespace; no host change).
if command -v nft >/dev/null && unshare -rn true 2>/dev/null; then
  if unshare -rn nft -f "$work/default/logship-expose-guard.nft" 2>&1; then echo "  ok   exposure guard ruleset loads (nft, private netns)"
  else echo "  FAIL exposure guard ruleset does not load"; fails=$((fails + 1)); fi
else echo "  SKIP exposure guard load check (no nft or no user namespaces)"; fi
# JSON extra vars: the Vector template tests the source toggles as plain
# truthiness, so a "false" string would still render the source.
run no-docker -e '{"logship_source_identity_enroll": false, "logship_source_app_http": false, "logship_source_app_auth": false, "logship_http_bandwidth_metrics": false}'
run bind-all -e '{"logship_bind_all": true}'
run no-counter -e '{"_logship_guard_counter": false}'
run no-ct -e '{"_logship_guard_ct": false}'
if command -v nft >/dev/null && unshare -rn true 2>/dev/null; then
  if unshare -rn nft -f "$work/no-ct/logship-expose-guard.nft" 2>&1; then echo "  ok   exposure guard ruleset without conntrack loads (nft, private netns)"
  else echo "  FAIL exposure guard ruleset without conntrack does not load"; fails=$((fails + 1)); fi
fi
[ "$fails" -eq 0 ] || { echo "goat-logship: render FAILED ($fails)"; exit 1; }
echo "goat-logship: render OK"
