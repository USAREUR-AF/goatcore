#!/usr/bin/env bash
# deploy-relay.sh — Full relay deployment pipeline
#
# Orchestrates: Terraform → inventory generation → shared secrets →
# Ansible (VPS bootstrap + NETHOST client config) → verification
#
# Idempotent — safe to re-run. Ansible playbooks skip already-configured hosts.
#
# Usage:
#   ops/relay/deploy-relay.sh          # Deploy/verify all regions
#   ops/relay/deploy-relay.sh fra      # Deploy/verify specific region
#
# Prerequisites:
#   - aws-vault configured (the profile RELAY_AWS_PROFILE names; default the
#     deployment's deploy.aws_profile in ops/goatbuilder/project.yml, else default)
#   - direnv loaded (.envrc + .envrc.local with CLOUDFLARE_API_TOKEN)
#   - terraform init already run in terraform/
#   - ansible installed (brew install ansible)
#
# Values a deployment sets in its ops/env-aliases.d/*.sh
# (ops/scripts/lib/env-aliases.sh), each with an example default:
#   NETHOST_HOST / NETHOST_USER / NETHOST_PASS   the network host to read the
#                          relay auth secret from (default user: deploy.ssh_user
#                          in ops/goatbuilder/project.yml; default host:
#                          NETHOST_MESH_ADDRESS)
#   NETHOST_NAME           the network host's name in the ansible inventory
#   NETBIRD_DOMAIN         the management domain the relays forward
#   GOAT_DEPLOYMENT        names the NetBird nameserver group, <name>-mesh-dns
#   RELAY_AWS_PROFILE      the aws-vault profile the relay tree is applied with

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
TF_DIR="$REPO_DIR/terraform"
ANSIBLE_DIR="$REPO_DIR/ops/ansible"

# Source credentials
if [ -f "$REPO_DIR/.envrc" ]; then
  set -a
  source "$REPO_DIR/.envrc" 2>/dev/null || true
  [ -f "$REPO_DIR/.envrc.local" ] && source "$REPO_DIR/.envrc.local" 2>/dev/null || true
  set +a
fi
# shellcheck source=../scripts/lib/env-aliases.sh
. "$REPO_DIR/ops/scripts/lib/env-aliases.sh"

FACTS="$REPO_DIR/ops/scripts/goatnet-facts.py"
RELAY_AWS_PROFILE="${RELAY_AWS_PROFILE:-$(python3 "$FACTS" deploy aws_profile 2>/dev/null || echo default)}"
AWS_CMD="aws-vault exec $RELAY_AWS_PROFILE --no-session --"

# Relay definitions: name:region
RELAYS=(
  "fra:eu-central-1"
  "isr:il-central-1"
  "mum:ap-south-1"
)

target="${1:-all}"

log() { echo "[$(date +%H:%M:%S)] $*"; }
err() { echo "[$(date +%H:%M:%S)] ERROR: $*" >&2; }

NETHOST_SSH="${NETHOST_USER:-$(python3 "$FACTS" deploy ssh_user 2>/dev/null || echo goat)}@${NETHOST_HOST:-${NETHOST_MESH_ADDRESS:-100.127.1.23}}"
NETHOST_INVENTORY_NAME="${NETHOST_NAME:-nethost}"
MGMT_DOMAIN="${NETBIRD_DOMAIN:-netbird.legacy.example}"
NS_GROUP_PREFIX="${GOAT_DEPLOYMENT:-goat}"
SHARED_TURN_SECRET=""
NB_RELAY_AUTH_SECRET=""
RELAY_OUTPUT=""

# ============================================================================
# Step 1: Terraform
# ============================================================================
step_terraform() {
  log "=== Step 1: Terraform (infrastructure) ==="
  cd "$TF_DIR"

  log "Running terraform plan..."
  local plan_output
  plan_output=$($AWS_CMD terraform plan -input=false -no-color 2>&1)
  local plan_summary
  plan_summary=$(echo "$plan_output" | grep '^Plan:' || echo "No changes.")

  if echo "$plan_summary" | grep -q "No changes"; then
    log "Infrastructure up to date."
  else
    log "$plan_summary"
    log "Applying..."
    $AWS_CMD terraform apply -input=false -auto-approve -no-color 2>&1 | tail -5
  fi

  # Extract relay info from outputs
  log "Reading relay outputs..."
  RELAY_OUTPUT=$($AWS_CMD terraform output -json relays 2>/dev/null)
}

# ============================================================================
# Step 2: Generate Ansible inventory
# ============================================================================
step_inventory() {
  log "=== Step 2: Generate Ansible inventory ==="
  "$ANSIBLE_DIR/inventory/generate.sh"
}

