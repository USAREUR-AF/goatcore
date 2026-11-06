#!/usr/bin/env bash
# fresh-peer-probe.sh — the Gate 1 four-part fresh-peer probe.
#
# STATUS: the four live checks need a real goatnet to run against.
# `--self-test` (config resolution, no mesh) is what CI/pre-standup runs.
#
# WHAT THIS IS FOR
# Gate 1 of the go-live gate (docs/design/environment-parity.md §6,
# docs/operations/<env>-standup-runbook.md §"Gate 1"). F-188 produced an
# environment whose every component was "up" while a factory-fresh peer got
# nothing. Component health is not the same claim as "a new peer can join and
# use this environment", and only the second one is what "live" means. This
# script makes the second claim testable.
#
# WHERE IT RUNS
# ON the factory-fresh peer, after enrollment — NOT on a mgmt host, and not
# on any peer that has ever held this environment's config. Running it from a
# mgmt host tests the thing that was already working during F-188 and is
# worse than not running it, because it produces a green that means nothing.
#
# THE FOUR PARTS (order is the runbook's, and it is deliberate — each part
# presupposes the one before it, so a FAIL early makes the rest unreadable)
#   1. dns      — a mesh name in the env's zone resolves FROM THE PEER
#   2. relay    — the peer reached a relay and completed enrollment
#   3. service  — a representative service answers BY NAME, not by IP
#   4. publish  — a publish lands and is readable back (fabric round-trip)
#
# HONESTY GRAMMAR — three verdicts, and UNKNOWN is not a soft pass
#   PASS     — grounded: the check ran and the environment answered correctly
#   FAIL     — grounded: the check ran and the environment answered wrongly
#   UNKNOWN  — NOT grounded: the check could not be run at all (a tool is
#              missing, config is still a TBD_ sentinel, no representative
#              service is declared yet). Per the environment-health-surface
#              tenet "honest unknown beats false green", UNKNOWN never rolls
#              up as PASS — it exits non-zero exactly like FAIL.
# §6 says "no partial credit": the gate is green only when all four are PASS.
#
# EXIT CODES (house convention: 0 clean / 1 finding / 2 usage-or-preflight)
#   0 — all four parts PASS
#   1 — at least one part FAIL or UNKNOWN
#   2 — usage error, or config for <env> could not be resolved at all
#
# USAGE
#   ops/scripts/fresh-peer-probe.sh <env> [--service <fqdn>] [--json]
#   ops/scripts/fresh-peer-probe.sh <env> --self-test
#
#   <env>            hyphenated goatnet name, e.g. site-a (NOT site_a)
#   --service <fqdn> override the representative service. Needed while an
#                    env's isitup_services list is still empty.
#   --json           emit one JSON object per part to stdout as well
#   --self-test      resolve config and validate arg parsing WITHOUT a mesh,
#                    then exit. Runs anywhere, including CI.
#
# CONFIG RESOLUTION (no new registry — reads the surfaces that already exist)
#   env -> group_vars dirs   ops/pins/fleet-pins.yaml  `goatnets.<env_>.members`
#   zone / site / services   ops/ansible/group_vars/<member>/vars.yml
#   relay IPs + names        ops/relay-env-<env>.sh    (generated, sourceable)
# Every one of those is generated or CI-gated elsewhere, so this script adds
# no third place for an environment's identity to drift.

# NOT `set -e`: a prober must survive a failing sub-check and still emit a
# verdict for the other three. Matching ops/relay/health/relay-health-probe.sh.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "${REPO_ROOT}/.." && pwd)"

ENV_NAME=""
SERVICE_OVERRIDE=""
SELF_TEST=0
EMIT_JSON=0

usage() {
  sed -n '/^# USAGE/,/^# CONFIG RESOLUTION/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --self-test) SELF_TEST=1 ;;
    --json)      EMIT_JSON=1 ;;
    --service)   SERVICE_OVERRIDE="${2:-}"; shift ;;
    -h|--help)   usage ;;
    -*)          echo "unknown arg: $1" >&2; usage ;;
    *)
      if [ -n "$ENV_NAME" ]; then echo "unexpected extra arg: $1" >&2; usage; fi
      ENV_NAME="$1"
      ;;
  esac
  shift
done

[ -n "$ENV_NAME" ] || usage

