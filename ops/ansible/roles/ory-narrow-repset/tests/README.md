# ory-narrow-repset role tests

Tests for the `ory-narrow-repset` Ansible role, per the Block 50H
role-test-parity discipline.

## Coverage levels

### CI wrapper — `syntax-check.sh`

The wrapper runs both the parse-only fixture and the focused readiness
fixture. CI invokes every role's `tests/syntax-check.sh`, so both layers are
gated by [`.github/workflows/ansible-syntax-check.yml`](../../../../../.github/workflows/ansible-syntax-check.yml).

The parse-only layer runs `ansible-playbook --syntax-check` over
`playbook.yml` (a fixture that includes the role with
`ory_spock_enabled: false` so the role short-circuits without trying to talk
to Postgres). It catches:

- YAML syntax errors in `tasks/main.yml`, `tasks/per_db.yml`,
  `tasks/per_subscription.yml`, `defaults/main.yml`.
- Jinja2 template syntax errors.
- Missing required role files.

### Container readiness — `test-container-health.py`

This layer includes the production role with an empty database list, so the
real `wait for kratos + hydra containers healthy` task runs but no SQL does.
The `fixtures/docker` stand-in accepts only the exact original or corrected
Docker inspection template, returns deterministic inspection states, and
records every call and format. The harness independently asserts the recorded
container sequence and format. The suite requires both configured containers
to be inspected on every attempt and covers:

- both containers running and healthy;
- unhealthy, starting, stopped-with-stale-health, no-healthcheck, paused, and
  restarting states. The paused and restarting fixtures deliberately combine
  those states with a synthetic `healthy` field to isolate the state check;
  they do not claim Docker retains healthy status in those states;
- realistic missing-container and daemon-unavailable inspection errors,
  including Docker's exit status and stderr;
- a fixture explicitly labelled synthetic that uses arbitrary exit status 125
  to prove a nonstandard status is preserved;
- either container failing, and both failing with both diagnostics retained;
- unhealthy-to-healthy retry and persistent-failure exhaustion.

These are production-task measurements against a stand-in Docker CLI, not
measurements against a Docker daemon. The harness refuses to count an
unreachable host or a run that never reached
the named production task as a behavioral pass. Expected Ansible task
failures are successful negative cases; command, dependency, or fixture
failures are not. Its summary reports scenario and assertion counts separately.

### Full integration test (queued) — `run-test.sh`

Mirrors [`ops/ansible/roles/spock-mesh-peering/tests/run-test.sh`](../../spock-mesh-peering/tests/run-test.sh)
shape. Brings up a 2-node docker-compose harness with:

- Two `postgres-spock` containers
- Two `kratos` containers (with `kratos migrate` pre-run)
- Two `hydra` containers (with `hydra migrate` pre-run)

Then runs the role on both nodes and verifies:

- Spock extension installed in both `kratos` and `hydra` DBs.
- `ory_narrow` repset created in both DBs at both nodes.
- All 10 inclusion-list tables are repset members.
- Zero exclusion-list tables are repset members (the role's
  `RAISE EXCEPTION` invariant holds).
- New identity row at site A replicates to site B.
- New Kratos flow at site A does NOT appear at site B.
- New Hydra auth code at site A is NOT consumable at site B.
- Re-running the role is idempotent (no changes on second run).

**Status:** stub. The integration harness is queued as a follow-on
to land before the operator-fired Sub-block D pilot activation.

## Notes

- The role uses `docker exec` against `postgres-a` for SQL. Local
  test fixtures need docker-compose service names matching
  `ory_spock_postgres_container` (default `postgres-a`).
- The `_REPLICATOR_PASS` env var is required pre-flight; the
  integration test fixture sets it to a test value before invoking
  the role.
