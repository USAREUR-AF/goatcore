#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# probe-launcher.sh — a deployment's probe launcher for phase 14's run-owned acceptance
# probes (docs/design/federation-acceptance.md §2.4). acceptance.sh calls it; set
#   FED_PROBE_LAUNCHER_OURS=ops/federation/standup/probe-launcher.sh   (a side's own, §6.4), or
#   FED_PROBE_LAUNCHER=ops/federation/standup/probe-launcher.sh        (a drill's, which owns both sides)
#
#   probe-launcher.sh start <agreed|forged|impersonate> <run id> <deadline, epoch seconds>
#       prints HANDLE=<handle>, then REST=<url> and ZID=<zenoh id>, one per line; the session
#       ends by itself at the deadline. Exit 1 when the session did not answer (it is removed).
#   probe-launcher.sh stop <handle>
#       removes the session and its key, and prints KEY_DELETED=verified once the edge host
#       shows the key gone; KEY_DELETED=failed and exit 1 when it is still there
#   probe-launcher.sh alive <handle>     exit 0 while it runs, 1 once it is gone
#
# A start that does not end with a session that answered (it fails, or is interrupted or
# terminated) removes what it put on the edge host and prints KEY_DELETED=<verified|failed>.
#
# A probe is a zenoh client session in a container on one edge host, holding one identity
# and dialling the far edge across the link. Which identities it holds is the caller's
# choice, by the certificates in FED_PL_DIR:
#   - a side's own launcher runs on OUR edge host with OUR identities (our agreed leaf, a
#     name nobody agreed from our CA, a leaf from our CA naming the partner's core) and
#     dials the partner's edge;
#   - a drill's launcher runs on the PARTNER's edge host with the partner's identities.
# The launcher carries no identity and mints nothing: it installs the files it is given,
# and cannot tell whose identity a certificate holds.
#
# Settings, from the environment phase 14 runs in:
#   FED_PL_SSH        user@address of the edge host, reached over ssh with sudo; or
#   FED_PL_LOCAL=1    this host is the edge host
#   FED_PL_REST_IP    the edge host's IPv4 address the probe's REST port binds to, and
#                     the caller reaches it at (never 0.0.0.0)
#   FED_PL_FAR_IP     the far edge's IPv4 link address, as the edge host reaches it
#   FED_PL_FAR_PORT   the far edge's link port (default 7448, the edge's listen port)
#   FED_PL_DIR        a folder holding <kind>-cert.pem and <kind>-key.pem for each kind,
#                     and far-ca.pem (the far side's CA certificate, which the probe
#                     trusts the far edge by)
#   FED_PL_PORT       the first REST port; agreed, forged, impersonate take +0, +1, +2
#   FED_PL_PINS       the edge host's group_vars pins.yml: its zenoh_router_image, pinned
#                     by digest, is the probe's image
#   FED_PL_TEMPLATE   the probe's zenoh config (default: probe.json5 beside this script)
#   FED_PL_STATE      where the probe's files live on the edge host (default
#                     /run/goat-probe: tmpfs, so a key never outlives a reboot)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
die() { echo "probe-launcher: $1 ($2)" >&2; exit "${3:-2}"; }
need() { # VAR VALUE WHAT: a required setting, named in the error when it is missing
  [ -n "$2" ] || die "$1 is not set: $3" X-PROBE-001
}
safe() { # VAR VALUE PATTERN: a setting that goes into a command on the edge host must match
  printf '%s' "$2" | grep -Eqx "$3" || die "$1 is not a well-formed value: '$2'" X-PROBE-001
}

if [ -z "${FED_PL_SSH:-}" ] && [ "${FED_PL_LOCAL:-}" != 1 ]; then
  die "FED_PL_SSH (user@address of the edge host) or FED_PL_LOCAL=1 (this host is the edge host) is not set" X-PROBE-001
fi
[ -z "${FED_PL_SSH:-}" ] || safe FED_PL_SSH "$FED_PL_SSH" '[A-Za-z0-9._-]+@[A-Za-z0-9.:_-]+'
STATE="${FED_PL_STATE:-/run/goat-probe}"
safe FED_PL_STATE "$STATE" '/[A-Za-z0-9._/-]+'

