#!/usr/bin/env bash
# test-readback.sh — goat-peering-forwarder's read-back after the carriers
# converge (tasks/readback.yml), against a stand-in wg on localhost.
#
# The stand-in answers each `wg show <interface> ...` form from values minted
# here at random: the carrier's own key pair, the edge's public key and a
# preshared key. Each case runs the play at -v, so every registered result is in
# the log, and then reads the whole log:
#   - a carrier that holds its edge at its carrier address and port passes;
#   - one that holds another peer, or listens on another port, is refused, the
#     message lists each peer's public key and allowed address, and the log holds
#     no key but peers' public keys;
#   - no task of the role reads a carrier with `wg show <interface> dump`.
#
# Usage:  ops/ansible/roles/goat-peering-forwarder/tests/test-readback.sh
# Exit:   0 = pass (or SKIP when ansible-playbook or openssl is missing); 1 = fail.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
for tool in ansible-playbook openssl; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "goat-peering-forwarder test-readback: SKIP ($tool not on PATH)"
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
OWN_A="$(key)"; OWN_B="$(key)"           # the carrier's own key pair
EDGE="$(key)"; OTHER="$(key)"            # the edge's public key, and another
PSK="$(key)"                             # a preshared key

# The stand-in wg: `wg show <if> dump|allowed-ips|listen-port|public-key`, from
# $STAND/<if>.peers ("<public key> <allowed-ips>" per line) and $STAND/<if>.port.
mkdir -p "$T/bin" "$T/stand"
cat > "$T/bin/wg" <<'SH'
#!/bin/sh
[ "$1" = show ] || exit 1
f="$STAND/$2.peers"
case "$3" in
  dump)
    printf '%s\t%s\t%s\toff\n' "$(cat "$STAND/$2.own_a")" "$(cat "$STAND/$2.own_b")" "$(cat "$STAND/$2.port")"
    while read -r pub ips; do printf '%s\t%s\t(none)\t%s\t0\t0\t0\t25\n' "$pub" "$(cat "$STAND/$2.psk")" "$ips"; done < "$f"
    ;;
  allowed-ips) while read -r pub ips; do printf '%s\t%s\n' "$pub" "$ips"; done < "$f" ;;
  listen-port) cat "$STAND/$2.port" ;;
  public-key) cat "$STAND/$2.own_b" ;;
  *) exit 1 ;;
esac
SH
chmod 755 "$T/bin/wg"

carrier() { # $1 interface, $2 peers text, $3 listen port
  printf '%s\n' "$OWN_A" > "$T/stand/$1.own_a"; printf '%s\n' "$OWN_B" > "$T/stand/$1.own_b"
  printf '%s\n' "$PSK" > "$T/stand/$1.psk"; printf '%s' "$2" > "$T/stand/$1.peers"
  printf '%s\n' "$3" > "$T/stand/$1.port"
}

LOG="$T/play.log"
play() {
  STAND="$T/stand" ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 \
  ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -v -i localhost, "$HERE/readback.yml" -e "stand_in_bin=$T/bin" \
    -e "{\"_pf_peerings\": [{\"interface\": \"wg-carry0\", \"edge_carrier_public_key\": \"$EDGE\", \"carrier_remote_ip\": \"169.254.88.2\", \"carrier_port\": 51901}]}" \
    > "$LOG" 2>&1
}
secret_free() {
  local v
  for v in "$OWN_A" "$PSK"; do grep -qF -- "$v" "$LOG" && return 1; done
  return 0
}

echo "goat-peering-forwarder test-readback"

carrier wg-carry0 "$EDGE 169.254.88.2/32
" 51901
if play && secret_free; then
  pass "a carrier that holds its edge passes, and the log holds no key but public ones"
else
  fail "a carrier that holds its edge"; tail -20 "$LOG"
fi

carrier wg-carry0 "$OTHER 169.254.88.9/32
" 51901
if play; then
  fail "a carrier holding another peer: the play passed"
elif grep -qF "does not hold what the declaration says" "$LOG" && grep -qF "$OTHER" "$LOG" \
     && grep -qF "169.254.88.9/32" "$LOG" && secret_free; then
  pass "a carrier holding another peer is refused; the message lists each peer's public key and allowed address, and nothing else"
else
  fail "a carrier holding another peer: wrong message, or a key that is not public in the log"; grep -E 'fatal' -A4 "$LOG" | head -8 || true
fi

carrier wg-carry0 "$EDGE 169.254.88.2/32
" 51999
if play; then
  fail "a carrier on another port: the play passed"
elif grep -qF "does not hold what the declaration says" "$LOG" && grep -qF "51999" "$LOG" && secret_free; then
  pass "a carrier on another port is refused, naming the port it listens on, and the log holds no key but public ones"
else
  fail "a carrier on another port: wrong message, or a key that is not public in the log"; grep -E 'fatal' -A4 "$LOG" | head -8 || true
fi

if grep -rnE 'wg show [^"]* dump' "$HERE/../tasks"; then
  fail "a task reads a carrier with wg show ... dump"
else
  pass "no task reads a carrier with wg show ... dump"
fi

if [ "$FAILS" -eq 0 ]; then
  echo "goat-peering-forwarder test-readback: PASS"
else
  echo "goat-peering-forwarder test-readback: $FAILS FAILED"
  exit 1
fi
