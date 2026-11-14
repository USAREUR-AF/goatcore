#!/usr/bin/env bash
# test-readback.sh — goat-peering-wg's read-back after the link converges
# (tasks/readback.yml), against a stand-in wg and ip on localhost.
#
# The stand-in answers each `wg show <interface> ...` form from values minted
# here at random: the interface's own key pair, the peer's public key and its
# preshared key. Each case runs the play at -v, so every registered result is in
# the log, and then reads the whole log:
#   - an interface that holds the agreed peer and address passes;
#   - one that does not is refused, the message lists each peer's public key and
#     allowed address, and the log holds no key but peers' public keys;
#   - the same for the carrier behind a forwarder.
#
# Usage:  ops/ansible/roles/goat-peering-wg/tests/test-readback.sh
# Exit:   0 = pass (or SKIP when ansible-playbook or openssl is missing); 1 = fail.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
for tool in ansible-playbook openssl; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "goat-peering-wg test-readback: SKIP ($tool not on PATH)"
    exit 0
  fi
done

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
FAILS=0
pass() { echo "  PASS  $1"; }
fail() { echo "  FAIL  $1"; FAILS=$((FAILS + 1)); }
key() { openssl rand -base64 32; }

# Values minted for this run only.
OWN_A="$(key)"; OWN_B="$(key)"           # the interface's own key pair
PEER="$(key)"; OTHER="$(key)"            # the agreed peer, and another
PSK="$(key)"                             # a preshared key
C_OWN_A="$(key)"; C_OWN_B="$(key)"; FWD="$(key)"; C_PSK="$(key)"

# The stand-in wg: `wg show <if> dump|allowed-ips|latest-handshakes`, from
# $STAND/<if>.peers, one "<public key> <allowed-ips> <handshake epoch>" per line.
mkdir -p "$T/bin" "$T/stand"
cat > "$T/bin/wg" <<'SH'
#!/bin/sh
[ "$1" = show ] || exit 1
f="$STAND/$2.peers"; own="$STAND/$2.own"
case "$3" in
  dump)
    [ -f "$own" ] && cat "$own"
    [ -f "$f" ] && while read -r pub ips hs; do printf '%s\t%s\t(none)\t%s\t%s\t0\t0\t25\n' "$pub" "$(cat "$STAND/$2.psk")" "$ips" "$hs"; done < "$f"
    ;;
  allowed-ips) [ -f "$f" ] && while read -r pub ips hs; do printf '%s\t%s\n' "$pub" "$ips"; done < "$f" ;;
  latest-handshakes) [ -f "$f" ] && while read -r pub ips hs; do printf '%s\t%s\n' "$pub" "$hs"; done < "$f" ;;
  *) exit 1 ;;
esac
SH
# The stand-in ip: `ip -o -4 addr show dev <if>`, from $STAND/<if>.addr.
cat > "$T/bin/ip" <<'SH'
#!/bin/sh
for a; do dev="$a"; done
[ -f "$STAND/$dev.addr" ] && printf '7: %s    inet %s scope global %s\n' "$dev" "$(cat "$STAND/$dev.addr")" "$dev"
exit 0
SH
chmod 755 "$T/bin/wg" "$T/bin/ip"

LOG="$T/play.log"
play() { # rest: extra ansible-playbook arguments
  STAND="$T/stand" ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 \
  ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -v -i localhost, "$HERE/readback.yml" \
    -e "stand_in_bin=$T/bin" -e "now_epoch=$(date +%s)" \
    -e peering_wg_interface=wg-fed0 -e "_pw_peer_key=$PEER" \
    -e peering_wg_remote_link_ip=169.254.77.1 -e peering_wg_local_link_ip=169.254.77.0 \
    -e peering_wg_link_prefix_len=31 "$@" > "$LOG" 2>&1
}
secret_free() { # nothing in the log is an interface's own key or a preshared key
  local v
  for v in "$OWN_A" "$OWN_B" "$PSK" "$C_OWN_A" "$C_OWN_B" "$C_PSK"; do
    grep -qF -- "$v" "$LOG" && return 1
  done
  return 0
}
interface() { # $1 name, $2 peers file text, $3 address
  printf '%s\t%s\t51820\toff\n' "$OWN_A" "$OWN_B" > "$T/stand/$1.own"
  printf '%s\n' "$PSK" > "$T/stand/$1.psk"
  printf '%s' "$2" > "$T/stand/$1.peers"
  if [ -n "$3" ]; then printf '%s\n' "$3" > "$T/stand/$1.addr"; else rm -f "$T/stand/$1.addr"; fi
}

