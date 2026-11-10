#!/usr/bin/env bash
# undeclared-runtime.sh — what is running here that nobody declared, and what
# is declared here that cannot possibly run.
#
# The registry pipeline proves ONE direction: a declared service gets a DNS
# record, fail-closed and CI-gated (ops/dns/generate-mesh-hosts.py, F-197 —
# "a declared service with no record deploys dark"). This script proves the
# other: a systemd unit left enabled after its service was retired can restart
# forever against a missing path and bury real errors in the journal, and no
# registry would otherwise say so.
#
# This is the reverse check. It is deliberately narrow — three findings with
# near-zero false positives, rather than a broad registry diff that would drown
# in OS units and teach everyone to ignore it.
#
# Read-only. Exits 0 with findings on stdout; --strict exits 1 if any found.
#
#   ops/scripts/undeclared-runtime.sh [--strict] [--json]
#
# Design: docs/design/dns-service-discovery.md §"Registration is bidirectional".

set -uo pipefail

STRICT=false
JSON=false
for a in "$@"; do
  case "$a" in
    --strict) STRICT=true ;;
    --json)   JSON=true ;;
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
  esac
done

# A unit restarting more than this many times is not "retrying", it is wedged.
# Anything past a few hundred is pathological.
RESTART_CEILING=200

findings=()
add() { findings+=("$1|$2|$3"); }   # severity|subject|detail

# --- 1. enabled units whose ExecStart target does not exist ----------------
# A unit that systemd will keep starting forever
# and that can never start even once. Unambiguous: no judgement call, no
# registry lookup, no false positives.
while read -r unit; do
  [[ -z "$unit" ]] && continue
  exec_line=$(systemctl show "$unit" -p ExecStart --value 2>/dev/null)
  # ExecStart=path=/opt/x/BIN ; argv[]=... — take the path= field.
  bin=$(sed -n 's/.*path=\([^ ;]*\).*/\1/p' <<<"$exec_line" | head -1)
  [[ -z "$bin" ]] && continue
  # Only absolute paths are checkable on disk. A bare name (grub-editenv,
  # seatd) is PATH-resolved by systemd at start time, and treating it as a
  # missing file reported both as broken on the first run — two false
  # positives out of two findings, which is precisely how a check gets
  # ignored. Resolve those through PATH instead.
  if [[ "$bin" == /* ]]; then
    [[ -e "$bin" ]] || add "broken" "$unit" "enabled, but ExecStart target $bin does not exist"
  else
    command -v "$bin" >/dev/null 2>&1 || add "broken" "$unit" "enabled, but ExecStart binary $bin is not on PATH"
  fi
done < <(systemctl list-unit-files --state=enabled --type=service --no-legend --plain 2>/dev/null | awk '{print $1}')

# --- 2. units in a restart loop -------------------------------------------
while read -r unit; do
  [[ -z "$unit" ]] && continue
  n=$(systemctl show "$unit" -p NRestarts --value 2>/dev/null)
  [[ "$n" =~ ^[0-9]+$ ]] || continue
  if (( n > RESTART_CEILING )); then
    add "looping" "$unit" "$n restarts — wedged, not retrying"
  fi
done < <(systemctl list-units --type=service --state=running,failed --no-legend --plain 2>/dev/null | awk '{print $1}')

# --- 3. running containers absent from the service registry ---------------
# Advisory, not fail-closed: infrastructure containers legitimately live in
# compose overlays rather than services.yaml. Named so a human can judge,
# because the alternative is silence.
# The registry files are every /opt/<root>/status/services.env (the status
# role's own, under whatever root this host's deploy uses) and the mgmt-host one.
REG=""
for c in /opt/*/status/services.env /opt/mgmt-host/services.env; do
  [[ -f "$c" ]] && REG="$REG $(cat "$c" 2>/dev/null)"
done
if command -v docker >/dev/null 2>&1 && [[ -n "$REG" ]]; then
  while read -r name; do
    [[ -z "$name" ]] && continue
    grep -q "$name" <<<"$REG" || add "unregistered" "container/$name" "running, not named in any services.env on this host"
  done < <(docker ps --format '{{.Names}}' 2>/dev/null)
fi

# --- report ----------------------------------------------------------------
if $JSON; then
  printf '{"host":"%s","findings":[' "$(hostname)"
  for i in "${!findings[@]}"; do
    IFS='|' read -r sev subj det <<<"${findings[$i]}"
    [[ $i -gt 0 ]] && printf ','
    printf '{"severity":"%s","subject":"%s","detail":"%s"}' "$sev" "$subj" "$det"
  done
  printf ']}\n'
else
  if [[ ${#findings[@]} -eq 0 ]]; then
    echo "$(hostname): nothing undeclared or wedged"
  else
    echo "$(hostname): ${#findings[@]} finding(s)"
    for f in "${findings[@]}"; do
      IFS='|' read -r sev subj det <<<"$f"
      printf '  [%-12s] %-40s %s\n' "$sev" "$subj" "$det"
    done
  fi
fi

if $STRICT && [[ ${#findings[@]} -gt 0 ]]; then exit 1; fi
exit 0
