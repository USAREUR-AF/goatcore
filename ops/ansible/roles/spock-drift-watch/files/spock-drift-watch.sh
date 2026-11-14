#!/usr/bin/env bash
#
# spock-drift-watch.sh — Block 52A.3 interim L2a drift detector.
#
# For every unordered pair of peers in $SPOCK_DRIFT_PEERS, for every
# (db, table) in $SPOCK_DRIFT_TABLES, run a count + content-hash diff
# and emit Prometheus textfile-collector metrics.
#
# For every peer whose kratos database is watched it also reports the
# platform-admin grants that peer's guard refused (ADR 1205 D10): the rows of
# its goat_ceiling_refused, spock_ceiling_refused_rows. Before counting it runs
# goat_ceiling_refused_stamp(), which fills in each refused grant's commit
# timestamp, when its user may; the newest is
# spock_ceiling_refused_last_commit_seconds. It is a count of the table as it
# stands, so an alert fires on an increase.
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
emit "# HELP spock_ceiling_guard_installed 1 when this peer's kratos database has the platform-admin guard's log (goat_ceiling_refused), 0 when it does not."
emit "# TYPE spock_ceiling_guard_installed gauge"
emit "# HELP spock_ceiling_refused_readable 1 when drift-watch's user can read this peer's goat_ceiling_refused, 0 when it cannot and no count is made."
emit "# TYPE spock_ceiling_refused_readable gauge"
emit "# HELP spock_ceiling_refused_rows Platform-admin grants this peer's guard refused: the rows of its goat_ceiling_refused. Alert on an increase."
emit "# TYPE spock_ceiling_refused_rows gauge"
emit "# HELP spock_ceiling_refused_last_commit_seconds Unix epoch of the newest refused grant's commit at the site that sent it, 0 when none is known."
emit "# TYPE spock_ceiling_refused_last_commit_seconds gauge"

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

# The platform-admin guard's log (ADR 1205 D10), in one peer's kratos
# database. Access is asked first, since a statement naming a table
# fails as it is parsed when the table is missing. 'stamp' means the user may
# also run goat_ceiling_refused_stamp(), which writes committed_at; 'read'
# means it may only count.
CEILING_DB=kratos
CEILING_ACCESS_SQL="SELECT CASE
  WHEN to_regclass('public.goat_ceiling_refused') IS NULL THEN 'absent'
  WHEN NOT has_table_privilege('public.goat_ceiling_refused', 'SELECT') THEN 'denied'
  WHEN to_regprocedure('public.goat_ceiling_refused_stamp()') IS NOT NULL
   AND has_function_privilege('public.goat_ceiling_refused_stamp()', 'EXECUTE')
   AND has_table_privilege('public.goat_ceiling_refused', 'UPDATE') THEN 'stamp'
  ELSE 'read' END;"
CEILING_STAMP_SQL="SELECT public.goat_ceiling_refused_stamp();"
CEILING_SQL="SELECT count(*) || '|' || coalesce(extract(epoch FROM max(committed_at))::bigint, 0)
               FROM public.goat_ceiling_refused;"

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

# Per peer: the platform-admin grants its guard refused, when its kratos
# database is watched.
if jq -e --arg db "$CEILING_DB" 'any(.[]; .db == $db)' <<<"$SPOCK_DRIFT_TABLES" >/dev/null; then
  for peer_json in "${PEERS_LIST[@]}"; do
    peer_name=$(jq -r '.name' <<<"$peer_json")
    labels="site=\"${SPOCK_DRIFT_SITE}\",peer=\"${peer_name}\",db=\"${CEILING_DB}\""
    access=$(peer_sql "$peer_json" "$CEILING_DB" "$CEILING_ACCESS_SQL")
    if [[ "$access" == "ERR" ]]; then
      fail=$((fail + 1))
      continue
    fi
    if [[ "$access" == "absent" ]]; then
      emit "spock_ceiling_guard_installed{${labels}} 0"
      continue
    fi
    emit "spock_ceiling_guard_installed{${labels}} 1"
    if [[ "$access" == "denied" ]]; then
      echo "spock-drift-watch: ${peer_name}/${CEILING_DB}: this user cannot read goat_ceiling_refused; no count" >&2
      emit "spock_ceiling_refused_readable{${labels}} 0"
      continue
    fi
    if [[ "$access" == "stamp" ]]; then
      if [[ "$(peer_sql "$peer_json" "$CEILING_DB" "$CEILING_STAMP_SQL")" == "ERR" ]]; then
        fail=$((fail + 1))
        continue
      fi
    else
      echo "spock-drift-watch: ${peer_name}/${CEILING_DB}: this user cannot run goat_ceiling_refused_stamp(); counting without it" >&2
    fi
    out=$(peer_sql "$peer_json" "$CEILING_DB" "$CEILING_SQL")
    if [[ "$out" == "ERR" ]]; then
      fail=$((fail + 1))
      continue
    fi
    emit "spock_ceiling_refused_readable{${labels}} 1"
    emit "spock_ceiling_refused_rows{${labels}} ${out%%|*}"
    emit "spock_ceiling_refused_last_commit_seconds{${labels}} ${out##*|}"
  done
fi

emit "spock_ace_drift_watch_last_run_seconds{site=\"${SPOCK_DRIFT_SITE}\"} $(date +%s)"

install -m 0644 "$TMP_OUT" "$SPOCK_DRIFT_TEXTFILE"

if (( fail > 0 )); then
  echo "spock-drift-watch: ${fail} peer-query failure(s); partial metrics written" >&2
  exit 2
fi
exit 0
