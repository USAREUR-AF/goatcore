#!/usr/bin/env bash
# generate-relays-azure.sh — Ansible inventory for goat relays on Azure.
#
# The Azure counterpart of generate-relays.sh, for a relay tier built with
# terraform/modules/goat-relay-azure. Same outputs, same TLS-posture contract;
# what differs is how the relays are reached:
#
#   AWS:   ansible_connection=community.aws.aws_ssm, key from Secrets Manager
#   Azure: plain SSH to 127.0.0.1:<port>, where `az network bastion tunnel`
#          forwards each port to one relay's :22 (ops/scripts/relay-tunnels-azure.sh).
#          Key from Key Vault, downloaded to a local file because Bastion's
#          native client cannot use a key held in Key Vault directly.
#
# Usage:
#   ops/ansible/inventory/generate-relays-azure.sh <env> <terraform_dir>
#
#   <env>            Environment identifier; names the inventory, key dir and
#                    relay-env file, and selects the TLS posture.
#   <terraform_dir>  Terraform root (relative to the repo root) exposing:
#                      relays  = { <name> = { public_ip, vm_id, relay_url, region,
#                                  admin_username, ssh_key_secret_name,
#                                  rathole_secret_name } }
#                      bastion = { name, resource_group }
#                      key_vault_name
#
# Environment:
#   RELAY_TUNNEL_BASE_PORT_<ENV_UPPER>  first local tunnel port (default 52200);
#                                       relays take consecutive ports in name order.
#   RELAY_TLS_POSTURE_<ENV_UPPER>       one-run posture override, as in generate-relays.sh.
#
# Outputs:
#   inventory/hosts-<env>.ini   [relay_servers], [goat_relay_hosts:children]
#   /tmp/<env>-ssh-keys/        SSH private keys (dir 0700, files 0600)
#   ops/relay-env-<env>.sh      relay metadata, read by relay-tunnels-azure.sh
#
# Host keys: the inventory checks each relay's SSH host key against
# inventory/<env>-relay-known_hosts (overriding ansible.cfg's
# host_key_checking = false). `relay-tunnels-azure.sh <env> pin` fills it
# through the Bastion tunnels; until then Ansible refuses to connect.
#
# Prerequisites: `az login` to the subscription; the root applied; the env
# declared in relay-tls-posture.sh; the reader may get Key Vault secrets.
#
# Exit codes: 64 usage · 65 missing terraform dir · 66 az not logged in ·
#   68 unreadable terraform output · 70 missing posture map ·
#   71 undeclared posture · 72 invalid posture · 73 key fetch failed.
#
# Test: ops/ansible/inventory/tests/generate-relays-azure-test.sh (hermetic).
# Bash 3.2 portable.

set -euo pipefail

if [ $# -ne 2 ]; then
  echo "Usage: $0 <env> <terraform_dir>" >&2
  exit 64
fi

ENV_NAME="$1"
TF_DIR_RELATIVE="$2"
ENV_UPPER=$(echo "$ENV_NAME" | tr '[:lower:]-' '[:upper:]_')

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ANSIBLE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_DIR="$(cd "$ANSIBLE_DIR/../.." && pwd)"
TF_DIR="$REPO_DIR/$TF_DIR_RELATIVE"
INVENTORY_FILE="$SCRIPT_DIR/hosts-${ENV_NAME}.ini"
KNOWN_HOSTS_FILE="$SCRIPT_DIR/${ENV_NAME}-relay-known_hosts"
KEY_DIR="/tmp/${ENV_NAME}-ssh-keys"
RELAY_ENV_FILE="$REPO_DIR/ops/relay-env-${ENV_NAME}.sh"

BASE_PORT_VAR="RELAY_TUNNEL_BASE_PORT_${ENV_UPPER}"
BASE_PORT="${!BASE_PORT_VAR:-52200}"
case "$BASE_PORT" in
  ''|*[!0-9]*) echo "ERROR: $BASE_PORT_VAR must be a port number, got '$BASE_PORT'" >&2; exit 64 ;;
esac

if [ ! -d "$TF_DIR" ]; then
  echo "ERROR: terraform dir not found: $TF_DIR" >&2
  exit 65
fi

# --- TLS posture: declared per env, never inferred (see generate-relays.sh) --
POSTURE_FILE="$SCRIPT_DIR/relay-tls-posture.sh"
POSTURE_OVERRIDE_VAR="RELAY_TLS_POSTURE_${ENV_UPPER}"
if [ -n "${!POSTURE_OVERRIDE_VAR:-}" ]; then
  POSTURE="${!POSTURE_OVERRIDE_VAR}"
  POSTURE_SOURCE="env override \$$POSTURE_OVERRIDE_VAR"
