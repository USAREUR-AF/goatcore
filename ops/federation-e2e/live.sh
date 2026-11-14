#!/usr/bin/env bash
# live.sh — rung 9 on two REAL Ubuntu hosts, one per sovereign, driven from a laptop
# over SSH. The same scenario as ops/federation-e2e/run.sh: the real driver, the real
# plays, a real WireGuard link selected by the agreement, real pinned zenohd and
# spire-server, and ops/federation/standup/acceptance.sh from both sides with
# FED_ACCEPTANCE_STRICT=1. The proof is read from the receiving side.
#
# ONE HOST PER SOVEREIGN, AND THAT DEPARTS FROM THE RUNBOOK. The runbook keeps the edge
# off the core's machine. Here each host runs its core router and spire-server in docker
# on host-local addresses (a dummy interface and loopback) and its edge natively beside
# them, because the drill provides one host per sovereign. What the departure costs: a
# compromise of the edge is a compromise of the core, which a real deployment must not
# accept. What it keeps: the only path between the two hosts is WireGuard over their
# public addresses, the core and the SPIRE server are reachable from nowhere off-host,
# and the bundle endpoint crosses inside the tunnel.
#
# Each host needs: Ubuntu 24.04 x86_64 (the binaries are built on 24.04), the wireguard
# kernel module, outbound internet for apt, pip, ghcr.io and Docker Hub, inbound UDP
# 51820 from the other host, and SSH with passwordless sudo from this laptop. The script
# installs everything else, idempotently. ops/federation-e2e/README.md has the full list.
#
# usage: ops/federation-e2e/live.sh [--dry-run] [--scenario 1|2|both] [--record DIR]
#          --goat BIN --goatbuilder BIN        Linux x86_64 binaries, from the federation-e2e
#                                              job's `federation-live-binaries` artifact
#          --pasture-host A --meadow-host B    each host's public address
#          [--pasture-user U] [--meadow-user U]   default ubuntu
#          [--pasture-key K]  [--meadow-key K]    default: ssh's own choice
#          [--local-goatbuilder BIN]           runs phase 14 for scenario 1 from this laptop;
#                                              default `goatbuilder` on PATH, else the script
#          [--ghcr-token-env NAME]             the variable holding a token that can read
#                                              ghcr.io/usareur-af/zenoh-pqc; default GHCR_READ_TOKEN
#          [--ghcr-user USER]                  default x-access-token
#          [--keep]                            leave the hosts as the run left them, logged in
# Every flag has an environment form: FED_LIVE_PASTURE_HOST, FED_LIVE_MEADOW_KEY, ...
#
# Scenario 1: pasture listens and goes first, `goatbuilder federate` on both hosts.
# Scenario 2: meadow listens and goes first, peering-standup.sh on both hosts.
# Each scenario starts from `reset`: new CAs, new keys, a new agreement.
#
# --dry-run prints every SSH call and every recorded step and runs nothing, here or there.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
GHCR_TOKEN_ENV="${FED_LIVE_GHCR_TOKEN_ENV:-GHCR_READ_TOKEN}"; GHCR_USER="${FED_LIVE_GHCR_USER:-x-access-token}"
DRY=0; SCEN="${FED_LIVE_SCENARIO:-both}"; RECORD="${FED_LIVE_RECORD:-}"; KEEP=0
GOAT="${FED_LIVE_GOAT:-}"; GB="${FED_LIVE_GOATBUILDER:-}"; LGB="${FED_LIVE_LOCAL_GOATBUILDER:-$(command -v goatbuilder || true)}"
# Read by name in host() and the wrappers below ("H_$s", "U_$s", "K_$s").
# shellcheck disable=SC2034
H_pasture="${FED_LIVE_PASTURE_HOST:-}"; H_meadow="${FED_LIVE_MEADOW_HOST:-}"
# shellcheck disable=SC2034
U_pasture="${FED_LIVE_PASTURE_USER:-ubuntu}"; U_meadow="${FED_LIVE_MEADOW_USER:-ubuntu}"
# shellcheck disable=SC2034
K_pasture="${FED_LIVE_PASTURE_KEY:-}"; K_meadow="${FED_LIVE_MEADOW_KEY:-}"
while [ $# -gt 0 ]; do
  # shellcheck disable=SC2034
  case "$1" in
    --dry-run) DRY=1; shift ;;
    --scenario) SCEN="$2"; shift 2 ;;
    --record) RECORD="$2"; shift 2 ;;
    --goat) GOAT="$2"; shift 2 ;;
    --goatbuilder) GB="$2"; shift 2 ;;
    --local-goatbuilder) LGB="$2"; shift 2 ;;
    --pasture-host) H_pasture="$2"; shift 2 ;;
    --meadow-host) H_meadow="$2"; shift 2 ;;
    --pasture-user) U_pasture="$2"; shift 2 ;;
    --meadow-user) U_meadow="$2"; shift 2 ;;
    --pasture-key) K_pasture="$2"; shift 2 ;;
    --meadow-key) K_meadow="$2"; shift 2 ;;
    --keep) KEEP=1; shift ;;
    --ghcr-token-env) GHCR_TOKEN_ENV="$2"; shift 2 ;;
    --ghcr-user) GHCR_USER="$2"; shift 2 ;;
    -h|--help) sed -n '2,46p' "$0"; exit 0 ;;
    *) echo "live.sh: unknown argument $1" >&2; exit 2 ;;
  esac
