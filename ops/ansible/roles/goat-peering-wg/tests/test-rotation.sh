#!/usr/bin/env bash
# test-rotation.sh — a carrier key (K5) or the peering key (K4) is rotated by
# setting it aside once, under the rotation's id, and making a new one
# (docs/design/federation-key-rotation.md §3). Runs tasks/carrier-key.yml of
# goat-peering-wg and of goat-peering-forwarder, and goat-peering-wg's
# tasks/transport-key.yml and tasks/transport-live.yml, on localhost against a
# stand-in wg that writes a fresh random key each time.
#
#   - with no rotation id the key is made once and a rerun keeps it;
#   - with a rotation id the key moves to .previous-<id>/ (0700) and a new one is
#     made; a rerun with the same id changes nothing;
#   - on the forwarder, only the named peering's carrier is set aside;
#   - an id that is not a plain word is refused before anything moves;
#   - after a peering key rotation the link is restarted while the interface holds
#     a key other than the one on disk, and not once it holds the new one.
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

# The stand-in wg: `wg genkey` prints a fresh random value, `wg pubkey` a digest of
# stdin, and `wg show <if> public-key` what the test wrote to live.pub, the public
# key the interface holds.
mkdir -p "$T/bin"
cat > "$T/bin/wg" <<SH
#!/bin/sh
case "\$1" in
  genkey) head -c 32 /dev/urandom | base64 ;;
  pubkey) cksum | cut -d' ' -f1 ;;
  show) cat "$T/live.pub" ;;
  *) exit 1 ;;
esac
SH
chmod 755 "$T/bin/wg"

LOG="$T/play.log"
edge() { # rest: extra arguments
  rm -f "$T/carrier-restarted"
  ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -i localhost, "$HERE/carrier-key.yml" -e "stand_in_bin=$T/bin" \
    -e "peering_wg_carrier_key_path=$T/edge/wg-carry0.key" -e "restart_marker=$T/carrier-restarted" "$@" > "$LOG" 2>&1
}
forwarder() { # rest: extra arguments
  ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -i localhost, "$FWD/carrier-key.yml" -e "stand_in_bin=$T/bin" \
    -e "peering_forwarder_config_dir=$T/fwd" \
    -e '{"_pf_peerings": [{"name": "xab", "interface": "wg-carry0"}, {"name": "xac", "interface": "wg-carry1"}]}' \
    "$@" > "$LOG" 2>&1
}
peering() { # rest: extra arguments
  rm -f "$T/restarted"
  ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -i localhost, "$HERE/transport-key.yml" -e "stand_in_bin=$T/bin" \
    -e "peering_wg_key_path=$T/edge/wg-fed0.key" -e "restart_marker=$T/restarted" "$@" > "$LOG" 2>&1
}
live_is() { { cksum < "$1" | cut -d' ' -f1 > "$T/live.pub"; } 2>/dev/null || true; } # the interface holds this key file
changed() { sed -n 's/.*changed=\([0-9]*\).*/\1/p' "$LOG" | head -1; }

echo "goat-peering-wg test-rotation"
mkdir -p "$T/edge"
K="$T/edge/wg-carry0.key"

if edge && [ -s "$K" ] && edge && [ "$(changed)" = 0 ] && [ ! -e "$T/carrier-restarted" ]; then
  pass "edge: the key is made once, a rerun keeps it, and the carrier is not restarted without a rotation"
else
  fail "edge: the key was not made, a rerun changed it, or the carrier restarted"; tail -15 "$LOG"
fi
OLD="$(cat "$K" 2>/dev/null || true)"
live_is "$K"

if edge -e peering_wg_carrier_rotation=r1 && [ -s "$K" ] && [ "$(cat "$K" 2>/dev/null)" != "$OLD" ] \
   && [ "$(cat "$T/edge/.previous-r1/wg-carry0.key" 2>/dev/null)" = "$OLD" ] && [ "$(mode_of "$T/edge/.previous-r1")" = 700 ] \
   && [ "$(mode_of "$K")" = 600 ] && [ -e "$T/carrier-restarted" ]; then
  pass "edge: a rotation sets the key aside in .previous-r1/ (0700), makes a new one (0600), and restarts the carrier onto it"
else
  fail "edge: the rotation did not set the key aside, make a new one and restart the carrier"; ls -la "$T/edge" "$T/edge/.previous-r1" 2>/dev/null || true; tail -15 "$LOG"
fi
NEW="$(cat "$K" 2>/dev/null || true)"
if edge -e peering_wg_carrier_rotation=r1 && [ "$(cat "$K" 2>/dev/null)" = "$NEW" ] && [ -e "$T/carrier-restarted" ]; then
  pass "edge: a rerun of the rotation moves nothing, and restarts a carrier still holding the old key"
else
  fail "edge: a rerun of the rotation moved a key, or left the carrier on the old key"; tail -15 "$LOG"
fi
live_is "$K"
if edge -e peering_wg_carrier_rotation=r1 && [ "$(changed)" = 0 ] && [ ! -e "$T/carrier-restarted" ]; then
  pass "edge: once the carrier holds the new key, a rerun of the rotation changes nothing"
