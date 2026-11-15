#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# probe-identities.sh — the launcher a side gives phase 14 for its own probes
# (FED_PROBE_LAUNCHER_OURS=ops/federation/standup/probe-identities.sh), in the contract
# acceptance.sh documents (docs/design/federation-acceptance.md §2.4). It gives each probe
# its identity from OUR peering CA and hands it to the probe launcher (probe-launcher.sh),
# which installs it on our edge host:
#   agreed       a copy of our outward leaf (FED_PROBE_PKI/peering-outward{,-key}.pem),
#                the leaf the edge already presents; nothing new is issued
#   forged       a leaf issued for this run: mint-peering-leaves.sh --probe <run id>
#   impersonate  a leaf issued for this run bearing the partner's core's name, with
#                --impersonate-for-run <run id>
#
#   probe-identities.sh start <agreed|forged|impersonate> <run id> <deadline, epoch seconds>
#       prints what the probe launcher prints (HANDLE=, REST=, ZID=, KEY_DELETED= on a
#       failed start), then KEY_DELETED_DRIVER=verified once this host's copy of the
#       identity is gone, or KEY_DELETED_DRIVER=failed and exit 1 when it is still there
#   probe-identities.sh stop <handle> | alive <handle>    the probe launcher's, unchanged
#
# THE DRIVER COPY. The identity is put in a fresh 0700 folder on memory-backed storage
# (FED_CA_KEY_TMP when set, as for the CA key; else /dev/shm; nothing else), only for as
# long as the probe launcher's start takes to copy it to the edge host. The folder is
# deleted on any exit (a success, a failure, an interrupt, a termination), and checked.
#
# Settings, besides the probe launcher's own (FED_PL_*) and the CA key's (FED_CA_CERT,
# FED_CA_KEY or FED_CA_KEY_FROM_AGE with FED_CA_KEY_AGE and FED_AGE_IDENTITY):
#   FED_PROBE_AGREEMENT        the agreement the leaves' names derive from
#   FED_PROBE_AS               which party we are, when the agreement does not say
#   FED_PROBE_PKI              our peering PKI folder, holding the outward leaf and its key
#   FED_PROBE_FAR_CA           the partner's CA certificate, which the probe trusts its edge by
#   FED_PROBE_ISSUED_RECORD    our CA's issued-certificates record
#   FED_PROBE_PARTNER_CORE_CN  the partner's core's name (impersonate only; never defaulted)
#   FED_PROBE_PL               the probe launcher (default probe-launcher.sh beside this)
#   FED_PROBE_TTL              the probe lifetime acceptance.sh uses (default 900)
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
PL="${FED_PROBE_PL:-$HERE/probe-launcher.sh}"
die() { echo "probe-identities: $1 ($2)" >&2; exit "${3:-2}"; }
need() { [ -n "$2" ] || die "$1 is not set: $3" X-PROBE-001; }

verb="${1:-}"
case "$verb" in
  stop|alive) exec "$PL" "$@" ;;
  start) ;;
  *) die "start|stop|alive" X-PROBE-001 ;;
esac
[ $# -eq 4 ] || die "start <agreed|forged|impersonate> <run id> <deadline, epoch seconds>" X-PROBE-001
kind="$2" run="$3" deadline="$4"
case "$kind" in agreed|forged|impersonate) ;; *) die "unknown probe kind '$kind'" X-PROBE-001 ;; esac
need FED_PROBE_FAR_CA "${FED_PROBE_FAR_CA:-}" "the partner's CA certificate"
[ -r "$FED_PROBE_FAR_CA" ] || die "FED_PROBE_FAR_CA ($FED_PROBE_FAR_CA) cannot be read" X-PROBE-001
if [ "$kind" = agreed ]; then
  need FED_PROBE_PKI "${FED_PROBE_PKI:-}" "our peering PKI folder, holding the outward leaf"
  for f in peering-outward.pem peering-outward-key.pem; do
    [ -r "$FED_PROBE_PKI/$f" ] || die "FED_PROBE_PKI has no readable $f" X-PROBE-001
  done