log()  { printf '[fresh-peer-probe] %s\n' "$*" >&2; }
die2() { log "PREFLIGHT: $*"; exit 2; }

# ---------------------------------------------------------------------------
# Config resolution
# ---------------------------------------------------------------------------
# Emits shell-assignable KEY=VALUE lines. Kept in python because fleet-pins.yaml
# and group_vars are yaml and bash has no honest yaml reader; the alternative
# (grep) is how a resolver starts silently returning the wrong environment's
# values. Fail-closed: unknown env, or an env whose vars are still TBD_
# sentinels, is reported rather than guessed at.
resolve_config() {
  python3 - "$REPO_ROOT" "$ENV_NAME" <<'PY'
import os, sys, shlex
try:
    import yaml
except ImportError:
    print("RESOLVE_ERROR=python3 yaml module not available"); sys.exit(0)

# Every emitted value is shell-quoted: these are multi-word strings that the
# caller `eval`s, and an unquoted one runs its second word as a command.
def out(key, value):
    print("%s=%s" % (key, shlex.quote(str(value))))

repo, env = sys.argv[1], sys.argv[2]
key = env.replace("-", "_")

fleet = os.path.join(repo, "ops/pins/fleet-pins.yaml")
if not os.path.exists(fleet):
    out("RESOLVE_ERROR", "missing %s" % fleet); sys.exit(0)
with open(fleet) as fh:
    doc = yaml.safe_load(fh) or {}

goatnets = doc.get("goatnets") or {}
if key not in goatnets:
    known = " ".join(sorted(k.replace("_", "-") for k in goatnets))
    out("RESOLVE_ERROR", "unknown env '%s'; fleet-pins.yaml knows: %s" % (env, known))
    sys.exit(0)

members = goatnets[key].get("members") or []
if not members:
    out("RESOLVE_ERROR", "env '%s' has no member group_vars dirs" % env); sys.exit(0)

# The first member is the mgmt group; that is where a goatnet's identity
# (zone, site_id, service registry) lives. Obs-only groups are deliberately
# not members (see fleet-pins.yaml), so this is not an arbitrary pick.
gv = os.path.join(repo, "ops/ansible/group_vars", members[0], "vars.yml")
if not os.path.exists(gv):
    out("RESOLVE_ERROR", "missing %s" % gv); sys.exit(0)
with open(gv) as fh:
    v = yaml.safe_load(fh) or {}

site_id = str(v.get("site_id") or "")
zone    = str(v.get("site_dns_zone") or "")
enroll  = str(v.get("site_enrollment_dns") or v.get("public_mgmt_fqdn") or "")

def render(s):
    # group_vars values are Jinja; only site_id/site_dns_zone appear in the
    # fields we read. An unresolved {{ }} is surfaced, never shipped as a name.
    s = s.replace("{{ site_id }}", site_id).replace("{{ site_dns_zone }}", zone)
    return s.strip()

svcs = []
for row in (v.get("isitup_services") or []):
    if isinstance(row, dict) and row.get("fqdn"):
        svcs.append(render(str(row["fqdn"])))
# airdefenders_url_host is a declared service FQDN outside isitup_services;
# it is a legitimate fallback for an env whose registry is not populated yet.
if not svcs and v.get("airdefenders_url_host"):
    svcs.append(render(str(v["airdefenders_url_host"])))

out("SITE_ID", site_id)
out("SITE_ZONE", zone)
out("ENROLL_DNS", render(enroll))
out("GROUP_VARS", members[0])
out("SERVICES", " ".join(s for s in svcs if s))

# TBD_ sentinels are the house pre-flight fuse (the bootstrap play's Phase -1).
tbd = sorted({str(val) for val in v.values() if isinstance(val, str) and val.startswith("TBD_")})
out("TBD_SENTINELS", " ".join(tbd))
PY
}

CFG="$(resolve_config)"
[ -n "$CFG" ] || die2 "config resolver produced no output for '$ENV_NAME'"
eval "$CFG"

if [ -n "${RESOLVE_ERROR:-}" ]; then
  die2 "$RESOLVE_ERROR"
fi

SITE_ID="${SITE_ID:-}"
SITE_ZONE="${SITE_ZONE:-}"
ENROLL_DNS="${ENROLL_DNS:-}"
SERVICES="${SERVICES:-}"
TBD_SENTINELS="${TBD_SENTINELS:-}"