else
  if [ ! -f "$POSTURE_FILE" ]; then
    echo "ERROR: relay TLS posture map not found: $POSTURE_FILE" >&2
    echo "       Declare this env there, or set $POSTURE_OVERRIDE_VAR for one run." >&2
    exit 70
  fi
  # shellcheck source=/dev/null
  source "$POSTURE_FILE"
  if POSTURE=$(relay_tls_posture_for "$ENV_NAME"); then
    POSTURE_SOURCE="relay-tls-posture.sh [$ENV_NAME]"
  else
    echo "ERROR: env '$ENV_NAME' declares no goat-relay TLS posture in $POSTURE_FILE." >&2
    echo "       There is no default: self-signed relays are rejected by peers (F-188)." >&2
    exit 71
  fi
fi
case "$POSTURE" in
  le-dns01|self-signed) ;;
  *) echo "ERROR: invalid relay TLS posture '$POSTURE' (from $POSTURE_SOURCE)" >&2; exit 72 ;;
esac
echo "==> goat-relay TLS posture: $POSTURE (from $POSTURE_SOURCE)"

# --- Azure login --------------------------------------------------------------
if ! az account show --query id -o tsv >/dev/null 2>&1; then
  echo "ERROR: the Azure CLI is not logged in. Run: az login --tenant <tenant-id>" >&2
  exit 66
fi

# --- Terraform outputs --------------------------------------------------------
# JSON only, parsed by python, so provider chatter on stdout can't leak into a value.
read_output() {
  (cd "$TF_DIR" && terraform output -json "$1" 2>/dev/null) \
    | python3 -c 'import sys,json; print(json.dumps(json.load(sys.stdin)))' 2>/dev/null || true
}

echo "==> Reading outputs from $TF_DIR_RELATIVE"
RELAY_JSON=$(read_output relays)
BASTION_JSON=$(read_output bastion)
KV_JSON=$(read_output key_vault_name)
if [ -z "$RELAY_JSON" ] || [ "$RELAY_JSON" = "{}" ] || [ -z "$BASTION_JSON" ] || [ -z "$KV_JSON" ]; then
  echo "ERROR: could not read the relays, bastion and key_vault_name outputs from $TF_DIR_RELATIVE." >&2
  echo "       Check: terraform init (backend reachable with your az login), then apply." >&2
  exit 68
fi

# One line per relay, sorted by name, tab-separated:
#   name public_ip vm_id relay_url region admin_username ssh_secret rathole_secret
RELAY_ROWS=$(echo "$RELAY_JSON" | python3 -c '
import sys, json
d = json.load(sys.stdin)
keys = ("public_ip", "vm_id", "relay_url", "region", "admin_username",
        "ssh_key_secret_name", "rathole_secret_name")
for name in sorted(d):
    r = d[name]
    print("\t".join([name] + [str(r.get(k, "")) for k in keys]))
')
BASTION_NAME=$(echo "$BASTION_JSON" | python3 -c 'import sys,json; print(json.load(sys.stdin)["name"])')
BASTION_RG=$(echo "$BASTION_JSON" | python3 -c 'import sys,json; print(json.load(sys.stdin)["resource_group"])')
KEY_VAULT=$(echo "$KV_JSON" | python3 -c 'import sys,json; print(json.load(sys.stdin))')

# --- SSH keys from Key Vault --------------------------------------------------
# Relays may share one secret (a key made outside Terraform), so fetch each
# secret once and name the file after the secret.
umask 077
mkdir -p "$KEY_DIR"
chmod 700 "$KEY_DIR"
FETCHED=" "
while IFS="$(printf '\t')" read -r name _ _ _ _ _ secret _; do
  [ -n "$secret" ] || { echo "ERROR: relay $name has no ssh_key_secret_name" >&2; exit 73; }
  case "$FETCHED" in *" $secret "*) continue ;; esac
  key_file="$KEY_DIR/${secret}.pem"
  echo "==> Fetching SSH key $secret from Key Vault $KEY_VAULT"
  if ! az keyvault secret show --vault-name "$KEY_VAULT" --name "$secret" \
        --query value -o tsv > "$key_file" 2>/dev/null || [ ! -s "$key_file" ]; then
    rm -f "$key_file"
    echo "ERROR: could not read secret $secret from $KEY_VAULT (Key Vault Secrets User or Officer needed)" >&2
    exit 73
  fi
  chmod 600 "$key_file"
  FETCHED="$FETCHED$secret "
done <<EOF
$RELAY_ROWS
EOF

# --- Inventory ----------------------------------------------------------------
{
  echo "# Auto-generated by ops/ansible/inventory/generate-relays-azure.sh"
  echo "# env=$ENV_NAME  terraform=$TF_DIR_RELATIVE"
  echo "# DO NOT EDIT — regenerated on every run."
  echo "#"
  echo "# Connection: SSH through Azure Bastion tunnels on 127.0.0.1. Open them first:"
  echo "#   ops/scripts/relay-tunnels-azure.sh $ENV_NAME up"
  echo
  echo "[relay_servers]"
} > "$INVENTORY_FILE"

