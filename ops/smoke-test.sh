#!/usr/bin/env bash
# smoke-test.sh — Distributed network and service diagnostic harness
#
# Probes every path, component, and service in the goat mesh infrastructure.
# Produces a structured status matrix suitable for terminal display or dashboard ingestion.
#
# Usage:
#   ops/smoke-test.sh                  # Full assay (all checks)
#   ops/smoke-test.sh --quick          # Fast checks only (no SSM, no SSH)
#   ops/smoke-test.sh --json           # JSON output for dashboard/logging
#   ops/smoke-test.sh --category dns   # Run specific category only
#
# Categories: dns, relay, stun, management, nethost, mesh, all (the network
# host's category, and its check names, carry NETHOST_NAME when it is set)
#
# Exit codes:
#   0 = all checks passed
#   1 = one or more checks failed
#   2 = script error
#
# Results are appended to ops/smoke-test-results.jsonl (one JSON line per run)
# for historical tracking and local dashboard consumption.
#
# The relays (names, addresses, regions, instances) come from ops/relay-env.sh.
# Values a deployment sets in its ops/env-aliases.d/*.sh
# (ops/scripts/lib/env-aliases.sh), each with an example default:
#   NETHOST_HOST / NETHOST_USER  the network host to check over the mesh (default
#                        host NETHOST_MESH_ADDRESS; default user deploy.ssh_user
#                        in ops/goatbuilder/project.yml)
#   NETBIRD_DOMAIN       the management domain
#   RELAY_DNS_NAMES      the relays' public names, space-separated
#   RELAY_AWS_PROFILE    the aws-vault profile the relays' SSM checks run with
#                        (default deploy.aws_profile, else default)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
RESULTS_FILE="$SCRIPT_DIR/smoke-test-results.jsonl"

# Source credentials (direnv functions stubbed for direct sourcing)
# Operator credential files are selected at runtime, outside the tracked tree.
# shellcheck source=/dev/null
source_env_if_exists() { [ -f "$1" ] && source "$1" 2>/dev/null || true; }
if [ -f "$REPO_DIR/.envrc" ]; then
  set -a
  source "$REPO_DIR/.envrc" 2>/dev/null || true
  [ -f "$REPO_DIR/.envrc.local" ] && source "$REPO_DIR/.envrc.local" 2>/dev/null || true
  set +a
fi
# shellcheck source=scripts/lib/env-aliases.sh
. "$SCRIPT_DIR/scripts/lib/env-aliases.sh"

# --- Config ---
source "$SCRIPT_DIR/relay-env.sh"
RELAYS=()
for i in "${!RELAY_NAMES[@]}"; do
  RELAYS+=("${RELAY_NAMES[$i]}:${RELAY_IPS[$i]}:${RELAY_REGIONS[$i]}:${RELAY_INSTANCE_IDS[$i]}")
done
FACTS="$SCRIPT_DIR/scripts/goatnet-facts.py"
NETHOST_MESH_IP="${NETHOST_HOST:-${NETHOST_MESH_ADDRESS:-100.127.1.23}}"
NETHOST_SSH_USER="${NETHOST_USER:-$(python3 "$FACTS" deploy ssh_user 2>/dev/null || echo goat)}"
NETBIRD_DOMAIN="${NETBIRD_DOMAIN:-netbird.legacy.example}"
read -r -a RELAY_DNS_NAMES <<< "${RELAY_DNS_NAMES:-relay.legacy.example relay-rlb.legacy.example relay-rlc.legacy.example}"
RELAY_AWS_PROFILE="${RELAY_AWS_PROFILE:-$(python3 "$FACTS" deploy aws_profile 2>/dev/null || echo default)}"
NETHOST_CATEGORY="${NETHOST_NAME:-nethost}"