done
die() { echo "live.sh: $*" >&2; exit 1; }
say() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
case "$SCEN" in 1|2|both) ;; *) die "--scenario is 1, 2 or both" ;; esac

# --- inputs, named before anything runs --------------------------------------------
missing=()
[ -n "$H_pasture" ] || missing+=(--pasture-host)
[ -n "$H_meadow" ] || missing+=(--meadow-host)
[ "$DRY" = 1 ] || { [ -f "$GOAT" ] || missing+=("--goat (a Linux x86_64 goat)"); [ -f "$GB" ] || missing+=("--goatbuilder (a Linux x86_64 goatbuilder)"); }
for t in ssh git curl openssl python3 tar gzip; do command -v "$t" >/dev/null || missing+=("$t on this laptop"); done
case "$GHCR_TOKEN_ENV" in ''|*[!A-Za-z0-9_]*) missing+=("--ghcr-token-env must name a variable") ;; esac
[ "$DRY" = 1 ] || [ -n "${!GHCR_TOKEN_ENV:-}" ] || missing+=("\$$GHCR_TOKEN_ENV (a token that can read ghcr.io/usareur-af/zenoh-pqc; the image is private)")
python3 -c 'import yaml' 2>/dev/null || missing+=("python PyYAML on this laptop (pip install pyyaml): the SPIRE pin is read from ops/pins/fleet-pins.yaml")
[ "$DRY" = 1 ] || python3 -c 'import jsonschema' 2>/dev/null || missing+=("python jsonschema on this laptop (pip install jsonschema): the run record is validated")
[ "${#missing[@]}" = 0 ] || die "missing: ${missing[*]}"
[ "$DRY" = 1 ] || [ -z "$(git -C "$REPO" status --porcelain -- ops)" ] || die "ops/ has uncommitted changes; the run ships a commit, so commit first"
COMMIT="$(git -C "$REPO" rev-parse HEAD)"
[ -n "$RECORD" ] || RECORD="$(mktemp -d "${TMPDIR:-/tmp}/fed-live.XXXXXX")"
mkdir -p "$RECORD"; RECORD="$(cd "$RECORD" && pwd)"
export E2E_RECORD_DIR="$RECORD"
# A socket path has a 104-byte limit, so it stays short whatever --record says.
SSH_CM_DIR="$(mktemp -d /tmp/fedcm.XXXXXX)"
printf 'commit %s\npasture %s\nmeadow %s\nstarted %s\n' "$COMMIT" "$H_pasture" "$H_meadow" "$(date -u +%FT%TZ)" > "$RECORD/run.txt"