PORT="$BASE_PORT"
while IFS="$(printf '\t')" read -r name public_ip vm_id relay_url region admin_user secret rathole_secret; do
  # HostKeyAlias keeps each relay's host key apart; they all sit on 127.0.0.1.
  # ansible_host_key_checking=true and StrictHostKeyChecking=yes make the key
  # count: ansible.cfg turns checking off, and then any process listening on a
  # tunnel port could pose as the relay.
  # IdentitiesOnly stops an agent offering its other keys first: sshd's MaxAuthTries
  # (6) is spent before the relay key is tried ("Too many authentication failures").
  echo "relay-${name} ansible_host=127.0.0.1 ansible_port=${PORT} ansible_user=${admin_user} ansible_ssh_private_key_file=${KEY_DIR}/${secret}.pem ansible_host_key_checking=true ansible_ssh_common_args='-o HostKeyAlias=relay-${ENV_NAME}-${name} -o UserKnownHostsFile=${KNOWN_HOSTS_FILE} -o StrictHostKeyChecking=yes -o IdentitiesOnly=yes' azure_vm_id=${vm_id} azure_region=${region} relay_public_ip=${public_ip} rathole_secret_name=${rathole_secret} relay_dns_name=${relay_url} goat_relay_public_ip=${public_ip} goat_relay_advertised_addr=${relay_url}:33073" >> "$INVENTORY_FILE"
  PORT=$((PORT + 1))
done <<EOF
$RELAY_ROWS
EOF

cat >> "$INVENTORY_FILE" <<VARS

[relay_servers:vars]
ansible_python_interpreter=/usr/bin/python3
relay_cloud=azure
relay_key_vault_name=${KEY_VAULT}
VARS

# Posture vars go in [relay_servers:vars], so before any later section header:
# under [goat_relay_hosts:children] they would be read as group names.
if [ "$POSTURE" = "le-dns01" ]; then
  cat >> "$INVENTORY_FILE" <<LE

# Posture le-dns01 ($POSTURE_SOURCE): publicly trusted certificate by DNS-01,
# through Azure DNS (relay_dns01_provider) because port 80 stays closed.
relay_cert_dns01=true
relay_dns01_provider=azure
goat_relay_le_dns01=true
goat_relay_self_signed_cert=false
LE
fi

cat >> "$INVENTORY_FILE" <<CHILDREN

# Every relay this tier builds runs goat-relay.
[goat_relay_hosts:children]
relay_servers
CHILDREN

# --- relay-env file -----------------------------------------------------------
{
  echo "# Relay metadata for env=$ENV_NAME — auto-generated by generate-relays-azure.sh."
  echo
  echo "RELAY_ENV_NAME=\"$ENV_NAME\""
  echo "RELAY_CLOUD=\"azure\""
  printf 'RELAY_NAMES=('
  echo "$RELAY_ROWS" | while IFS="$(printf '\t')" read -r n _; do printf "'%s' " "$n"; done
  echo ")"
  printf 'RELAY_IPS=('
  echo "$RELAY_ROWS" | while IFS="$(printf '\t')" read -r _ ip _; do printf "'%s' " "$ip"; done
  echo ")"
  printf 'RELAY_REGIONS=('
  echo "$RELAY_ROWS" | while IFS="$(printf '\t')" read -r _ _ _ _ rg _; do printf "'%s' " "$rg"; done
  echo ")"
  printf 'RELAY_VM_IDS=('
  echo "$RELAY_ROWS" | while IFS="$(printf '\t')" read -r _ _ vm _; do printf "'%s' " "$vm"; done
  echo ")"
  echo "RELAY_TUNNEL_BASE_PORT=\"$BASE_PORT\""
  echo "BASTION_NAME=\"$BASTION_NAME\""
  echo "BASTION_RESOURCE_GROUP=\"$BASTION_RG\""
  echo "RELAY_KEY_DIR=\"$KEY_DIR\""
  echo "RELAY_KNOWN_HOSTS=\"$KNOWN_HOSTS_FILE\""
  echo "RELAY_TF_DIR=\"$TF_DIR_RELATIVE\""
} > "$RELAY_ENV_FILE"

echo ""
echo "==> Inventory:  $INVENTORY_FILE"
echo "==> Relay env:  $RELAY_ENV_FILE"
if [ ! -s "$KNOWN_HOSTS_FILE" ]; then
  echo "==> No pinned relay host keys yet ($KNOWN_HOSTS_FILE). Ansible refuses the relays until:"
  echo "      ops/scripts/relay-tunnels-azure.sh $ENV_NAME up && ops/scripts/relay-tunnels-azure.sh $ENV_NAME pin"
fi
echo "==> SSH keys:   $KEY_DIR/"
echo ""
echo "Next: ops/scripts/relay-tunnels-azure.sh $ENV_NAME up"
echo "      ansible -i $INVENTORY_FILE relay_servers -m ping"
