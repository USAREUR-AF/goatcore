#!/usr/bin/env bash
# shellcheck disable=SC2034  # RUN, SCRATCH, LOGDUMP and the helpers are read by the sourced scenarios
# Island-mode partition harness (ADR 1165 D5; design island-mode-multi-writer.md §6;
# scenarios: island-mode-write-inventory.md §10).
#
# Grown from ../partition-revoke/run.sh and sharing its two-site library
# (../lib/two-site.sh). Two postgres-spock sites carry the real NetBird
# management, Kratos and Hydra schemas at the fleet's pinned versions, joined
# by Spock subscriptions both ways over the repsets the roles declare
# (spock-mesh-peering: netbird_mgmt; ory-narrow-repset: ory_narrow), with the
# ADR 1193 revocation guard installed as those roles install it.
#
# Each scenario in scenarios/<name>.sh (named exactly as inventory §10):
#   seed        optional, at A before the cut, replicated to B
#   write_a     at A while cut off
#   write_b     at B while cut off
#   after_heal  optional, the hub (A) after heal
#   check       the scenario's own pass condition
#   EXPECT      every spock.exception_log / spock.resolutions row the
#               scenario accounts for: "<site> <db> <exception|resolution> <table> <type> <count>"
# Partition = both subscriptions disabled; heal = both enabled; then wait
# until each site's slots have confirmed the other's WAL.
#
# Common pass conditions (design §6), checked after every scenario:
#   1. drift-watch (ops/ansible/roles/spock-drift-watch) reports every
#      replicated table equal;
#   2. every exception_log / resolutions row is in the scenario's EXPECT set;
#   3. no setup key, token, peer key or login identifier is issued to two
#      subjects, and an identifier resolves to the same identity at both sites.
#
# Usage (Linux Docker, CI):  IMAGE=postgres-spock:smoke ./run.sh [scenario ...]
#   KEEP=1 leaves the containers up.
#   Two runs on one machine need their own names and ports (a run removes only
#   containers under its own prefix): HARNESS_PFX (default ph), HARNESS_NET
#   (partition-harness-net), and the host ports HARNESS_PORT_PG_A (55432),
#   HARNESS_PORT_PG_B (55433), HARNESS_PORT_KRATOS_A (54434),
#   HARNESS_PORT_KRATOS_B (54534), HARNESS_PORT_HYDRA (54445).

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../../../.." && pwd)"
cd "$HERE"

IMAGE="${IMAGE:?set IMAGE to a postgres-spock image}"
PFX="${HARNESS_PFX:-ph}"
NET="${HARNESS_NET:-partition-harness-net}"
PW="partition"
TS_PORT_a="${HARNESS_PORT_PG_A:-55432}"
TS_PORT_b="${HARNESS_PORT_PG_B:-55433}"
# shellcheck source=../lib/two-site.sh
source ../lib/two-site.sh

PY=(python3 "$HERE/harness.py")
DBS=(netbird kratos hydra)
ACCT="harnessacct000000000"
RUN="$(date +%s)"
WORK="$(mktemp -d)"
RESULTS="${RESULTS:-$WORK/results.tsv}"
: > "$RESULTS"
KRATOS_ADMIN_a="${HARNESS_PORT_KRATOS_A:-54434}"; KRATOS_ADMIN_b="${HARNESS_PORT_KRATOS_B:-54534}"
HYDRA_ADMIN="${HARNESS_PORT_HYDRA:-54445}"
SHARED_NID="$(python3 -c 'import yaml,sys; print(yaml.safe_load(open(sys.argv[1]))["ory_shared_nid"])' "$REPO/ops/ansible/roles/ory/defaults/main.yml")"
PRESEED_TS="$(python3 -c 'import yaml,sys; print(yaml.safe_load(open(sys.argv[1]))["ory_shared_preseed_ts"])' "$REPO/ops/ansible/roles/ory/defaults/main.yml")"