# --- Parse args ---
MODE="full"
OUTPUT="terminal"
CATEGORY="all"
while [[ $# -gt 0 ]]; do
  case $1 in
    --quick) MODE="quick"; shift ;;
    --json) OUTPUT="json"; shift ;;
    --category) CATEGORY="$2"; shift 2 ;;
    --help|-h) echo "Usage: $0 [--quick] [--json] [--category dns|relay|stun|management|$NETHOST_CATEGORY|mesh|all]"; exit 0 ;;
    *) echo "Unknown option: $1"; exit 2 ;;
  esac
done

# --- State ---
declare -a CHECK_RESULTS=()
PASS_COUNT=0
FAIL_COUNT=0
WARN_COUNT=0
RUN_ID=$(date +%Y%m%dT%H%M%S)
RUN_START=$(date +%s)

# --- Helpers ---
record() {
  local category="$1" name="$2" status="$3" detail="${4:-}"
  local latency="${5:-}"
  # Sanitize detail: strip control chars, collapse whitespace, escape quotes
  detail=$(printf '%s' "$detail" | tr -d '\000-\011\013-\037' | tr -s ' ' | sed 's/"/\\"/g')
  # Tab-delimited internal format (avoids JSON parse fragility)
  CHECK_RESULTS+=("${category}	${name}	${status}	${detail}	${latency}")
  case $status in
    PASS) PASS_COUNT=$((PASS_COUNT + 1)) ;;
    FAIL) FAIL_COUNT=$((FAIL_COUNT + 1)) ;;
    WARN) WARN_COUNT=$((WARN_COUNT + 1)) ;;
  esac
}

measure() {
  # Run command, capture exit code and duration in ms
  local start
  start=$(python3 -c "import time; print(int(time.time()*1000))") || return 1
  eval "$1" >/dev/null 2>&1
  local rc=$?
  local end
  end=$(python3 -c "import time; print(int(time.time()*1000))") || return 1
  local ms=$((end - start))
  echo "$rc:$ms"
}

# ===========================================================================
# DNS checks
# ===========================================================================
check_dns() {
  local category="dns"

  # Management domain resolves
  local ips
  ips=$(dig +short "$NETBIRD_DOMAIN" A 2>/dev/null | sort | tr '\n' ',')
  if [ -n "$ips" ]; then
    local count
    count=$(echo "$ips" | tr ',' '\n' | grep -c '.')
    record "$category" "netbird-dns-resolve" "PASS" "${count} A records: ${ips%,}"
  else
    record "$category" "netbird-dns-resolve" "FAIL" "No A records"
  fi

  # Check each relay DNS name
  for dns_name in "${RELAY_DNS_NAMES[@]}"; do
    local ip
    ip=$(dig +short "$dns_name" A 2>/dev/null | head -1)
    if [ -n "$ip" ]; then
      record "$category" "relay-dns:$dns_name" "PASS" "$ip"
    else
      record "$category" "relay-dns:$dns_name" "FAIL" "No A record"
    fi
  done

  # Verify round-robin has every relay's IP
  local expected_ips="${RELAY_IPS[*]}"
  for eip in $expected_ips; do
    if echo "$ips" | grep -q "$eip"; then
      record "$category" "round-robin:$eip" "PASS" "in pool"
    else
      record "$category" "round-robin:$eip" "FAIL" "missing from pool"
    fi
  done
}

