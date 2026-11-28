#!/usr/bin/env bash
# Live proof for GHSA-rr3p on a real zenohd (pinned eclipse/zenoh:1.9.0): the
# router admits only the CURRENT leaf of an ASSIGNED slot. Renders the
# access_control block of config.json5.j2 with the slot fragment the portal
# would write at each step, restarts the router on it (zenoh 1.9 loads an ACL
# only at start, F-156), and observes what is routed:
#
#   phase 1  slot 0 held by A (generation 1)      A writes and reads
#   phase 2  A offboarded, slot 0 given to B (g3) A's leaf writes nothing and
#                                                 reads nothing; B's works
#   phase 3  B rotates (g4)                       B's g3 leaf is refused, g4 works
#   always   slot 2 unassigned                    its leaves (g1 and bare) refused
#            a bare-anchor leaf for slot 0        refused once slot 0 moved on
#
# Writes are observed by a named reader; reads by each leaf's own subscriber
# while slot 1's leaf publishes. A denial is silent at the client (F-177), so
# absence at the subscriber is the evidence. Same shape as isolation-proof.sh.
# Requires docker. Tears everything down on exit.
#   ops/ansible/roles/goat-zenoh/tests/rr3p-live-proof.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROLE="$(cd "$HERE/.." && pwd)"
# Every container, network and compose project carries this prefix, so a
# concurrent run (or another harness) can neither clobber nor be clobbered.
P="${RR3P_PROOF_PREFIX:-rr3p}"
WORK="$(mktemp -d -t "$P-live.XXXXXX")"
CERTS="$WORK/certs"; mkdir -p "$CERTS"
NET="${P}-net"
IMG="${ZENOH_ROUTER_IMAGE:-eclipse/zenoh:1.10.2@sha256:1885c59e7df9f38f73cc3ec0ba5b8efccb566b059a9ae3dd9c3095dcf760b86f}"   # candidate suite overrides (ADR 1262)
ZENOH_CLIENT_VERSION="${ZENOH_CLIENT_VERSION:-1.10.2}"
CLIENT_IMG="slotpool-iso-client:${ZENOH_CLIENT_VERSION}"
SALT="rr3p-live"
FAILS=0
SUBS=()

OBS=""
cleanup() {
  [ "$BASHPID" = "$$" ] || return 0   # never from a pipeline's subshell
  docker compose -p "$P-proof" -f "$WORK/live/compose.yml" down >/dev/null 2>&1
  docker rm -f ${P}-router "${SUBS[@]}" >/dev/null 2>&1
  docker network rm "$NET" >/dev/null 2>&1
  rm -rf "$WORK"
}
trap cleanup EXIT
ok(){ echo "  PASS: $1"; }
bad(){ echo "  FAIL: $1"; FAILS=$((FAILS+1)); }

anchor(){ python3 -c "import hashlib,sys;print(hashlib.sha256(f'$SALT:{sys.argv[1]}'.encode()).hexdigest()[:32])" "$1"; }
A0="$(anchor 0)"; A1="$(anchor 1)"; A2="$(anchor 2)"
echo "slot anchors: 0=$A0  1=$A1  2=$A2"