EXTRA=("$PFX-nb-a" "$PFX-nb-b" "$PFX-kratos-a" "$PFX-kratos-b" "$PFX-hydra-a")
cleanup() {
  [[ "${KEEP:-0}" == 1 ]] && { echo "KEEP=1, leaving containers up"; return; }
  ts_down "${EXTRA[@]}"
  rm -rf "$WORK"
}
trap cleanup EXIT
ts_down "${EXTRA[@]}"

eval "$("${PY[@]}" pins)"
echo "== pins: $NB_IMAGE"
echo "         $KRATOS_IMAGE"
echo "         $HYDRA_IMAGE"
docker pull -q "$NB_IMAGE" >/dev/null &
docker pull -q "$KRATOS_IMAGE" >/dev/null &
docker pull -q "$HYDRA_IMAGE" >/dev/null &

echo "== bring up two sites"
ts_up
for s in a b; do
  for db in kratos hydra; do q "$s" "$db" "CREATE EXTENSION IF NOT EXISTS spock;"; done
done
wait

# ---------------------------------------------------------------- schemas
echo "== NetBird management schema (AutoMigrate of the pinned image, each site)"
cat > "$WORK/management.json" <<'JSON'
{
  "Stuns": [{"Proto":"udp","URI":"stun:localhost:3478"}],
  "TURNConfig": {"Turns":[],"CredentialsTTL":"12h","Secret":"none"},
  "Signal": {"Proto":"http","URI":"localhost:80"},
  "Datadir": "/var/lib/netbird",
  "DataStoreEncryptionKey": "2KktuRrqu+C9aImt77zNPEdTTqe7iVQ3I2RrWQtAkkM=",
  "HttpConfig": {"Address":"0.0.0.0:80","AuthAudience":"harness","AuthIssuer":"https://harness.invalid/",
                 "AuthKeysLocation":"http://127.0.0.1:1/jwks.json","AuthUserIDClaim":"sub",
                 "IdpSignKeyRefreshEnabled": false},
  "IdpManagerConfig": {"ManagerType":"none"},
  "ReverseZone": "in-addr.arpa.",
  "StoreConfig": {"Engine":"postgres"}
}
JSON
chmod 644 "$WORK/management.json"
NB_TABLES=()
# goat_* tables are goat's ledgers, not NetBird's or Ory's: insert-only, never reset.
goat_table() { [[ "$1" == goat_* ]]; }
while read -r t; do goat_table "$t" || NB_TABLES+=("$t"); done < <("${PY[@]}" tables netbird)
nb_list="'$(IFS=,; echo "${NB_TABLES[*]}" | sed "s/,/','/g")'"
for s in a b; do
  docker run -d --name "$PFX-nb-$s" --network "$NET" \
    -e NETBIRD_RUN_MIGRATIONS=true -e NETBIRD_STORE_ENGINE=postgres -e NB_DISABLE_GEOLOCATION=true \
    -e NB_STORE_ENGINE_POSTGRES_DSN="host=$PFX-$s user=postgres password=$PW dbname=netbird port=5432 sslmode=disable" \
    -v "$WORK/management.json:/etc/netbird/management.json:ro" \
    "$NB_IMAGE" management --log-file console --disable-anonymous-metrics >/dev/null
done
for s in a b; do
  have=0
  for _ in $(seq 1 90); do
    have=$(q "$s" netbird "SELECT count(*) FROM information_schema.tables WHERE table_schema='public' AND table_name IN ($nb_list);")
    [[ "$have" == "${#NB_TABLES[@]}" ]] && break
    docker inspect -f '{{.State.Running}}' "$PFX-nb-$s" | grep -q true || break
    sleep 2
  done
  if [[ "$have" != "${#NB_TABLES[@]}" ]]; then
    echo "::error::site $s: $have of ${#NB_TABLES[@]} repset tables exist after NetBird AutoMigrate"
    q "$s" netbird "SELECT 'missing: ' || t FROM unnest(ARRAY[$nb_list]) t
                    WHERE t NOT IN (SELECT table_name FROM information_schema.tables WHERE table_schema='public');"
    docker logs "$PFX-nb-$s" 2>&1 | tail -40
    exit 1
  fi
