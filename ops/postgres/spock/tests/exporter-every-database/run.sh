#!/usr/bin/env bash
# shellcheck disable=SC2034  # PFX, NET, PW are read by ../lib/two-site.sh
# The mgmt host's postgres-exporter sees a Spock conflict in every
# replicated database, not only in the one its DSN names.
#
# Two postgres-spock sites replicate netbird and kratos both ways, as a
# goatnet's two host sites do. The test cuts kratos's replication, writes
# the same row at both sites, heals, and waits for Spock to record the
# conflict in kratos's spock.resolutions. It then runs postgres-exporter
# exactly as ops/compose/mgmt-host/docker-compose.obs.yml configures it
# (its image, its environment, and monitoring/postgres-exporter-queries.yml),
# against one site, with the replicated databases the observability role
# hands it, and requires spock_resolutions_total for kratos to count that
# conflict. That metric is what the SpockConflictLogged alert reads.
#
# The monitoring role gets the grants the observability role gives it.
#
# Substrate: Linux Docker only (CI). Two bidirectional Spock nodes on macOS
# Docker Desktop wedge the daemon.
#
#   IMAGE=postgres-spock:smoke ./run.sh

set -euo pipefail
cd "$(dirname "$0")"

IMAGE="${IMAGE:?set IMAGE to a postgres-spock image}"
PFX="pe"
NET="exporter-every-database-net"
PW="exporter"
MON_PW="monitoring-fixture"
REPO="$(cd ../../../../.. && pwd)"
COMPOSE="$REPO/ops/compose/mgmt-host/docker-compose.obs.yml"
QUERIES="$REPO/ops/compose/mgmt-host/monitoring/postgres-exporter-queries.yml"
# The databases this fixture replicates, as the observability role names
# them to the exporter (POSTGRES_EXPORTER_SPOCK_DATABASES).
DATABASES="netbird,kratos"
# shellcheck source=../lib/two-site.sh
source ../lib/two-site.sh

EXPORTER="$PFX-exporter"
cleanup() {
  [[ "${KEEP:-0}" == 1 ]] && { echo "KEEP=1, leaving $PFX-a $PFX-b $EXPORTER running"; return; }
  ts_down "$EXPORTER"
}
trap cleanup EXIT
cleanup

echo "== bring up two sites"
ts_up
for s in a b; do
  for db in netbird kratos; do q "$s" "$db" "CREATE EXTENSION IF NOT EXISTS spock;"; done
done

echo "== one replicated table in each database"
for s in a b; do
  for db in netbird kratos; do
    q "$s" "$db" "CREATE TABLE t (id int PRIMARY KEY, v text);"
  done
done
for db in netbird kratos; do
  ts_node_create "$db"
  for s in a b; do q "$s" "$db" "SELECT spock.repset_add_table('default', 't');" >/dev/null; done
  ts_sub_create "$db" default
done
ts_wait_replicating netbird kratos
q a kratos "INSERT INTO t VALUES (1, 'first');"
for _ in $(seq 1 30); do
  [[ "$(q b kratos "SELECT count(*) FROM t;")" == 1 ]] && break
  sleep 1
done
[[ "$(q b kratos "SELECT count(*) FROM t;")" == 1 ]] || { echo "::error::the row never reached site b"; exit 1; }

echo "== a conflict in kratos: cut, write the same row at both sites, heal"
ts_cut kratos
q a kratos "UPDATE t SET v = 'site a' WHERE id = 1;"
sleep 1
q b kratos "UPDATE t SET v = 'site b' WHERE id = 1;"
ts_heal kratos
got=0
for _ in $(seq 1 60); do
  got=$(( $(q a kratos "SELECT count(*) FROM spock.resolutions;") + $(q b kratos "SELECT count(*) FROM spock.resolutions;") ))
  [[ "$got" -gt 0 ]] && break
  sleep 1
done
[[ "$got" -gt 0 ]] || { echo "::error::Spock recorded no resolution for the conflict in kratos"; exit 1; }
site=a
[[ "$(q a kratos "SELECT count(*) FROM spock.resolutions;")" -gt 0 ]] || site=b
want="$(q "$site" kratos "SELECT count(*) FROM spock.resolutions;")"
echo "  site $site: kratos spock.resolutions holds $want row(s); netbird holds $(q "$site" netbird "SELECT count(*) FROM spock.resolutions;")"