# ===========================================================================
# Relay checks (rathole control channel + HTTPS forwarding)
# ===========================================================================
check_relay() {
  local category="relay"

  for entry in "${RELAYS[@]}"; do
    IFS=":" read -r name ip region instance_id <<< "$entry"

    # Rathole control channel (TCP 2333)
    local result
    result=$(measure "nc -z -w 3 $ip 2333")
    IFS=":" read -r rc ms <<< "$result"
    if [ "$rc" = "0" ]; then
      record "$category" "rathole-ctl:$name" "PASS" "TCP 2333 open" "$ms"
    else
      record "$category" "rathole-ctl:$name" "FAIL" "TCP 2333 unreachable" "$ms"
    fi

    # Management forwarding (HTTPS 443 via relay)
    local start
    start=$(python3 -c "import time; print(int(time.time()*1000))")
    local mgmt_result
    mgmt_result=$(curl -sk --connect-timeout 5 --max-time 10 \
      --resolve "$NETBIRD_DOMAIN:443:$ip" \
      "https://$NETBIRD_DOMAIN/api/accounts" 2>/dev/null || echo "")
    local end
    end=$(python3 -c "import time; print(int(time.time()*1000))")
    local ms=$((end - start))

    if echo "$mgmt_result" | grep -q '"code":401'; then
      record "$category" "mgmt-fwd:$name" "PASS" "401 (management reachable)" "$ms"
    elif echo "$mgmt_result" | grep -q '"code"'; then
      record "$category" "mgmt-fwd:$name" "WARN" "unexpected response" "$ms"
    else
      record "$category" "mgmt-fwd:$name" "FAIL" "no response" "$ms"
    fi
  done
}

# ===========================================================================
# STUN checks (Coturn on each VPS)
# ===========================================================================
check_stun() {
  local category="stun"

  for entry in "${RELAYS[@]}"; do
    IFS=":" read -r name ip region instance_id <<< "$entry"

    # UDP 3478 port open
    local result
    result=$(measure "nc -u -z -w 3 $ip 3478")
    IFS=":" read -r rc ms <<< "$result"

    # STUN Binding Request (RFC 5389)
    local start
    start=$(python3 -c "import time; print(int(time.time()*1000))")
    local stun_response
    stun_response=$(echo -ne '\x00\x01\x00\x00\x21\x12\xa4\x42\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c' \
      | nc -u -w 2 "$ip" 3478 2>/dev/null | xxd -p | head -1)
    local end
    end=$(python3 -c "import time; print(int(time.time()*1000))")
    local ms=$((end - start))

    if [ -n "$stun_response" ] && echo "$stun_response" | grep -q "^0101"; then
      record "$category" "stun-binding:$name" "PASS" "valid response" "$ms"
    elif [ -n "$stun_response" ]; then
      record "$category" "stun-binding:$name" "WARN" "response but unexpected format" "$ms"
    else
      record "$category" "stun-binding:$name" "FAIL" "no response" "$ms"
    fi
  done
}

# ===========================================================================
# Management API checks
# ===========================================================================
check_management() {
  local category="management"

  # Direct management API (round-robin)
  local start
  start=$(python3 -c "import time; print(int(time.time()*1000))")
  local result
  result=$(curl -sk --connect-timeout 5 --max-time 10 \
    "https://$NETBIRD_DOMAIN/api/accounts" 2>/dev/null || echo "")
  local end
  end=$(python3 -c "import time; print(int(time.time()*1000))")
  local ms=$((end - start))

  if echo "$result" | grep -q '"code":401'; then
    record "$category" "api-roundrobin" "PASS" "401 via round-robin" "$ms"
  else
    record "$category" "api-roundrobin" "FAIL" "no valid response" "$ms"
  fi

  # TLS certificate validity
  local cert_info
  cert_info=$(echo | openssl s_client -connect "$NETBIRD_DOMAIN:443" -servername "$NETBIRD_DOMAIN" 2>/dev/null \
    | openssl x509 -noout -dates 2>/dev/null || echo "")
  if [ -n "$cert_info" ]; then
    local expiry
    expiry=$(echo "$cert_info" | grep 'notAfter' | cut -d= -f2)
    local expiry_epoch
    expiry_epoch=$(date -j -f "%b %d %T %Y %Z" "$expiry" +%s 2>/dev/null || date -d "$expiry" +%s 2>/dev/null || echo "0")
    local now_epoch
    now_epoch=$(date +%s)
    local days_left=$(( (expiry_epoch - now_epoch) / 86400 ))

    if [ "$days_left" -gt 14 ]; then
      record "$category" "tls-cert" "PASS" "${days_left}d until expiry"
    elif [ "$days_left" -gt 0 ]; then
      record "$category" "tls-cert" "WARN" "${days_left}d until expiry — renew soon"
    else
      record "$category" "tls-cert" "FAIL" "expired"
    fi
  else
    record "$category" "tls-cert" "WARN" "could not check certificate"
  fi
}