done
sleep 5  # AutoMigrate creates indexes after the tables
for s in a b; do docker logs "$PFX-nb-$s" 2>&1 | grep -iE 'version|migrat' | head -3 | sed "s/^/  nb-$s: /"; done
docker rm -f "$PFX-nb-a" "$PFX-nb-b" >/dev/null
echo "  ${#NB_TABLES[@]} repset tables present at both sites; management stopped (writes below are SQL)"

echo "== Kratos and Hydra schemas (migrate of the pinned images, each site)"
mkdir -p "$WORK/ory"
cat > "$WORK/ory/identity-schema.json" <<'JSON'
{"$id":"https://harness.test/person.schema.json","$schema":"http://json-schema.org/draft-07/schema#","title":"Person","type":"object",
 "properties":{"traits":{"type":"object","properties":{
   "email":{"type":"string","format":"email","ory.sh/kratos":{"credentials":{"password":{"identifier":true}},
            "verification":{"via":"email"},"recovery":{"via":"email"}}},
   "name":{"type":"string"}},
  "required":["email","name"],"additionalProperties":false}}}
JSON
cat > "$WORK/ory/kratos.yaml" <<'YAML'
version: v1.3.1
serve:
  public: { base_url: "http://127.0.0.1:4433/" }
  admin:  { base_url: "http://127.0.0.1:4434/" }
identity:
  default_schema_id: default
  schemas:
    - { id: default, url: "file:///etc/config/identity-schema.json" }
selfservice:
  default_browser_return_url: "http://127.0.0.1:4433/"
  methods: { password: { enabled: true } }
  flows:
    settings:     { ui_url: "http://127.0.0.1:4433/settings" }
    login:        { ui_url: "http://127.0.0.1:4433/login" }
    registration: { ui_url: "http://127.0.0.1:4433/register" }
    recovery:     { enabled: true, ui_url: "http://127.0.0.1:4433/recovery" }
    verification: { enabled: true, ui_url: "http://127.0.0.1:4433/verification" }
secrets:
  cookie: ["partition-harness-cookie-secret-fixture"]
  cipher: ["partitionharnesscipher32bytesxxx"]
hashers: { algorithm: bcrypt, bcrypt: { cost: 4 } }
courier: { smtp: { connection_uri: "smtps://localhost:1025/?disable_starttls=true" } }
YAML
cat > "$WORK/ory/hydra.yaml" <<'YAML'
serve: { public: { port: 4444 }, admin: { port: 4445 } }
urls:
  self: { issuer: "http://127.0.0.1:4444/" }
  consent: "http://127.0.0.1:3000/consent"
  login: "http://127.0.0.1:3000/login"
secrets: { system: ["partition-harness-hydra-system-secret"] }
oidc: { subject_identifiers: { supported_types: [public] } }
YAML
chmod -R a+rX "$WORK/ory"
for s in a b; do
  docker run --rm --network "$NET" -e DSN="postgres://postgres:$PW@$PFX-$s:5432/kratos?sslmode=disable" \
    -v "$WORK/ory:/etc/config:ro" "$KRATOS_IMAGE" migrate -c /etc/config/kratos.yaml sql -e --yes \
    >"$WORK/migrate.log" 2>&1 || { cat "$WORK/migrate.log"; exit 1; }
  docker run --rm --network "$NET" -e DSN="postgres://postgres:$PW@$PFX-$s:5432/hydra?sslmode=disable" \
    -v "$WORK/ory:/etc/config:ro" "$HYDRA_IMAGE" migrate -c /etc/config/hydra.yaml sql up -e --yes \
    >"$WORK/migrate.log" 2>&1 || { cat "$WORK/migrate.log"; exit 1; }
  # roles/ory: pre-seed the shared networks row before either service starts.
  for db in kratos hydra; do
    q "$s" "$db" "INSERT INTO networks (id, created_at, updated_at) VALUES ('$SHARED_NID', '$PRESEED_TS', '$PRESEED_TS') ON CONFLICT (id) DO NOTHING;"
  done
