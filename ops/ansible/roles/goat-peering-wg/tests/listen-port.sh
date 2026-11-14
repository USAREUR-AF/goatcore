#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# goat-peering-wg: the listen port recorded in the link environment follows
# peering_wg_listen_port when the link never came up, and the run refuses when the
# link is up on the recorded port. Runs tasks/listen_port.yml on localhost with a
# stub `wg`; needs ansible-core, not root and not WireGuard.
#
#   bash ops/ansible/roles/goat-peering-wg/tests/listen-port.sh
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

mkdir -p "$WORK/bin"
cat > "$WORK/bin/wg" <<'EOF'
#!/bin/sh
# `wg show <if> listen-port`: print the port the case says the interface holds, or
# fail as wg does when the interface does not exist.
[ -n "${STUB_WG_PORT:-}" ] || { echo "Unable to access interface: No such device" >&2; exit 1; }
echo "$STUB_WG_PORT"
EOF
chmod +x "$WORK/bin/wg"

pass=0
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok   $*"; pass=$((pass + 1)); }

# run <case> <configured port> <live port or empty>; the env file is $WORK/<case>.env
run() {
  rm -f "$WORK/$1.restarted"
  PATH="$WORK/bin:$PATH" STUB_WG_PORT="$3" \
    ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -i localhost, -c local "$HERE/listen-port.yml" \
    -e "peering_wg_env_path=$WORK/$1.env" \
    -e "peering_wg_listen_port=$2" \
    -e "restart_marker=$WORK/$1.restarted" >"$WORK/$1.log" 2>&1
}

env_file() { # <case> <recorded port>
  printf 'WG_IFNAME=wg-fed0\nWG_KEY_FILE=/etc/goat-peering-wg/wg-fed0.key\nWG_LISTEN_PORT=%s\n' "$2" > "$WORK/$1.env"
}

# 1. First run: no link environment yet. Nothing to compare, nothing written.
run first 51823 "" || { cat "$WORK/first.log"; fail "first run refused"; }
[ ! -e "$WORK/first.env" ] || fail "first run wrote a link environment; that is prepare.yml's job"
[ ! -e "$WORK/first.restarted" ] || fail "first run restarted the link"
ok "first run: nothing recorded, nothing changed"

# 2. The issue's case: phase 2 failed on 51820 (another process holds it), so the
#    interface never listened. The configured port must take effect on the rerun.
env_file never 51820
run never 51823 "" || { cat "$WORK/never.log"; fail "a link that never came up was refused"; }
grep -qx 'WG_LISTEN_PORT=51823' "$WORK/never.env" || fail "the recorded port did not follow the configuration: $(cat "$WORK/never.env")"
grep -qx 'WG_KEY_FILE=/etc/goat-peering-wg/wg-fed0.key' "$WORK/never.env" || fail "the rewrite touched other lines"
[ -e "$WORK/never.restarted" ] || fail "the link was not restarted to pick up the new port"
ok "never came up: 51820 -> 51823 written, link restarted"

# 3. The interface exists but listens on a different port than recorded (a half-made
#    link): it never came up on the recorded port either, so the same rewrite applies.
env_file half 51820
run half 51823 "0" || { cat "$WORK/half.log"; fail "a half-made link was refused"; }
grep -qx 'WG_LISTEN_PORT=51823' "$WORK/half.env" || fail "half-made link kept the old port"
ok "half-made link: rewritten"

# 4. A running link: up on the recorded port. Moving it would cut the peering, so the
#    run refuses, names both ports and leaves the file alone.
env_file live 51820
if run live 51823 "51820"; then fail "a running link's port was moved"; fi
grep -q 'listens on UDP 51820' "$WORK/live.log" || { cat "$WORK/live.log"; fail "the refusal does not name the running port"; }
grep -q 'peering_wg_listen_port: 51820' "$WORK/live.log" || fail "the refusal does not say how to keep the link"
grep -qx 'WG_LISTEN_PORT=51820' "$WORK/live.env" || fail "the refusal still rewrote the file"
[ ! -e "$WORK/live.restarted" ] || fail "the refusal still restarted the link"
ok "running link: refused, file and link untouched"

# 5. Configuration and file agree: no wg call needed, nothing written, no restart.
env_file same 51823
run same 51823 "51823" || { cat "$WORK/same.log"; fail "matching ports were refused"; }
grep -qx 'WG_LISTEN_PORT=51823' "$WORK/same.env" || fail "matching ports changed the file"
[ ! -e "$WORK/same.restarted" ] || fail "matching ports restarted the link"
ok "ports agree: unchanged"

echo "listen-port: $pass passed"