[ -n "$SITE_ZONE" ] || die2 "env '$ENV_NAME' resolved no site_dns_zone"

# Relay facts come from the generated per-env file, when it exists.
RELAY_ENV_FILE="${REPO_ROOT}/ops/relay-env-${ENV_NAME}.sh"
RELAY_IPS=()
if [ -f "$RELAY_ENV_FILE" ]; then
  # shellcheck disable=SC1090
  . "$RELAY_ENV_FILE"
fi

REPRESENTATIVE=""
if [ -n "$SERVICE_OVERRIDE" ]; then
  REPRESENTATIVE="$SERVICE_OVERRIDE"
elif [ -n "$SERVICES" ]; then
  REPRESENTATIVE="$(echo "$SERVICES" | awk '{print $1}')"
fi

# ---------------------------------------------------------------------------
# Verdict plumbing
# ---------------------------------------------------------------------------
N_PASS=0; N_FAIL=0; N_UNKNOWN=0
RESULTS=()

emit() {
  local part="$1" verdict="$2" evidence="$3"
  case "$verdict" in
    PASS)    N_PASS=$((N_PASS + 1)) ;;
    FAIL)    N_FAIL=$((N_FAIL + 1)) ;;
    UNKNOWN) N_UNKNOWN=$((N_UNKNOWN + 1)) ;;
  esac
  RESULTS+=("$(printf '%-9s %-8s %s' "$part" "$verdict" "$evidence")")
  printf '%-9s %-8s %s\n' "$part" "$verdict" "$evidence"
  if [ "$EMIT_JSON" -eq 1 ]; then
    printf '{"ts":"%s","env":"%s","part":"%s","verdict":"%s","evidence":"%s"}\n' \
      "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$ENV_NAME" "$part" "$verdict" \
      "$(printf '%s' "$evidence" | sed 's/"/\\"/g')"
  fi
}

have() { command -v "$1" >/dev/null 2>&1; }

# ---------------------------------------------------------------------------
# Part 1 — DNS resolves from the peer
# ---------------------------------------------------------------------------
check_dns() {
  local name="${REPRESENTATIVE:-$ENROLL_DNS}"
  if [ -z "$name" ]; then
    emit dns UNKNOWN "no name to resolve: env declares no service and no enrollment DNS"
    return
  fi
  if ! have dig && ! have getent && ! have dscacheutil; then
    emit dns UNKNOWN "none of dig, getent or dscacheutil present on this peer"
    return
  fi

  # Two different questions, asked in this order on purpose.
  #
  # `dig` asks a nameserver directly. `getent hosts` asks the OS resolver —
  # NSS, /etc/hosts, whatever the peer is actually configured to use — which is
  # the path a real peer's traffic takes. They can disagree, and when they do
  # the OS answer is the one that matters.
  #
  # macOS has no getent, which left it with only the dig path: a probe that can
  # report PASS while the system resolver returns nothing. `dscacheutil` is the
  # macOS system-resolver query and closes that gap, so the fallback is
  # getent-or-dscacheutil rather than getent-or-nothing.
  local answer=""
  if have dig; then
    answer="$(dig +short +timeout=3 +tries=1 "$name" A 2>/dev/null | grep -E '^[0-9.]+$' | head -1)"
  fi
  if [ -z "$answer" ] && have getent; then
    answer="$(getent hosts "$name" 2>/dev/null | awk '{print $1}' | head -1)"
  fi
  if [ -z "$answer" ] && have dscacheutil; then
    answer="$(dscacheutil -q host -a name "$name" 2>/dev/null \
              | awk -F': ' '/^ip_address/ {print $2; exit}')"
  fi

  if [ -z "$answer" ]; then
    emit dns FAIL "$name did not resolve from this peer"
    return
  fi
  emit dns PASS "$name -> $answer"
}