done
echo "  migrated; shared nid $SHARED_NID pre-seeded"

# ---------------------------------------------------------------- spock
echo "== Spock: nodes, repsets from the role defaults, revocation guard, subscriptions"
wire() { # wire <db> <repset> [sql file ...]   (the files the roles install before curating the repset)
  local db="$1" repset="$2" s t f
  shift 2
  ts_node_create "$db"
  for s in a b; do
    q "$s" "$db" "SELECT spock.repset_create('$repset');" >/dev/null
    for f in "$@"; do
      docker exec -i "$PFX-$s" psql -U postgres -d "$db" -v ON_ERROR_STOP=1 -q -c "SET client_min_messages = warning;" -f - < "$f"
    done
    while read -r t; do
      q "$s" "$db" "SELECT spock.repset_add_table('$repset', 'public.$t'::regclass, false);" >/dev/null
    done < <("${PY[@]}" tables "$db")
    local want got
    want=$("${PY[@]}" tables "$db" | wc -l | tr -d ' ')
    got=$(q "$s" "$db" "SELECT count(*) FROM spock.replication_set_table rt JOIN spock.replication_set rs ON rs.set_id = rt.set_id WHERE rs.set_name = '$repset';")
    [[ "$got" == "$want" ]] || { echo "::error::site $s $db: $repset holds $got tables, want $want"; exit 1; }
  done
  ts_sub_create "$db" "$repset"
}
wire netbird netbird_mgmt "$REPO/ops/postgres/spock/revocation-guard/netbird.sql" \
                          "$REPO/ops/postgres/spock/hub-authority/netbird.sql"
wire kratos  ory_narrow   "$REPO/ops/postgres/spock/revocation-guard/kratos.sql" \
                          "$REPO/ops/postgres/spock/hub-authority/kratos.sql"
wire hydra   ory_narrow
ts_wait_replicating "${DBS[@]}"
echo "  replicating: ${DBS[*]}"
# Each site's own configuration names the hub (ADR 1205 D8), as the server
# setting goat.hub_site, written as ory-narrow-repset writes it from
# goatnet_hub_site; never a replicated row. Set after the subscriptions, so
# the scenarios also show that running apply workers read it after a reload.
for s in a b; do
  printf '%s\n' "SET goat.hub_site = '';" "ALTER SYSTEM SET goat.hub_site = 'site_a';" "SELECT pg_reload_conf();" \
    | docker exec -i "$PFX-$s" psql -U postgres -d kratos -v ON_ERROR_STOP=1 -q >/dev/null
done
sleep 1
for s in a b; do
  [[ "$(q "$s" kratos "SELECT current_setting('goat.hub_site', true);")" == site_a ]] \
    || { echo "::error::$PFX-$s does not read goat.hub_site"; exit 1; }
done
echo "  goat.hub_site = site_a at both sites"

# Every site's slots have confirmed the other's current WAL, twice: once for
# the writes, once for what the guard's re-apply triggers wrote on arrival.
sync_all() {
  local db s
  for _ in 1 2; do
    for db in "${DBS[@]}"; do
      for s in a b; do
        q "$s" "$db" "SET statement_timeout = '90s'; SELECT spock.wait_slot_confirm_lsn(NULL, NULL);" >/dev/null
      done
    done
  done
}

echo "== Kratos at both sites, Hydra at the hub (A)"
for s in a b; do
  port="KRATOS_ADMIN_$s"
  docker run -d --name "$PFX-kratos-$s" --network "$NET" -p "127.0.0.1:${!port}:4434" \
    -e DSN="postgres://postgres:$PW@$PFX-$s:5432/kratos?sslmode=disable" \
    -v "$WORK/ory:/etc/config:ro" "$KRATOS_IMAGE" serve -c /etc/config/kratos.yaml --dev >/dev/null