# Run a script on the edge host as root; the script comes on stdin, never inside a quoted
# argument, so no value is parsed twice.
on_host() {
  if [ -n "${FED_PL_SSH:-}" ]; then ssh -o BatchMode=yes "$FED_PL_SSH" sudo sh -s
  elif [ "$(id -u)" = 0 ]; then sh -s
  else sudo sh -s; fi
}
# Write stdin to PATH on the edge host with MODE, created private first.
put_host() { # MODE PATH
  if [ -n "${FED_PL_SSH:-}" ]; then ssh -o BatchMode=yes "$FED_PL_SSH" sudo sh -c "'umask 077; cat > $2 && chmod $1 $2'"
  elif [ "$(id -u)" = 0 ]; then sh -c "umask 077; cat > $2 && chmod $1 $2"
  else sudo sh -c "umask 077; cat > $2 && chmod $1 $2"; fi
}
handle_ok() { printf '%s' "$1" | grep -Eqx 'goat-probe-(agreed|forged|impersonate)-[A-Za-z0-9-]{1,40}'; }
hex() { if command -v sha256sum >/dev/null 2>&1; then sha256sum; else shasum -a 256; fi; }
# Remove a probe (its container and its folder, the key with it) and check on the edge host
# that the key is gone. Prints KEY_DELETED=verified or KEY_DELETED=failed; prints nothing when
# the edge host did not answer.
remove_probe() { # HANDLE
  on_host <<EOF 2>/dev/null || true
docker rm -f "$1" >/dev/null 2>&1 || true
rm -rf "$STATE/$1"
if [ -e "$STATE/$1/key.pem" ]; then echo KEY_DELETED=failed; else echo KEY_DELETED=verified; fi
EOF
}

verb="${1:-}"
case "$verb" in
  start)
    [ $# -eq 4 ] || die "start <agreed|forged|impersonate> <run id> <deadline, epoch seconds>" X-PROBE-001
    kind="$2" run="$3" deadline="$4"
    case "$kind" in agreed) off=0 ;; forged) off=1 ;; impersonate) off=2 ;; *) die "unknown probe kind '$kind'" X-PROBE-001 ;; esac
    printf '%s' "$deadline" | grep -Eqx '[0-9]{1,12}' || die "the deadline is not epoch seconds: '$deadline'" X-PROBE-001
    need FED_PL_REST_IP "${FED_PL_REST_IP:-}" "the edge host's address the probe's REST port binds to"
    need FED_PL_FAR_IP "${FED_PL_FAR_IP:-}" "the far edge's link address"
    need FED_PL_DIR "${FED_PL_DIR:-}" "the folder holding the probe identities"
    need FED_PL_PORT "${FED_PL_PORT:-}" "the first REST port"
    need FED_PL_PINS "${FED_PL_PINS:-}" "the edge host's group_vars pins.yml"
    safe FED_PL_REST_IP "$FED_PL_REST_IP" '[0-9]{1,3}(\.[0-9]{1,3}){3}'
    safe FED_PL_FAR_IP "$FED_PL_FAR_IP" '[0-9]{1,3}(\.[0-9]{1,3}){3}'
    [ "$FED_PL_REST_IP" != 0.0.0.0 ] || die "FED_PL_REST_IP must name one address, not every address: the probe's REST port writes as the probe's identity" X-PROBE-001
    safe FED_PL_PORT "$FED_PL_PORT" '[0-9]{1,5}'
    FAR_PORT="${FED_PL_FAR_PORT:-7448}"
    safe FED_PL_FAR_PORT "$FAR_PORT" '[0-9]{1,5}'
    template="${FED_PL_TEMPLATE:-$HERE/probe.json5}"
    [ -r "$template" ] || die "the probe config template $template cannot be read" X-PROBE-001
    for f in "$kind-cert.pem" "$kind-key.pem" far-ca.pem; do
      [ -r "$FED_PL_DIR/$f" ] || die "FED_PL_DIR has no readable $f" X-PROBE-001
    done
    [ -r "$FED_PL_PINS" ] || die "FED_PL_PINS ($FED_PL_PINS) cannot be read" X-PROBE-002
    image=$(sed -n 's/^zenoh_router_image:[[:space:]]*//p' "$FED_PL_PINS" | head -1 | sed 's/[[:space:]]*#.*$//; s/^["'\'']//; s/["'\'']$//')
    printf '%s' "$image" | grep -Eqx '[A-Za-z0-9./_:-]+@sha256:[0-9a-f]{64}' \
      || die "$FED_PL_PINS has no zenoh_router_image pinned by digest (found '$image')" X-PROBE-002

    port=$((FED_PL_PORT + off))
    name="goat-probe-$kind-$(printf '%s' "$run" | tr -c 'a-zA-Z0-9-' '-' | cut -c1-40)"
    zid="f$(printf '%s-%s' "$run" "$kind" | hex | cut -c1-31)"
    dir="$STATE/$name"
    ttl=$((deadline - $(date +%s))); [ "$ttl" -gt 0 ] || ttl=1

    # From here on the key may be on the edge host. Any way out but a session that answered
    # (a failure, an interrupt, a termination) removes the probe and checks its key is gone.
    STARTED_OK=""; INTERRUPTED=""
    # shellcheck disable=SC2329 # called by the EXIT trap below
    start_exit() {
      local rc=$? del
      [ -z "$STARTED_OK" ] || return 0
      del=$(remove_probe "$name")
      printf '%s\n' "${del:-KEY_DELETED=unknown}"
      [ -z "$INTERRUPTED" ] || echo "probe-launcher: interrupted; removed $name (${del:-the edge host did not answer}) (X-PROBE-003)" >&2
      exit "$rc"
    }
    trap start_exit EXIT
    trap 'INTERRUPTED=1; exit 130' INT
    trap 'INTERRUPTED=1; exit 143' TERM

    on_host <<EOF
