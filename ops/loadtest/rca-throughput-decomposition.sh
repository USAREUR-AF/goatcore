#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# rca-throughput-decomposition.sh
# ===============================================================================
# Self-contained, REPEATABLE-WITHOUT-AN-LLM reproduction of the zenoh
# throughput RCA on a router host (nova-a AGX Thor). One command stands up isolated
# throwaway routers, decomposes where the per-message cost lives, and renders a
# self-contained HTML report. It NEVER touches the production router, the real
# audit trail, or any real vendor data — everything runs on a private docker
# network with rocksdb pointed at a scratch dir that is wiped on exit.
#
# WHAT IT PROVES
#   The goat fabric's earlier "~7k msg/s ceiling" was a measurement
#   artifact of an un-pipelined load generator, NOT a property of zenoh. With a
#   pipelined publisher the SAME hardware routes ~1.2M msg/s. The real durable
#   limit is the in-process rocksdb write coupled to the routing thread (~40k/s
#   per publisher), not the router and not the external MinIO subscriber.
#
# THE DECOMPOSITION (each phase adds exactly one variable)
#   A. reference  : zenoh's own z_pub_thr/z_sub_thr, P2P then routed — the raw
#                   hardware baseline, free of our code. (Optional: needs the
#                   reference binaries; see --build-reference. Skipped if absent.)
#   B. route-only : our (fixed, pipelined) goat-loadgen -> throwaway router with
#                   NO storage, congestion=drop, unbounded — the offered ceiling.
#   C. durable    : goat-loadgen -> throwaway router WITH in-process rocksdb
#                   (exact replica of the prod storage path) on a scratch dir,
#                   congestion=block, a paced rate ladder — measures stored/sent
#                   completeness + router CPU per rate => the durable ceiling.
#   D. aggregate  : sweep K publisher connections through one router (route-only
#                   + durable) — does one box scale across cores with more
#                   publishers? => vertical-vs-horizontal scaling verdict.
#
# USAGE (on the router host)
#   ops/loadtest/rca-throughput-decomposition.sh
#   ops/loadtest/rca-throughput-decomposition.sh --ladder 10000,20000,50000,100000 --dur 5
#   ops/loadtest/rca-throughput-decomposition.sh --build-reference   # build z_pub_thr/z_sub_thr first
#
# OUTPUT
#   <out>/rca-throughput-<stamp>.jsonl   (evidence; one JSON object per line)
#   <out>/rca-throughput-<stamp>.html    (the self-contained report)
#
# PREREQUISITES (all already present on a goat router host)
#   - docker; the prod router image is reused (eclipse/zenoh:1.9.0 pinned below)
#   - the goat-loadgen:local image, built from ops/zenoh-trial-peers/loadgen/
#     (the apparatus build step). If absent the script prints how to build it.
#   - the host rocksdb backend plugin dir (PLUGIN_DIR below), bind-mounted into
#     the throwaway storage router exactly as the prod router mounts it.
#   - python3 (stdlib only) for the renderer.
# ===============================================================================
set -euo pipefail

# --- knobs (env-overridable; sane defaults for a goat router host) -------------
ROUTER_IMAGE="${ROUTER_IMAGE:-eclipse/zenoh:1.9.0@sha256:157965d71e0bfd0a044d76a985ff0e5c306ad3968929168fb9678cd2a7fec23f}"
LOADGEN_IMAGE="${LOADGEN_IMAGE:-goat-loadgen:local}"
PLUGIN_DIR="${PLUGIN_DIR:-/opt/goat-zenoh/plugins}"   # holds libzenoh_backend_rocksdb.so
ZBIN_DIR="${ZBIN_DIR:-/tmp/zbin}"                           # z_pub_thr / z_sub_thr (optional)
REFERENCE_RUNTIME_IMAGE="${REFERENCE_RUNTIME_IMAGE:-cgr.dev/chainguard/rust:latest-dev@sha256:54120f0b57f28063bd9f23e29113a28826ae08d731bb0bff11d6a6fa53bf74cf}"
NET="${NET:-rca-net}"
SCRATCH_ROCKSDB="${SCRATCH_ROCKSDB:-/tmp/rca-rocksdb}"
REST_HOST_PORT="${REST_HOST_PORT:-127.0.0.1:18099}"
OUT_DIR="${OUT_DIR:-/opt/loadtest/out}"
PAYLOAD_BYTES="${PAYLOAD_BYTES:-100}"
SESSIONS="${SESSIONS:-8}"
PIPELINE_DEPTH="${PIPELINE_DEPTH:-256}"
LADDER="${LADDER:-10000,20000,30000,50000,75000,100000,150000}"
AGG_K="${AGG_K:-1,2,4,8}"   # publisher-connection counts for the Phase D scaling sweeps
DUR="${DUR:-5}"
STAMP="${STAMP:-manual}"   # caller may pass a date stamp; scripts can't use Date.now under some runners
BUILD_REFERENCE=0