done
docker run -d --name "$PFX-hydra-a" --network "$NET" -p "127.0.0.1:$HYDRA_ADMIN:4445" \
  -e DSN="postgres://postgres:$PW@$PFX-a:5432/hydra?sslmode=disable" \
  -v "$WORK/ory:/etc/config:ro" "$HYDRA_IMAGE" serve -c /etc/config/hydra.yaml all --dev >/dev/null
for url in "http://127.0.0.1:$KRATOS_ADMIN_a/admin/health/ready" "http://127.0.0.1:$KRATOS_ADMIN_b/admin/health/ready" \
           "http://127.0.0.1:$HYDRA_ADMIN/health/ready"; do
  for _ in $(seq 1 60); do curl -fs -o /dev/null "$url" && break; sleep 1; done
  curl -fsS -o /dev/null "$url" || { echo "::error::$url not ready"; for c in "$PFX-kratos-a" "$PFX-kratos-b" "$PFX-hydra-a"; do docker logs "$c" 2>&1 | tail -20; done; exit 1; }
done
echo "  ready"

# ---------------------------------------------------------------- helpers for scenarios
nb() { q "$1" netbird "$2"; }
kr() { q "$1" kratos "$2"; }
hy() { q "$1" hydra "$2"; }
xid() { openssl rand -hex 10; }  # not tr|head: tr dies of SIGPIPE, which pipefail turns into a failed step
kapi() { # kapi <site> <METHOD> <path> [json]
  local port="KRATOS_ADMIN_$1"
  curl -fsS -X "$2" "http://127.0.0.1:${!port}$3" -H 'Content-Type: application/json' ${4:+-d "$4"}
}
hapi() { # hapi <METHOD> <path> [json]   (Hydra runs at the hub only)
  curl -fsS -X "$1" "http://127.0.0.1:$HYDRA_ADMIN$2" -H 'Content-Type: application/json' ${3:+-d "$3"}
}
kratos_create() { # kratos_create <site> <email> <name> [metadata_public json]  -> prints the identity id; the creating site owns it (ADR 1205 D5)
  kapi "$1" POST /admin/identities "$(jq -cn --arg e "$2" --arg n "$3" --arg o "site_$1" --argjson m "${4:-null}" \
    '{schema_id:"default", traits:{email:$e, name:$n}, metadata_admin:{owner_site:$o},
      credentials:{password:{config:{password:"harness-Passw0rd-not-secret"}}}} + (if $m then {metadata_public:$m} else {} end)')" | jq -r .id
}
kname() { kr "$1" "SELECT traits->>'name' FROM identities WHERE id='$2';"; }
pa_admin() { # pa_admin <site> <identity id>  -> yes|no: metadata_public.groups holds PLATFORM-ADMIN
  kr "$1" "SELECT CASE WHEN coalesce(metadata_public->'groups', '[]'::jsonb) ? 'PLATFORM-ADMIN' THEN 'yes' ELSE 'no' END
             FROM identities WHERE id = '$2';"
}

# Hub directives (ADR 1205). Site A is the hub. The applier is the script the
# spock-hub-directives role installs, run here against one site.
HUB=site_a
DIRECTIVES_SH="$REPO/ops/ansible/roles/spock-hub-directives/files/spock-hub-directives.sh"
for s in a b; do
  kport="KRATOS_ADMIN_$s"
  cat > "$WORK/directives-$s.env" <<ENV