# render <out> <slot0 generation or -> : slot 1 at g1, slot 2 free, pool of 4.
render() {
  python3 - "$ROLE/templates/config.json5.j2" "$HERE/slot-fragment" "$SALT" "$2" > "$1" <<'PY'
import hashlib, sys
from jinja2 import Environment, BaseLoader
tpl, fragdir, salt, g0 = sys.argv[1:5]
sys.path.insert(0, fragdir)
import slotfragment as sf
a = lambda k: hashlib.sha256(f"{salt}:{k}".encode()).hexdigest()[:32]
binds = [(a(0), "" if g0 == "-" else sf.leaf_cn(a(0), int(g0))), (a(1), a(1) + ".g1"), (a(2), ""), (a(3), "")]
e = Environment(loader=BaseLoader(), trim_blocks=True, lstrip_blocks=True)
e.filters["hash"] = lambda s, algo="sha256": hashlib.new(algo, str(s).encode()).hexdigest()
e.filters["bool"] = lambda v: v if isinstance(v, bool) else str(v).strip().lower() in ("true", "yes", "1", "on")
rendered = e.from_string(open(tpl).read()).render(
    goat_zenoh_listen_host="0.0.0.0", goat_zenoh_listen_port=7447,
    goat_zenoh_rest_port=8000, goat_zenoh_retention_days=7, goat_zenoh_audit_retention_days=365,
    goat_zenoh_tx_queue_size=None, goat_zenoh_tx_block_wait_before_close_us=2000000,
    goat_zenoh_storage_path="/data", goat_zenoh_tls_dir="/certs",
    zenoh_seed_vendors=[], zenoh_identity_model="per-uuid-slot-pool",
    zenoh_slot_pool_size=4, zenoh_slot_pool_salt=salt,
    zenoh_universal_read_cns=["reader-trial-peer"],
    zenoh_slot_fragment_content=sf.render(binds))
i = rendered.index("access_control: {")
j = rendered.index("{", i); depth = 0
for k in range(j, len(rendered)):
    if rendered[k] == "{": depth += 1
    elif rendered[k] == "}":
        depth -= 1
        if depth == 0: end = k + 1; break
print("{\n  mode: \"router\",\n  listen: { endpoints: [\"tls/0.0.0.0:7447\"] },")
print("""  transport: { link: { tls: {
    listen_certificate: "/certs/router-cert.pem",
    listen_private_key: "/certs/router-key.pem",
    root_ca_certificate: "/certs/ca-root-cert.pem",
    enable_mtls: true,
  } } },""")
print("  " + rendered[i:end] + ",\n}")
PY
}

# --- certs: CA, router, the leaves under test, a named reader ---
cd "$CERTS" || exit 1
openssl ecparam -name prime256v1 -genkey -noout -out ca-key.pem 2>/dev/null
openssl req -x509 -new -key ca-key.pem -days 1 -subj "/CN=rr3p-ca" -out ca-root-cert.pem 2>/dev/null
mkcert(){ # $1=file stem $2=CN $3=SAN
  openssl ecparam -name prime256v1 -genkey -noout -out "$1-key.pem" 2>/dev/null
  openssl req -new -key "$1-key.pem" -subj "/CN=$2" -out "$1.csr" 2>/dev/null
  openssl x509 -req -in "$1.csr" -CA ca-root-cert.pem -CAkey ca-key.pem -CAcreateserial \
    -days 1 -out "$1-cert.pem" -extfile <(printf "subjectAltName=%s" "$3") 2>/dev/null
}
mkcert router  ${P}-router       "DNS:${P}-router,DNS:localhost"
mkcert a       "$A0.g1"          "DNS:a"        # A's leaf
mkcert b       "$A0.g3"          "DNS:b"        # B's first leaf
mkcert b2      "$A0.g4"          "DNS:b2"       # B's rotated leaf
mkcert legacy  "$A0"             "DNS:legacy"   # a generation-0 leaf for slot 0
mkcert s1      "$A1.g1"          "DNS:s1"       # slot 1's current leaf (the publisher for read tests)
mkcert free    "$A2.g1"          "DNS:free"     # a leaf for unassigned slot 2
mkcert free0   "$A2"             "DNS:free0"    # and its bare-anchor leaf
mkcert reader  reader-trial-peer "DNS:reader"
ok "minted CA, router and leaves"
cd "$HERE" || exit 1
docker image inspect "$CLIENT_IMG" >/dev/null 2>&1 || {
  printf 'FROM python:3.12-slim\nRUN pip install --no-cache-dir eclipse-zenoh==%s\n' "$ZENOH_CLIENT_VERSION" > "$WORK/Dockerfile.client"
  docker build -q -t "$CLIENT_IMG" -f "$WORK/Dockerfile.client" "$WORK" >/dev/null 2>&1 || bad "client image build failed"
}
docker network create "$NET" >/dev/null 2>&1

client() { # name mode stem docker-flag client-args...
  local name=$1 mode=$2 stem=$3 flag=$4; shift 4
  docker run "$flag" --name "$name" --network "$NET" -v "$CERTS:/certs:ro" -v "$HERE/iso_client.py:/c.py:ro" \
    "$CLIENT_IMG" python /c.py "$mode" "/certs/$stem-cert.pem" "/certs/$stem-key.pem" /certs/ca-root-cert.pem tls/${P}-router:7447 "$@"
}

