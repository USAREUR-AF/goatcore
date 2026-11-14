#!/usr/bin/env bash
# Unit cases for the platform-admin guard (ADR 1205 D10,
# ops/postgres/spock/hub-authority/kratos.sql) in one postgres-spock
# container, no replication.
#
# The guard takes a node to be the hub only when its name is exactly
# goat.hub_site or exactly <goat.hub_site>_<database>: never a prefix, a
# suffix or a pattern. These cases check goat_is_hub_node, which both the
# local and the replicated path use, and a local grant at a Spock node whose
# name only starts with the hub's (<hub>_other), which must be refused.
#
# Usage (Linux Docker, CI):  IMAGE=postgres-spock:smoke ./unit.sh
#   UNIT_CONTAINER names the container (default hub-authority-unit-<pid>).

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../../../.." && pwd)"
IMAGE="${IMAGE:?set IMAGE to a postgres-spock image}"
C="${UNIT_CONTAINER:-hub-authority-unit-$$}"
GUARD="${GUARD_SQL:-$REPO/ops/postgres/spock/hub-authority/kratos.sql}"

cleanup() { docker rm -f "$C" >/dev/null 2>&1 || true; }
trap cleanup EXIT
cleanup

docker run -d --name "$C" --network none -e POSTGRES_PASSWORD=unit-not-secret "$IMAGE" \
  postgres -c config_file=/etc/postgresql/postgresql.conf >/dev/null
for _ in $(seq 1 60); do
  docker exec "$C" pg_isready -h 127.0.0.1 -U postgres >/dev/null 2>&1 && break
  sleep 1
done
docker exec "$C" pg_isready -h 127.0.0.1 -U postgres >/dev/null

q() { # q <sql>   in database kratos
  docker exec -i "$C" psql -U postgres -d kratos -v ON_ERROR_STOP=1 -tAXq -c "$1"
}
# The image's init creates the kratos database (init/01-databases.sql).
[[ "$(docker exec "$C" psql -U postgres -tAXq -c "SELECT 1 FROM pg_database WHERE datname = 'kratos';")" == 1 ]] \
  || docker exec "$C" psql -U postgres -v ON_ERROR_STOP=1 -q -c "CREATE DATABASE kratos;"
q "CREATE EXTENSION IF NOT EXISTS spock;"
q "CREATE TABLE identities (id uuid PRIMARY KEY DEFAULT gen_random_uuid(), metadata_public jsonb);"
docker exec -i "$C" psql -U postgres -d kratos -v ON_ERROR_STOP=1 -q \
  -c "SET client_min_messages = warning;" -f - < "$GUARD"

fails=0
check() { # check <what> <got> <want>
  if [[ "$2" == "$3" ]]; then
    echo "  ok    $1 = $3"
  else
    echo "  FAIL  $1: got '$2', want '$3'"
    fails=$((fails + 1))
  fi
}
is_hub() { # is_hub <goat.hub_site> <node name>  -> t|f
  q "SET goat.hub_site = '$1'; SELECT goat_is_hub_node('$2');"
}

echo "== goat_is_hub_node, database kratos"
check "hub zagros_a, node zagros_a"            "$(is_hub zagros_a zagros_a)"            t
check "hub zagros_a, node zagros_a_kratos"     "$(is_hub zagros_a zagros_a_kratos)"     t
check "hub zagros_a, node zagros_a_other"      "$(is_hub zagros_a zagros_a_other)"      f
check "hub zagros_a, node zagros_a_kratos_b"   "$(is_hub zagros_a zagros_a_kratos_b)"   f
check "hub zagros_a, node zagros_a_hydra"      "$(is_hub zagros_a zagros_a_hydra)"      f
check "hub zagros_a, node zagros"              "$(is_hub zagros_a zagros)"              f
check "hub zagros_a, node x_zagros_a"          "$(is_hub zagros_a x_zagros_a)"          f
check "hub zagros_a, node zagrosXa (_ is no wildcard)" "$(is_hub zagros_a zagrosXa)"    f
check "hub zagros_%, node zagros_a (% is no wildcard)" "$(is_hub 'zagros_%' zagros_a)"  f
check "hub zagros, node zagros_a (a prefix is not the hub)" "$(is_hub zagros zagros_a)" f
check "hub unset, node zagros_a"               "$(is_hub '' zagros_a)"                  f
check "hub unset, node _kratos"                "$(is_hub '' _kratos)"                   f

grant() { # grant <goat.hub_site>  -> allowed | refused   (a local grant at this node)
  if q "SET goat.hub_site = '$1';
        INSERT INTO identities (metadata_public) VALUES ('{\"groups\":[\"PLATFORM-ADMIN\"]}');" >/dev/null 2>&1; then
    echo allowed
  else
    echo refused
  fi
}
node() { # node <name>   make <name> this database's only Spock node
  q "SELECT spock.node_drop(node_name, true) FROM spock.node;" >/dev/null
  q "SELECT spock.node_create('$1', 'host=unit dbname=kratos');" >/dev/null
}

echo "== a local grant of PLATFORM-ADMIN, by this site's node name"
node zagros_a_other
check "node zagros_a_other, hub zagros_a"  "$(grant zagros_a)"  refused
node zagros_a_kratos
check "node zagros_a_kratos, hub zagros_a" "$(grant zagros_a)"  allowed
check "node zagros_a_kratos, hub zagros"   "$(grant zagros)"    refused
node zagros_a
check "node zagros_a, hub zagros_a"        "$(grant zagros_a)"  allowed
check "node zagros_a, hub unset"           "$(grant '')"        refused

if (( fails > 0 )); then
  echo "== $fails case(s) failed"
  exit 1
fi
echo "== all cases passed"