echo "goat-peering-wg test-readback"

interface wg-fed0 "$PEER 169.254.77.1/32 0
" 169.254.77.0/31
if play -e peering_wg_via_forwarder=false && secret_free; then
  pass "an interface that holds the agreement passes, and the log holds no key but public ones"
else
  fail "an interface that holds the agreement"; tail -20 "$LOG"
fi

interface wg-fed0 "$OTHER 169.254.77.9/32 0
" ""
if play -e peering_wg_via_forwarder=false; then
  fail "an interface without the agreed peer: the play passed"
elif grep -qF "The interface does not hold what the agreement says" "$LOG" \
     && grep -qF "$OTHER" "$LOG" && grep -qF "169.254.77.9/32" "$LOG" && secret_free; then
  pass "an interface without the agreed peer is refused; the message lists each peer's public key and allowed address, and nothing else"
else
  fail "an interface without the agreed peer: wrong message, or a key that is not public in the log"; grep -E 'fatal' -A4 "$LOG" | head -8 || true
fi

interface wg-fed0 "" ""
if play -e peering_wg_via_forwarder=false; then
  fail "an interface with no peer at all: the play passed"
elif grep -qF "The interface does not hold what the agreement says" "$LOG" && secret_free; then
  pass "an interface with no peer at all is refused, and the log holds no key but public ones"
else
  fail "an interface with no peer at all: wrong message, or a key that is not public in the log"; grep -E 'fatal' -A4 "$LOG" | head -8 || true
fi

# Behind a forwarder: the carrier holds one peer, the forwarder.
interface wg-fed0 "$PEER 169.254.77.1/32 0
" 169.254.77.0/31
CARRIER=(-e peering_wg_via_forwarder=true -e peering_wg_carrier_interface=wg-car0
  -e "peering_wg_forwarder_carrier_public_key=$FWD" -e peering_wg_carrier_remote_ip=169.254.88.1
  -e peering_wg_forwarder_endpoint=192.0.2.40:51900 -e peering_wg_carrier_keepalive=25)
carrier() { # $1 peers text
  printf '%s\t%s\t51900\toff\n' "$C_OWN_A" "$C_OWN_B" > "$T/stand/wg-car0.own"
  printf '%s\n' "$C_PSK" > "$T/stand/wg-car0.psk"
  printf '%s' "$1" > "$T/stand/wg-car0.peers"
}
carrier "$FWD 169.254.88.1/32 $(date +%s)
"
if play "${CARRIER[@]}" && grep -qF "last handshake" "$LOG" && secret_free; then
  pass "a carrier that holds the forwarder passes, reports its handshake, and the log holds no key but public ones"
else
  fail "a carrier that holds the forwarder"; tail -20 "$LOG"
fi
carrier "$OTHER 169.254.88.9/32 0
"
if play "${CARRIER[@]}"; then
  fail "a carrier holding another peer: the play passed"
elif grep -qF "does not hold one peer" "$LOG" && grep -qF "$OTHER" "$LOG" && secret_free; then
  pass "a carrier holding another peer is refused, listing its public key and allowed address, and nothing else"
else
  fail "a carrier holding another peer: wrong message, or a key that is not public in the log"; grep -E 'fatal' -A4 "$LOG" | head -8 || true
fi

if [ "$FAILS" -eq 0 ]; then
  echo "goat-peering-wg test-readback: PASS"
else
  echo "goat-peering-wg test-readback: $FAILS FAILED"
  exit 1
fi