HUB_DIRECTIVES_SITE=site_$s
HUB_DIRECTIVES_HUB=$HUB
HUB_DIRECTIVES_DBS="${DBS[*]}"
HUB_DIRECTIVES_KRATOS=http://127.0.0.1:${!kport}
HUB_DIRECTIVES_TEXTFILE=
HUB_DIRECTIVES_POSTGRES=$PFX-$s
ENV
done
directive() { # directive <site> <kind> <subject> [body json]  -> prints the directive id
  local body="${4:-}"
  [[ -n "$body" ]] || body='{}'
  nb "$1" "INSERT INTO goat_hub_directives (kind, subject, body, actor)
           VALUES ('$2', '$3', '$body'::jsonb, 'harness') RETURNING id;"
}
apply_directives() { # apply_directives <site>
  HUB_DIRECTIVES_ENV_FILE="$WORK/directives-$1.env" bash "$DIRECTIVES_SH" | sed "s/^/      [$1] /"
}
verdict() { # verdict <site> <directive id>
  nb "$1" "SELECT verdict || ' ' || coalesce(origin, '-') FROM goat_hub_directive_applied WHERE id = '$2';"
}
nb_mint_key() { # nb_mint_key <site> <id> <name>   NetBird SaveSetupKey
  nb "$1" "INSERT INTO setup_keys (id, account_id, key, key_secret, name, type, created_at, expires_at, updated_at,
                                   revoked, used_times, auto_groups, usage_limit, ephemeral, allow_extra_dns_labels)
           VALUES ('$2', '$ACCT', encode(sha256(random()::text::bytea), 'base64'), 'HARN****', '$3', 'one-off',
                   now(), now() + interval '1 day', now(), false, 0, '[]', 1, false, false);"
}
nb_peer() { # nb_peer <site> <id> <name>   a registered peer (AddPeer)
  nb "$1" "INSERT INTO peers (id, account_id, key, name, dns_label, user_id, created_at, peer_status_connected,
                              peer_status_login_expired, peer_status_last_seen)
           VALUES ('$2', '$ACCT', encode(sha256(random()::text::bytea), 'base64'), '$3', '$3', '', now(), false, false, now());"
}
FAILF=""
ok()   { echo "    ok    $*"; }
fail() { echo "    FAIL  $*"; echo "$*" >> "$FAILF"; }
expect_eq() { # expect_eq <what> <got> <want>
  if [[ "$2" == "$3" ]]; then ok "$1 = $3"; else fail "$1: got '$2', want '$3'"; fi
}

clear_logs() {
  local db s
  for db in "${DBS[@]}"; do for s in a b; do
    q "$s" "$db" "DELETE FROM spock.exception_log; DELETE FROM spock.resolutions;" >/dev/null
  done; done
}
dump_logs() { # dump_logs <file>
  local db s
  : > "$1"
  for s in a b; do for db in "${DBS[@]}"; do
    echo "$s $db exception $(q "$s" "$db" "SELECT coalesce(json_agg(to_jsonb(e)), '[]') FROM spock.exception_log e;")" >> "$1"
    echo "$s $db resolution $(q "$s" "$db" "SELECT coalesce(json_agg(to_jsonb(r)), '[]') FROM spock.resolutions r;")" >> "$1"
  done; done
}

# Common condition 1: the drift-watch role's own compare, over every table
# the repsets declare.
export HARNESS_PG_PW="$PW"
"${PY[@]}" drift-env "$WORK/drift.env" "$TS_PORT_a" "$TS_PORT_b" HARNESS_PG_PW
DRIFT_SH="$REPO/ops/ansible/roles/spock-drift-watch/files/spock-drift-watch.sh"
drift_check() { # drift_check <label>  -> fail() per drifting table, with the differing rows
  local rc=0 line db tab
  SPOCK_DRIFT_ENV_FILE="$WORK/drift.env" bash "$DRIFT_SH" >/dev/null 2>"$WORK/drift.err" || rc=$?
  if [[ "$rc" != 0 ]]; then fail "$1: drift-watch exited $rc: $(tr '\n' ' ' < "$WORK/drift.err")"; return; fi
  local n=0
  while read -r line; do
    db=$(sed -E 's/.*db="([^"]+)".*/\1/' <<<"$line")
    tab=$(sed -E 's#.*table_pair="[^/]+/([^"]+)".*#\1#' <<<"$line")
    fail "$1: drift-watch: $db.$tab differs between the sites"
    diff <(q a "$db" "SELECT t::text FROM \"$tab\" t ORDER BY 1;") \
         <(q b "$db" "SELECT t::text FROM \"$tab\" t ORDER BY 1;") | head -8 | sed 's/^/          /' || true
    n=$((n+1))
  done < <(grep '^spock_ace_drift_rows' "$WORK/drift.prom" | grep -v ' 0$' || true)
  if [[ "$n" == 0 ]]; then ok "$1: drift-watch equal on $(grep -c '^spock_ace_drift_rows' "$WORK/drift.prom") replicated tables"; fi
}