# The same digests run.sh pins.
ZENOH_IMAGE="ghcr.io/usareur-af/zenoh-pqc@sha256:372de39d2e8b64fe7157194173f1b7ac6eac2700ff04c49cce2ce3c284a6fdb2"
SPIRE_IMAGE="$(python3 -c 'import sys, yaml; print(((yaml.safe_load(open(sys.argv[1])).get("components") or {}).get("spire") or {}).get("fleet") or "")' "$REPO/ops/pins/fleet-pins.yaml" 2>/dev/null || true)"
[[ "$SPIRE_IMAGE" =~ ^ghcr\.io/spiffe/spire-server:[^@[:space:]]+@sha256:[0-9a-f]{64}$ ]] || die "ops/pins/fleet-pins.yaml declares no digest-pinned SPIRE image (components.spire.fleet)"
# The outbound republisher: the published image of record, components.goat-fed-republisher.
REPUBLISHER_IMAGE="$(python3 -c 'import sys, yaml; print(str(((yaml.safe_load(open(sys.argv[1])).get("components") or {}).get("goat-fed-republisher") or {}).get("fleet") or "").replace("{{ ghcr_org }}", "usareur-af"))' "$REPO/ops/pins/fleet-pins.yaml" 2>/dev/null || true)"
[[ "$REPUBLISHER_IMAGE" =~ @sha256:[0-9a-f]{64}$ ]] || die "ops/pins/fleet-pins.yaml declares no digest-pinned republisher image (components.goat-fed-republisher.fleet)"

other() { [ "$1" = pasture ] && echo meadow || echo pasture; }
core_addr() { [ "$1" = pasture ] && echo 10.77.1.10 || echo 10.77.2.10; }
host() { eval "echo \$H_$1"; }
port_base() { [ "$1" = pasture ] && echo 18 || echo 28; }   # 18000 core REST, 18101-18103 probes

# One wrapper per host, so a command string handed to acceptance.sh needs no quoting.
BIN="$RECORD/bin"; mkdir -p "$BIN"
for s in pasture meadow; do
  u="$(eval "echo \$U_$s")"; k="$(eval "echo \$K_$s")"
  {
    echo '#!/bin/sh'
    printf 'exec ssh -o BatchMode=yes -o ConnectTimeout=15 -o ServerAliveInterval=15 -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=%q' "$RECORD/known_hosts"
    printf ' -o ControlMaster=auto -o ControlPath=%q -o ControlPersist=600' "$SSH_CM_DIR/$s"
    [ -z "$k" ] || printf ' -i %q -o IdentitiesOnly=yes' "$k"
    printf ' %q "$@"\n' "$u@$(host "$s")"
  } > "$BIN/on-$s"
  chmod +x "$BIN/on-$s"
done

# hx <sovereign> <verb> [args...]: one verb of live-host.sh, as root, on that host.
hx() {
  local s="$1" args cmd; shift
  args="$(printf '%q ' "$@")"
  cmd="sudo env FED_ZENOH_IMAGE=$ZENOH_IMAGE FED_SPIRE_IMAGE=$SPIRE_IMAGE FED_REPUBLISHER_IMAGE=$REPUBLISHER_IMAGE bash /opt/fed-live/live-host.sh $args"
  if [ "$DRY" = 1 ]; then echo "DRY  [$s] live-host.sh $args" >&2; return 0; fi
  "$BIN/on-$s" "$cmd"
}
# hput <sovereign> <remote path> [mode] < stdin
hput() {
  if [ "$DRY" = 1 ]; then cat >/dev/null; echo "DRY  [$1] write $2" >&2; return 0; fi
  "$BIN/on-$1" "sudo install -d /opt/fed-live && sudo bash -c $(printf '%q' "install -d \$(dirname $2); cat > $2; chmod ${3:-0644} $2")"
}
# hget <sovereign> <remote path> > stdout
hget() {
  if [ "$DRY" = 1 ]; then echo "DRY  [$1] read $2" >&2; echo "dry-run"; return 0; fi
  "$BIN/on-$1" "sudo cat $2"
}

