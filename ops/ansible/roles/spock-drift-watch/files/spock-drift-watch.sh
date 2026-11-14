#!/usr/bin/env bash
#
# spock-drift-watch.sh — Block 52A.3 interim L2a drift detector.
#
# For every unordered pair of peers in $SPOCK_DRIFT_PEERS, for every
# (db, table) in $SPOCK_DRIFT_TABLES, run a count + content-hash diff
# and emit Prometheus textfile-collector metrics.
#
# A heal can lose a row that the diff then reports as equal: when two sites
# wrote different rows under one key, last-write-wins keeps one at both. So
# for every peer and watched table it also reports how many rows Spock's own
# log (spock.resolutions) shows replaced with different content:
# spock_ace_lww_lost_rows. It is a count of the log as it stands, so an alert
# fires on an increase; nothing is kept between runs.
#
# Inputs (from /etc/spock-drift-watch.env, or $SPOCK_DRIFT_ENV_FILE):
#   SPOCK_DRIFT_SITE        — site label
#   SPOCK_DRIFT_TEXTFILE    — path to the .prom output file
#   SPOCK_DRIFT_PEERS       — JSON array of peers
#                             [{"name","host","port","dbname","user","password_env"}, ...]
#   SPOCK_DRIFT_TABLES      — JSON array of {db, table[, where]}; `where` limits
#                             the compare to the rows every site must share
#   <password_env vars>     — actual passwords, referenced by name from peer entries
#
# Output: atomic write to $SPOCK_DRIFT_TEXTFILE. Failures during the
# run leave the previous file intact so a transient failure doesn't
# blank the dashboard.
#
# Exit codes: 0 on success (including no-peers case), 1 on config error,
# 2 on persistent psql failure (already retried 3x). Non-zero exit
# leaves stale metrics; the SpockACEDriftWatchStale alert covers that.

set -euo pipefail

# SPOCK_DRIFT_ENV_FILE overrides the path; the partition harness
# (ops/postgres/spock/tests/partition-harness) runs this same compare.
ENV_FILE="${SPOCK_DRIFT_ENV_FILE:-/etc/spock-drift-watch.env}"
if [[ ! -r "$ENV_FILE" ]]; then
  echo "spock-drift-watch: $ENV_FILE not readable" >&2
  exit 1
fi
# shellcheck disable=SC1090
source "$ENV_FILE"

: "${SPOCK_DRIFT_SITE:?missing}"
: "${SPOCK_DRIFT_TEXTFILE:?missing}"
: "${SPOCK_DRIFT_PEERS:?missing}"
: "${SPOCK_DRIFT_TABLES:?missing}"

if ! command -v psql >/dev/null 2>&1; then
  echo "spock-drift-watch: psql not on PATH" >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "spock-drift-watch: jq not on PATH" >&2
  exit 1
fi

TMP_OUT=$(mktemp -t spock-drift-XXXXXX.prom)
trap 'rm -f "$TMP_OUT"' EXIT

emit() { printf '%s\n' "$*" >>"$TMP_OUT"; }

emit "# HELP spock_ace_drift_rows Per-table-pair drift count (rows differing or count mismatch)."
emit "# TYPE spock_ace_drift_rows gauge"
emit "# HELP spock_ace_drift_watch_last_run_seconds Unix epoch of last successful run."
emit "# TYPE spock_ace_drift_watch_last_run_seconds gauge"
emit "# HELP spock_ace_drift_watch_peers Number of peers configured (drift-watch is dormant when <2)."
emit "# TYPE spock_ace_drift_watch_peers gauge"
emit "# HELP spock_ace_lww_lost_rows Rows of a watched table this peer's spock.resolutions shows last-write-wins replaced with different content (insert_exists, update_update). Alert on an increase."
emit "# TYPE spock_ace_lww_lost_rows gauge"
emit "# HELP spock_ace_lww_lost_readable 1 when drift-watch's user can read this peer's spock.resolutions (or the database has none), 0 when it cannot and no loss count is made."
emit "# TYPE spock_ace_lww_lost_readable gauge"