# Common condition 3: nothing issued to two subjects.
unique_check() {
  local s
  for s in a b; do
    expect_eq "$s: setup key secrets on two keys" "$(nb "$s" "SELECT count(*) FROM (SELECT key FROM setup_keys GROUP BY key HAVING count(*) > 1) d;")" 0
    expect_eq "$s: tokens on two token rows"     "$(nb "$s" "SELECT count(*) FROM (SELECT hashed_token FROM personal_access_tokens GROUP BY 1 HAVING count(*) > 1) d;")" 0
    expect_eq "$s: peer keys on two peers"       "$(nb "$s" "SELECT count(*) FROM (SELECT key FROM peers WHERE key <> '' GROUP BY key HAVING count(*) > 1) d;")" 0
    expect_eq "$s: emails on two active identities" \
      "$(kr "$s" "SELECT count(*) FROM (SELECT lower(traits->>'email') FROM identities WHERE state = 'active' GROUP BY 1 HAVING count(*) > 1) d;")" 0
  done
  local map="SELECT coalesce(string_agg(ci.identifier || '=' || c.identity_id, ',' ORDER BY ci.identifier), '')
               FROM identity_credential_identifiers ci JOIN identity_credentials c ON c.id = ci.identity_credential_id;"
  expect_eq "login identifiers resolve to the same identity at both sites" "$(kr a "$map" | md5sum)" "$(kr b "$map" | md5sum)"
}

# ---------------------------------------------------------------- baseline
echo "== seed the shared NetBird account at A"
nb a "INSERT INTO accounts (id, created_by, created_at, domain, domain_category, is_domain_primary_account,
                            network_identifier, network_net, network_serial)
      VALUES ('$ACCT', 'harness', now(), 'harness.test', 'private', true, 'harness-net',
              '{\"IP\":\"100.64.0.0\",\"Mask\":\"//8AAA==\"}', 0);"
sync_all
FAILF="$WORK/baseline.fail"; : > "$FAILF"
echo "== baseline (before any scenario)"
drift_check baseline
if [[ -s "$FAILF" ]]; then
  echo "::error::the two sites differ before any scenario ran; the harness cannot attribute drift"
  exit 1
fi

# Scenarios leave rows behind, and one that diverges would make every later
# one fail on its drift. After each scenario, both sites go back to the rows
# they held at baseline. The revocation ledger is insert-only and keeps every
# row; no scenario reuses an id.
snapshot_baseline() {
  local s db t
  for s in a b; do for db in "${DBS[@]}"; do
    q "$s" "$db" "CREATE SCHEMA harness_baseline;"
    while read -r t; do
      goat_table "$t" && continue
      q "$s" "$db" "CREATE TABLE harness_baseline.\"$t\" AS SELECT * FROM public.\"$t\";"
    done < <("${PY[@]}" tables "$db")
  done; done
}
reset_to_baseline() {
  local s db t sql
  for s in a b; do for db in "${DBS[@]}"; do
    sql="SET session_replication_role = replica;"
    while read -r t; do
      goat_table "$t" && continue
      sql+=" DELETE FROM public.\"$t\" x WHERE NOT EXISTS (SELECT 1 FROM harness_baseline.\"$t\" b WHERE b::text = x::text);"
      sql+=" INSERT INTO public.\"$t\" SELECT * FROM harness_baseline.\"$t\" b WHERE NOT EXISTS (SELECT 1 FROM public.\"$t\" x WHERE x::text = b::text);"
    done < <("${PY[@]}" tables "$db")
    q "$s" "$db" "$sql" >/dev/null
  done; done
  sync_all
}
snapshot_baseline