# ---------------------------------------------------------------------------
# Part 2 — relay path works / enrollment completed
# ---------------------------------------------------------------------------
# Two independent signals, because either alone lies: netbird reporting
# "Connected" proves enrollment but not that THIS env's relays are the ones
# answering, and a reachable relay port proves nothing about enrollment.
check_relay() {
  local enrolled="" relay_ok="" detail=""

  if have netbird; then
    local st; st="$(netbird status 2>/dev/null)"
    if printf '%s' "$st" | grep -qiE 'management: *connected|Management: *Connected'; then
      enrolled="yes"
    elif [ -n "$st" ]; then
      enrolled="no"
    fi
  fi

  local n_up=0 n_tried=0 ip
  # Only count attempts when nc exists: with `have nc` inside the loop a
  # missing nc silently produced "0/N — no relay answered" for a port nobody
  # measured. No nc ⇒ relays stay "unmeasured".
  if have nc; then
    for ip in "${RELAY_IPS[@]:-}"; do
      [ -n "$ip" ] || continue
      n_tried=$((n_tried + 1))
      # 33073 is the goat-relay port these envs advertise (rels://<host>:33073).
      if nc -z -w 3 "$ip" 33073 >/dev/null 2>&1; then
        n_up=$((n_up + 1))
      fi
    done
  fi
  [ "$n_tried" -gt 0 ] && relay_ok="${n_up}/${n_tried}"

  if [ -z "$enrolled" ] && [ -z "$relay_ok" ]; then
    emit relay UNKNOWN "no netbird binary and no relay list for '$ENV_NAME' — nothing was measured"
    return
  fi

  detail="enrollment=${enrolled:-unmeasured} relays_reachable=${relay_ok:-unmeasured}"
  if [ "$enrolled" = "no" ]; then
    emit relay FAIL "$detail"
    return
  fi
  if [ -n "$relay_ok" ] && [ "$n_up" -eq 0 ]; then
    emit relay FAIL "$detail (no relay answered :33073)"
    return
  fi
  if [ "$enrolled" != "yes" ]; then
    emit relay UNKNOWN "$detail (relays answer, but enrollment state was not readable)"
    return
  fi
  emit relay PASS "$detail"
}

# ---------------------------------------------------------------------------
# Part 3 — a representative service answers BY NAME
# ---------------------------------------------------------------------------
# By name, never by IP: reaching the IP while the name fails IS the F-188
# failure, so an IP-based check would report green on the exact defect.
check_service() {
  if [ -z "$REPRESENTATIVE" ]; then
    emit service UNKNOWN "env '$ENV_NAME' declares no service (isitup_services empty); pass --service <fqdn>"
    return
  fi
  if ! have curl; then
    emit service UNKNOWN "curl not present on this peer"
    return
  fi

  local code
  code="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 \
            "https://${REPRESENTATIVE}/" 2>/dev/null)"

  case "$code" in
    000|"")
      emit service FAIL "https://${REPRESENTATIVE}/ unreachable by name (curl code 000)"
      ;;
    2*|3*|401|403)
      # 401/403 are a PASS for reachability: the service answered, and an
      # auth-gated surface answering with a challenge is the service working.
      emit service PASS "https://${REPRESENTATIVE}/ -> HTTP ${code}"
      ;;
    *)
      emit service FAIL "https://${REPRESENTATIVE}/ -> HTTP ${code}"
      ;;
  esac
}

# ---------------------------------------------------------------------------
# Part 4 — a publish lands and is readable
# ---------------------------------------------------------------------------
# `goat pub --confirm` is the round-trip primitive: plain put is
# fire-and-forget and returns ok even when the router denied the write, so
# only --confirm distinguishes landed from silently-denied.
# The payload is unique per run because an identical stale value already in
# storage would mask a denial.
check_publish() {
  if ! have goat; then
    emit publish UNKNOWN "goat CLI not present on this peer — fabric round-trip not measured"
    return
  fi

  local stamp key payload out rc
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  key="ops/probe/fresh-peer/${SITE_ID:-$ENV_NAME}/${stamp}"
  payload="fresh-peer-probe ${ENV_NAME} ${stamp} $$"

  out="$(goat pub "$key" --payload "$payload" --confirm 2>&1)"; rc=$?

  if [ $rc -eq 0 ] && printf '%s' "$out" | grep -q 'landed'; then
    emit publish PASS "$key confirmed in router storage"
    return
  fi
  if [ $rc -eq 0 ]; then
    # `--confirm` exits 0 on an INCONCLUSIVE read-back. That is not a pass:
    # it is precisely the un-grounded case UNKNOWN exists for.
    emit publish UNKNOWN "$key published but read-back inconclusive: $(printf '%s' "$out" | tr '\n' ' ' | tail -c 160)"
    return
  fi
  emit publish FAIL "$key not confirmed in router storage: $(printf '%s' "$out" | tr '\n' ' ' | tail -c 160)"
}

