#!/usr/bin/env bash
# ops/netbird/tests/peer-groups-query.sh: measure mesh-enroll-host.yml's peer-groups
# query against the pinned NetBird management's own schema.
#
# join-node's phase 10 mints a setup key with the groups the minting host's peer
# holds (mesh-enroll-host.yml, "read the groups off that peer"). This script:
#   1. starts the pinned postgres-spock and the pinned netbird-mgmt (both from
#      ops/pins/fleet-pins.yaml) on a disposable network; management creates its
#      schema;
#   2. creates an account with mgmt-host-bootstrap, three groups in the shape that
#      tool writes, and two single-use setup keys (two groups on one, one on the
#      other);
#   3. enrols two real NetBird clients (netstack mode) with those keys, so
#      management itself writes the peers and their group_peers rows;
#   4. runs tests/test_mesh_enroll_peer_groups.py, whose live case runs the play's
#      own query there.
# Every container and the network carry HARNESS_PFX and are removed on exit.
#
# Required, with no default (a shared default prefix or port would collide with
# another run):
#   HARNESS_PFX           container name prefix, e.g. phx
#   HARNESS_NET           docker network name, e.g. ph-net-x
#   HARNESS_PORT_PG_A     loopback port for Postgres
# Optional:
#   GHCR_ORG              image namespace (else ghcr_org in ops/ansible/group_vars/all.yml)
#   NETBIRD_CLIENT_IMAGE  the client image (default netbirdio/netbird:0.71.4, the
#                         upstream release the pinned management is built from)
#
# Needs docker, go and python3 (pyyaml, jinja2, pytest).

set -euo pipefail

: "${HARNESS_PFX:?set HARNESS_PFX (a prefix of your own)}"
: "${HARNESS_NET:?set HARNESS_NET (a network of your own)}"
: "${HARNESS_PORT_PG_A:?set HARNESS_PORT_PG_A (a loopback port of your own)}"

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
PINS="$REPO/ops/pins/fleet-pins.yaml"

if [ -z "${GHCR_ORG:-}" ]; then
  GHCR_ORG="$(sed -nE "s/^ghcr_org:.*[\"' ]([A-Za-z0-9][A-Za-z0-9._-]*)[\"']?[ }\"]*\$/\\1/p" \
    "$REPO/ops/ansible/group_vars/all.yml" 2>/dev/null | head -1 || true)"
fi
if [ -z "${GHCR_ORG:-}" ]; then
  echo "peer-groups-query: no image namespace; export GHCR_ORG" >&2
  exit 64
fi

pin() {
  python3 -c 'import sys, yaml
c = yaml.safe_load(open(sys.argv[1]))["components"][sys.argv[2]]["fleet"]
print(c.replace("{{ ghcr_org }}", sys.argv[3]))' "$PINS" "$1" "$GHCR_ORG"
}
PG_IMAGE="$(pin postgres-spock)"
MGMT_IMAGE="$(pin netbird-mgmt)"
CLIENT_IMAGE="${NETBIRD_CLIENT_IMAGE:-netbirdio/netbird:0.71.4}"

PG="${HARNESS_PFX}-pg"
MGMT="${HARNESS_PFX}-mgmt"
WORK="$(mktemp -d)"
PG_PASS="$(head -c 18 /dev/urandom | base64 | tr -d '/+=')"