else
  fail "edge: a rerun restarted a carrier that already holds the new key"; tail -15 "$LOG"
fi

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

# A key path with a space in it: the carrier's live check reads the key on disk
# through a shell redirect, so the path is quoted there.
mkdir -p "$T/edge sp"
KS="$T/edge sp/wg-carry0.key"
if edge -e "{\"peering_wg_carrier_key_path\": \"$KS\"}" && [ -s "$KS" ] && live_is "$KS" \
   && edge -e "{\"peering_wg_carrier_key_path\": \"$KS\"}" -e peering_wg_carrier_rotation=r2 \
   && [ -e "$T/carrier-restarted" ] && ! grep -q 'No such file\|ambiguous redirect' "$LOG"; then
  pass "edge: a carrier key path with a space is read whole, and the rotation restarts the carrier"
else
  fail "edge: a carrier key path with a space broke the live check"; tail -15 "$LOG"
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

P="$T/edge/wg-fed0.key"
if peering && [ -s "$P" ] && [ ! -e "$T/restarted" ] && peering && [ "$(changed)" = 0 ]; then
  pass "peering: the key is made once, a rerun keeps it, and nothing restarts without a rotation"
else
  fail "peering: the key was not made, a rerun changed it, or the link restarted"; tail -15 "$LOG"
fi
POLD="$(cat "$P" 2>/dev/null || true)"
live_is "$P"

if peering -e peering_wg_transport_rotation=t1 && [ -s "$P" ] && [ "$(cat "$P" 2>/dev/null)" != "$POLD" ] \
   && [ "$(cat "$T/edge/.previous-t1/wg-fed0.key" 2>/dev/null)" = "$POLD" ] && [ "$(mode_of "$T/edge/.previous-t1")" = 700 ] \
   && [ "$(mode_of "$P")" = 600 ] && [ -e "$T/restarted" ] && [ -s "$K" ] && [ ! -e "$T/edge/.previous-t1/wg-carry0.key" ]; then
  pass "peering: a rotation sets the key aside in .previous-t1/ (0700), makes a new one (0600), restarts the link, and leaves the carrier"
else
  fail "peering: the rotation did not set the key aside, make a new one and restart the link"; ls -la "$T/edge" "$T/edge/.previous-t1" 2>/dev/null || true; tail -15 "$LOG"
fi
PNEW="$(cat "$P" 2>/dev/null || true)"

if peering -e peering_wg_transport_rotation=t1 && [ "$(cat "$P" 2>/dev/null)" = "$PNEW" ] && [ -e "$T/restarted" ] \
   && [ "$(cat "$T/edge/.previous-t1/wg-fed0.key" 2>/dev/null)" = "$POLD" ]; then
  pass "peering: a rerun of the rotation moves nothing, and restarts the link while the interface holds the old key"
else
  fail "peering: a rerun of the rotation moved a key, or did not restart a link holding the old key"; tail -15 "$LOG"
fi
live_is "$P"
if peering -e peering_wg_transport_rotation=t1 && [ "$(changed)" = 0 ] && [ ! -e "$T/restarted" ]; then
  pass "peering: once the interface holds the new key, a rerun of the rotation changes nothing"
else
  fail "peering: a rerun restarted a link that already holds the new key"; tail -15 "$LOG"
fi

if peering -e '{"peering_wg_transport_rotation": "a b"}'; then
  fail "peering: a rotation id that is not a plain word was accepted"
elif grep -qF "is not a rotation id" "$LOG" && [ "$(cat "$P" 2>/dev/null)" = "$PNEW" ]; then
  pass "peering: a rotation id that is not a plain word is refused, and nothing moves"
else
  fail "peering: wrong refusal for a bad rotation id, or the key moved"; grep -E 'fatal' -A3 "$LOG" | head -6 || true
fi

# Paths with a space: the peering key's generation and live check, and the
# forwarder's carrier keys, reach their files through a shell redirect, so each
# path is quoted there.
mkdir -p "$T/edge sp2"
PS="$T/edge sp2/wg-fed0.key"
if peering -e "{\"peering_wg_key_path\": \"$PS\"}" && [ -s "$PS" ] && live_is "$PS" \
   && peering -e "{\"peering_wg_key_path\": \"$PS\"}" -e peering_wg_transport_rotation=t2 \
   && [ -s "$PS" ] && [ -e "$T/restarted" ]; then
  pass "peering: a key path with a space is made and read whole, and the rotation restarts the link"
else
  fail "peering: a key path with a space broke the key tasks"; tail -15 "$LOG"
fi
mkdir -p "$T/fwd sp"
if forwarder -e "{\"peering_forwarder_config_dir\": \"$T/fwd sp\"}" \
   && [ -s "$T/fwd sp/wg-carry0.key" ] && [ -s "$T/fwd sp/wg-carry1.key" ]; then
  pass "forwarder: a config directory with a space gets each carrier's key"
else
  fail "forwarder: a config directory with a space broke the carrier keys"; tail -15 "$LOG"
fi