PEER_COUNT=$(jq 'length' <<<"$SPOCK_DRIFT_PEERS")
emit "spock_ace_drift_watch_peers{site=\"${SPOCK_DRIFT_SITE}\"} ${PEER_COUNT}"

if [[ "$PEER_COUNT" -lt 2 ]]; then
  echo "spock-drift-watch: ${PEER_COUNT} peer(s) configured; need ≥2 for diff. Emitting heartbeat only."
  emit "spock_ace_drift_watch_last_run_seconds{site=\"${SPOCK_DRIFT_SITE}\"} $(date +%s)"
  install -m 0644 "$TMP_OUT" "$SPOCK_DRIFT_TEXTFILE"
  exit 0
fi

# Helper: query one peer for (count, hash) on a (db, table). Resolves
# password from the peer's named env var. 3-attempt retry on psql
# error (network blips, brief postgres bounce). Echoes
# "<count>|<hash>" or "ERR" on persistent failure.
peer_query() {
  local peer_json="$1" db="$2" table="$3" where="${4:-}"
  local sql
  # Identifier interpolation: table comes from config (operator-controlled,
  # not user input), so direct interpolation is safe; we still use
  # quote_ident-equivalent defensive double-quoting.
  sql="SELECT COUNT(*)::text || '|' || COALESCE(md5(string_agg(md5(t::text), ',' ORDER BY t::text)), '') FROM \"${table//\"/\"\"}\" t${where:+ WHERE $where};"

  peer_sql "$peer_json" "$db" "$sql"
}

# Helper: run one statement on one peer's database, with the peer's password
# and a 3-attempt retry. Echoes the rows, or "ERR" on persistent failure.
peer_sql() {
  local peer_json="$1" db="$2" sql="$3"
  local host port user password_env password
  host=$(jq -r '.host'        <<<"$peer_json")
  port=$(jq -r '.port'        <<<"$peer_json")
  user=$(jq -r '.user'        <<<"$peer_json")
  password_env=$(jq -r '.password_env' <<<"$peer_json")
  password="${!password_env:-}"
  if [[ -z "$password" ]]; then
    echo "ERR"
    return
  fi
  local _attempt out
  for _attempt in 1 2 3; do
    out=$(PGPASSWORD="$password" psql \
            -h "$host" -p "$port" -U "$user" -d "$db" \
            -X -A -t -q -c "$sql" 2>/dev/null) && {
      printf '%s' "$out"
      return
    }
    sleep 2
  done
  echo "ERR"
}

# Rows last-write-wins replaced with different content, per table, from one
# peer's spock.resolutions in one database: "<table>|<count>" lines. A
# resolution whose two versions are equal (both sites wrote the same row) is
# no loss. Whether this peer's user may read the log is asked first: a
# statement naming spock.resolutions fails as it is parsed when the table is
# missing or its schema is not readable. Absent is no loss; denied is
# reported as spock_ace_lww_lost_readable 0, not as a failed run, since the
# user drift-watch connects as need not be a superuser.
RESOLUTIONS_ACCESS_SQL="SELECT CASE
  WHEN NOT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'spock') THEN 'absent'
  WHEN NOT has_schema_privilege('spock', 'USAGE') THEN 'denied'
  WHEN to_regclass('spock.resolutions') IS NULL THEN 'absent'
  WHEN has_table_privilege('spock.resolutions', 'SELECT') THEN 'readable'
  ELSE 'denied' END;"
LOST_SQL="SELECT regexp_replace(relname, '^.*[.]', '') || '|' || count(*)
            FROM spock.resolutions
           WHERE conflict_type IN ('insert_exists', 'update_update')
             AND local_tuple IS DISTINCT FROM remote_tuple
           GROUP BY regexp_replace(relname, '^.*[.]', '');"

mapfile -t PEERS_LIST < <(jq -c '.[]' <<<"$SPOCK_DRIFT_PEERS")
mapfile -t TABLES_LIST < <(jq -c '.[]' <<<"$SPOCK_DRIFT_TABLES")