REC="$REPO/ops/e2e/rec.py"; FAILS=0
rec() {
  if [ "$DRY" = 1 ]; then
    local name="$1"; shift; while [ $# -gt 0 ] && [ "$1" != "--" ]; do shift; done; shift || true
    echo "DRY  step $name: $*" >&2; return 0
  fi
  python3 "$REC" "$@" || FAILS=$((FAILS + 1))
}

FWD_PIDS=()
forwards() { # the cores' REST and the probes, on this laptop's loopback only
  [ "$DRY" = 1 ] && { echo "DRY  ssh -N -L 18000,18101-18103 to pasture; 28000,28101-28103 to meadow" >&2; return 0; }
  local s b
  for s in pasture meadow; do
    b="$(port_base "$s")"
    "$BIN/on-$s" -N -o ExitOnForwardFailure=yes -L "127.0.0.1:${b}000:127.0.0.1:8000" \
      -L "127.0.0.1:${b}101:127.0.0.1:8101" -L "127.0.0.1:${b}102:127.0.0.1:8102" -L "127.0.0.1:${b}103:127.0.0.1:8103" &
    FWD_PIDS+=($!)
  done
}
LOGGED_IN=()
# The registry credential leaves each host when the run ends, however it ends.
logout_all() {
  [ "$KEEP" = 1 ] && return 0
  local s; for s in "${LOGGED_IN[@]:-}"; do [ -z "$s" ] || hx "$s" logout >/dev/null || true; done
  LOGGED_IN=()
}
finish() {
  logout_all
  local p; for p in "${FWD_PIDS[@]:-}"; do [ -z "$p" ] || kill "$p" 2>/dev/null || true; done
  [ "$DRY" = 1 ] || for s in pasture meadow; do "$BIN/on-$s" -O exit >/dev/null 2>&1 || true; done
}
trap finish EXIT

probe_zid() { printf 'f%s' "$(printf '%s-%s' "$1" "$2" | openssl dgst -sha256 -r | cut -c1-31)"; }

# --- once: the hosts ready, the binaries and the tree in place ----------------------
say "commit $COMMIT, record $RECORD"
TARBALL="$RECORD/ops.tar.gz"
if [ "$DRY" = 1 ]; then echo "DRY  git archive HEAD ops | gzip > $TARBALL" >&2; else git -C "$REPO" archive --format=tar HEAD ops | gzip -c > "$TARBALL"; fi
for s in pasture meadow; do
  hput "$s" /opt/fed-live/live-host.sh 0755 < "$HERE/live-host.sh"
  if [ "$DRY" = 1 ]; then echo "DRY  [$s] install goat and goatbuilder to /usr/local/bin" >&2
  else hput "$s" /usr/local/bin/goat 0755 < "$GOAT"; hput "$s" /usr/local/bin/goatbuilder 0755 < "$GB"; fi
  if [ "$DRY" = 1 ]; then echo "DRY  [$s] write /opt/fed-live/ops.tar.gz" >&2; else hput "$s" /opt/fed-live/ops.tar.gz 0644 < "$TARBALL"; fi
  rec "$s-prereqs" --expect-in "prereqs ok" -- "$BIN/on-$s" "sudo env FED_ZENOH_IMAGE=$ZENOH_IMAGE FED_SPIRE_IMAGE=$SPIRE_IMAGE FED_REPUBLISHER_IMAGE=$REPUBLISHER_IMAGE bash /opt/fed-live/live-host.sh prereqs"
  # The zenoh image is private. The token goes from this shell's variable, through
  # printf (a builtin, so never an argument to a process), down SSH's stdin to
  # `docker login --password-stdin`. Nothing on this laptop holds it in a file.
  if [ "$DRY" = 1 ]; then
    echo "DRY  [$s] docker login ghcr.io -u $GHCR_USER --password-stdin, the token from \$$GHCR_TOKEN_ENV over SSH stdin" >&2
  else
    printf '%s' "${!GHCR_TOKEN_ENV}" | "$BIN/on-$s" "sudo bash /opt/fed-live/live-host.sh login $GHCR_USER" || die "docker login on $s failed"
  fi
  LOGGED_IN+=("$s")
  rec "$s-images" --expect-in "images ok" -- "$BIN/on-$s" "sudo env FED_ZENOH_IMAGE=$ZENOH_IMAGE FED_SPIRE_IMAGE=$SPIRE_IMAGE FED_REPUBLISHER_IMAGE=$REPUBLISHER_IMAGE bash /opt/fed-live/live-host.sh images"
done

scenario() { # $1 number, $2 first side (it also listens), $3 avenue
  local n="$1" first="$2" avenue="$3" second s them w; second="$(other "$first")"
  say "scenario $n: $first listens and goes first; avenue $avenue"
  for s in pasture meadow; do hx "$s" reset; done
  for s in pasture meadow; do
    them="$(other "$s")"
    hx "$s" sovereign "$s" "$them" "$(core_addr "$s")" > "$RECORD/s$n-$s-signing.pub"
    hx "$s" clone "$s" "$them" "$(core_addr "$s")"
    hx "$s" core "$s" "$them" "$(core_addr "$s")"
  done
  hx "$first" edge "$first" "$second" true "$(host "$first"):51820"
  hx "$second" edge "$second" "$first" false "$(host "$second"):51820"

  say "scenario $n: what is reachable between the hosts, before any phase runs"
  for s in pasture meadow; do
    them="$(other "$s")"
    rec "s$n-$s-core-rest-not-on-its-public-address" --expect-exit 1 -- "$BIN/on-$them" "curl -s -m 5 -o /dev/null http://$(host "$s"):8000/ || exit 1"
    rec "s$n-$s-fabric-port-not-on-its-public-address" --expect-exit 1 -- "$BIN/on-$them" "timeout 5 bash -c 'exec 3<>/dev/tcp/$(host "$s")/7447' || exit 1"
  done

  say "scenario $n: phases 1-5, each side"
  for s in "$first" "$second"; do
    rec "s$n-$s-phases-1-to-5" --expect-in "exchange packages" -- "$BIN/on-$s" "sudo bash /opt/fed-live/live-host.sh drive $s $(other "$s") $avenue 1 5"
  done

  say "scenario $n: phase 5, the exchange, as two operators do it"
  for s in pasture meadow; do
    them="$(other "$s")"
    hget "$s" "/var/lib/goat-federation/$them/ours.canonical.json" > "$RECORD/s$n-$s-package.json"
    hget "$s" "/var/lib/goat-federation/$them/ours.spiffe-bundle" > "$RECORD/s$n-$s.spiffe-bundle"
    hget "$s" /opt/fed-live/keys/ca.pem > "$RECORD/s$n-$s-ca.pem"
  done
  for s in pasture meadow; do
    them="$(other "$s")"; w="/var/lib/goat-federation/$them"
    hput "$s" "$w/theirs.canonical.json" < "$RECORD/s$n-$them-package.json"
    hput "$s" "$w/theirs.spiffe-bundle" < "$RECORD/s$n-$them.spiffe-bundle"
    hput "$s" "$w/theirs-ca.pem" < "$RECORD/s$n-$them-ca.pem"
    printf '%s=%s\n%s=%s\n' "$s" "$(cat "$RECORD/s$n-$s-signing.pub")" "$them" "$(cat "$RECORD/s$n-$them-signing.pub")" | hput "$s" "$w/keys.txt"
  done

  say "scenario $n: phases 6-10, each side"
  for s in "$first" "$second"; do
    rec "s$n-$s-phases-6-to-10" --expect-in "review what apply rendered" -- "$BIN/on-$s" "sudo bash /opt/fed-live/live-host.sh drive $s $(other "$s") $avenue 6 10"
  done
  rec "s$n-both-sides-hold-one-agreement" -- bash -c \
    "a=\$('$BIN/on-pasture' sudo sha256sum /var/lib/goat-federation/meadow/agreement.json | cut -d' ' -f1); b=\$('$BIN/on-meadow' sudo sha256sum /var/lib/goat-federation/pasture/agreement.json | cut -d' ' -f1); echo \"\$a \$b\"; [ -n \"\$a\" ] && [ \"\$a\" = \"\$b\" ]"
  rec "s$n-the-agreement-selected-wireguard" --expect-in '"kind":"wireguard"' -- "$BIN/on-pasture" sudo cat /var/lib/goat-federation/meadow/agreement.json
  for s in "$first" "$second"; do
    rec "s$n-$s-review-and-admit" --expect-in "admit ok" -- "$BIN/on-$s" "sudo bash /opt/fed-live/live-host.sh admit $s $(other "$s")"
  done

  say "scenario $n: phases 11-12, each side: the transport, then the edge"
  for s in "$first" "$second"; do
    rec "s$n-$s-phases-11-to-12" --expect-in "open the wireguard transport" -- "$BIN/on-$s" "sudo bash /opt/fed-live/live-host.sh drive $s $(other "$s") $avenue 11 12"
  done
  rec "s$n-a-wireguard-handshake" -- bash -c \
    "for _ in \$(seq 1 30); do t=\$('$BIN/on-pasture' sudo wg show wg-fed0 latest-handshakes | cut -f2); [ \"\${t:-0}\" != 0 ] && break; sleep 2; done; echo \"latest handshake \$t\"; [ \"\${t:-0}\" != 0 ]"

  say "scenario $n: phase 13, each side: the SPIRE trust domains"
  for s in "$first" "$second"; do
    rec "s$n-$s-phase-13" --expect-in "federate the SPIRE trust domains" -- "$BIN/on-$s" "sudo bash /opt/fed-live/live-host.sh drive $s $(other "$s") $avenue 13 13"
  done

  say "scenario $n: phase 14 from both sides, from this laptop"
  for s in pasture meadow; do
    them="$(other "$s")"
    hx "$s" responder "$s" "$them"
    hx "$s" probe "$s" "$them" agreed "/var/lib/goat-federation/$them/pki/peering-outward.pem" "/var/lib/goat-federation/$them/pki/peering-outward-key.pem" 8101 "$(probe_zid "$s" agreed)"
    hx "$s" probe "$s" "$them" forged /opt/fed-live/keys/forged.pem /opt/fed-live/keys/forged-key.pem 8102 "$(probe_zid "$s" forged)"
    hx "$s" probe "$s" "$them" impersonate /opt/fed-live/keys/impersonate.pem /opt/fed-live/keys/impersonate-key.pem 8103 "$(probe_zid "$s" impersonate)"
  done
  for s in "$first" "$second"; do accept_as "$n" "$s" "$avenue"; done

  say "scenario $n: the failures that make the passes mean something"
  for s in pasture meadow; do
    rec "s$n-$s-core-trusts-only-its-own-ca" -- bash -c \
      "[ \"\$('$BIN/on-$s' sudo grep -c BEGIN /etc/goat-core/ca.pem)\" = 1 ] && [ \"\$('$BIN/on-$s' sudo openssl x509 -in /etc/goat-core/ca.pem -noout -subject)\" = 'subject=O = $s, OU = offline-ca, CN = $s Root CA' ]"
    rec "s$n-$s-tunnel-carried-it" -- bash -c "'$BIN/on-$s' sudo wg show wg-fed0 transfer | awk '{print; if (\$2 < 1 || \$3 < 1) exit 1}'"
  done
  local b; b="$(port_base "$first")"
  local rt="BASE=http://127.0.0.1:${b}000 '$REPO/ops/federation/roundtrip/fedcheck.sh' release/all from/$second"
  rec "s$n-cut-the-link" -- "$BIN/on-$second" sudo systemctl stop goat-peering-wg@wg-fed0
  rec "s$n-round-trip-fails-when-cut" --expect-exit 1 --expect-in "FAIL" -- bash -c "$rt 15"
  rec "s$n-restore-the-link" -- "$BIN/on-$second" sudo systemctl start goat-peering-wg@wg-fed0
  rec "s$n-round-trip-passes-again" --expect-in "PASS" -- bash -c \
    "for _ in 1 2 3 4 5 6; do out=\$($rt 20) && { echo \"\$out\"; exit 0; }; sleep 10; done; echo \"\$out\"; exit 1"
  local lim="--limit ${first}_edge -e fed_edge_group=${first}_edge -e fed_work=/var/lib/goat-federation/$first -e fed_counterparty=/var/lib/goat-federation/$first/theirs.canonical.json"
  rec "s$n-wireguard-refused-without-a-waiver" --expect-exit 2 --expect-in "goat-peering-wg refuses" -- "$BIN/on-$second" \
    "sudo bash /opt/fed-live/live-host.sh ansible -i inventory/peering-edges.ini $lim -e peering_wg_fips_waived_reason= peering-wg.yml"
  rec "s$n-wireguard-refused-under-another-transport" --expect-exit 2 --expect-in "goat-peering-wg refuses" -- "$BIN/on-$second" \
    "sudo bash /opt/fed-live/live-host.sh ansible -i inventory/peering-edges.ini $lim -e goat_peering_transport_kind=ipsec-fips -e peering_wg_kind=ipsec-fips peering-wg.yml"
  for s in pasture meadow; do
    if [ "$DRY" = 1 ]; then echo "DRY  [$s] collect > $RECORD/s$n-$s-host.log" >&2; else hx "$s" collect "$s" "$(other "$s")" > "$RECORD/s$n-$s-host.log" 2>&1 || true; fi
  done
}

# Phase 14 runs where one party owns both sides: only a drill has that. The driver's own
# acceptance.sh, unmodified, against this laptop's forwards to each host's loopback.
accept_as() { # $1 scenario, $2 sovereign, $3 avenue
  local n="$1" s="$2" avenue="$3" them w b tb clone; them="$(other "$s")"; b="$(port_base "$s")"; tb="$(port_base "$them")"
  w="$RECORD/s$n-$s-fedwork"; mkdir -p "$w"
  hget "$s" "/var/lib/goat-federation/$them/agreement.json" > "$w/agreement.json"
  local env=(
    "FED_WORK=$w" "GOAT_AWS_MODE=none" "FED_ACCEPTANCE_STRICT=1" "FED_NO_DIRECTORY=1"
    "FED_OURS_REST=http://127.0.0.1:${b}000" "FED_THEIRS_REST=http://127.0.0.1:${tb}000"
    "FED_EDGE_LOG_OURS=$BIN/on-$s sudo docker logs goat-zenoh-edge-$them"
    "FED_PROBE_THEIRS_REST=http://127.0.0.1:${tb}101"
    "FED_PROBE_FORGED_REST=http://127.0.0.1:${tb}102" "FED_PROBE_FORGED_ZID=$(probe_zid "$them" forged)"
    "FED_PROBE_IMPERSONATE_REST=http://127.0.0.1:${tb}103" "FED_PROBE_IMPERSONATE_ZID=$(probe_zid "$them" impersonate)"
    "FED_SPIRE_OURS=$BIN/on-$s sudo docker exec spire-server /opt/spire/bin/spire-server"
    "FED_SPIRE_THEIRS=$BIN/on-$them sudo docker exec spire-server /opt/spire/bin/spire-server"
  )
  if [ "$avenue" = goatbuilder ] && [ -n "$LGB" ]; then
    # goatbuilder refuses an undeclared peering (C1), so it reads this sovereign's own
    # declaration, fetched from its host, over a copy of the shipped tree.
    clone="$RECORD/s$n-$s-tree"; rm -rf "$clone"; mkdir -p "$clone"
    if [ "$DRY" = 0 ]; then
      tar -xzf "$TARBALL" -C "$clone"; git -C "$clone" init -q
      hget "$s" /opt/goat/ops/goatbuilder/goatbuilder.yaml > "$clone/ops/goatbuilder/goatbuilder.yaml"
    fi
    rec "s$n-acceptance-as-$s" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" --expect-not-in "ACCEPTANCE FAILED" --expect-not-in "ACCEPTANCE INCOMPLETE" \
      -- env "${env[@]}" "$LGB" -C "$clone" --yes federate "$them" --as "$s" --work "$w" --from 14 --to 14
  else
    rec "s$n-acceptance-as-$s" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" --expect-not-in "ACCEPTANCE FAILED" --expect-not-in "ACCEPTANCE INCOMPLETE" \
      -- env "${env[@]}" "$REPO/ops/scripts/peering-standup.sh" "$them" --as "$s" --from 14 --to 14
  fi
}

forwards
if [ "$SCEN" = 1 ] || [ "$SCEN" = both ]; then scenario 1 pasture goatbuilder; fi
if [ "$SCEN" = 2 ] || [ "$SCEN" = both ]; then scenario 2 meadow script; fi
if [ "$KEEP" = 0 ]; then for s in pasture meadow; do hx "$s" reset; done; fi
logout_all

say "the record"
[ "$DRY" = 1 ] && { echo "DRY  nothing ran; the record directory holds only run.txt and the host wrappers"; exit 0; }
grep -h "acceptance-counts:" "$RECORD"/*acceptance-as-*.log | tee "$RECORD/acceptance-counts.txt" || true
python3 "$REPO/ops/e2e/report.py" "$RECORD" --check --title "cross-sovereign federation, live hosts" \
  || { echo "live.sh: the run record did not validate (above)" >&2; FAILS=$((FAILS + 1)); }
echo "record: $RECORD"
[ "$FAILS" = 0 ] && exit 0 || exit 1
