#!/usr/bin/env bash
# test-peering-carrier-keys.sh — ops/ansible/peering-carrier-keys.yml copies the
# carrier public keys into the peering's certificate directory and leaves that
# directory 0700. Phase 8 creates it 0700 and later stores private leaf keys in
# it, so the play must never widen it.
#
# Runs the play on this machine (connection local, no become), with a stub `wg`
# that prints a fixed public key. Needs ansible-playbook and python3.
# usage: ops/ci/test-peering-carrier-keys.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
log() { printf '[peering-carrier-keys] %s\n' "$*"; }
fail() { printf '[peering-carrier-keys] FAIL: %s\n' "$*" >&2; exit 1; }

command -v ansible-playbook >/dev/null || { log "SKIP (ansible-playbook not on PATH)"; exit 0; }

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin" "$T/wgconf" "$T/fwdconf" "$T/edge-pki"
KEY="AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
printf '#!/bin/sh\ncat >/dev/null\necho %s\n' "$KEY" > "$T/bin/wg"
chmod 0755 "$T/bin/wg"
printf 'private\n' > "$T/wgconf/wg-carry0.key"
printf 'private\n' > "$T/fwdconf/wg-carry0.key"
# As phase 8 leaves it: the edge's directory exists, 0700. The forwarder key's
# directory does not exist yet; the play creates it.
chmod 0700 "$T/edge-pki"
PY="$(command -v python3)"

cat > "$T/inventory.ini" <<EOF
[peering_edges]
edge-x ansible_connection=local ansible_python_interpreter=$PY peering_wg_via_forwarder=true peering_wg_carrier_interface=wg-carry0 peering_wg_config_dir=$T/wgconf peering_carrier_edge_key_out=$T/edge-pki/edge-carrier.pub

[peering_forwarders]
fwd-x ansible_connection=local ansible_python_interpreter=$PY peering_forwarder_config_dir=$T/fwdconf

[peering_forwarders:vars]
peering_carrier_forwarder_key_out={"wg-carry0": "$T/fwd-pki/forwarder-carrier.pub"}
EOF

run() {
  PATH="$T/bin:$PATH" ansible-playbook -i "$T/inventory.ini" -e ansible_become=false \
    -e ansible_python_interpreter="$PY" "$ROOT/ops/ansible/peering-carrier-keys.yml" \
    > "$T/play.log" 2>&1 || { cat "$T/play.log"; fail "the play did not run"; }
}
mode() { "$PY" -c 'import os,stat,sys; print(oct(stat.S_IMODE(os.stat(sys.argv[1]).st_mode))[2:])' "$1"; }

for pass in 1 2; do
  run
  [ "$(cat "$T/edge-pki/edge-carrier.pub")" = "$KEY" ] || fail "pass $pass: the edge's carrier key was not copied"
  [ "$(cat "$T/fwd-pki/forwarder-carrier.pub")" = "$KEY" ] || fail "pass $pass: the forwarder's carrier key was not copied"
  [ "$(mode "$T/edge-pki")" = 700 ] || fail "pass $pass: the edge's certificate directory is $(mode "$T/edge-pki"), not 700"
  [ "$(mode "$T/fwd-pki")" = 700 ] || fail "pass $pass: the forwarder key's directory is $(mode "$T/fwd-pki"), not 700"
  log "ok: pass $pass, both keys copied, both directories 0700"
done
log "PASS"
