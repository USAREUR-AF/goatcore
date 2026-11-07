#!/usr/bin/env bash
set -euo pipefail

# goat — Mesh status and diagnostics
#
# Quick view of your Netbird identity, peers, and DNS on the mesh.
#
# Usage:
#   bash ops/mesh-info.sh              Show everything
#   bash ops/mesh-info.sh ip           Just your Netbird IP
#   bash ops/mesh-info.sh peers        Connected peers
#   bash ops/mesh-info.sh dns          DNS resolution test
#   bash ops/mesh-info.sh scan [port]  Scan peers for open ports
#
# Values a deployment sets in its ops/env-aliases.d/*.sh
# (ops/scripts/lib/env-aliases.sh), each with an example default:
#   NETBIRD_DOMAIN         the mesh's NetBird domain
#   NETHOST_MESH_ADDRESS   the network host's mesh address, where its CoreDNS answers
#   MESH_KNOWN_NAMES       names under NETBIRD_DOMAIN to test when no mesh-hosts file is found

# shellcheck source=scripts/lib/env-aliases.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/scripts/lib/env-aliases.sh"

CMD="${1:-all}"
DOMAIN="${NETBIRD_DOMAIN:-netbird.legacy.example}"

# Colors
B='\033[1m'
G='\033[0;32m'
R='\033[0;31m'
Y='\033[0;33m'
N='\033[0m'

show_ip() {
  local status
  status=$(netbird status 2>&1)
  local ip
  ip=$(echo "$status" | grep "NetBird IP" | awk '{print $NF}') || true
  local fqdn
  fqdn=$(echo "$status" | grep "FQDN" | awk '{print $NF}') || true
  local mgmt
  mgmt=$(echo "$status" | grep "Management" | awk '{print $2}') || true
  local signal
  signal=$(echo "$status" | grep "Signal" | sed 's/.*: //') || true
  local ns
  ns=$(echo "$status" | grep "Nameservers" | awk '{print $2}') || true
  local peers
  peers=$(echo "$status" | grep "Peers count" | awk '{print $NF}') || true

  printf "${B}Netbird IP:${N}   %s\n" "$ip"
  printf "${B}FQDN:${N}        %s\n" "$fqdn"
  printf "${B}Management:${N}  %s\n" "$mgmt"
  printf "${B}Signal:${N}      %s\n" "$signal"
  printf "${B}Nameservers:${N} %s\n" "$ns"
  printf "${B}Peers:${N}       %s\n" "$peers"
}

show_peers() {
  local detail
  detail=$(netbird status --detail 2>&1)

  # Parse peer lines
  echo "$detail" | awk '
  /Peers detail:/ { found=1; next }
  found && /^ / {
    # Peer block
    if (/Hostname:/) { hostname=$NF }
    if (/NetBird IP:/) { ip=$NF }
    if (/Status:/) { status=$NF }
    if (/Connection type:/) { conntype=$0; sub(/.*: /, "", conntype) }
    if (/Last seen:/) {
      lastseen=$0; sub(/.*: /, "", lastseen)
      printf "  %-20s %-18s %-12s %s\n", hostname, ip, status, conntype
      hostname=""; ip=""; status=""; conntype=""
    }
  }
  BEGIN { printf "  %-20s %-18s %-12s %s\n", "HOSTNAME", "NETBIRD IP", "STATUS", "CONNECTION" }
  '
}

show_dns() {
  local nethost_dns="${NETHOST_MESH_ADDRESS:-100.127.1.23}"

  printf "${B}DNS resolver test:${N}\n"

  # Check if Netbird DNS is in system resolvers
  local nb_resolver
  nb_resolver=$(scutil --dns 2>/dev/null | grep -B5 "$DOMAIN" | grep nameserver | head -1 || echo "")
  if [ -n "$nb_resolver" ]; then
    printf "  Netbird resolver: ${G}configured${N} (%s)\n" "$(echo $nb_resolver | awk '{print $NF}')"
  else
    printf "  Netbird resolver: ${R}not in system DNS${N}\n"
    printf "  Expected: nameserver for $DOMAIN → $nethost_dns\n"
  fi

  echo ""
  printf "  %-40s %s\n" "NAME" "RESOLVES TO"

  # Read mesh-hosts if available
  local hosts_file
  for f in "$(dirname "$0")/dns/mesh-hosts" "$(dirname "$0")/../ops/dns/mesh-hosts"; do
    [ -f "$f" ] && hosts_file="$f" && break
  done

  if [ -n "${hosts_file:-}" ]; then
    while IFS= read -r line; do
      [[ "$line" =~ ^#.*$ ]] && continue
      [[ -z "$line" ]] && continue
      local ip
      ip=$(echo "$line" | awk '{print $1}')
      local fqdn
      fqdn=$(echo "$line" | awk '{print $2}')
      [ -z "$fqdn" ] && continue

      # Try system DNS resolution
      local resolved
      resolved=$(getent hosts "$fqdn" 2>/dev/null | awk '{print $1}' || echo "")
      if [ -z "$resolved" ]; then
        # Try direct query to NETHOST CoreDNS (listens on :5353, migrated from dnsmasq in Block 13A)
        resolved=$(dig +short @"$nethost_dns" -p 5353 "$fqdn" 2>/dev/null || echo "")
      fi

      if [ "$resolved" = "$ip" ]; then
        printf "  %-40s ${G}%s${N}\n" "$fqdn" "$resolved"
      elif [ -n "$resolved" ]; then
        printf "  %-40s ${Y}%s (expected %s)${N}\n" "$fqdn" "$resolved" "$ip"
      else
        printf "  %-40s ${R}FAILED${N} (expected %s)\n" "$fqdn" "$ip"
      fi
    done < "$hosts_file"
  else
    echo "  (mesh-hosts file not found — checking known names)"
    for name in ${MESH_KNOWN_NAMES:-goat airdefenders}; do
      local fqdn="$name.$DOMAIN"
      local resolved
      resolved=$(dig +short @"$nethost_dns" -p 5353 "$fqdn" 2>/dev/null || echo "")
      if [ -n "$resolved" ]; then
        printf "  %-40s ${G}%s${N}\n" "$fqdn" "$resolved"
      else
        printf "  %-40s ${R}FAILED${N}\n" "$fqdn"
      fi
    done
  fi
}

scan_peers() {
  local port="${2:-80,443,8080,8787,3478,51820}"
  echo "Scanning mesh peers for open ports ($port)..."
  echo ""

  # Get peer IPs from netbird status
  local peers
  peers=$(netbird status --detail 2>&1 | grep "NetBird IP:" | awk '{print $NF}' | sed 's|/.*||')

  if [ -z "$peers" ]; then
    echo "No peers found."
    return
  fi

  printf "  %-18s %s\n" "PEER IP" "OPEN PORTS"
  for ip in $peers; do
    local open=""
    IFS=',' read -ra PORTS <<< "$port"
    for p in "${PORTS[@]}"; do
      if nc -z -w1 "$ip" "$p" 2>/dev/null; then
        open="$open $p"
      fi
    done
    if [ -n "$open" ]; then
      printf "  %-18s${G}%s${N}\n" "$ip" "$open"
    else
      printf "  %-18s${Y} none${N}\n" "$ip"
    fi
  done
}

case "$CMD" in
  ip)    show_ip ;;
  peers) show_peers ;;
  dns)   show_dns ;;
  scan)  scan_peers "$@" ;;
  all)
    show_ip
    echo ""
    show_peers
    echo ""
    show_dns
    ;;
  *)
    echo "Usage: mesh-info.sh [ip|peers|dns|scan [ports]]"
    exit 1
    ;;
esac