# phase <label> <slot0 generation or -> <leaf stems to test...>
# Each tested leaf: subscribes ** (read test) and publishes <slot0>/<label>-<stem>
# (write test). The named reader observes writes; slot 1 publishes for reads.
start_router() { # config file
  docker rm -f ${P}-router >/dev/null 2>&1
  docker run -d --name ${P}-router --network "$NET" -v "$CERTS:/certs:ro" \
    -v "$1:/router.json5:ro" "$IMG" -c /router.json5 >/dev/null 2>&1
}
phase() {
  local label=$1 g0=$2; shift 2
  render "$WORK/router-$label.json5" "$g0"
  start_router "$WORK/router-$label.json5"
  sleep 4
  observe "$label" "$@"
}
# observe <label> <leaf stems...>: against the running router.
observe() {
  local label=$1; shift
  docker ps --filter name=${P}-router --filter status=running -q | grep -q . \
    || { bad "[$label] zenohd not running"; docker logs ${P}-router 2>&1 | tail -15; return; }
  client "${P}-obs-$label" sub reader -d 20 >/dev/null; SUBS+=("${P}-obs-$label")
  for s in "$@"; do client "${P}-sub-$label-$s" sub "$s" -d 20 >/dev/null; SUBS+=("${P}-sub-$label-$s"); done
  # every subscriber declared before anything is published
  for c in "${P}-obs-$label" "${@/#/${P}-sub-$label-}"; do
    for _ in $(seq 20); do docker logs "$c" 2>&1 | grep -q SUB-READY && break; sleep 0.5; done
  done
  sleep 1
  for s in "$@"; do
    case $s in free|free0) target=$A2 ;; *) target=$A0 ;; esac
    client "${P}-pub-$label-$s" pub "$s" --rm "$target/$label-$s" >/dev/null 2>&1
  done
  client "${P}-pub-$label-s1" pub s1 --rm "$A1/$label-from-s1" >/dev/null 2>&1
  sleep 6
  OBS="$(docker logs "${P}-obs-$label" 2>&1)"
  echo "$OBS" | grep -qx "RECV $A1/$label-from-s1" \
    || { bad "[$label] the named reader saw nothing from slot 1: harness broken"; echo "$OBS" | sed 's/^/    obs: /'; }
}
# Whole-line matches: "p3-b" must not match the "p3-b2" sample.
wrote() { echo "$OBS" | grep -qx "RECV $1"; }
read_ok() { docker logs "${P}-sub-$1-$2" 2>&1 | grep -qx "RECV $A1/$1-from-s1"; }

echo "== phase 1: slot 0 held by A at generation 1 =="
phase p1 1 a legacy free free0
wrote "$A0/p1-a" && ok "A's leaf writes its namespace" || bad "A's leaf could not write (ACL misrendered?)"
read_ok p1 a && ok "A's leaf reads the fabric" || bad "A's leaf could not read"
for s in legacy free free0; do
  case $s in free|free0) t=$A2 ;; *) t=$A0 ;; esac
  wrote "$t/p1-$s" && bad "[$s] leaf wrote $t/p1-$s" || ok "[$s] leaf's write refused"
  read_ok p1 $s && bad "[$s] leaf read the fabric" || ok "[$s] leaf's read refused"
done

echo "== phase 2: A offboarded, slot 0 reassigned to B at generation 3 =="
phase p2 3 a b
wrote "$A0/p2-a" && bad "A's leaf still writes as the slot's new holder" || ok "A's leaf cannot write as B"
read_ok p2 a && bad "A's leaf still reads the fabric" || ok "A's leaf cannot read"
wrote "$A0/p2-b" && ok "B's leaf writes" || bad "B's leaf could not write"
read_ok p2 b && ok "B's leaf reads" || bad "B's leaf could not read"

echo "== phase 3: B rotates to generation 4 =="
phase p3 4 b b2 legacy
wrote "$A0/p3-b" && bad "B's previous leaf still writes" || ok "B's previous leaf's write refused"
read_ok p3 b && bad "B's previous leaf still reads" || ok "B's previous leaf's read refused"
wrote "$A0/p3-b2" && ok "B's rotated leaf writes" || bad "B's rotated leaf could not write"
read_ok p3 b2 && ok "B's rotated leaf reads" || bad "B's rotated leaf could not read"
wrote "$A0/p3-legacy" && bad "a bare-anchor leaf writes after the slot moved on" || ok "a bare-anchor leaf's write refused"