# ===========================================================================
# NETHOST checks (via SSH over mesh)
# ===========================================================================
check_nethost() {
  local category="$NETHOST_CATEGORY"

  # SSH connectivity via mesh IP
  local result
  result=$(measure "ssh -o ConnectTimeout=5 -o BatchMode=yes ${NETHOST_SSH_USER}@$NETHOST_MESH_IP 'echo ok'")
  IFS=":" read -r rc ms <<< "$result"
  if [ "$rc" = "0" ]; then
    record "$category" "ssh-mesh" "PASS" "SSH via $NETHOST_MESH_IP" "$ms"
  else
    record "$category" "ssh-mesh" "FAIL" "SSH unreachable" "$ms"
    return  # Can't do further NETHOST checks
  fi

  # Docker containers
  local containers
  containers=$(ssh -o ConnectTimeout=5 -o BatchMode=yes "${NETHOST_SSH_USER}@$NETHOST_MESH_IP" \
    'docker ps --format "{{.Names}}:{{.Status}}" 2>/dev/null' 2>/dev/null || echo "")

  for expected in netbird-server netbird-dashboard netbird-traefik netbird-postgres; do
    local status
    status=$(echo "$containers" | grep "^${expected}:" | cut -d: -f2-)
    if echo "$status" | grep -qi "up"; then
      record "$category" "container:$expected" "PASS" "$status"
    elif [ -n "$status" ]; then
      record "$category" "container:$expected" "WARN" "$status"
    else
      record "$category" "container:$expected" "FAIL" "not running"
    fi
  done

  # Rathole clients
  local client_status
  client_status=$(ssh -o ConnectTimeout=5 -o BatchMode=yes "${NETHOST_SSH_USER}@$NETHOST_MESH_IP" \
    'systemctl list-units "rathole-client@*" --no-pager --no-legend 2>/dev/null' 2>/dev/null || echo "")

  for relay_name in fra isr mum; do
    if echo "$client_status" | grep -q "rathole-client@${relay_name}.service.*running"; then
      record "$category" "rathole-client:$relay_name" "PASS" "active"
    else
      record "$category" "rathole-client:$relay_name" "FAIL" "not running"
    fi
  done

  # Disk usage
  local disk_pct
  disk_pct=$(ssh -o ConnectTimeout=5 -o BatchMode=yes "${NETHOST_SSH_USER}@$NETHOST_MESH_IP" \
    "df / --output=pcent | tail -1 | tr -d ' %'" 2>/dev/null || echo "")
  if [ -n "$disk_pct" ] && [ "$disk_pct" -lt 80 ] 2>/dev/null; then
    record "$category" "disk-usage" "PASS" "${disk_pct}%"
  elif [ -n "$disk_pct" ] && [ "$disk_pct" -lt 95 ] 2>/dev/null; then
    record "$category" "disk-usage" "WARN" "${disk_pct}%"
  elif [ -n "$disk_pct" ]; then
    record "$category" "disk-usage" "FAIL" "${disk_pct}% — critical"
  fi
}