echo "== the monitoring role, with the observability role's grants"
q "$site" postgres "CREATE ROLE monitoring LOGIN PASSWORD '$MON_PW'; GRANT pg_monitor TO monitoring;"
for db in netbird kratos; do
  q "$site" postgres "GRANT CONNECT ON DATABASE $db TO monitoring;"
  q "$site" "$db" "GRANT USAGE ON SCHEMA spock TO monitoring; GRANT SELECT ON ALL TABLES IN SCHEMA spock TO monitoring;"
done
docker exec --user root "$PFX-$site" bash -c \
  "echo 'host all monitoring 0.0.0.0/0 scram-sha-256' >> /etc/postgresql/pg_hba.conf"
q "$site" postgres "SELECT pg_reload_conf();" >/dev/null
# The compose file reaches the database as postgres-a.
docker network disconnect "$NET" "$PFX-$site"
docker network connect --alias postgres-a "$NET" "$PFX-$site"

echo "== postgres-exporter as docker-compose.obs.yml runs it"
# The service's image and environment, with the .env values the
# observability role writes substituted in.
spec=()
while IFS= read -r line; do spec+=("$line"); done < <(POSTGRES_EXPORTER_PASSWORD="$MON_PW" POSTGRES_EXPORTER_SPOCK_DATABASES="$DATABASES" python3 - "$COMPOSE" <<'EOF'
import os, re, sys
import yaml
svc = yaml.safe_load(open(sys.argv[1]))["services"]["postgres-exporter"]
def expand(v):
    def one(m):
        name, rest = m.group(1), m.group(2) or ""
        if name in os.environ:
            return os.environ[name]
        if rest.startswith(":-"):
            return rest[2:]
        sys.exit(f"{name} is not set and has no default in the compose file")
    return re.sub(r"\$\{([A-Z0-9_]+)((?::[-?])[^}]*)?\}", one, str(v))
print(svc["image"])
for k, v in (svc.get("environment") or {}).items():
    print(f"{k}={expand(v)}")
EOF
)
image="${spec[0]}"
envs=()
for kv in "${spec[@]:1}"; do envs+=(-e "$kv"); done
docker run -d --name "$EXPORTER" --network "$NET" "${envs[@]}" \
  -v "$QUERIES:/etc/postgres-exporter/queries.yml:ro" "$image" >/dev/null

metrics=""
for _ in $(seq 1 30); do
  metrics="$(docker exec "$PFX-$site" bash -c "exec 3<>/dev/tcp/$EXPORTER/9187 && printf 'GET /metrics HTTP/1.0\r\nHost: x\r\n\r\n' >&3 && cat <&3" 2>/dev/null || true)"
  grep -q '^pg_up ' <<<"$metrics" && break
  sleep 1
done
grep -q '^pg_up 1' <<<"$metrics" || { echo "::error::the exporter never answered with pg_up 1"; docker logs "$EXPORTER" 2>&1 | tail -20; exit 1; }
echo "$metrics" | grep -E '^spock_resolutions_total' | sed 's/^/  /' || true

fail=0
line="$(grep -E '^spock_resolutions_total\{[^}]*datname="kratos"' <<<"$metrics" || true)"
if [[ -z "$line" ]]; then
  echo "::error::no spock_resolutions_total series for kratos: the exporter does not read kratos's spock.resolutions, so SpockConflictLogged cannot see its conflicts"
  docker logs "$EXPORTER" 2>&1 | grep -i spock | tail -5
  fail=1
elif [[ "${line##* }" != "$want" ]]; then
  echo "::error::spock_resolutions_total for kratos is ${line##* }, kratos's spock.resolutions holds $want"
  fail=1
fi
grep -qE '^spock_resolutions_total\{[^}]*datname="netbird"' <<<"$metrics" || {
  echo "::error::no spock_resolutions_total series for netbird"
  fail=1
}
if grep -q '^An error has occurred while serving metrics' <<<"$metrics"; then
  echo "::error::the exporter failed to serve /metrics"
  fail=1
fi
[[ "$fail" == 0 ]] && echo "PASS: the exporter counts kratos's conflict ($want) and reads netbird too"
exit "$fail"
