#!/usr/bin/env bash
# Hermetic test for ops/scripts/relay-tunnels-azure.sh: stubs az (a wrapper
# whose child holds the port, as the real one is) and ssh-keyscan on PATH.
# Contacts nothing; listens only on 127.0.0.1.
#   ops/ansible/inventory/tests/relay-tunnels-azure-test.sh
# Bash 3.2 portable.

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$HERE/../../../scripts/relay-tunnels-azure.sh"
WORK="$(mktemp -d)"
cleanup() {
  if [ -x "$WORK/ops/scripts/relay-tunnels-azure.sh" ]; then
    XDG_STATE_HOME="$WORK/state" PATH="$WORK/bin:$PATH" "$WORK/ops/scripts/relay-tunnels-azure.sh" azt down >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "ok   $*"; }

mkdir -p "$WORK/ops/scripts" "$WORK/bin" "$WORK/state"
cp "$SCRIPT" "$WORK/ops/scripts/"
BASE=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
# Two consecutive free ports.
until python3 -c 'import socket,sys
for p in (int(sys.argv[1]), int(sys.argv[1])+1):
    s=socket.socket(); s.bind(("127.0.0.1",p)); s.close()' "$BASE" 2>/dev/null; do BASE=$((BASE + 2)); done

cat > "$WORK/ops/relay-env-azt.sh" <<EOF
RELAY_CLOUD="azure"
RELAY_NAMES=('a' 'b')
RELAY_VM_IDS=('/vm/a' '/vm/b')
RELAY_TUNNEL_BASE_PORT="$BASE"
BASTION_NAME="bastion"
BASTION_RESOURCE_GROUP="rg"
RELAY_KNOWN_HOSTS="$WORK/azt-relay-known_hosts"
EOF

# az: like the real one, a shell that runs Python as a child (no exec).
cat > "$WORK/bin/az" <<'EOF'
#!/usr/bin/env bash
port=""
while [ $# -gt 0 ]; do [ "$1" = "--port" ] && port="$2"; shift; done
python3 -c 'import socket,sys,time
s=socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", int(sys.argv[1]))); s.listen(5)
while True:
    c,_ = s.accept(); c.close()' "$port"
EOF
# ssh-keyscan: the key a relay presents is FAKE_KEY_<name>, by port.
cat > "$WORK/bin/ssh-keyscan" <<EOF
#!/usr/bin/env bash
port=""
while [ \$# -gt 0 ]; do [ "\$1" = "-p" ] && port="\$2"; shift; done
if [ "\$port" = "$BASE" ]; then k="\$FAKE_KEY_A"; else k="\$FAKE_KEY_B"; fi
echo "# 127.0.0.1:\$port SSH-2.0-OpenSSH_9.6"
echo "[127.0.0.1]:\$port ssh-ed25519 \$k"
EOF
chmod +x "$WORK/bin/az" "$WORK/bin/ssh-keyscan"

T=(env XDG_STATE_HOME="$WORK/state" PATH="$WORK/bin:$PATH" RELAY_TUNNEL_UP_TIMEOUT=10
   FAKE_KEY_A=AAAAkeyA FAKE_KEY_B=AAAAkeyB "$WORK/ops/scripts/relay-tunnels-azure.sh" azt)
open() { python3 -c 'import socket,sys; s=socket.socket(); s.settimeout(1); sys.exit(0 if s.connect_ex(("127.0.0.1", int(sys.argv[1]))) == 0 else 1)' "$1"; }

# 1. pin needs our own tunnels.
code=0; "${T[@]}" pin >/dev/null 2>&1 || code=$?
[ "$code" -eq 66 ] || fail "pin without tunnels should exit 66, got $code"
pass "pin refuses without tunnels of its own"

# 2. up, then pin records both keys under their aliases.
"${T[@]}" up >/dev/null
if ! { open "$BASE" && open $((BASE + 1)); }; then fail "tunnels not listening after up"; fi
"${T[@]}" pin >/dev/null
grep -qx 'relay-azt-a ssh-ed25519 AAAAkeyA' "$WORK/azt-relay-known_hosts" || fail "relay a not pinned"
grep -qx 'relay-azt-b ssh-ed25519 AAAAkeyB' "$WORK/azt-relay-known_hosts" || fail "relay b not pinned"
pass "up, then pin records each relay's key under its HostKeyAlias"

# 3. Same keys again: no change. A changed key: refused, file untouched.
before=$(cat "$WORK/azt-relay-known_hosts")
"${T[@]}" pin >/dev/null
[ "$(cat "$WORK/azt-relay-known_hosts")" = "$before" ] || fail "re-pinning the same keys changed the file"
code=0
env XDG_STATE_HOME="$WORK/state" PATH="$WORK/bin:$PATH" FAKE_KEY_A=AAAAkeyA FAKE_KEY_B=AAAAimpostor \
  "$WORK/ops/scripts/relay-tunnels-azure.sh" azt pin >/dev/null 2>&1 || code=$?
[ "$code" -eq 67 ] || fail "a changed host key should exit 67, got $code"
[ "$(cat "$WORK/azt-relay-known_hosts")" = "$before" ] || fail "a changed key was written"
pass "a changed host key is refused and not written"

# 4. A port held by something else is refused, not reused.
"${T[@]}" down >/dev/null
sleep 1
python3 -c 'import socket,sys,time; s=socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1",int(sys.argv[1]))); s.listen(1); time.sleep(30)' "$BASE" &
squat=$!
sleep 1
code=0; "${T[@]}" up >/dev/null 2>&1 || code=$?
kill "$squat" 2>/dev/null || true; wait "$squat" 2>/dev/null || true
[ "$code" -eq 66 ] || fail "up with a port in use should exit 66, got $code"
"${T[@]}" down >/dev/null
pass "up refuses a port another process holds"

# 5. down frees the ports: the listener is az's child, not the recorded pid.
"${T[@]}" up >/dev/null
"${T[@]}" down >/dev/null
sleep 1
if open "$BASE" || open $((BASE + 1)); then fail "a tunnel still listens after down"; fi
pass "down stops the tunnels, child processes included"

echo "relay-tunnels-azure: all tests passed"
