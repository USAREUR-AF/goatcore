#!/usr/bin/env bash
# test-rotation.sh — a carrier key is rotated by setting it aside once, under the
# rotation's id, and making a new one (docs/design/federation-key-rotation.md §3,
# K5). Runs tasks/carrier-key.yml of goat-peering-wg and of goat-peering-forwarder
# on localhost against a stand-in wg that writes a fresh random key each time.
#
#   - with no rotation id the key is made once and a rerun keeps it;
#   - with a rotation id the key moves to .previous-<id>/ (0700) and a new one is
#     made; a rerun with the same id changes nothing;
#   - on the forwarder, only the named peering's carrier is set aside;
#   - an id that is not a plain word is refused before anything moves.
#
# Usage:  ops/ansible/roles/goat-peering-wg/tests/test-rotation.sh
# Exit:   0 = pass (or SKIP when ansible-playbook is missing); 1 = fail.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
FWD="$HERE/../../goat-peering-forwarder/tests"
if ! command -v ansible-playbook >/dev/null 2>&1; then
  echo "goat-peering-wg test-rotation: SKIP (ansible-playbook not on PATH)"
  exit 0
fi

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
FAILS=0
pass() { echo "  PASS  $1"; }
fail() { echo "  FAIL  $1"; FAILS=$((FAILS + 1)); }
mode_of() { if stat -c %a "$1" >/dev/null 2>&1; then stat -c %a "$1"; else stat -f %Lp "$1"; fi; }

# The stand-in wg: `wg genkey` prints a fresh random value, `wg pubkey` a digest of stdin.
mkdir -p "$T/bin"
cat > "$T/bin/wg" <<'SH'
#!/bin/sh
case "$1" in
  genkey) head -c 32 /dev/urandom | base64 ;;
  pubkey) cksum | cut -d' ' -f1 ;;
  *) exit 1 ;;
esac
SH
chmod 755 "$T/bin/wg"

LOG="$T/play.log"
edge() { # rest: extra arguments
  ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -i localhost, "$HERE/carrier-key.yml" -e "stand_in_bin=$T/bin" \
    -e "peering_wg_carrier_key_path=$T/edge/wg-carry0.key" "$@" > "$LOG" 2>&1
}
forwarder() { # rest: extra arguments
  ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -i localhost, "$FWD/carrier-key.yml" -e "stand_in_bin=$T/bin" \
    -e "peering_forwarder_config_dir=$T/fwd" \
    -e '{"_pf_peerings": [{"name": "xab", "interface": "wg-carry0"}, {"name": "xac", "interface": "wg-carry1"}]}' \
    "$@" > "$LOG" 2>&1
}
changed() { sed -n 's/.*changed=\([0-9]*\).*/\1/p' "$LOG" | head -1; }

echo "goat-peering-wg test-rotation"
mkdir -p "$T/edge"
K="$T/edge/wg-carry0.key"

if edge && [ -s "$K" ] && edge && [ "$(changed)" = 0 ]; then
  pass "edge: the key is made once, and a rerun keeps it"
else
  fail "edge: the key was not made, or a rerun changed it"; tail -15 "$LOG"
fi
OLD="$(cat "$K" 2>/dev/null || true)"

if edge -e peering_wg_carrier_rotation=r1 && [ -s "$K" ] && [ "$(cat "$K" 2>/dev/null)" != "$OLD" ] \
   && [ "$(cat "$T/edge/.previous-r1/wg-carry0.key" 2>/dev/null)" = "$OLD" ] && [ "$(mode_of "$T/edge/.previous-r1")" = 700 ] \
   && [ "$(mode_of "$K")" = 600 ]; then
  pass "edge: a rotation sets the key aside in .previous-r1/ (0700) and makes a new one (0600)"
else
  fail "edge: the rotation did not set the key aside and make a new one"; ls -la "$T/edge" "$T/edge/.previous-r1" 2>/dev/null || true; tail -15 "$LOG"
fi
NEW="$(cat "$K" 2>/dev/null || true)"

if edge -e peering_wg_carrier_rotation=r1 && [ "$(cat "$K" 2>/dev/null)" = "$NEW" ] && [ "$(changed)" = 0 ] \
   && [ "$(cat "$T/edge/.previous-r1/wg-carry0.key" 2>/dev/null)" = "$OLD" ]; then
  pass "edge: a rerun of the same rotation changes nothing"
else
  fail "edge: a rerun of the same rotation changed the key or what was set aside"
fi

if edge -e 'peering_wg_carrier_rotation=../x'; then
  fail "edge: a rotation id that is not a plain word was accepted"
elif grep -qF "is not a rotation id" "$LOG" && [ "$(cat "$K" 2>/dev/null)" = "$NEW" ]; then
  pass "edge: a rotation id that is not a plain word is refused, and nothing moves"
else
  fail "edge: wrong refusal for a bad rotation id, or the key moved"; grep -E 'fatal' -A3 "$LOG" | head -6 || true
fi

mkdir -p "$T/fwd"
if forwarder && [ -s "$T/fwd/wg-carry0.key" ] && [ -s "$T/fwd/wg-carry1.key" ]; then
  pass "forwarder: each carrier's key is made"
else
  fail "forwarder: the carrier keys were not made"; tail -15 "$LOG"
fi
F0="$(cat "$T/fwd/wg-carry0.key" 2>/dev/null || true)"; F1="$(cat "$T/fwd/wg-carry1.key" 2>/dev/null)"
if forwarder -e peering_forwarder_carrier_rotation=r2 -e peering_forwarder_carrier_rotation_name=xab \
   && [ "$(cat "$T/fwd/wg-carry0.key" 2>/dev/null)" != "$F0" ] && [ "$(cat "$T/fwd/.previous-r2/wg-carry0.key" 2>/dev/null)" = "$F0" ] \
   && [ "$(cat "$T/fwd/wg-carry1.key" 2>/dev/null)" = "$F1" ] && [ ! -e "$T/fwd/.previous-r2/wg-carry1.key" ]; then
  pass "forwarder: a rotation of peering xab sets its carrier's key aside and makes a new one, and leaves xac's"
else
  fail "forwarder: the rotation of xab touched the wrong carrier or did not rotate"; ls -la "$T/fwd" "$T/fwd/.previous-r2" 2>/dev/null || true; tail -15 "$LOG"
fi
if forwarder -e peering_forwarder_carrier_rotation=r2 -e peering_forwarder_carrier_rotation_name=xyz; then
  fail "forwarder: a rotation naming no entry was accepted"
elif grep -qF "names no entry of peering_forwarder_peerings" "$LOG"; then
  pass "forwarder: a rotation naming no entry is refused"
else
  fail "forwarder: wrong refusal for a rotation naming no entry"; grep -E 'fatal' -A3 "$LOG" | head -6 || true
fi

if [ "$FAILS" -eq 0 ]; then
  echo "goat-peering-wg test-rotation: PASS"
else
  echo "goat-peering-wg test-rotation: $FAILS FAILED"
  exit 1
fi
