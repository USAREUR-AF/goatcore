#!/usr/bin/env bash
# ops/scripts/herd-audit-forward.sh — put herd's operator-action rows on the bus.
#
# Herd records each gear session and each denial in the OperatorActionV1 field
# shape but cannot publish them: the wire payload is age(protobuf(...)) and
# goat-cli holds the only implementation of that encoding (ADR 1131,
# docs/design/herd-adoption.md §4b). This script is the forwarder between them.
#
# RUNS ON THE HOST, NOT IN THE CONTAINER, and that is the load-bearing choice.
# `goat audit emit` needs an age identity from the operator roster. Putting
# roster key material inside the herd container — a browser-facing process with
# a shell broker attached — would trade a missing audit record for a much worse
# problem. The host already holds operator keys for exactly this work.
#
# A ROW IS MARKED FORWARDED ONLY ON EXIT 0. `goat audit emit` uses the fallible
# emit path precisely so that exit 0 means "it reached the bus". Marking on
# anything weaker would lose records while reporting success — the failure this
# whole path exists to prevent.
set -euo pipefail

ENV_NAME="${GOAT_ENV:?set GOAT_ENV to the name of this goatnet}"
PG_CONTAINER="${HERD_PG_CONTAINER:-herd-postgres}"
PG_USER="${HERD_PG_USER:-herd}"
PG_DB="${HERD_PG_DB:-herd}"
# Bounded so a backlog cannot wedge the timer or flood the bus in one tick.
BATCH="${HERD_AUDIT_BATCH:-200}"
GOAT="${GOAT_BIN:-goat}"
CONTEXT="${GOAT_CONTEXT:-${ENV_NAME}-op}"

psql() { docker exec -i "$PG_CONTAINER" psql -qtAX -U "$PG_USER" -d "$PG_DB" "$@"; }

# Oldest first: an audit trail read in order is worth more than a fast one, and
# a partial batch should leave the OLDEST unforwarded rows behind, not a random
# scattering.
rows=$(psql -c "
  SELECT id, COALESCE(actor,''), COALESCE(action,''), COALESCE(target,''),
         COALESCE(outcome,''), COALESCE(\"errorMsg\",''), COALESCE(\"argsDigest\",'')
  FROM \"AuditLog\"
  WHERE \"forwardedAt\" IS NULL AND actor IS NOT NULL AND actor <> ''
  ORDER BY \"createdAt\" ASC
  LIMIT ${BATCH};" | sed 's/\r$//')

[ -z "$rows" ] && { echo "herd-audit-forward: nothing to forward"; exit 0; }

sent=0; failed=0
while IFS='|' read -r id actor action target outcome errmsg digest; do
  [ -z "$id" ] && continue

  args=(audit emit --context "$CONTEXT"
        --actor "$actor" --surface herd --action "$action"
        --target "$target" --env "$ENV_NAME" --outcome "$outcome")
  [ -n "$errmsg" ] && args+=(--error "$errmsg")
  [ -n "$digest" ] && args+=(--args-digest "$digest")

  if "$GOAT" "${args[@]}" >/dev/null 2>&1; then
    # Marked ONLY here. A row that failed to emit stays unforwarded and is
    # retried on the next tick — at-least-once, because a duplicated audit
    # record is recoverable and a dropped one is not.
    psql -c "UPDATE \"AuditLog\" SET \"forwardedAt\" = now() WHERE id = '${id}';" >/dev/null
    sent=$((sent + 1))
  else
    failed=$((failed + 1))
    echo "herd-audit-forward: emit FAILED for row ${id} (actor=${actor} action=${action}) — leaving unforwarded" >&2
  fi
done <<< "$rows"

echo "herd-audit-forward: forwarded ${sent}, failed ${failed}"

# A persistent failure is not a nuisance, it is an audit gap. Exit non-zero so
# the systemd unit records a failure and someone can see it, rather than the
# timer looking healthy while nothing reaches the trail.
[ "$failed" -eq 0 ]