while [ $# -gt 0 ]; do
  case "$1" in
    --ladder) LADDER="$2"; shift 2;;
    --dur) DUR="$2"; shift 2;;
    --sessions) SESSIONS="$2"; shift 2;;
    --depth) PIPELINE_DEPTH="$2"; shift 2;;
    --out) OUT_DIR="$2"; shift 2;;
    --stamp) STAMP="$2"; shift 2;;
    --build-reference) BUILD_REFERENCE=1; shift;;
    -h|--help) sed -n '2,60p' "$0"; exit 0;;
    *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done

SUDO="sudo"; command -v sudo >/dev/null 2>&1 || SUDO=""
D="$SUDO docker"
mkdir -p "$OUT_DIR"
EV="${OUT_DIR}/rca-throughput-${STAMP}.jsonl"
HTML="${OUT_DIR}/rca-throughput-${STAMP}.html"
: > "$EV"
SELF_DIR="$(cd "$(dirname "$0")" && pwd)"

log() { printf '\033[36m[rca]\033[0m %s\n' "$*" >&2; }
jrec() { printf '%s\n' "$1" >> "$EV"; }   # $1 = a single-line JSON object

cleanup() {
  log "teardown"
  $D rm -f rca-router rca-router2 rca-stor rca-sub rca-pub rca-lg >/dev/null 2>&1 || true
  $D network rm "$NET" >/dev/null 2>&1 || true
  $SUDO rm -rf "$SCRATCH_ROCKSDB" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# --- preflight -----------------------------------------------------------------
if ! $D image inspect "$LOADGEN_IMAGE" >/dev/null 2>&1; then
  echo "ERROR: loadgen image '$LOADGEN_IMAGE' not found. Build it first:" >&2
  echo "  cd ops/zenoh-trial-peers/loadgen && docker build -t goat-loadgen:local ." >&2
  exit 1
fi
$D network create "$NET" >/dev/null 2>&1 || true
$SUDO rm -rf "$SCRATCH_ROCKSDB"; mkdir -p "$SCRATCH_ROCKSDB"

jrec "{\"kind\":\"meta\",\"stamp\":\"${STAMP}\",\"router_image\":\"${ROUTER_IMAGE%%@*}\",\"loadgen_image\":\"${LOADGEN_IMAGE}\",\"payload_bytes\":${PAYLOAD_BYTES},\"sessions\":${SESSIONS},\"pipeline_depth\":${PIPELINE_DEPTH},\"ladder\":\"${LADDER}\",\"dur_secs\":${DUR}}"

# ===============================================================================
# Phase A — zenoh reference (optional; needs z_pub_thr/z_sub_thr binaries)
# ===============================================================================
if [ "$BUILD_REFERENCE" = "1" ] || { [ -x "${ZBIN_DIR}/z_pub_thr" ] && [ -x "${ZBIN_DIR}/z_sub_thr" ]; }; then
  if [ "$BUILD_REFERENCE" = "1" ] && { [ ! -x "${ZBIN_DIR}/z_pub_thr" ] || [ ! -x "${ZBIN_DIR}/z_sub_thr" ]; }; then
    log "building z_pub_thr/z_sub_thr from zenoh ${ROUTER_IMAGE%%@*#*} source (one-time, ~7min)"
    mkdir -p "$ZBIN_DIR"
    $D run --rm --user root -v "${ZBIN_DIR}:/out" --entrypoint /bin/sh "$REFERENCE_RUNTIME_IMAGE" -c '
      set -e; cd /tmp
      cargo install --quiet --root /usr/local --example z_pub_thr --example z_sub_thr zenoh-examples@1.9.0 2>/dev/null || {
        git clone --depth 1 --branch 1.9.0 https://github.com/eclipse-zenoh/zenoh /z >/dev/null 2>&1
        cd /z && cargo build --release --example z_pub_thr --example z_sub_thr
        cp target/release/examples/z_pub_thr target/release/examples/z_sub_thr /out/
      }' || log "reference build failed; continuing without Phase A"
  fi
  if [ -x "${ZBIN_DIR}/z_pub_thr" ] && [ -x "${ZBIN_DIR}/z_sub_thr" ]; then
    measure_reference() {  # $1 label, $2 sub-args, $3 pub-args
      $D rm -f rca-sub rca-pub >/dev/null 2>&1 || true
      $D run -d --name rca-sub --network "$NET" --entrypoint /out/z_sub_thr -v "${ZBIN_DIR}:/out" \
        "$REFERENCE_RUNTIME_IMAGE" $2 -n 1000000 >/dev/null
      sleep 4
      $D run -d --name rca-pub --network "$NET" --entrypoint /out/z_pub_thr -v "${ZBIN_DIR}:/out" \
        "$REFERENCE_RUNTIME_IMAGE" $3 "$PAYLOAD_BYTES" >/dev/null
      sleep 18
      local mid; mid=$($D logs rca-sub 2>&1 | grep "msg/s" | tail -6 | awk '{s+=$1;n++} END{if(n)printf "%.0f", s/n; else print 0}')
      $D rm -f rca-sub rca-pub >/dev/null 2>&1 || true
      echo "${mid:-0}"
    }
    log "Phase A: zenoh reference (P2P, plain TCP, no router/storage)"
    P2P=$(measure_reference p2p "-m peer -l tcp/0.0.0.0:7460" "-m peer -e tcp/rca-sub:7460")
    jrec "{\"kind\":\"reference\",\"config\":\"p2p_plain\",\"desc\":\"two peers, direct, no router/TLS/ACL/storage\",\"msg_per_s\":${P2P:-0}}"
    log "  P2P plain: ${P2P} msg/s"

    log "Phase A: zenoh reference (routed, plain TCP, no storage)"
    $D run -d --name rca-router --network "$NET" "$ROUTER_IMAGE" -l tcp/0.0.0.0:7461 --no-multicast-scouting >/dev/null
    sleep 6
    ROUTED=$(measure_reference routed "-m client -e tcp/rca-router:7461" "-m client -e tcp/rca-router:7461")
    $D rm -f rca-router >/dev/null 2>&1 || true
    jrec "{\"kind\":\"reference\",\"config\":\"routed_plain\",\"desc\":\"1 pub -> router -> 1 sub, no ACL/storage\",\"msg_per_s\":${ROUTED:-0}}"
    log "  routed plain: ${ROUTED} msg/s"
  fi
else
  log "Phase A skipped (z_pub_thr/z_sub_thr not at ${ZBIN_DIR}; pass --build-reference to build them)"
  jrec "{\"kind\":\"reference\",\"config\":\"skipped\",\"desc\":\"reference binaries absent; run with --build-reference\",\"msg_per_s\":0}"
fi

# ===============================================================================
# Phase B — route-only ceiling with OUR (fixed, pipelined) loadgen, no storage
# ===============================================================================
log "Phase B: route-only offered ceiling (fixed loadgen, drop, unbounded, no storage)"
$D run -d --name rca-router --network "$NET" "$ROUTER_IMAGE" -l tcp/0.0.0.0:7450 --no-multicast-scouting >/dev/null
sleep 5
$D rm -f rca-lg >/dev/null 2>&1 || true
$D run -d --name rca-lg --network "$NET" "$LOADGEN_IMAGE" \
  --endpoint tcp/rca-router:7450 --run-id routeonly --rate 0 --duration-secs $((DUR+8)) \
  --sessions "$SESSIONS" --pipeline-depth "$PIPELINE_DEPTH" --congestion drop --progress-secs 3 >/dev/null
sleep $((DUR+3))
RO_CPU=$($D stats --no-stream --format "{{.CPUPerc}}" rca-router 2>/dev/null | tr -d '%')
$D wait rca-lg >/dev/null 2>&1 || true
RO_RATE=$($D logs rca-lg 2>&1 | grep '"rate_msg_s"' | tail -4 | grep -oE '"rate_msg_s":[0-9.]+' | cut -d: -f2 | awk '{s+=$1;n++} END{if(n)printf "%.0f", s/n; else print 0}')
$D rm -f rca-lg rca-router >/dev/null 2>&1 || true
jrec "{\"kind\":\"route_only\",\"desc\":\"fixed loadgen offered rate, no storage, congestion=drop\",\"offered_msg_per_s\":${RO_RATE:-0},\"router_cpu_pct\":${RO_CPU:-0}}"
log "  route-only offered: ${RO_RATE} msg/s (router CPU ${RO_CPU}%)"

# ===============================================================================
# Phase C — durable ceiling: loadgen -> router WITH in-process rocksdb (scratch)
# ===============================================================================
log "Phase C: durable sweep (in-process rocksdb on scratch dir, congestion=block)"
CFG="${SCRATCH_ROCKSDB}/rca-cfg.json5"
cat > "$CFG" <<JSON5
{
  mode: "router",
  listen: { endpoints: ["tcp/0.0.0.0:7450"] },
  scouting: { multicast: { enabled: false } },
  plugins_loading: { enabled: true, search_dirs: ["/", "/opt/zenoh/plugins"] },
  plugins: {
    rest: { http_port: "0.0.0.0:8000" },
    storage_manager: {
      backend_search_dirs: ["/opt/zenoh/plugins"],
      volumes: { rocksdb: {} },
      storages: {
        loadtest: {
          key_expr: "loadtest/**",
          volume: { id: "rocksdb", dir: "/data/loadtest", on_closure: "do_nothing", create_db: true, db_recovery_enabled: false }
        }
      }
    }
  }
}
JSON5

$D run -d --name rca-stor --network "$NET" -p "${REST_HOST_PORT}:8000" \
  -v "${CFG}:/etc/zenoh/config.json5:ro" \
  -v "${PLUGIN_DIR}:/opt/zenoh/plugins:ro" \
  -v "${SCRATCH_ROCKSDB}:/data" \
  "$ROUTER_IMAGE" --config /etc/zenoh/config.json5 >/dev/null
sleep 7
if ! $D logs rca-stor 2>&1 | grep -q "Spawning volume 'rocksdb'"; then
  echo "ERROR: rocksdb backend did not spawn — check PLUGIN_DIR ($PLUGIN_DIR) holds libzenoh_backend_rocksdb.so" >&2
  $D logs rca-stor 2>&1 | tail -20 >&2; exit 1
fi

IFS=',' read -ra RATES <<< "$LADDER"
for r in "${RATES[@]}"; do
  run="d${r}"
  $D rm -f rca-lg >/dev/null 2>&1 || true
  $D run -d --name rca-lg --network "$NET" "$LOADGEN_IMAGE" \
    --endpoint tcp/rca-stor:7450 --run-id "$run" --rate "$r" --duration-secs "$DUR" \
    --sessions "$SESSIONS" --pipeline-depth "$PIPELINE_DEPTH" --congestion block --progress-secs 0 >/dev/null
  sleep $(( DUR > 3 ? 3 : 1 ))
  cpu=$($D stats --no-stream --format "{{.CPUPerc}}" rca-stor 2>/dev/null | tr -d '%')
  $D wait rca-lg >/dev/null 2>&1 || true
  sent=$($D logs rca-lg 2>&1 | grep '"event":"final"' | grep -oE '"sent":[0-9]+' | head -1 | cut -d: -f2)
  sent=${sent:-0}
  $D rm -f rca-lg >/dev/null 2>&1 || true
  sleep 3   # let the storage subscriber drain to rocksdb
  stored=$(curl -s "http://${REST_HOST_PORT}/loadtest/${run}/**" 2>/dev/null | grep -o "loadtest/${run}/" | wc -l | tr -d ' ')
  stored=${stored:-0}
  comp=$(awk -v s="$sent" -v st="$stored" 'BEGIN{ if(s>0) printf "%.3f", st/s; else print 0 }')
  sps=$(awk -v st="$stored" -v d="$DUR" 'BEGIN{ printf "%.0f", st/d }')
  ach=$(awk -v s="$sent" -v d="$DUR" 'BEGIN{ printf "%.0f", s/d }')
  jrec "{\"kind\":\"durable\",\"rate_target\":${r},\"achieved_sent_per_s\":${ach},\"stored_per_s\":${sps},\"sent\":${sent},\"stored\":${stored},\"completeness\":${comp},\"router_cpu_pct\":${cpu:-0}}"
  log "  target ${r}/s: achieved ${ach}/s, stored ${sps}/s, completeness ${comp}, CPU ${cpu}%"
done

# ===============================================================================
# Phase D — aggregate scaling: does ONE box scale with more publisher connections?
#   route-aggregate   : K loadgen connections -> no-storage router (drop) => offered + CPU
#   durable-aggregate : K loadgen connections -> the rca-stor storage router (block) => stored/s + CPU
# Answers vertical (one process spreads across cores) vs horizontal (add processes/machines).
# ===============================================================================
log "Phase D: aggregate scaling sweeps (K = ${AGG_K})"
IFS=',' read -ra KS <<< "$AGG_K"

# D1 — routing aggregate against a fresh no-storage router
$D rm -f rca-router2 >/dev/null 2>&1 || true
$D run -d --name rca-router2 --network "$NET" "$ROUTER_IMAGE" -l tcp/0.0.0.0:7451 --no-multicast-scouting >/dev/null
sleep 5
for k in "${KS[@]}"; do
  $D rm -f rca-lg >/dev/null 2>&1 || true
  $D run -d --name rca-lg --network "$NET" "$LOADGEN_IMAGE" \
    --endpoint tcp/rca-router2:7451 --run-id "ar${k}" --rate 0 --duration-secs $((DUR+5)) \
    --sessions "$k" --pipeline-depth "$PIPELINE_DEPTH" --congestion drop --progress-secs 3 >/dev/null
  sleep $((DUR+1)); cpu=$($D stats --no-stream --format "{{.CPUPerc}}" rca-router2 2>/dev/null | tr -d '%')
  $D wait rca-lg >/dev/null 2>&1 || true
  off=$($D logs rca-lg 2>&1 | grep '"rate_msg_s"' | tail -3 | grep -oE '"rate_msg_s":[0-9.]+' | cut -d: -f2 | awk '{s+=$1;n++} END{if(n)printf "%.0f", s/n; else print 0}')
  $D rm -f rca-lg >/dev/null 2>&1 || true
  jrec "{\"kind\":\"aggregate_route\",\"connections\":${k},\"offered_msg_per_s\":${off:-0},\"router_cpu_pct\":${cpu:-0}}"
  log "  route  K=${k}: offered ${off}/s, router CPU ${cpu}%"
done
$D rm -f rca-router2 >/dev/null 2>&1 || true

# D2 — durable aggregate against the Phase-C storage router (still up)
for k in "${KS[@]}"; do
  run="ad${k}"
  $D rm -f rca-lg >/dev/null 2>&1 || true
  $D run -d --name rca-lg --network "$NET" "$LOADGEN_IMAGE" \
    --endpoint tcp/rca-stor:7450 --run-id "$run" --rate 0 --duration-secs "$DUR" \
    --sessions "$k" --pipeline-depth "$PIPELINE_DEPTH" --congestion block --progress-secs 0 >/dev/null
  sleep $(( DUR > 3 ? 3 : 1 )); cpu=$($D stats --no-stream --format "{{.CPUPerc}}" rca-stor 2>/dev/null | tr -d '%')
  $D wait rca-lg >/dev/null 2>&1 || true
  $D rm -f rca-lg >/dev/null 2>&1 || true
  sleep 3
  stored=$(curl -s "http://${REST_HOST_PORT}/loadtest/${run}/**" 2>/dev/null | grep -o "loadtest/${run}/" | wc -l | tr -d ' ')
  sps=$(awk -v st="${stored:-0}" -v d="$DUR" 'BEGIN{ printf "%.0f", st/d }')
  jrec "{\"kind\":\"aggregate_durable\",\"connections\":${k},\"stored_per_s\":${sps},\"router_cpu_pct\":${cpu:-0}}"
  log "  durable K=${k}: stored ${sps}/s, router CPU ${cpu}%"
done

# ===============================================================================
# Render
# ===============================================================================
log "rendering report -> $HTML"
python3 "${SELF_DIR}/render-rca.py" "$EV" "$HTML"
log "DONE"
echo "$HTML"
