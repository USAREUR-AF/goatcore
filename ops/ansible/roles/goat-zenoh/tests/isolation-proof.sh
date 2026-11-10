#!/usr/bin/env bash
# Live isolation proof for the per-uuid slot-pool ACL (ADR 1025): runs a real
# zenohd (pinned eclipse/zenoh:1.9.0) with the slot-pool access_control that
# config.json5.j2 generates, and proves the router ENFORCES per-slot isolation:
# a publisher whose cert CN = slot-0's anchor can write slot-0's namespace and
# is DENIED slot-1's namespace (observed via a subscriber, since Zenoh denial
# is silent at the publisher, F-177).
#
# Requires docker. Self-contained + idempotent; tears everything down on exit.
#   ops/ansible/roles/goat-zenoh/tests/isolation-proof.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROLE="$(cd "$HERE/.." && pwd)"
WORK="$(mktemp -d -t slotpool-iso.XXXXXX)"
CERTS="$WORK/certs"; mkdir -p "$CERTS"
NET="slotpool-iso-net"
IMG="eclipse/zenoh:1.9.0@sha256:157965d71e0bfd0a044d76a985ff0e5c306ad3968929168fb9678cd2a7fec23f"
CLIENT_IMG="slotpool-iso-client:local"
SALT="isoproof"
FAILS=0

cleanup() {
  docker rm -f iso-router iso-sub iso-pub >/dev/null 2>&1
  docker network rm "$NET" >/dev/null 2>&1
  rm -rf "$WORK"
}
trap cleanup EXIT
ok(){ echo "  PASS: $1"; }
bad(){ echo "  FAIL: $1"; FAILS=$((FAILS+1)); }

anchor(){ python3 -c "import hashlib,sys;print(hashlib.sha256(f'$SALT:{sys.argv[1]}'.encode()).hexdigest()[:32])" "$1"; }
A0="$(anchor 0)"; A1="$(anchor 1)"
echo "slot anchors: 0=$A0  1=$A1"

# --- 1. minimal zenohd config: render the template's access_control, strip storage/rest ---
python3 - "$ROLE/templates/config.json5.j2" "$CERTS" > "$WORK/router.json5" <<'PY'
import hashlib, sys
from jinja2 import Environment, BaseLoader
tpl, certs = sys.argv[1], sys.argv[2]
e = Environment(loader=BaseLoader(), trim_blocks=True, lstrip_blocks=True)
e.filters["hash"] = lambda s, algo="sha256": hashlib.new(algo, str(s).encode()).hexdigest()
rendered = e.from_string(open(tpl).read()).render(
    goat_zenoh_listen_host="0.0.0.0", goat_zenoh_listen_port=7447,
    goat_zenoh_rest_port=8000, goat_zenoh_retention_days=7,
    goat_zenoh_storage_path="/data", goat_zenoh_tls_dir="/certs",
    zenoh_seed_vendors=[], zenoh_identity_model="per-uuid-slot-pool",
    zenoh_slot_pool_size=4, zenoh_slot_pool_salt="isoproof")
# brace-extract the access_control: { ... } block
i = rendered.index("access_control: {")
j = rendered.index("{", i); depth = 0
for k in range(j, len(rendered)):
    if rendered[k] == "{": depth += 1
    elif rendered[k] == "}":
        depth -= 1
        if depth == 0: end = k + 1; break
acl = rendered[i:end]
print("{\n  mode: \"router\",\n  listen: { endpoints: [\"tls/0.0.0.0:7447\"] },")
print("""  transport: { link: { tls: {
    listen_certificate: "/certs/router-cert.pem",
    listen_private_key: "/certs/router-key.pem",
    root_ca_certificate: "/certs/ca-root-cert.pem",
    enable_mtls: true,
  } } },""")
print("  " + acl + ",\n}")
PY
grep -q "slot-w-$A0" "$WORK/router.json5" && ok "config carries slot-0 write rule" || bad "config missing slot-0 rule"