# ---------------------------------------------------------------- scenarios
run_scenario() { # run_scenario <file>
  local f="$1" name; name="$(basename "$f" .sh)"
  FAILF="$WORK/$name.fail"; : > "$FAILF"
  SCRATCH="$WORK/$name.d"; mkdir -p "$SCRATCH"; : > "$SCRATCH/expect"
  LOGDUMP="$WORK/$name.logs"
  echo "== $name"
  clear_logs
  local rc=0
  # A subshell on the left of || runs with errexit off, so a failing step
  # would be ignored; run it plainly and read its status instead.
  set +e
  (
    set -euo pipefail
    # shellcheck disable=SC1090
    source "$f"
    printf '%s\n' "${EXPECT:-}" > "$SCRATCH/expect"
    if declare -F seed >/dev/null; then seed; sync_all; fi
    ts_cut "${DBS[@]}"
    if declare -F write_a >/dev/null; then write_a; fi
    if declare -F write_b >/dev/null; then write_b; fi
    ts_heal "${DBS[@]}"
    ts_wait_replicating "${DBS[@]}"
    sync_all
    if declare -F after_heal >/dev/null; then after_heal; sync_all; fi
    dump_logs "$LOGDUMP"
    check
  )
  rc=$?
  set -e
  if [[ "$rc" != 0 ]]; then
    fail "harness error: the scenario's steps exited $rc"
    ts_heal "${DBS[@]}" 2>/dev/null || true
    ts_wait_replicating "${DBS[@]}" || true
    sync_all || true
    [[ -f "$LOGDUMP" ]] || dump_logs "$LOGDUMP"
  fi
  drift_check "$name"
  if "${PY[@]}" account "$SCRATCH/expect" "$LOGDUMP" > "$WORK/account.out"; then
    ok "every exception_log / resolutions row is in the scenario's expected set"
  else
    fail "spock.exception_log / spock.resolutions rows outside the expected set"
    cat "$WORK/account.out"
  fi
  unique_check
  reset_to_baseline
  if [[ -s "$FAILF" ]]; then
    printf '%s\tFAIL\t%s\n' "$name" "$(head -1 "$FAILF")" >> "$RESULTS"
  else
    printf '%s\tPASS\t\n' "$name" >> "$RESULTS"
  fi
}

if [[ $# -gt 0 ]]; then files=(); for n in "$@"; do files+=("scenarios/$n.sh"); done
else files=(scenarios/*.sh); fi
for f in "${files[@]}"; do run_scenario "$f"; done

# Soak (SOAK_SECONDS > 0): run the same scenarios again, each pass with fresh
# ids, until the time is spent. A scenario that fails in any pass, other than
# one listed in expected-failing.tsv, fails the soak.
if [[ "${SOAK_SECONDS:-0}" -gt 0 ]]; then
  end=$(( $(date +%s) + SOAK_SECONDS )); pass=1
  cp "$RESULTS" "$WORK/soak-all.tsv"
  while (( $(date +%s) < end )); do
    pass=$((pass+1)); RUN="$(date +%s)$pass"
    echo "== soak pass $pass ($(( end - $(date +%s) ))s left)"
    : > "$RESULTS"
    for f in "${files[@]}"; do run_scenario "$f"; done
    cat "$RESULTS" >> "$WORK/soak-all.tsv"
  done
  echo "== soak: $pass passes"
  bad=0
  while IFS=$'\t' read -r n st why; do
    [[ "$st" == FAIL ]] || continue
    grep -q "^$n"$'\t' scenarios/expected-failing.tsv && continue
    echo "  FAIL $n: $why"; bad=$((bad+1))
  done < "$WORK/soak-all.tsv"
  awk -F'\t' '{c[$1" "$2]++} END {for (k in c) print "  " k, c[k]}' "$WORK/soak-all.tsv" | sort
  [[ -z "${SOAK_RESULTS:-}" ]] || cp "$WORK/soak-all.tsv" "$SOAK_RESULTS"
  if (( bad > 0 )); then echo "== soak: $bad unexpected failure(s)"; exit 1; fi
  echo "== soak: ok"
fi

"${PY[@]}" coverage "$RESULTS"