# Pair-wise comparison across peers.
fail=0
for ((i=0; i<${#PEERS_LIST[@]}; i++)); do
  for ((j=i+1; j<${#PEERS_LIST[@]}; j++)); do
    a_json="${PEERS_LIST[$i]}"
    b_json="${PEERS_LIST[$j]}"
    a_name=$(jq -r '.name' <<<"$a_json")
    b_name=$(jq -r '.name' <<<"$b_json")
    pair="${a_name}-vs-${b_name}"

    for tab_json in "${TABLES_LIST[@]}"; do
      db=$(jq -r '.db'    <<<"$tab_json")
      table=$(jq -r '.table' <<<"$tab_json")
      where=$(jq -r '.where // empty' <<<"$tab_json")

      a_out=$(peer_query "$a_json" "$db" "$table" "$where")
      b_out=$(peer_query "$b_json" "$db" "$table" "$where")

      if [[ "$a_out" == "ERR" || "$b_out" == "ERR" ]]; then
        # Skip — keep going. Heartbeat still emits at end iff at
        # least one pair completed cleanly. Track failure for exit.
        fail=$((fail + 1))
        continue
      fi

      a_count="${a_out%%|*}"; a_hash="${a_out##*|}"
      b_count="${b_out%%|*}"; b_hash="${b_out##*|}"

      drift=0
      if [[ "$a_count" != "$b_count" ]]; then
        # Absolute count delta is the lower bound of differing rows.
        if (( a_count > b_count )); then
          drift=$(( a_count - b_count ))
        else
          drift=$(( b_count - a_count ))
        fi
      elif [[ "$a_hash" != "$b_hash" ]]; then
        # Counts match, content differs ⇒ at least one column-level
        # divergence. We can't tell
        # how many without per-row diff; report 1 to fire the alert.
        drift=1
      fi

      emit "spock_ace_drift_rows{site=\"${SPOCK_DRIFT_SITE}\",db=\"${db}\",table_pair=\"${pair}/${table}\"} ${drift}"
    done
  done
done

# Per peer: what last-write-wins replaced on apply, for every watched table.
mapfile -t LOST_DBS < <(jq -r '[.[].db] | unique | .[]' <<<"$SPOCK_DRIFT_TABLES")
for peer_json in "${PEERS_LIST[@]}"; do
  peer_name=$(jq -r '.name' <<<"$peer_json")
  for db in "${LOST_DBS[@]}"; do
    access=$(peer_sql "$peer_json" "$db" "$RESOLUTIONS_ACCESS_SQL")
    lost=""
    [[ "$access" == "readable" ]] && lost=$(peer_sql "$peer_json" "$db" "$LOST_SQL")
    if [[ "$access" == "ERR" || "$lost" == "ERR" ]]; then
      fail=$((fail + 1))
      continue
    fi
    if [[ "$access" == "denied" ]]; then
      echo "spock-drift-watch: ${peer_name}/${db}: this user cannot read spock.resolutions; no loss count" >&2
      emit "spock_ace_lww_lost_readable{site=\"${SPOCK_DRIFT_SITE}\",peer=\"${peer_name}\",db=\"${db}\"} 0"
      continue
    fi
    emit "spock_ace_lww_lost_readable{site=\"${SPOCK_DRIFT_SITE}\",peer=\"${peer_name}\",db=\"${db}\"} 1"
    while read -r table; do
      n=$(awk -F'|' -v t="$table" '$1 == t {print $2}' <<<"$lost")
      emit "spock_ace_lww_lost_rows{site=\"${SPOCK_DRIFT_SITE}\",peer=\"${peer_name}\",db=\"${db}\",table=\"${table}\"} ${n:-0}"
    done < <(jq -r --arg db "$db" '.[] | select(.db == $db) | .table' <<<"$SPOCK_DRIFT_TABLES")
  done
done

emit "spock_ace_drift_watch_last_run_seconds{site=\"${SPOCK_DRIFT_SITE}\"} $(date +%s)"

install -m 0644 "$TMP_OUT" "$SPOCK_DRIFT_TEXTFILE"

if (( fail > 0 )); then
  echo "spock-drift-watch: ${fail} peer-query failure(s); partial metrics written" >&2
  exit 2
fi
exit 0
