#!/usr/bin/env bash
# 2-node compose smoke for spock-mesh-peering.
#
# Brings up two postgres-spock containers, runs the role against a
# synthetic 2-site inventory, and verifies bidirectional `replicating` +
# re-run idempotency. Tear down on success; leave running on failure for
# debug.
#
# Run from the role's tests/ directory:
#   ./run-test.sh
#
# Pre-reqs: docker (compose v2), ansible, postgres-spock image pullable.

set -euo pipefail

cd "$(dirname "$0")"

PROJECT="spock-mesh-test"
TEST_PASSWORD="spockmeshtest"

dc() { docker compose -p "$PROJECT" "$@"; }

cleanup() {
  if [[ "${KEEP:-0}" == "1" ]]; then
    echo "[run-test] KEEP=1 — leaving stack running."
    return
  fi
  echo "[run-test] tearing down..."
  dc down -v --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

# The image under test. The compose file carries no reference of its own:
# POSTGRES_SPOCK_IMAGE names it, else it is the `postgres-spock` row of the
# fleet pin table, with `ghcr_org` resolved from group_vars/all.yml.
if [[ -z "${POSTGRES_SPOCK_IMAGE:-}" ]]; then
  pins="../../../../pins/fleet-pins.yaml"
  if [[ ! -f "$pins" ]]; then
    echo "[run-test] FAIL: POSTGRES_SPOCK_IMAGE is unset and $pins does not exist." >&2
    echo "[run-test]   export POSTGRES_SPOCK_IMAGE=<registry>/postgres-spock@sha256:<digest>" >&2
    exit 1
  fi
  POSTGRES_SPOCK_IMAGE="$(
    ansible localhost -i localhost, -c local -m ansible.builtin.debug \
      -e @../../../group_vars/all.yml -e "@$pins" \
      -a 'msg={{ components["postgres-spock"].fleet }}' 2>/dev/null \
      | sed -n 's/.*"msg": "\([^"]*\)".*/\1/p' || true
  )"
  if [[ -z "$POSTGRES_SPOCK_IMAGE" ]]; then
    echo "[run-test] FAIL: could not read the postgres-spock pin from $pins." >&2
    exit 1
  fi
fi
export POSTGRES_SPOCK_IMAGE
echo "[run-test] image under test: $POSTGRES_SPOCK_IMAGE"

echo "[run-test] phase 1 — bring up 2-node postgres-spock fixture"
dc up -d --wait

echo "[run-test] phase 1b — wait for postgres steady state on both"
# The image entrypoint runs initdb + a TEMP server (init scripts), then
# restarts into the final server. --wait can return during the temp-server
# window; a psql that lands on the restart gets "the database system is
# shutting down". Require 3 consecutive OK probes 2s apart per container.
for cn in test-postgres-a test-postgres-b; do
  ok=0
  for i in $(seq 1 60); do
    if docker exec -u postgres "$cn" psql -U postgres -d netbird -tAc "SELECT 1" >/dev/null 2>&1; then
      ok=$((ok+1)); [[ $ok -ge 3 ]] && break
    else
      ok=0
    fi
    sleep 2
  done
  if [[ $ok -lt 3 ]]; then
    echo "[run-test] FAIL — $cn never reached steady state" >&2
    exit 1
  fi
  echo "[run-test]   $cn steady"
done

echo "[run-test] phase 2 — pre-seed: install spock extension in netbird DB on both"
# postgres-spock image's init scripts handle this for fresh data dirs;
# this idempotent re-create is belt-and-suspenders for anyone who attaches
# a recycled volume.
for cn in test-postgres-a test-postgres-b; do
  docker exec -u postgres "$cn" psql -U postgres -d netbird -v ON_ERROR_STOP=1 \
    -c "CREATE EXTENSION IF NOT EXISTS spock;" >/dev/null
done

echo "[run-test] phase 2b — pre-create the curated-repset table (64R path) on both"
# The test group_vars put spock_repset_smoke on the test_mgmt named repset
# (ADR 1084 curated-membership path). The table must exist before the role
# runs repset_add_table. DDL is not replicated — create on both sides.
for cn in test-postgres-a test-postgres-b; do
  docker exec -u postgres "$cn" psql -U postgres -d netbird -v ON_ERROR_STOP=1 -c "
    CREATE TABLE IF NOT EXISTS public.spock_repset_smoke (
      id   text PRIMARY KEY,
      site text NOT NULL,
      note text
    );" >/dev/null
done

echo "[run-test] phase 3 — export cluster-shared replicator password env"
# All sites share the same replicator pass. The role looks up
# <SITE_UPPER>_REPLICATOR_PASS per site; export the same value under both
# names so either site's lookup succeeds.
export TEST_A_REPLICATOR_PASS="$TEST_PASSWORD"
export TEST_B_REPLICATOR_PASS="$TEST_PASSWORD"