# ===========================================================================
# Mesh checks (Netbird peer connectivity)
# ===========================================================================
check_mesh() {
  local category="mesh"

  # Local Netbird status
  if command -v netbird >/dev/null 2>&1; then
    local nb_status
    nb_status=$(netbird status 2>/dev/null || echo "")

    if echo "$nb_status" | grep -qi "connected"; then
      record "$category" "local-netbird" "PASS" "connected"
    elif echo "$nb_status" | grep -qi "connecting"; then
      record "$category" "local-netbird" "WARN" "connecting..."
    else
      record "$category" "local-netbird" "FAIL" "not connected"
    fi

    # Peer count
    local peer_count
    peer_count=$(echo "$nb_status" | grep -c "Peer:" 2>/dev/null || echo "0")
    local connected_peers
    connected_peers=$(echo "$nb_status" | grep -c "Connected" 2>/dev/null || echo "0")
    record "$category" "peers" "PASS" "${connected_peers}/${peer_count} connected"

    # Ping NETHOST via mesh
    local result
    result=$(measure "ping -c 1 -W 3 $NETHOST_MESH_IP")
    IFS=":" read -r rc ms <<< "$result"
    if [ "$rc" = "0" ]; then
      record "$category" "mesh-ping-$NETHOST_CATEGORY" "PASS" "reachable" "$ms"
    else
      record "$category" "mesh-ping-$NETHOST_CATEGORY" "FAIL" "unreachable" "$ms"
    fi
  else
    record "$category" "local-netbird" "WARN" "netbird CLI not installed"
  fi
}

# ===========================================================================
# VPS deep checks (via SSM — full mode only)
# ===========================================================================
check_vps_deep() {
  [ "$MODE" = "quick" ] && return

  local category="vps-deep"

  if ! command -v aws >/dev/null 2>&1; then
    record "$category" "ssm-available" "WARN" "aws CLI not installed"
    return
  fi

  for entry in "${RELAYS[@]}"; do
    IFS=":" read -r name ip region instance_id <<< "$entry"

    # Check SSM agent
    local ssm_status
    ssm_status=$(aws-vault exec "$RELAY_AWS_PROFILE" --no-session -- \
      aws ssm describe-instance-information --region "$region" \
      --filters "Key=InstanceIds,Values=$instance_id" \
      --query "InstanceInformationList[].PingStatus" --output text 2>/dev/null || echo "")

    if [ "$ssm_status" = "Online" ]; then
      record "$category" "ssm-agent:$name" "PASS" "online"
    else
      record "$category" "ssm-agent:$name" "FAIL" "status: ${ssm_status:-unknown}"
      continue
    fi

    # Check services via SSM
    local cmd_id
    cmd_id=$(aws-vault exec "$RELAY_AWS_PROFILE" --no-session -- \
      aws ssm send-command --region "$region" --instance-ids "$instance_id" \
      --document-name "AWS-RunShellScript" \
      --parameters '{"commands":["echo RATHOLE=$(systemctl is-active rathole-server 2>/dev/null); echo COTURN=$(systemctl is-active coturn 2>/dev/null); echo DISK=$(df / --output=pcent | tail -1 | tr -d \" %\"); echo UPTIME=$(uptime -s 2>/dev/null || echo unknown)"]}' \
      --query "Command.CommandId" --output text 2>/dev/null || echo "")

    if [ -n "$cmd_id" ]; then
      sleep 4
      local output
      output=$(aws-vault exec "$RELAY_AWS_PROFILE" --no-session -- \
        aws ssm get-command-invocation --region "$region" \
        --command-id "$cmd_id" --instance-id "$instance_id" \
        --query "StandardOutputContent" --output text 2>/dev/null || echo "")

      local rathole_svc
      rathole_svc=$(echo "$output" | grep RATHOLE | cut -d= -f2) || true
      local coturn_svc
      coturn_svc=$(echo "$output" | grep COTURN | cut -d= -f2) || true
      local disk_pct
      disk_pct=$(echo "$output" | grep DISK | cut -d= -f2) || true

      [ "$rathole_svc" = "active" ] && record "$category" "svc-rathole:$name" "PASS" "active" \
        || record "$category" "svc-rathole:$name" "FAIL" "$rathole_svc"

      [ "$coturn_svc" = "active" ] && record "$category" "svc-coturn:$name" "PASS" "active" \
        || record "$category" "svc-coturn:$name" "FAIL" "$coturn_svc"

      if [ -n "$disk_pct" ] && [ "$disk_pct" -lt 80 ] 2>/dev/null; then
        record "$category" "vps-disk:$name" "PASS" "${disk_pct}%"
      elif [ -n "$disk_pct" ]; then
        record "$category" "vps-disk:$name" "WARN" "${disk_pct}%"
      fi
    fi
  done
}