# ============================================================================
# Step 3: Resolve shared secrets
# ============================================================================
get_shared_turn_secret() {
  log "=== Step 3a: Shared TURN secret ==="

  # Try reading from first relay's Secrets Manager entry
  for entry in "${RELAYS[@]}"; do
    IFS=":" read -r name region <<< "$entry"
    local secret_name
    secret_name=$(echo "$RELAY_OUTPUT" | python3 -c "import sys,json; print(json.load(sys.stdin)['$name']['rathole_secret_name'])" 2>/dev/null) || continue

    local creds
    creds=$($AWS_CMD aws secretsmanager get-secret-value \
      --region "$region" \
      --secret-id "$secret_name" \
      --query SecretString --output text 2>/dev/null || echo "")

    if [ -n "$creds" ] && [ "$creds" != "{}" ]; then
      local ts
      ts=$(echo "$creds" | python3 -c "import sys,json; print(json.load(sys.stdin).get('turn_secret',''))" 2>/dev/null)
      if [ -n "$ts" ]; then
        SHARED_TURN_SECRET="$ts"
        log "Using existing TURN secret from $name"
        return
      fi
    fi
  done

  # None found — generate new shared secret
  SHARED_TURN_SECRET=$(openssl rand -hex 32)
  log "Generated new shared TURN secret"
}

