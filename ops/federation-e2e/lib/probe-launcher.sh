#!/usr/bin/env bash
# probe-launcher.sh — the drill's probe launcher for run-owned acceptance probes
# (FED_PROBE_LAUNCHER, docs/design/federation-acceptance.md §2.4). Phase 14 calls it; the
# harness starts no probe itself.
#
#   probe-launcher.sh start <agreed|forged|impersonate> <run id> <deadline, epoch seconds>
#       prints REST=<url>, ZID=<zenoh id>, HANDLE=<handle>; the session ends at the deadline
#   probe-launcher.sh stop <handle>
#   probe-launcher.sh alive <handle>     exit 0 while it runs, 1 once it is gone
#
# A probe is a zenoh client session on the PARTNER's edge host, holding one of the
# partner's identities (its agreed leaf, a name nobody agreed, a leaf naming our core) and
# dialling our edge across the link: only the one party of a drill, which owns both sides,
# can launch it there. Its settings come from the environment the harness gives phase 14:
#   FE2E_PL_SSH       user@address of the partner's edge host, reached over ssh (phase 14 run
#                     on our driver host, as an operator runs it); or
#   FE2E_PL_HOST      the partner's edge host as a container on this machine (docker exec)
#   FE2E_PL_REST_IP   that host's address the caller reaches its REST port at
#   FE2E_PL_FAR_IP    our edge's link address, as the partner's edge reaches it
#   FE2E_PL_DIR       a folder holding <kind>-cert.pem, <kind>-key.pem and far-ca.pem
#   FE2E_PL_PORT      the first REST port; agreed, forged, impersonate take +0, +1, +2
#   FE2E_PL_TEMPLATE  probe.json5          FE2E_PL_IMAGE  the zenoh image
set -euo pipefail
: "${FE2E_PL_REST_IP:?}" "${FE2E_PL_FAR_IP:?}" "${FE2E_PL_DIR:?}" "${FE2E_PL_PORT:?}" "${FE2E_PL_TEMPLATE:?}" "${FE2E_PL_IMAGE:?}"
[ -n "${FE2E_PL_SSH:-}" ] || : "${FE2E_PL_HOST:?FE2E_PL_SSH or FE2E_PL_HOST}"
# A command on the partner's edge host, as root.
on_far() { if [ -n "${FE2E_PL_SSH:-}" ]; then ssh "$FE2E_PL_SSH" "sudo sh -c '$*'"; else docker exec "$FE2E_PL_HOST" sh -c "$*"; fi; }
put_far() { # $1 local file, $2 path there
  if [ -n "${FE2E_PL_SSH:-}" ]; then ssh "$FE2E_PL_SSH" "sudo tee '$2' >/dev/null" < "$1"; else docker cp -q "$1" "$FE2E_PL_HOST:$2"; fi
}
verb="${1:-}"
case "$verb" in
  start)
    kind="$2" run="$3" deadline="$4"
    case "$kind" in agreed) off=0 ;; forged) off=1 ;; impersonate) off=2 ;; *) echo "probe-launcher: unknown probe kind $kind" >&2; exit 2 ;; esac
    port=$((FE2E_PL_PORT + off))
    name="fe2e-probe-$kind-$(printf '%s' "$run" | tr -c 'a-zA-Z0-9-' - | cut -c1-40)"
    zid="f$(printf '%s-%s' "$run" "$kind" | sha256sum | cut -c1-31)"
    dir="/etc/fe2e-probe/$name"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    sed "s/__FAR_LINK_IP__/$FE2E_PL_FAR_IP/; s/__REST_ADDR__/0.0.0.0/; s/__REST_PORT__/$port/; s/__ZID__/$zid/" "$FE2E_PL_TEMPLATE" > "$tmp/config.json5"
    cp "$FE2E_PL_DIR/$kind-cert.pem" "$tmp/cert.pem"; cp "$FE2E_PL_DIR/$kind-key.pem" "$tmp/key.pem"; cp "$FE2E_PL_DIR/far-ca.pem" "$tmp/far-ca.pem"
    chmod 0644 "$tmp"/*
    on_far "install -d -m 0755 $dir"
    for f in config.json5 cert.pem key.pem far-ca.pem; do put_far "$tmp/$f" "$dir/$f"; done
    on_far "chmod 0644 $dir/*; docker rm -f $name >/dev/null 2>&1; docker run -d --name $name --network host -v $dir:/probe:ro -e RUST_LOG=zenoh=info $FE2E_PL_IMAGE -c /probe/config.json5 >/dev/null"
    # The session ends by itself at the deadline, whether or not stop is ever called.
    ttl=$((deadline - $(date +%s))); [ "$ttl" -gt 0 ] || ttl=1
    on_far "nohup sh -c \"sleep $ttl; docker rm -f $name >/dev/null 2>&1; rm -rf $dir\" >/dev/null 2>&1 &"
    for _ in $(seq 1 30); do
      curl -s -m 2 -o /dev/null "http://$FE2E_PL_REST_IP:$port/@/**" && break
      sleep 2
    done
    printf 'REST=http://%s:%s\nZID=%s\nHANDLE=%s\n' "$FE2E_PL_REST_IP" "$port" "$zid" "$name"
    ;;
  stop)
    on_far "docker rm -f $2 >/dev/null 2>&1; rm -rf /etc/fe2e-probe/$2"
    ;;
  alive)
    on_far "docker inspect $2 >/dev/null 2>&1" && exit 0
    exit 1
    ;;
  *) echo "probe-launcher: start|stop|alive" >&2; exit 2 ;;
esac