cleanup() {
  for c in "$PG" "$MGMT" "${HARNESS_PFX}-peer1" "${HARNESS_PFX}-peer2"; do
    docker rm -f "$c" >/dev/null 2>&1 || true
  done
  docker network rm "$HARNESS_NET" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

docker network create "$HARNESS_NET" >/dev/null

echo "peer-groups-query: postgres ($PG_IMAGE)"
docker run -d --name "$PG" --network "$HARNESS_NET" -p "127.0.0.1:${HARNESS_PORT_PG_A}:5432" \
  -e POSTGRES_PASSWORD="$PG_PASS" "$PG_IMAGE" \
  postgres -c config_file=/etc/postgresql/postgresql.conf -c hba_file=/etc/postgresql/pg_hba.conf >/dev/null
ready=0
for _ in $(seq 1 90); do
  # Over TCP: the image's first-boot server listens on the socket only, then restarts.
  if docker exec -e PGPASSWORD="$PG_PASS" "$PG" psql -h 127.0.0.1 -U postgres -d netbird -tAc 'select 1' >/dev/null 2>&1; then ready=1; break; fi
  sleep 2
done
[ "$ready" = 1 ] || { echo "peer-groups-query: postgres did not come up" >&2; docker logs "$PG" | tail -20 >&2; exit 1; }

echo "peer-groups-query: management ($MGMT_IMAGE)"
ENC_KEY="$(head -c 32 /dev/urandom | base64)"
cat > "$WORK/management.json" <<EOF
{
  "Stuns": [{"Proto":"udp","URI":"stun:localhost:3478"}],
  "TURNConfig": {"Turns":[],"CredentialsTTL":"12h","Secret":"none"},
  "Signal": {"Proto":"http","URI":"localhost:10000"},
  "Datadir": "/var/lib/netbird",
  "DataStoreEncryptionKey": "$ENC_KEY",
  "HttpConfig": {"Address": "0.0.0.0:80", "AuthAudience": "netbird",
                 "AuthIssuer": "https://auth.example.org/", "AuthKeysLocation": "http://localhost:1/jwks.json",
                 "AuthUserIDClaim": "sub"},
  "IdpManagerConfig": {"ManagerType":"none"},
  "ReverseZone": "in-addr.arpa.",
  "StoreConfig": {"Engine":"postgres"}
}
EOF
chmod 0644 "$WORK/management.json"
start_mgmt() {
  docker rm -f "$MGMT" >/dev/null 2>&1 || true
  docker run -d --name "$MGMT" --network "$HARNESS_NET" \
    -e NETBIRD_STORE_ENGINE_POSTGRES_DSN="host=$PG user=postgres password=$PG_PASS dbname=netbird sslmode=disable client_encoding=UTF8" \
    -e NETBIRD_RUN_MIGRATIONS=true \
    -v "$WORK/management.json:/etc/netbird/management.json:ro" "$MGMT_IMAGE" >/dev/null
  for _ in $(seq 1 60); do
    if docker logs "$MGMT" 2>&1 | grep -q 'running HTTP server and gRPC server'; then return 0; fi
    sleep 2
  done
  echo "peer-groups-query: management did not come up" >&2
  docker logs "$MGMT" 2>&1 | tail -20 >&2
  return 1
}
start_mgmt

echo "peer-groups-query: account, groups and keys (mgmt-host-bootstrap)"
(cd "$REPO/ops/mgmt-host-bootstrap" && go build -o "$WORK/mhb" .)
export PGDSN="host=127.0.0.1 port=${HARNESS_PORT_PG_A} user=postgres password=$PG_PASS dbname=netbird sslmode=disable client_encoding=UTF8"
"$WORK/mhb" create-account --owner owner@example.org --site-name zagros --network 100.127.0.0/16 > "$WORK/account.out"
ACCOUNT="$(sed -nE 's/^account_id: *([^ ]+)$/\1/p' "$WORK/account.out")"
[ -n "$ACCOUNT" ] || { echo "peer-groups-query: no account id" >&2; cat "$WORK/account.out" >&2; exit 1; }
docker exec "$PG" psql -U postgres -d netbird -v ON_ERROR_STOP=1 -qc \
  "INSERT INTO groups (id, account_id, name, issued) VALUES
   ('grp_zagros_infra', '$ACCOUNT', 'zagros-INFRA', 'api'),
   ('grp_zagros_core', '$ACCOUNT', 'zagros-CORE', 'api'),
   ('grp_zagros_ops', '$ACCOUNT', 'zagros-OPS', 'api')"
# Management reads the account at start; restart it so it sees the rows above.
start_mgmt

enrol() { # <n> <auto-groups>
  "$WORK/mhb" create-setup-key --account "$ACCOUNT" --role bootstrap --ttl 1h --usage 1 \
    --name "peer-groups-$1" --auto-groups "$2" > "$WORK/key$1.out"
  sed -nE 's/^plaintext: *([^ ]+)$/NB_SETUP_KEY=\1/p' "$WORK/key$1.out" > "$WORK/key$1.env"
  chmod 0600 "$WORK/key$1.env"
  docker run -d --name "${HARNESS_PFX}-peer$1" --network "$HARNESS_NET" --env-file "$WORK/key$1.env" \
    -e NB_MANAGEMENT_URL="http://$MGMT:80" -e NB_USE_NETSTACK_MODE=true -e NB_HOSTNAME="peer$1" \
    "$CLIENT_IMAGE" >/dev/null
}
enrol 1 grp_zagros_infra,grp_zagros_core
enrol 2 grp_zagros_ops

peer_ip() {
  docker exec "$PG" psql -U postgres -d netbird -tAc "SELECT ip FROM peers WHERE name = 'peer$1'" | tr -d '" '
}
for _ in $(seq 1 60); do
  if [ -n "$(peer_ip 1)" ] && [ -n "$(peer_ip 2)" ]; then break; fi
  sleep 2
done
IP1="$(peer_ip 1)"
IP2="$(peer_ip 2)"
[ -n "$IP1" ] && [ -n "$IP2" ] || { echo "peer-groups-query: the clients did not enrol" >&2; docker logs "${HARNESS_PFX}-peer1" 2>&1 | tail -10 >&2; exit 1; }
echo "peer-groups-query: peer1 $IP1, peer2 $IP2; the stored ip column reads: $(docker exec "$PG" psql -U postgres -d netbird -tAc "SELECT ip FROM peers WHERE name = 'peer1'")"

PEER_GROUPS_PSQL="docker exec $PG psql -U postgres -d netbird" \
PEER_GROUPS_ACCOUNT="$ACCOUNT" \
PEER_GROUPS_PEER_IP="$IP1" PEER_GROUPS_WANT="grp_zagros_core,grp_zagros_infra" \
PEER_GROUPS_OTHER_IP="$IP2" PEER_GROUPS_OTHER_WANT="grp_zagros_ops" \
  python3 -m pytest -q -rs "$REPO/tests/test_mesh_enroll_peer_groups.py"