# The link environment files name the key's path for systemd's EnvironmentFile,
# which splits an unquoted value at whitespace: the path is written in double
# quotes with \ " $ and ` escaped, so systemd's reading (the quotes and the
# escapes removed) and a shell's `. file` both give it back whole and literal. A
# path holding a newline is refused.
mkdir -p "$T/env"
envval() { # $1 file, $2 name: the value as systemd reads a double-quoted one
  sed -n "s/^$2=\"\(.*\)\"\$/\1/p" "$1" | sed 's/\\\([\\"$`]\)/\1/g'
}
envfiles() { # $1 peering key path, $2 carrier key path, $3 directory for the rulesets and the forwarder's keys
  ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -i localhost, "$HERE/env-files.yml" -e "env_out=$T/env" \
    -e "$(python3 -c 'import json,sys; print(json.dumps({"peering_wg_key_path": sys.argv[1], "peering_wg_carrier_key_path": sys.argv[2], "peering_wg_nft_path": sys.argv[3] + "/wg-fed0.nft", "peering_forwarder_config_dir": sys.argv[3], "peering_forwarder_nft_path": sys.argv[3] + "/forwarder.nft"}))' "$1" "$2" "${3:-/etc/goat-peering-wg}")" \
    > "$LOG" 2>&1
}
sourced() { # $1 file, $2 name (default WG_KEY_FILE): the value as a shell that sources the file reads it
  sh -c '. "$1"; eval "printf %s \"\${$2}\""' sh "$1" "${2:-WG_KEY_FILE}"
}
# shellcheck disable=SC2016 # a literal $ and backtick in the path, not expansions
DS="$T/edge \$HOME \`id\`/wg-fed0.key"
for pair in "$PS|$KS" "$DS|$DS"; do
  p="${pair%%|*}"; c="${pair#*|}"
  if envfiles "$p" "$c" \
     && [ "$(envval "$T/env/link-interface.env" WG_KEY_FILE)" = "$p" ] \
     && [ "$(envval "$T/env/link.env" WG_KEY_FILE)" = "$p" ] && [ "$(envval "$T/env/carrier.env" WG_KEY_FILE)" = "$c" ] \
     && [ "$(sourced "$T/env/link-interface.env")" = "$p" ] && [ "$(sourced "$T/env/link.env")" = "$p" ] \
     && [ "$(sourced "$T/env/carrier.env")" = "$c" ]; then
    pass "env files: the key path '$p' is written quoted and read back whole by systemd's rules and by a shell"
  else
    fail "env files: the key path '$p' is not read back whole"; grep WG_KEY_FILE "$T/env/"*.env 2>/dev/null || true; tail -15 "$LOG"
  fi
done
# The ruleset paths beside them (WG_NFT_FILE) and the forwarder's carrier key,
# under a directory with a space, and one with $HOME and a backtick.
# shellcheck disable=SC2016 # a literal $ and backtick in the path, not expansions
for dir in "$T/etc sp" "$T/etc \$HOME \`id\`"; do
  if envfiles "$PS" "$KS" "$dir" \
     && [ "$(envval "$T/env/link.env" WG_NFT_FILE)" = "$dir/wg-fed0.nft" ] \
     && [ "$(sourced "$T/env/link.env" WG_NFT_FILE)" = "$dir/wg-fed0.nft" ] \
     && [ "$(envval "$T/env/forwarder-carrier.env" WG_NFT_FILE)" = "$dir/forwarder.nft" ] \
     && [ "$(sourced "$T/env/forwarder-carrier.env" WG_NFT_FILE)" = "$dir/forwarder.nft" ] \
     && [ "$(envval "$T/env/forwarder-carrier.env" WG_KEY_FILE)" = "$dir/wg-carry0.key" ] \
     && [ "$(sourced "$T/env/forwarder-carrier.env")" = "$dir/wg-carry0.key" ]; then
    pass "env files: the ruleset and forwarder key paths under '$dir' are written quoted and read back whole by both readers"
  else
    fail "env files: the ruleset or forwarder key paths under '$dir' are not read back whole"; grep -E 'WG_(NFT|KEY)_FILE' "$T/env/"*.env 2>/dev/null || true; tail -15 "$LOG"
  fi
done
NLD="$T/etc
x"
if envfiles "$PS" "$KS" "$NLD"; then
  fail "env files: a ruleset path holding a newline was written"
elif grep -qF "peering_wg_nft_path holds a newline" "$LOG"; then
  pass "env files: a ruleset path holding a newline is refused, naming why"
else
  fail "env files: wrong refusal for a ruleset path holding a newline"; tail -15 "$LOG"
fi
NL="$T/edge
x/wg-fed0.key"
if envfiles "$NL" "$KS"; then
  fail "env files: a key path holding a newline was written"
elif grep -qF "holds a newline" "$LOG"; then
  pass "env files: a key path holding a newline is refused, naming why"
else
  fail "env files: wrong refusal for a key path holding a newline"; tail -15 "$LOG"
fi

if [ "$FAILS" -eq 0 ]; then
  echo "goat-peering-wg test-rotation: PASS"
else
  echo "goat-peering-wg test-rotation: $FAILS FAILED"
  exit 1
fi