# --- 2. certs: CA, router (SAN iso-router), two slot certs (CN=anchor), reader ---
cd "$CERTS" || exit 1
openssl ecparam -name prime256v1 -genkey -noout -out ca-key.pem 2>/dev/null
openssl req -x509 -new -key ca-key.pem -days 1 -subj "/CN=iso-ca" -out ca-root-cert.pem 2>/dev/null
mkcert(){ # $1=name(file stem) $2=CN $3=SAN
  openssl ecparam -name prime256v1 -genkey -noout -out "$1-key.pem" 2>/dev/null
  openssl req -new -key "$1-key.pem" -subj "/CN=$2" -out "$1.csr" 2>/dev/null
  openssl x509 -req -in "$1.csr" -CA ca-root-cert.pem -CAkey ca-key.pem -CAcreateserial \
    -days 1 -out "$1-cert.pem" -extfile <(printf "subjectAltName=%s" "$3") 2>/dev/null
}
mkcert router  iso-router        "DNS:iso-router,DNS:localhost,IP:127.0.0.1"
mkcert slot0   "$A0"             "DNS:slot0"
mkcert slot1   "$A1"             "DNS:slot1"
mkcert reader  reader-trial-peer "DNS:reader"
ok "minted CA + router + slot0(CN=$A0) + slot1(CN=$A1) + reader certs"

# --- 3. client image (python + zenoh) ---
printf 'FROM python:3.12-slim\nRUN pip install --no-cache-dir eclipse-zenoh==1.9.0\n' > "$WORK/Dockerfile.client"
docker build -q -t "$CLIENT_IMG" -f "$WORK/Dockerfile.client" "$WORK" >/dev/null 2>&1 && ok "built zenoh client image" || bad "client image build failed"

# --- 4. run router ---
docker network create "$NET" >/dev/null 2>&1
docker run -d --name iso-router --network "$NET" \
  -v "$CERTS:/certs:ro" -v "$WORK/router.json5:/router.json5:ro" \
  "$IMG" -c /router.json5 >/dev/null 2>&1
sleep 4
docker ps --filter name=iso-router --filter status=running -q | grep -q . && ok "zenohd running with slot-pool ACL" || { bad "zenohd not running"; docker logs iso-router 2>&1 | tail -15; }

# --- 5. subscriber (reader) in background, then publisher (slot0 cert) ---
docker run -d --name iso-sub --network "$NET" -v "$CERTS:/certs:ro" -v "$HERE/iso_client.py:/c.py:ro" \
  "$CLIENT_IMG" python /c.py sub /certs/reader-cert.pem /certs/reader-key.pem /certs/ca-root-cert.pem tls/iso-router:7447 8 >/dev/null 2>&1
sleep 3  # let the subscriber connect + declare
docker run --rm --name iso-pub --network "$NET" -v "$CERTS:/certs:ro" -v "$HERE/iso_client.py:/c.py:ro" \
  "$CLIENT_IMG" python /c.py pub /certs/slot0-cert.pem /certs/slot0-key.pem /certs/ca-root-cert.pem tls/iso-router:7447 \
  "$A0/test" "$A1/test" 2>&1 | sed 's/^/  pub: /'
sleep 2
SUB="$(docker logs iso-sub 2>&1)"
echo "$SUB" | sed 's/^/  sub: /'

# --- 6. assert isolation ---
echo "== isolation verdict =="
echo "$SUB" | grep -q "RECV $A0/test"  && ok "slot0 cert's write to its OWN anchor ($A0/test) was admitted + routed" \
                                        || bad "slot0's legit write did not arrive (handshake/ACL misconfig?)"
echo "$SUB" | grep -q "RECV $A1/test"  && bad "ISOLATION BROKEN: slot0 cert wrote into slot1's anchor ($A1/test)" \
                                        || ok "slot0 cert's write to slot1's anchor ($A1/test) was DENIED (not routed) — isolation holds"

echo
[ $FAILS -eq 0 ] && echo "ISOLATION PROOF PASSED" || echo "$FAILS CHECK(S) FAILED"
exit $FAILS
