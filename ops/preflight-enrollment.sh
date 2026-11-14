#!/usr/bin/env bash
# preflight-enrollment.sh — Block 1D: Netbird enrollment pre-flight check
#
# Validates prerequisites before attempting `netbird up`. Reports pass/fail
# per check and exits non-zero if any required check fails.
#
# With VPS relay (Block 2B) the chain is short — checks cover:
#   1. netbird CLI installed
#   2. no stale wt0 interface from a previous enrollment
#   3. NETBIRD_SETUP_KEY env var present and well-formed
#   4. management URL reachable (default netbird.legacy.example)
#   5. NTP clock not grossly skewed (±60s from network time)
#
# This is a pre-flight for the traditional setup-key enrollment path.
# Offline-enrollment bundle flow (Block 2C) has its own pre-checks
# inside the validator sidecar and the device-side consumer — not this script.
#
# Usage:
#   bash ops/preflight-enrollment.sh [--key-at-enrolment]
#
#   --key-at-enrolment  the setup key is minted at enrolment, where it is used
#                       (join-node phase 10, mesh-enroll-host.yml), so no key is
#                       given here: check 3 reports SKIP instead of FAIL
#   NETBIRD_MANAGEMENT_URL=https://netbird.example bash ops/preflight-enrollment.sh
#
# Exit codes:
#   0  all required checks passed
#   1  a required check failed (enrollment will not succeed)
#   2  a warning fired (enrollment may succeed but something is off)

set -uo pipefail

MGMT_URL="${NETBIRD_MANAGEMENT_URL:-https://netbird.legacy.example}"
SETUP_KEY="${NETBIRD_SETUP_KEY:-}"
KEY_AT_ENROLMENT=0
for arg in "$@"; do
    [[ "${arg}" == --key-at-enrolment ]] && KEY_AT_ENROLMENT=1
done

PASS=0
FAIL=0
WARN=0

ok()   { printf '  [ \033[32mOK\033[0m ] %s\n' "$1"; PASS=$((PASS+1)); }
bad()  { printf '  [\033[31mFAIL\033[0m] %s\n' "$1"; FAIL=$((FAIL+1)); }
warn() { printf '  [\033[33mWARN\033[0m] %s\n' "$1"; WARN=$((WARN+1)); }

echo "Netbird enrollment pre-flight"
echo "management URL: ${MGMT_URL}"
echo

# 1. netbird CLI installed
echo "1. netbird CLI"
if command -v netbird >/dev/null 2>&1; then
    ver=$(netbird version 2>/dev/null | head -1 || echo "unknown")
    ok "netbird installed (${ver})"
else
    bad "netbird not found in PATH — install from https://docs.netbird.io/get-started/install"
fi

# 2. no stale wt0 interface
echo
echo "2. stale interface check"
if ip link show wt0 >/dev/null 2>&1; then
    state=$(ip -br link show wt0 | awk '{print $2}')
    if [[ "${state}" == "UP" ]] && pgrep -x netbird >/dev/null 2>&1; then
        warn "wt0 exists and netbird is running — node appears already enrolled. Run 'netbird down' first if re-enrolling."
    else
        bad "wt0 interface exists but netbird is not running — stale state. Remove with: sudo ip link delete wt0"
    fi
else
    ok "no stale wt0 interface"
fi

# 3. setup key presence and format
echo
echo "3. setup key"
if [[ "${KEY_AT_ENROLMENT}" == 1 ]]; then
    printf '  [SKIP] %s\n' "no setup key here: it is minted at enrolment, where it is used, and never leaves the hosts"
elif [[ -z "${SETUP_KEY}" ]]; then
    bad "NETBIRD_SETUP_KEY env var not set — export it or pass via --setup-key"
elif [[ ! "${SETUP_KEY}" =~ ^[A-Z0-9]{8}-[A-Z0-9]{4}-[A-Z0-9]{4}-[A-Z0-9]{4}-[A-Z0-9]{12}$ ]]; then
    bad "NETBIRD_SETUP_KEY format looks wrong (expected UUID-like AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA). Got: ${SETUP_KEY:0:8}…"
else
    ok "setup key format valid"
fi

# 4. management URL reachable
echo
echo "4. management URL reachable"
if ! command -v curl >/dev/null 2>&1; then
    warn "curl not installed — skipping reachability check"
else
    http_code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 "${MGMT_URL}/api/users" || echo "000")
    case "${http_code}" in
        401|403)
            ok "management API reachable (${http_code} — auth required, as expected)"
            ;;
        200)
            ok "management API reachable (200)"
            ;;
        000)
            bad "management URL unreachable — check DNS, network, VPN, and relay status"
            ;;
        5*)
            bad "management API returned ${http_code} — server-side problem at the relay or network host"
            ;;
        *)
            warn "management API returned unexpected ${http_code} — investigate before enrolling"
            ;;
    esac
fi

# 5. clock skew
echo
echo "5. NTP clock sanity"
if ! command -v curl >/dev/null 2>&1; then
    warn "curl not installed — skipping clock check"
else
    # Compare against HTTP Date header from the management endpoint. Accurate to ~1s.
    remote_date=$(curl -sI --max-time 10 "${MGMT_URL}" | awk -F': ' 'tolower($1)=="date"{sub(/\r$/,"",$2); print $2}')
    if [[ -z "${remote_date}" ]]; then
        warn "could not read Date header from ${MGMT_URL} — skipping clock check"
    else
        remote_epoch=$(date -d "${remote_date}" +%s 2>/dev/null || date -j -f "%a, %d %b %Y %H:%M:%S %Z" "${remote_date}" +%s 2>/dev/null || echo "")
        if [[ -z "${remote_epoch}" ]]; then
            warn "could not parse remote Date — skipping clock check"
        else
            local_epoch=$(date -u +%s)
            skew=$((local_epoch - remote_epoch))
            abs_skew=${skew#-}
            if [[ ${abs_skew} -le 60 ]]; then
                ok "clock within ±60s of management server (skew ${skew}s)"
            else
                bad "clock skew ${skew}s exceeds 60s — fix NTP before enrolling. TLS handshake may fail and certs may not validate."
            fi
        fi
    fi
fi

echo
echo "────────────────────────────────────────"
printf "  pass: %d  fail: %d  warn: %d\n" "${PASS}" "${FAIL}" "${WARN}"
echo "────────────────────────────────────────"

if [[ ${FAIL} -gt 0 ]]; then
    echo "enrollment will not succeed — fix the FAIL items above"
    exit 1
fi
if [[ ${WARN} -gt 0 ]]; then
    echo "proceed with caution — review WARN items"
    exit 2
fi
echo "ready to enroll — run:"
# Intra-mesh PQC (Rosenpass, ADR 0602): append the flags when the goatnet has
# it enabled. Default off — set ROSENPASS_ENABLED=1 (mirrors the ansible
# rosenpass_enabled group_var) to have this printed command PQ-secure the peer.
if [[ "${ROSENPASS_ENABLED:-0}" == "1" || "${ROSENPASS_ENABLED:-}" == "true" ]]; then
    echo "  sudo netbird up --management-url ${MGMT_URL} --setup-key \$NETBIRD_SETUP_KEY \\"
    echo "    --enable-rosenpass --rosenpass-permissive"
else
    echo "  sudo netbird up --management-url ${MGMT_URL} --setup-key \$NETBIRD_SETUP_KEY"
fi
exit 0