# ===========================================================================
# Output
# ===========================================================================
print_terminal() {
  local run_end
  run_end=$(date +%s)
  local duration=$((run_end - RUN_START))

  echo ""
  echo "==============================================================================="
  echo "  goat Smoke Test — $RUN_ID"
  echo "  Mode: $MODE | Duration: ${duration}s"
  echo "==============================================================================="
  echo ""

  local current_category=""
  for result in "${CHECK_RESULTS[@]}"; do
    IFS=$'\t' read -r cat name status detail latency <<< "$result"

    if [ "$cat" != "$current_category" ]; then
      echo ""
      printf "  %-12s\n" "[$cat]"
      printf "  %-12s %-30s %-6s %s\n" "" "CHECK" "STATUS" "DETAIL"
      printf "  %-12s %-30s %-6s %s\n" "" "-----" "------" "------"
      current_category="$cat"
    fi

    local status_color=""
    local reset=""
    if [ -t 1 ]; then  # Only colorize if terminal
      case $status in
        PASS) status_color="\033[32m"; reset="\033[0m" ;;
        FAIL) status_color="\033[31m"; reset="\033[0m" ;;
        WARN) status_color="\033[33m"; reset="\033[0m" ;;
      esac
    fi

    local latency_str=""
    [ -n "$latency" ] && [ "$latency" != "" ] && latency_str=" (${latency}ms)"

    printf "  %-12s %-30s ${status_color}%-6s${reset} %s%s\n" "" "$name" "$status" "$detail" "$latency_str"
  done

  echo ""
  echo "  ─────────────────────────────────────────────────────────────────"
  printf "  PASS: %d  FAIL: %d  WARN: %d  TOTAL: %d\n" "$PASS_COUNT" "$FAIL_COUNT" "$WARN_COUNT" "$((PASS_COUNT + FAIL_COUNT + WARN_COUNT))"
  echo "  ─────────────────────────────────────────────────────────────────"
  echo ""
}

print_json() {
  local run_end
  run_end=$(date +%s)
  local duration=$((run_end - RUN_START))

  local checks_json="["
  local first=true
  for result in "${CHECK_RESULTS[@]}"; do
    IFS=$'\t' read -r cat name status detail latency <<< "$result"
    $first || checks_json+=","
    checks_json+=$(printf '{"category":"%s","name":"%s","status":"%s","detail":"%s","latency_ms":"%s"}' \
      "$cat" "$name" "$status" "$detail" "$latency")
    first=false
  done
  checks_json+="]"

  printf '{"run_id":"%s","timestamp":"%s","mode":"%s","duration_s":%d,"pass":%d,"fail":%d,"warn":%d,"checks":%s}\n' \
    "$RUN_ID" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$MODE" "$duration" \
    "$PASS_COUNT" "$FAIL_COUNT" "$WARN_COUNT" "$checks_json"
}

save_result() {
  print_json >> "$RESULTS_FILE"
}

# ===========================================================================
# Main
# ===========================================================================
run_checks() {
  case $CATEGORY in
    dns)        check_dns ;;
    relay)      check_relay ;;
    stun)       check_stun ;;
    management) check_management ;;
    "$NETHOST_CATEGORY") check_nethost ;;
    mesh)       check_mesh ;;
    all)
      check_dns
      check_relay
      check_stun
      check_management
      if [ "$MODE" != "quick" ]; then
        check_nethost
        check_mesh
        check_vps_deep
      fi
      ;;
    *) echo "Unknown category: $CATEGORY"; exit 2 ;;
  esac
}

run_checks

if [ "$OUTPUT" = "json" ]; then
  print_json
else
  print_terminal
fi

# Always save to history
save_result 2>/dev/null || true

# Exit with failure if any checks failed
[ "$FAIL_COUNT" -eq 0 ]