get_relay_auth_secret() {
  log "=== Step 3b: Netbird relay auth secret ==="

  # Build SSH command
  local ssh_cmd
  if [ -n "${NETHOST_PASS:-}" ]; then
    ssh_cmd="sshpass -p '$NETHOST_PASS' ssh -o StrictHostKeyChecking=no -o ConnectTimeout=10 $NETHOST_SSH"
  else
    ssh_cmd="ssh -o StrictHostKeyChecking=no -o ConnectTimeout=10 $NETHOST_SSH"
  fi

  # Read authSecret from Netbird config.yaml on NETHOST
  local secret
  secret=$(eval "$ssh_cmd 'sudo python3 -c \"
import yaml
for p in [\\\"/opt/netbird/config.yaml\\\"]:
    try:
        with open(p) as f:
            print(yaml.safe_load(f).get(\\\"server\\\",{}).get(\\\"authSecret\\\",\\\"\\\"))
            break
    except: pass
\"'" 2>/dev/null | tr -d '[:space:]')

  if [ -n "$secret" ]; then
    NB_RELAY_AUTH_SECRET="$secret"
    log "Read relay auth secret from NETHOST config.yaml"
  else
    log "Could not read relay auth secret from NETHOST — relay containers will not be deployed"
  fi
}

# ============================================================================
# Step 4: Ansible — bootstrap VPS relays + configure NETHOST
# ============================================================================
step_ansible() {
  log "=== Step 4: Ansible deployment ==="
  cd "$ANSIBLE_DIR"

  # Build extra-vars
  local extra_vars=""
  [ -n "$SHARED_TURN_SECRET" ] && extra_vars="$extra_vars -e shared_turn_secret=$SHARED_TURN_SECRET"
  [ -n "$NB_RELAY_AUTH_SECRET" ] && extra_vars="$extra_vars -e nb_relay_auth_secret=$NB_RELAY_AUTH_SECRET"

  # Build limit flag
  local limit_flag=""
  if [ "$target" != "all" ]; then
    limit_flag="--limit relay-${target},${NETHOST_INVENTORY_NAME}"
  fi

  log "Running site.yml..."
  ansible-playbook site.yml $extra_vars $limit_flag
}

# ============================================================================
# Step 5: Configure Netbird DNS nameserver group (API)
# ============================================================================
step_dns_nameserver_group() {
  log "=== Step 5: Netbird DNS nameserver group ==="

  if [ -z "${NETBIRD_API_TOKEN:-}" ]; then
    log "NETBIRD_API_TOKEN not set — skipping nameserver group config"
    return
  fi

  local mgmt_url="https://$MGMT_DOMAIN"

  # Collect VPS relay IPs
  local ns_entries=""
  local comma=""
  for entry in "${RELAYS[@]}"; do
    IFS=":" read -r name region <<< "$entry"
    local ip
    ip=$(echo "$RELAY_OUTPUT" | python3 -c "import sys,json; print(json.load(sys.stdin)['$name']['public_ip'])" 2>/dev/null) || continue
    ns_entries="${ns_entries}${comma}{\"ip\":\"$ip\",\"ns_type\":\"udp\",\"port\":53}"
    comma=","
  done

  # Find the "All" group ID
  local all_group_id
  all_group_id=$(curl -sk -H "Authorization: Token $NETBIRD_API_TOKEN" \
    "$mgmt_url/api/groups" 2>/dev/null \
    | python3 -c "import sys,json; groups=json.load(sys.stdin); print(next(g['id'] for g in groups if g['name']=='All'))" 2>/dev/null)

  if [ -z "$all_group_id" ]; then
    err "Could not find 'All' group ID — configure nameserver group manually"
    return
  fi

  # Check if nameserver group already exists
  local existing
  existing=$(curl -sk -H "Authorization: Token $NETBIRD_API_TOKEN" \
    "$mgmt_url/api/dns/nameservers" 2>/dev/null \
    | python3 -c "
import sys,json
groups=json.load(sys.stdin)
matches=[g for g in groups if '$NS_GROUP_PREFIX' in g.get('name','').lower()]
print(matches[0]['id'] if matches else '')
" 2>/dev/null)

  local payload="{
    \"name\": \"${NS_GROUP_PREFIX}-mesh-dns\",
    \"description\": \"Mesh DNS — resolves *.${MGMT_DOMAIN} via VPS relay dnsmasq.\",
    \"nameservers\": [$ns_entries],
    \"domains\": [\"${MGMT_DOMAIN}\", \"${MGMT_DOMAIN#*.}\"],
    \"groups\": [\"$all_group_id\"],
    \"enabled\": true,
    \"primary\": false,
    \"search_domains_enabled\": true
  }"

  local result
  if [ -n "$existing" ]; then
    log "Updating existing nameserver group ($existing)..."
    result=$(curl -sk -X PUT -H "Authorization: Token $NETBIRD_API_TOKEN" \
      -H "Content-Type: application/json" \
      "$mgmt_url/api/dns/nameservers/$existing" \
      -d "$payload" 2>&1)
  else
    log "Creating nameserver group..."
    result=$(curl -sk -X POST -H "Authorization: Token $NETBIRD_API_TOKEN" \
      -H "Content-Type: application/json" \
      "$mgmt_url/api/dns/nameservers" \
      -d "$payload" 2>&1)
  fi

  if echo "$result" | python3 -c "import sys,json; json.load(sys.stdin)['id']" >/dev/null 2>&1; then
    log "Nameserver group configured ($(echo "$result" | python3 -c "import sys,json; ns=json.load(sys.stdin)['nameservers']; print(f'{len(ns)} servers')"))"
  else
    err "Nameserver group config failed: $result"
  fi
}

# ============================================================================
# Step 6: Verify end-to-end
# ============================================================================
verify_relays() {
  log "=== Step 5: Verification ==="

  local all_ok=true
  for entry in "${RELAYS[@]}"; do
    IFS=":" read -r name region <<< "$entry"
    [ "$target" != "all" ] && [ "$target" != "$name" ] && continue

    local public_ip
    public_ip=$(echo "$RELAY_OUTPUT" | python3 -c "import sys,json; print(json.load(sys.stdin)['$name']['public_ip'])" 2>/dev/null)

    # Test management forwarding
    local mgmt_result
    mgmt_result=$(curl -sk --connect-timeout 5 --resolve "$MGMT_DOMAIN:443:$public_ip" \
      "https://$MGMT_DOMAIN/api/accounts" 2>/dev/null)

    local mgmt_ok="FAIL"
    echo "$mgmt_result" | grep -q "401" && mgmt_ok="OK"

    # Test STUN
    local stun_ok="FAIL"
    local stun_result
    stun_result=$(echo -ne '\x00\x01\x00\x00\x21\x12\xa4\x42\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c' \
      | nc -u -w 2 "$public_ip" 3478 2>/dev/null | xxd -p | head -1)
    [ -n "$stun_result" ] && echo "$stun_result" | grep -q "^0101" && stun_ok="OK"

    # Test rathole control
    local ctl_ok="FAIL"
    nc -z -w 3 "$public_ip" 2333 2>/dev/null && ctl_ok="OK"

    # Test Netbird relay
    local relay_ok="SKIP"
    if [ -n "$NB_RELAY_AUTH_SECRET" ]; then
      relay_ok="FAIL"
      nc -z -w 3 "$public_ip" 33073 2>/dev/null && relay_ok="OK"
    fi

    # Test DNS (dnsmasq)
    local dns_ok="FAIL"
    local dns_result
    dns_result=$(dig +short "@$public_ip" "airdefenders.$MGMT_DOMAIN" 2>/dev/null)
    [ -n "$dns_result" ] && dns_ok="OK"

    printf "  %-4s %-15s  mgmt=%s  stun=%s  rathole=%s  relay=%s  dns=%s\n" "$name" "$public_ip" "$mgmt_ok" "$stun_ok" "$ctl_ok" "$relay_ok" "$dns_ok"

    [ "$mgmt_ok" = "FAIL" ] || [ "$stun_ok" = "FAIL" ] || [ "$ctl_ok" = "FAIL" ] && all_ok=false
    [ "$relay_ok" = "FAIL" ] && all_ok=false
  done

  $all_ok && log "All relays operational." || err "Some checks failed."
}

# ============================================================================
# Main
# ============================================================================
log "=== goat Relay Deployment ==="
log "Target: $target"
echo ""

step_terraform
step_inventory
get_shared_turn_secret
get_relay_auth_secret
step_ansible
step_dns_nameserver_group
verify_relays

log "=== Deployment complete ==="
