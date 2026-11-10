#!/usr/bin/env bash
# capacity-probe — measure link CAPACITY (what the link CAN do, i.e. a speed
# test), NOT usage. This is the honest answer to "how fast is this link" — it
# actively runs a short iperf3 transfer to a stable target and reports the
# achieved bits/s. Distinct from host-net-ratesample.sh, which reports peak
# USAGE (only reaches capacity if real traffic happens to saturate the link).
#
# Runs hourly (systemd timer) AND on demand. On-demand is rate-limited to at
# most once per CAPACITY_MIN_INTERVAL_S (default 60s) via a lockfile mtime, and
# each run is bounded to CAPACITY_DURATION_S (default 4s, ~5s wall) so it never
# puts synthetic load on a link for long. Measures the MESH path (iperf3 over
# WireGuard to the target's mesh IP) and, when configured, the PUBLIC path
# (same target's public IP, needs the SG open) — so we can see if/how much the
# mesh path diverges from the raw internet path, and why.
#
# Writes a small JSON state file that host-net-publish.sh folds into its payload
# (capacity block) — decoupling the ~hourly measurement from the 60s publish so
# the dashboard always shows the last measured capacity with its timestamp.
set -u

: "${STATE_DIR:=/var/lib/host-net-publish}"
: "${CAPACITY_TARGET_MESH:=}"        # target mesh IP, e.g. 100.127.0.120 (cloud box)
: "${CAPACITY_TARGET_PUBLIC:=}"      # target public IP (optional; needs SG:5201 open)
: "${CAPACITY_TARGET_NAME:=cloud}"   # provenance label for the dashboard
# Cross-node federation leg: on a multi-node goatnet the
# PRIMARY signal-federation transport is the Thor<->Thor mesh path. Measuring
# it answers "does multi-node degrade perf?" — the mesh-primary leg's raw
# capacity + (implicitly) its latency via host-net's own rtt. The relay-BACKUP
# federation leg rides the relay VPS, so its capacity is already the existing
# relay-path capacity measurement — not re-probed here. Optional; unset on
# single-node goatnets.
: "${CAPACITY_TARGET_CROSSNODE:=}"   # peer mgmt-node mesh IP, e.g. 100.127.0.201 (zagros-b)
: "${CAPACITY_CROSSNODE_NAME:=peer-node}"  # provenance label
: "${CAPACITY_PORT:=5201}"
: "${CAPACITY_DURATION_S:=4}"        # per-measurement; keep total <= ~10s
: "${CAPACITY_MIN_INTERVAL_S:=60}"   # on-demand floor
: "${CAPACITY_DIRECTION:=down}"      # down | up | both

OUT="$STATE_DIR/capacity.json"
LOCK="$STATE_DIR/capacity.lock"
ONDEMAND=0; [ "${1:-}" = "--on-demand" ] && ONDEMAND=1
mkdir -p "$STATE_DIR" 2>/dev/null || true

# on-demand rate-limit: refuse if the last run was < MIN_INTERVAL ago.
if [ "$ONDEMAND" = 1 ] && [ -f "$LOCK" ]; then
  last=$(stat -c %Y "$LOCK" 2>/dev/null || echo 0); nowl=$(date +%s)
  if [ $((nowl - last)) -lt "$CAPACITY_MIN_INTERVAL_S" ]; then
    echo "capacity-probe: rate-limited ($((nowl-last))s < ${CAPACITY_MIN_INTERVAL_S}s floor)" >&2
    exit 3
  fi
fi
touch "$LOCK" 2>/dev/null || true

[ -z "$CAPACITY_TARGET_MESH" ] && { echo "capacity-probe: CAPACITY_TARGET_MESH unset — nothing to do" >&2; exit 2; }
command -v iperf3 >/dev/null 2>&1 || { echo "capacity-probe: iperf3 not installed" >&2; exit 2; }

# One measurement: iperf3 -J to $1; $2 = "-R" for download (server->client) or "".
# Returns the receiver-sum bits/s (delivered throughput), or the literal null.
measure() {
  local host="$1" rev="$2" j
  j=$(timeout $((CAPACITY_DURATION_S + 8)) iperf3 -c "$host" -p "$CAPACITY_PORT" \
        -t "$CAPACITY_DURATION_S" --connect-timeout 4000 -J $rev 2>/dev/null) || { echo null; return; }
  printf '%s' "$j" | python3 -c 'import sys,json
try:
    d=json.load(sys.stdin); print(int(d["end"]["sum_received"]["bits_per_second"]))
except Exception:
    print("null")'
}

# Direction plan: default download only (the "how fast can I pull" capacity,
# ~5s). "both" adds upload (~10s total). Upload-only for asymmetric links.
do_down=1; do_up=0
case "$CAPACITY_DIRECTION" in up) do_down=0; do_up=1;; both) do_up=1;; esac

meas_pair() { # $1=host -> "down up"
  local dn=null up=null
  [ "$do_down" = 1 ] && dn=$(measure "$1" "-R")
  [ "$do_up" = 1 ] && up=$(measure "$1" "")
  echo "$dn $up"
}

read -r mesh_down mesh_up < <(meas_pair "$CAPACITY_TARGET_MESH")
pub_down=null; pub_up=null
[ -n "$CAPACITY_TARGET_PUBLIC" ] && read -r pub_down pub_up < <(meas_pair "$CAPACITY_TARGET_PUBLIC")
# Cross-node federation-primary leg (Thor<->Thor mesh). Absent = null (single
# node, or no iperf3-target on the peer Thor).
cn_down=null; cn_up=null
[ -n "$CAPACITY_TARGET_CROSSNODE" ] && read -r cn_down cn_up < <(meas_pair "$CAPACITY_TARGET_CROSSNODE")

now=$(date +%s)
cat > "$OUT.tmp" <<JSON
{"measured_at":$now,"target":"$CAPACITY_TARGET_NAME","duration_s":$CAPACITY_DURATION_S,"direction":"$CAPACITY_DIRECTION","on_demand":$ONDEMAND,"mesh":{"down_bps":$mesh_down,"up_bps":$mesh_up},"public":{"down_bps":$pub_down,"up_bps":$pub_up},"crossnode":{"name":"$CAPACITY_CROSSNODE_NAME","down_bps":$cn_down,"up_bps":$cn_up}}
JSON
mv "$OUT.tmp" "$OUT"
echo "capacity-probe: target=$CAPACITY_TARGET_NAME mesh(down=$mesh_down up=$mesh_up) public(down=$pub_down up=$pub_up) crossnode=$CAPACITY_CROSSNODE_NAME(down=$cn_down up=$cn_up)" >&2