else
  need FED_PROBE_AGREEMENT "${FED_PROBE_AGREEMENT:-}" "the agreement the probe leaf's name derives from"
  need FED_PROBE_ISSUED_RECORD "${FED_PROBE_ISSUED_RECORD:-}" "our CA's issued-certificates record"
  [ "$kind" != impersonate ] || need FED_PROBE_PARTNER_CORE_CN "${FED_PROBE_PARTNER_CORE_CN:-}" \
    "the partner's core's name, which the impersonation leaf bears"
  # The forged probe needs our PKI folder too only in the agreed case; checked above.
fi

# Memory-backed storage only: FED_CA_KEY_TMP (the operator's choice for the CA key, too), else
# /dev/shm. A disk is never a fallback for a private key.
parent="${FED_CA_KEY_TMP:-}"
if [ -z "$parent" ]; then
  shm="${FED_PROBE_SHM:-/dev/shm}"
  [ -d "$shm" ] && [ -w "$shm" ] || die "no memory-backed storage for the probe identity: $shm is not a writable folder; set FED_CA_KEY_TMP to one" X-PROBE-001
  parent="$shm"
fi
[ -d "$parent" ] && [ -w "$parent" ] || die "FED_CA_KEY_TMP ($parent) is not a writable folder" X-PROBE-001

DIR=""; CHILD=""
# shellcheck disable=SC2329 # called by the traps below
driver_exit() {
  local rc=$?
  trap - EXIT INT TERM HUP
  # A start still running is ended first, so its own exit trap removes what it put on the
  # edge host, before this host's copy goes.
  if [ -n "$CHILD" ] && kill -0 "$CHILD" 2>/dev/null; then
    # The start's shell runs its traps only once its foreground command ends, so that
    # command (a sleep, a curl or an ssh) is ended first, then the start itself, which
    # removes the probe as it exits. In the other order a start without a trap dies at
    # once and its command, no longer its child, runs on.
    pkill -TERM -P "$CHILD" 2>/dev/null
    kill -TERM "$CHILD" 2>/dev/null
    wait "$CHILD" 2>/dev/null
  fi
  if [ -n "$DIR" ]; then
    rm -rf "$DIR"
    if [ -e "$DIR" ]; then
      echo KEY_DELETED_DRIVER=failed
      echo "probe-identities: the $kind probe's identity is still on this host at $DIR; remove it by hand (X-PROBE-003)" >&2
      [ "$rc" -ne 0 ] || rc=1
    else
      echo KEY_DELETED_DRIVER=verified
    fi
  fi
  exit "$rc"
}
trap driver_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

DIR="$(umask 077 && mktemp -d "$parent/goat-probe-ids.XXXXXX")" || { DIR=""; die "no private folder under $parent" X-PROBE-001; }
chmod 0700 "$DIR"
case "$kind" in
  agreed)
    ( umask 077 && cp "$FED_PROBE_PKI/peering-outward.pem" "$DIR/agreed-cert.pem" \
        && cp "$FED_PROBE_PKI/peering-outward-key.pem" "$DIR/agreed-key.pem" ) || die "could not copy the outward leaf" X-PROBE-001 1 ;;
  forged|impersonate)
    margs=("$FED_PROBE_AGREEMENT" "$DIR" --probe "$run" --probe-kind "$kind" --issued-record "$FED_PROBE_ISSUED_RECORD")
    [ -z "${FED_PROBE_AS:-}" ] || margs+=(--as "$FED_PROBE_AS")
    [ "$kind" != impersonate ] || margs+=(--impersonate-for-run "$run" --partner-core-cn "$FED_PROBE_PARTNER_CORE_CN")
    # The probe folder must be empty when the leaf is issued; the partner's CA goes in after.
    bash "$HERE/mint-peering-leaves.sh" "${margs[@]}" >&2 || die "the $kind probe leaf was not issued" X-PROBE-001 1 ;;
esac
cp "$FED_PROBE_FAR_CA" "$DIR/far-ca.pem" || die "could not copy the partner's CA certificate" X-PROBE-001 1
chmod 0600 "$DIR/$kind-key.pem"

# Not exec, and waited for: the folder is deleted when the launcher's start returns, whatever
# it returns, and a signal to this script is handled while the start runs.
FED_PL_DIR="$DIR" "$PL" start "$kind" "$run" "$deadline" &
CHILD=$!
wait "$CHILD"; rc=$?
CHILD=""
exit "$rc"
