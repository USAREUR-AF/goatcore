# `spock-mesh-peering/tests/` — 2-node compose smoke harness

Two parallel test paths, both runnable from this directory:

| Script | What it does | Cost | When to run |
|---|---|---|---|
| `syntax-check.sh` | `ansible-playbook --syntax-check` on the test playbook AND the real `site-bootstrap.yml`. Optional `ansible-lint` on the role. | seconds | every change to the role or site-bootstrap.yml |
| `run-test.sh` | Brings up two `postgres-spock` containers, runs the role against a synthetic 2-site inventory, verifies bidirectional `replicating` + idempotent re-run. | ~60–90 s + image pull | before merging substrate changes; before next real-site cutover |

## What `run-test.sh` validates

1. **Initial wiring (cold path).** Both sites start with no Spock nodes
   and no subscriptions. The role's discovery + node_create + sub_create
   path must produce two replicating subscriptions.
2. **Naming convention.** The actual subscription names landed in the
   catalogs are checked: `sub_test_a_to_test_b` (on test-b, pulls from
   test-a) and `sub_test_b_to_test_a` (on test-a, pulls from test-b).
3. **End-to-end data path.** A row inserted on test-a's
   `spock_mesh_smoke` table appears on test-b within 10 s, proving the
   subscription is genuinely replicating, not just status-reporting.
4. **Idempotent re-run.** A second invocation of the playbook is a
   silent no-op for catalog mutations. (One exception:
   `ALTER ROLE replicator WITH PASSWORD` always reports `changed=true`
   even when the password is unchanged — accepted noise.)

## What `run-test.sh` does NOT validate

- N>2 site shapes (3-site star, 4-site full mesh). The two-node
  fixture exercises the load-bearing wiring code path; cross-product
  scaling is the domain of the real-site cutover.
- A/B-pair sites. The fixture is single-Orin × 2 (`intra_site_pair=false`
  on both). The role handles A/B sites via the `router_role==A` filter
  in inventory discovery; that path is not exercised by this harness.
- Cross-region wg-cp0 — local docker network is not WireGuard.
  Replication latency under WAN conditions belongs to soak (Block 52).
- Failure injection (sub_create against unreachable peer; replicator
  password mismatch; two sites mid-`sub_create` race). Belongs to
  Block 38 chaos.

## Pre-requisites

- Docker (compose v2)
- `ansible-playbook` ≥ 2.13
- The postgres-spock image pullable: `run-test.sh` takes it from
  `POSTGRES_SPOCK_IMAGE`, else from the `postgres-spock` row of
  `ops/pins/fleet-pins.yaml`

## Operator's runbook

```bash
# from ops/ansible/roles/spock-mesh-peering/tests/

./syntax-check.sh                    # cheap parse-only smoke
./run-test.sh                        # 60-90 s end-to-end
KEEP=1 ./run-test.sh                 # leave containers running for debug

# Inspect state interactively while KEEP=1:
docker exec -u postgres -it test-postgres-a psql -U postgres -d netbird
# \dx     → spock extension version
# select * from spock.node;
# select * from spock.subscription;
# select * from spock.sub_show_status();

docker compose -p spock-mesh-test down -v
```

## Output

`run-test.sh` exits 0 on PASS, prints failed verification step on
non-zero exit. No JSONL evidence file today; if the role lands first
real production deployment we'll wire structured evidence into the
operator-fired drill (paralleling the Block 68 shot-clock pattern at
`docs/operations/rapid-bringup-drill-runbook.md`).