echo "== phase 4: the host reload watcher loads changes with no manual restart =="
# The portal's part, modelled: rewrite the live config atomically. The router's
# part: files/slot-fragment-reload.py, run as its systemd timer would, with a
# short revocation gap and a batch interval long enough to observe.
LIVE="$WORK/live"; mkdir -p "$LIVE"
put_config() { render "$LIVE/config.json5.new" "$1"; mv "$LIVE/config.json5.new" "$LIVE/config.json5"; }
# The router as the role deploys it: compose, config.json5 bind-mounted as a
# single FILE, which the portal replaces by rename. The watcher's restart is
# the one the role's unit runs: compose up --force-recreate.
cat > "$LIVE/compose.yml" <<YML
services:
  router:
    image: $IMG
    container_name: ${P}-router
    command: ["--config", "/etc/zenoh/config.json5"]
    volumes:
      - "$CERTS:/certs:ro"
      - "$LIVE/config.json5:/etc/zenoh/config.json5:ro"
    networks: [fabric]
networks:
  fabric:
    name: $NET
    external: true
YML
watch_once() { # interval
  python3 "$ROLE/files/slot-fragment-reload.py" --config "$LIVE/config.json5" --container ${P}-router \
    --interval "$1" --min-gap 3 --settle 4 \
    --restart "docker compose -p $P-proof -f $LIVE/compose.yml up -d --force-recreate >/dev/null 2>&1"
}
digest_of() { grep -o 'slot fragment sha256:[0-9a-f]*' | head -1; }
docker rm -f ${P}-router >/dev/null 2>&1
put_config 4
watch_once 600 | sed 's/^/    watcher: /'   # no load record yet: loads at once
[ -f "$LIVE/slot-fragment.loaded.json5" ] && ok "the watcher recorded what the router loaded" || bad "no load record written"
observe p4a b2
wrote "$A0/p4a-b2" && ok "B's current leaf works under the watcher-loaded config" || bad "B's current leaf refused after the watcher load"

put_config -              # B offboarded: a revocation
want="$(digest_of < "$LIVE/config.json5")"
seen="$(docker exec ${P}-router cat /etc/zenoh/config.json5 | digest_of)"
[ "$seen" != "$want" ] && ok "F1: the running router does not see the renamed config (single-file bind mount keeps the old inode)" \
  || bad "the running container already sees the new config: the bind-mount premise changed"
sleep 3
start=$(date +%s)
for _ in $(seq 10); do
  out="$(watch_once 600)"; echo "$out" | sed 's/^/    watcher: /'
  echo "$out" | grep -q '^reloaded' && break
  sleep 2
done
took=$(( $(date +%s) - start ))
seen="$(docker exec ${P}-router cat /etc/zenoh/config.json5 | digest_of)"
[ "$seen" = "$want" ] && ok "the watcher's compose --force-recreate mounts the new config file" \
  || bad "after the watcher's recreate the router still reads the old config ($seen != $want)"
observe p4b b2
wrote "$A0/p4b-b2" && bad "B's leaf still writes after the revoke" || ok "a revoke took effect ${took}s after the config changed, with no manual restart"
read_ok p4b b2 && bad "B's leaf still reads after the revoke" || ok "and B's leaf reads nothing"

put_config 5              # slot 0 onboarded to C (generation 5): an addition only
( cd "$CERTS" && mkcert c "$A0.g5" "DNS:c" )
before="$(cat "$LIVE/slot-fragment.loaded.json5")"
out="$(watch_once 600)"; echo "$out" | sed 's/^/    watcher: /'
echo "$out" | grep -q '^pending' && [ "$before" = "$(cat "$LIVE/slot-fragment.loaded.json5")" ] \
  && ok "an addition-only change waits for the batch window (no recreate)" || bad "an addition was loaded immediately"
observe p4c c
wrote "$A0/p4c-c" && bad "C's leaf works before the batch window" || ok "C's leaf is refused until the batched load"
out="$(watch_once 0)"; echo "$out" | sed 's/^/    watcher: /'   # the window has passed
sleep 3
observe p4d c
wrote "$A0/p4d-c" && ok "C's leaf works after the batched load" || bad "C's leaf refused after the batched load"

echo
[ $FAILS -eq 0 ] && echo "RR3P LIVE PROOF PASSED" || echo "$FAILS CHECK(S) FAILED"
exit $FAILS