echo "[run-test] phase 4 — run the role via test playbook (initial mesh wiring)"
ansible-playbook \
  -i inventory.ini \
  playbook.yml \
  -v

echo "[run-test] phase 5 — verify both subscriptions report 'replicating'"
status_a_pull_b=$(docker exec -u postgres test-postgres-a psql -U postgres -d netbird -tA \
  -c "SELECT status FROM spock.sub_show_status() WHERE subscription_name='sub_test_b_to_test_a';" \
  | tr -d '[:space:]')
status_b_pull_a=$(docker exec -u postgres test-postgres-b psql -U postgres -d netbird -tA \
  -c "SELECT status FROM spock.sub_show_status() WHERE subscription_name='sub_test_a_to_test_b';" \
  | tr -d '[:space:]')

echo "[run-test] sub_test_b_to_test_a (on test-a): $status_a_pull_b"
echo "[run-test] sub_test_a_to_test_b (on test-b): $status_b_pull_a"

if [[ "$status_a_pull_b" != "replicating" || "$status_b_pull_a" != "replicating" ]]; then
  echo "[run-test] FAIL — subscriptions did not reach replicating" >&2
  exit 1
fi

echo "[run-test] phase 6 — write smoke: insert on a, expect to see on b"
docker exec -u postgres test-postgres-a psql -U postgres -d netbird -v ON_ERROR_STOP=1 -c "
  CREATE TABLE IF NOT EXISTS public.spock_mesh_smoke (
    id   bigserial PRIMARY KEY,
    site text NOT NULL,
    note text,
    ts   timestamptz NOT NULL DEFAULT now()
  );
  SELECT spock.repset_add_table('default', 'public.spock_mesh_smoke');
" >/dev/null 2>&1 || true
# Mirror the table on B so the row can replicate (DDL is not auto-replicated)
docker exec -u postgres test-postgres-b psql -U postgres -d netbird -v ON_ERROR_STOP=1 -c "
  CREATE TABLE IF NOT EXISTS public.spock_mesh_smoke (
    id   bigserial PRIMARY KEY,
    site text NOT NULL,
    note text,
    ts   timestamptz NOT NULL DEFAULT now()
  );
" >/dev/null

TOKEN="smoke-$(date +%s)"
docker exec -u postgres test-postgres-a psql -U postgres -d netbird -v ON_ERROR_STOP=1 \
  -c "INSERT INTO spock_mesh_smoke (site, note) VALUES ('test-a', '$TOKEN');" >/dev/null
for i in $(seq 1 10); do
  count=$(docker exec -u postgres test-postgres-b psql -U postgres -d netbird -tA \
    -c "SELECT COUNT(*) FROM spock_mesh_smoke WHERE note='$TOKEN';" \
    | tr -d '[:space:]')
  if [[ "$count" == "1" ]]; then
    echo "[run-test] write replicated to b in ${i}s"
    break
  fi
  sleep 1
  if [[ $i -eq 10 ]]; then
    echo "[run-test] FAIL — write did not replicate" >&2
    exit 1
  fi
done

echo "[run-test] phase 6b — 64R repset smoke: insert on a via test_mgmt repset, expect on b"
TOKEN2="repset-$(date +%s)"
docker exec -u postgres test-postgres-a psql -U postgres -d netbird -v ON_ERROR_STOP=1 \
  -c "INSERT INTO spock_repset_smoke (id, site, note) VALUES ('$TOKEN2', 'test-a', 'curated');" >/dev/null
for i in $(seq 1 10); do
  count=$(docker exec -u postgres test-postgres-b psql -U postgres -d netbird -tA \
    -c "SELECT COUNT(*) FROM spock_repset_smoke WHERE id='$TOKEN2';" \
    | tr -d '[:space:]')
  if [[ "$count" == "1" ]]; then
    echo "[run-test] 64R curated-repset write replicated to b in ${i}s"
    break
  fi
  sleep 1
  if [[ $i -eq 10 ]]; then
    echo "[run-test] FAIL — curated-repset write did not replicate" >&2
    exit 1
  fi
done

echo "[run-test] phase 7 — re-run role; expect zero-change idempotency"
ansible-playbook \
  -i inventory.ini \
  playbook.yml \
  | tee /tmp/spock-mesh-rerun.txt

# Heuristic idempotency check: changed=0 across all plays on the rerun.
# changed=2 on first run (sub_create + node_create) but should be 0 on
# the rerun — every catalog mutation is guarded by EXISTS.
if grep -E 'changed=[1-9][0-9]*' /tmp/spock-mesh-rerun.txt | grep -v 'changed=0' >/dev/null; then
  echo "[run-test] WARN — re-run reports changed!=0 lines (review above)"
  # Don't fail hard — replicator-role ALTER PASSWORD always reports
  # changed=true even when password is unchanged. The catalog mutations
  # are the load-bearing ones; smoke validates by status above.
fi

echo "[run-test] PASS"