set -e
install -d -m 0700 "$STATE" "$dir"
docker rm -f "$name" >/dev/null 2>&1 || true
EOF
    sed "s/__FAR_LINK_IP__/$FED_PL_FAR_IP/; s/__FAR_LINK_PORT__/$FAR_PORT/; s/__REST_ADDR__/$FED_PL_REST_IP/; s/__REST_PORT__/$port/; s/__ZID__/$zid/" "$template" | put_host 0644 "$dir/config.json5"
    put_host 0644 "$dir/cert.pem" < "$FED_PL_DIR/$kind-cert.pem"
    put_host 0600 "$dir/key.pem" < "$FED_PL_DIR/$kind-key.pem"
    put_host 0644 "$dir/far-ca.pem" < "$FED_PL_DIR/far-ca.pem"
    # The session ends by itself at the deadline, whether or not stop is ever called. The
    # timer keeps no descriptor of its caller open, so nothing waiting on one is held.
    on_host <<EOF
set -e
docker run -d --name "$name" --label goat-probe=1 --network host -v "$dir:/probe:ro" -e RUST_LOG=zenoh=info "$image" -c /probe/config.json5 >/dev/null
nohup sh -c 'sleep $ttl; docker rm -f "$name" >/dev/null 2>&1; rm -rf "$dir"' </dev/null >/dev/null 2>&1 3>&- &
EOF
    # The handle first: a run that is stopped from here on still knows what to clean up.
    printf 'HANDLE=%s\n' "$name"
    for _ in $(seq 1 30); do
      if curl -s -m 2 -o /dev/null "http://$FED_PL_REST_IP:$port/@/**"; then
        printf 'REST=http://%s:%s\nZID=%s\n' "$FED_PL_REST_IP" "$port" "$zid"
        STARTED_OK=1
        exit 0
      fi
      sleep "${FED_PL_WAIT_STEP:-2}"
    done
    die "the $kind probe's REST port http://$FED_PL_REST_IP:$port did not answer; the probe is removed" X-PROBE-003 1
    ;;
  stop)
    if [ $# -ne 2 ] || ! handle_ok "$2"; then die "stop <handle>: not a handle this launcher gives: '${2:-}'" X-PROBE-001; fi
    del=$(remove_probe "$2")
    case "$del" in
      KEY_DELETED=verified) echo "$del" ;;
      KEY_DELETED=failed) echo "$del"; die "the key of $2 is still on the edge host after stop" X-PROBE-003 1 ;;
      *) die "the edge host did not answer while stopping $2" X-PROBE-003 2 ;;
    esac
    ;;
  alive)
    if [ $# -ne 2 ] || ! handle_ok "$2"; then die "alive <handle>: not a handle this launcher gives: '${2:-}'" X-PROBE-001; fi
    # Only a host that answered "gone" is gone (exit 1): a host that cannot be reached, or
    # any other answer, is exit 2, so acceptance.sh never counts an unknown as clean.
    ans=$(printf 'if docker inspect "%s" >/dev/null 2>&1; then echo alive; else echo gone; fi\n' "$2" | on_host 2>/dev/null) || ans=""
    case "$ans" in
      alive) exit 0 ;;
      gone) exit 1 ;;
      *) echo "probe-launcher: the edge host did not answer whether $2 runs (X-PROBE-003)" >&2; exit 2 ;;
    esac
    ;;
  *) die "start|stop|alive" X-PROBE-001 ;;
esac
