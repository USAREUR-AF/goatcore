#!/usr/bin/env bash
# Hermetic test for generate-relays-azure.sh: stubs terraform and az on PATH,
# runs the generator in a scratch copy of the tree, checks the outputs.
#   ops/ansible/inventory/tests/generate-relays-azure-test.sh
# Bash 3.2 portable.

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
GENERATOR="$HERE/../generate-relays-azure.sh"
WORK="$(mktemp -d)"
KEY_DIR=""
trap 'rm -rf "$WORK"; if [ -n "$KEY_DIR" ]; then rm -rf "$KEY_DIR"; fi' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "ok   $*"; }

# Scratch tree: <WORK>/ops/ansible/inventory/<generator> and <WORK>/tf.
mkdir -p "$WORK/ops/ansible/inventory" "$WORK/tf" "$WORK/bin"
cp "$GENERATOR" "$WORK/ops/ansible/inventory/"
GEN="$WORK/ops/ansible/inventory/generate-relays-azure.sh"
cat > "$WORK/ops/ansible/inventory/relay-tls-posture.sh" <<'EOF'
relay_tls_posture_for() {
  case "$1" in
    azt) echo le-dns01 ;;
    *) return 1 ;;
  esac
}
EOF

cat > "$WORK/bin/terraform" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  "output -json relays") cat <<'J'
{"b":{"public_ip":"192.0.2.2","vm_id":"/vm/b","relay_url":"goat-relay-b.goat.example","region":"northeurope","admin_username":"ubuntu","ssh_key_secret_name":"shared-ssh","rathole_secret_name":"rh-b"},
 "a":{"public_ip":"192.0.2.1","vm_id":"/vm/a","relay_url":"goat-relay-a.goat.example","region":"northeurope","admin_username":"ubuntu","ssh_key_secret_name":"shared-ssh","rathole_secret_name":"rh-a"}}
J
  ;;
  "output -json bastion") echo '{"name":"bas","resource_group":"rg-relay"}' ;;
  "output -json key_vault_name") echo '"kv-test"' ;;
  *) exit 1 ;;
esac
EOF

cat > "$WORK/bin/az" <<EOF
#!/usr/bin/env bash
echo "\$*" >> "$WORK/az.calls"
case "\$1 \$2" in
  "account show") echo sub ;;
  "keyvault secret") printf -- '-----BEGIN OPENSSH PRIVATE KEY-----\nfake\n-----END OPENSSH PRIVATE KEY-----\n' ;;
  *) exit 1 ;;
esac
EOF
chmod +x "$WORK/bin/terraform" "$WORK/bin/az" "$GEN"
export PATH="$WORK/bin:$PATH"

# 1. Undeclared env: refuses before any cloud call.
set +e
"$GEN" undeclared tf >/dev/null 2>&1; code=$?
set -e
[ "$code" -eq 71 ] || fail "undeclared env should exit 71, got $code"
[ ! -f "$WORK/az.calls" ] || fail "undeclared env must not call az"
pass "undeclared posture exits 71 before any az call"

# 2. Happy path.
RELAY_TUNNEL_BASE_PORT_AZT=52300 "$GEN" azt tf >/dev/null
INV="$WORK/ops/ansible/inventory/hosts-azt.ini"
ENVF="$WORK/ops/relay-env-azt.sh"
KEY_DIR=$(sed -n 's/^RELAY_KEY_DIR="\(.*\)"$/\1/p' "$ENVF")
[ -d "$KEY_DIR" ] || fail "private key directory missing"
if ! { [ -f "$INV" ] && [ -f "$ENVF" ]; }; then fail "outputs not written"; fi

grep -q '^relay-a ansible_host=127.0.0.1 ansible_port=52300 ' "$INV" || fail "relay-a not on 52300"
grep -q '^relay-b ansible_host=127.0.0.1 ansible_port=52301 ' "$INV" || fail "relay-b not on 52301"
grep -q "HostKeyAlias=relay-azt-a" "$INV" || fail "missing HostKeyAlias"
grep -q "IdentitiesOnly=yes'" "$INV" || fail "missing IdentitiesOnly (an agent with many keys exhausts MaxAuthTries)"
# ansible.cfg sets host_key_checking = false; each relay line must turn it back on.
[ "$(grep -c '^relay-.* ansible_host_key_checking=true .*StrictHostKeyChecking=yes' "$INV")" -eq 2 ] \
  || fail "a relay line does not check its host key"
grep -q "UserKnownHostsFile=$WORK/ops/ansible/inventory/azt-relay-known_hosts " "$INV" \
  || fail "relay host keys not read from inventory/azt-relay-known_hosts"
grep -q 'goat_relay_advertised_addr=goat-relay-a.goat.example:33073' "$INV" || fail "missing advertised addr"
grep -q '^relay_dns01_provider=azure$' "$INV" || fail "le-dns01 must emit the azure DNS-01 provider"
grep -q 'aws' "$INV" && fail "inventory mentions aws"
# Each key=value must sit in a :vars section; under :children it is read as a group name.
python3 - "$INV" <<'PY' || fail "a variable sits outside a :vars section"
import sys
section = None
for line in open(sys.argv[1]):
    line = line.strip()
    if not line or line.startswith("#"):
        continue
    if line.startswith("["):
        section = line
    elif "=" in line and " " not in line.split("=", 1)[0]:
        if not (section or "").endswith(":vars]"):
            sys.exit("%s in %s" % (line, section))
PY
pass "inventory: ports in name order, host keys checked against a pinned file, posture vars, no AWS"

[ "$(grep -c 'keyvault secret show' "$WORK/az.calls")" -eq 1 ] || fail "a shared secret must be fetched once"
[ "$(stat -c %a "$KEY_DIR/shared-ssh.pem" 2>/dev/null || stat -f %Lp "$KEY_DIR/shared-ssh.pem")" = "600" ] || fail "key file not 0600"
pass "shared key fetched once, written 0600"

# shellcheck source=/dev/null
( source "$ENVF"
  if ! { [ "${RELAY_NAMES[0]}" = "a" ] && [ "${RELAY_NAMES[1]}" = "b" ]; }; then fail "relay-env names"; fi
  [ "${RELAY_VM_IDS[1]}" = "/vm/b" ] || fail "relay-env vm ids"
  if ! { [ "$BASTION_NAME" = "bas" ] && [ "$RELAY_CLOUD" = "azure" ] && [ "$RELAY_TUNNEL_BASE_PORT" = "52300" ]; }; then fail "relay-env bastion/cloud/port"; fi
)
pass "relay-env file round-trips"

# 3. A key that can't be read fails closed.
sed -i.bak 's/"keyvault secret") printf.*/"keyvault secret") exit 1 ;;/' "$WORK/bin/az"
rm -rf /tmp/azt-ssh-keys
set +e
"$GEN" azt tf >/dev/null 2>&1; code=$?
set -e
[ "$code" -eq 73 ] || fail "unreadable key should exit 73, got $code"
pass "unreadable key exits 73"

echo "all passed"
