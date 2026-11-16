#!/usr/bin/env bash
# spock-forward-origins.sh — read sub_forward_origins of every Spock
# subscription in every database of one Postgres container.
#
# A subscription with forward_origins set in a mesh replicates in a loop: each
# change goes on to the next site and comes back, without end, while every
# subscription still reads `replicating` and nothing is logged. The
# spock-forward-origins-check role refuses such a subscription at install
# time (GB-351), and the alert SpockSubscriptionForwardOriginsSet fires on the
# metric this script writes.
#
# usage:
#   spock-forward-origins.sh list <container>
#       one line per subscription, tab-separated:
#       <database> <subscription> <0|1> <forward_origins, comma-joined>
#       (1 = forward_origins set: anything but empty or 'none')
#   spock-forward-origins.sh prom <container> <file>
#       write the same as Prometheus text to <file> for node-exporter's
#       textfile collector, atomically (temp file in the same directory,
#       then mv), as spock_subscription_forward_origins{datname,subscription_name}.
#
# Reads through `docker exec -u postgres <container> psql` over the local
# socket: no password is passed or printed. A database without the spock
# schema is skipped. Exits non-zero, leaving <file> as it was, when Postgres
# cannot be read.
#
# Runs on bash 3.2.

set -euo pipefail

usage() {
  echo "usage: $0 list <container> | $0 prom <container> <file>" >&2
  exit 2
}

[ "$#" -ge 2 ] || usage
mode="$1"
container="$2"
case "$mode" in
  list) [ "$#" -eq 2 ] || usage ;;
  prom) [ "$#" -eq 3 ] || usage; out="$3" ;;
  *) usage ;;
esac

TAB="$(printf '\t')"

psql_() { # psql_ <db> <sql>
  docker exec -u postgres "$container" \
    psql -X -U postgres -d "$1" -v ON_ERROR_STOP=1 -tAq -F "$TAB" -c "$2"
}

read_all() {
  local dbs db has
  dbs="$(psql_ postgres "SELECT datname FROM pg_database WHERE datallowconn AND NOT datistemplate ORDER BY datname;")" || return 1
  while IFS= read -r db; do
    [ -n "$db" ] || continue
    has="$(psql_ "$db" "SELECT to_regclass('spock.subscription') IS NOT NULL;")" || return 1
    [ "$has" = t ] || continue
    # The forward_origins column goes last: tab is a whitespace IFS
    # character, so `read` would fold an empty middle field into the next one.
    psql_ "$db" "SELECT current_database(), sub_name,
        CASE WHEN COALESCE(cardinality(array_remove(array_remove(sub_forward_origins, 'none'), '')), 0) > 0
             THEN 1 ELSE 0 END,
        COALESCE(array_to_string(sub_forward_origins, ','), '')
      FROM spock.subscription ORDER BY sub_name;" || return 1
  done <<EOF
$dbs
EOF
}

rows="$(read_all)" || { echo "$0: cannot read the subscriptions in $container" >&2; exit 1; }

if [ "$mode" = list ]; then
  [ -z "$rows" ] || printf '%s\n' "$rows"
  exit 0
fi

esc() { # Prometheus label value: escape backslash and double quote
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

dir="$(dirname "$out")"
tmp="$(mktemp "$dir/.spock_forward_origins.XXXXXX")"
trap 'rm -f "$tmp"' EXIT
{
  echo "# HELP spock_subscription_forward_origins 1 if the Spock subscription has forward_origins set (anything but empty or none; in a mesh it replicates in a loop), else 0."
  echo "# TYPE spock_subscription_forward_origins gauge"
  if [ -n "$rows" ]; then
    while IFS="$TAB" read -r db sub forwarding _values; do
      printf 'spock_subscription_forward_origins{datname="%s",subscription_name="%s"} %s\n' \
        "$(esc "$db")" "$(esc "$sub")" "$forwarding"
    done <<EOF
$rows
EOF
  fi
} > "$tmp"
chmod 0644 "$tmp"
mv -f "$tmp" "$out"
trap - EXIT