# ---------------------------------------------------------------------------
# --self-test — config resolution only, no mesh required
# ---------------------------------------------------------------------------
self_test() {
  local fails=0
  echo "self-test: ${ENV_NAME}  (config resolution only — no mesh touched)"
  echo

  printf '  %-22s %s\n' "group_vars"      "${GROUP_VARS:-<unresolved>}"
  printf '  %-22s %s\n' "site_id"         "${SITE_ID:-<unset>}"
  printf '  %-22s %s\n' "site_dns_zone"   "${SITE_ZONE:-<unset>}"
  printf '  %-22s %s\n' "enrollment_dns"  "${ENROLL_DNS:-<unset>}"
  printf '  %-22s %s\n' "relay_env_file"  "$([ -f "$RELAY_ENV_FILE" ] && echo "$RELAY_ENV_FILE" || echo '<absent>')"
  printf '  %-22s %s\n' "relay_ips"       "${RELAY_IPS[*]:-<none>}"
  printf '  %-22s %s\n' "representative"  "${REPRESENTATIVE:-<none declared>}"
  echo

  # A resolvable zone is the one hard requirement; without it no part can run.
  if [ -n "$SITE_ZONE" ]; then
    echo "  [ok]   site_dns_zone resolved"
  else
    echo "  [FAIL] site_dns_zone did not resolve"; fails=$((fails + 1))
  fi

  if [ -n "$REPRESENTATIVE" ]; then
    echo "  [ok]   representative service declared"
  else
    echo "  [note] no representative service declared — part 3 will report UNKNOWN"
    echo "         (expected while isitup_services is empty; pass --service <fqdn>)"
  fi

  if [ ${#RELAY_IPS[@]} -gt 0 ]; then
    echo "  [ok]   ${#RELAY_IPS[@]} relay IP(s) resolved"
  else
    echo "  [note] no relay IPs — part 2 falls back to enrollment state alone"
  fi

  if [ -n "$TBD_SENTINELS" ]; then
    echo "  [note] TBD_ sentinels still in group_vars: $TBD_SENTINELS"
    echo "         The env is pre-standup; a live run is not meaningful yet."
  fi

  echo
  # Tool presence is reported, never enforced: --self-test is meant to run on
  # a laptop and in CI, neither of which has netbird/goat.
  local t
  # getent and dscacheutil are the same capability on two platforms (the OS
  # resolver), so neither being absent is interesting on its own — the DNS part
  # only reports UNKNOWN when dig AND both of them are missing.
  for t in dig getent dscacheutil nc curl netbird goat; do
    if have "$t"; then printf '  tool %-11s present\n' "$t"
    else                printf '  tool %-11s ABSENT — its part will report UNKNOWN on a live run\n' "$t"; fi
  done

  echo
  if [ $fails -gt 0 ]; then
    echo "self-test: FAIL ($fails)"
    return 1
  fi
  echo "self-test: OK — config resolves; live-fire pending first standup"
  return 0
}

if [ "$SELF_TEST" -eq 1 ]; then
  self_test
  exit $?
fi

# ---------------------------------------------------------------------------
# Live run
# ---------------------------------------------------------------------------
echo "fresh-peer-probe ${ENV_NAME} — Gate 1 (zone ${SITE_ZONE})"
echo "run this ON the factory-fresh peer; a run from a mgmt host proves nothing"
echo

if [ -n "$TBD_SENTINELS" ]; then
  log "WARNING: group_vars still carry TBD_ sentinels ($TBD_SENTINELS) — env is pre-standup"
fi

check_dns
check_relay
check_service
check_publish

echo
echo "env=${ENV_NAME} : ${N_PASS} pass, ${N_FAIL} fail, ${N_UNKNOWN} unknown (of 4)"

if [ "$N_FAIL" -gt 0 ] || [ "$N_UNKNOWN" -gt 0 ]; then
  echo "GATE 1 NOT MET — §6 allows no partial credit; UNKNOWN is not a pass."
  echo "Record all four results in the validation-state.md row regardless of outcome."
  exit 1
fi

echo "GATE 1 MET — all four parts grounded and PASS."
exit 0
