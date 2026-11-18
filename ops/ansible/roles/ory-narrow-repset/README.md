# ory-narrow-repset

Idempotent Ansible role landing the narrow Ory Spock repset per
[ADR 1010 D8](../../../../docs/adr/1010-ory-on-spock-mindful-operation.md).

## What it does

For each Ory database (`kratos`, `hydra`) on the local site's
`postgres-a` container:

1. **Installs the Spock extension** (`CREATE EXTENSION IF NOT EXISTS spock`)
   — the production init script
   ([`ops/postgres/spock/init/02-spock-extension.sql`](../../../../ops/postgres/spock/init/02-spock-extension.sql))
   explicitly does NOT do this; the inline comment gates the change on
   "when intra-site Spock for kratos+hydra is validated." Pattern C
   IS that validation.
2. **Creates the per-DB Spock node** if missing
   (`spock.node_create`, idempotent via catalog `EXISTS` check).
3. **Creates the `ory_narrow` repset** if missing
   (`spock.replication_set_create`).
4. **Adds the canonical 10 inclusion-list tables** to the repset
   (Kratos `identities` + `identity_credentials` +
   `identity_credential_identifiers` + `identity_credential_types` +
   `identity_recovery_addresses` + `identity_verifiable_addresses` +
   `networks`; Hydra `hydra_client` + `hydra_jwk` + `networks`),
   skipping any that are already members.
5. **Asserts no exclusion-list table is a member** of the repset
   (RAISE EXCEPTION if violated — manual recovery required, see
   schema audit).
6. **Wires cross-site subscriptions** for `ory_narrow` to each peer
   site (mirrors `spock-mesh-peering` shape; full-mesh discovery via
   `groups['mgmt_hosts']` or explicit list via
   `inter_site_spock_peers`).

## When it runs

The role short-circuits unless **all three** are true:

- `ory_spock_enabled: true` (per-site opt-in; default OFF)
- `participates_in_spock: true` (site participates in Spock mesh)
- `inter_site_spock_topology` ∈ `{full_mesh, explicit}` (not `none`)

This is the Sub-block D pilot activation gate. Until an operator
flips `ory_spock_enabled` on at a site, the role does nothing.

## Pre-flight

Before any SQL runs, the role:

- Resolves `<SITE_UPPER>_REPLICATOR_PASS` from env (refuses to start
  if empty) — same convention as `spock-mesh-peering`.
- Waits up to `ory_spock_preflight_wait_seconds` (default 60) for both
  `kratos-a` and `hydra-a` containers to have Docker state `running` and health
  exactly `healthy`. Every attempt inspects both containers; a missing
  healthcheck, an `exited`, `paused`, `restarting`, or other non-running state,
  or any inspection error fails the
  attempt and is reported with the container name and observed state. This
  ensures the Ory-side migrations have populated the tables before we try to
  add them to a repset.

Before the role runs, the playbook runs
[`spock-forward-origins-check`](../spock-forward-origins-check/README.md):
a subscription with forward_origins set in a mesh replicates in a loop, so a
non-empty `ory_spock_forward_origins` or any existing subscription with
forwarding set is refused (GB-351).

## Idempotency

Every SQL operation is wrapped in a catalog `EXISTS` check (or
`to_regclass()` null-check for the table presence). Re-running the
role on a site already at the desired state is a no-op. The change
detection in Ansible reports `changed=false` even on successful
applies, because Spock's catalog-level dedup is the source of truth.

## Composition

- **Sibling to `spock-mesh-peering`** (which handles `netbird` DB +
  `default` repset). Both can run on the same site; their
  subscriptions and repsets are independent.
- **Independent from `postgres-streaming-replication`** (intra-site
  A↔B WAL streaming, unrelated to inter-site Spock).
- **Composes with [ADR 1009](../../../../docs/adr/1009-ory-shared-network-nid-preseed.md)**
  — the `networks` row pre-seed makes the inclusion-list `networks`
  table replication semantically meaningful (without the shared
  nid, the row would replicate but each site's Kratos / Hydra
  would filter by its own different nid).

## Variables (high-level)

See [`defaults/main.yml`](defaults/main.yml) for the full set.
Operators typically only need to set:

- `ory_spock_enabled: true` (per-site opt-in)

Everything else inherits from site-wide defaults
(`inter_site_spock_topology`, replicator user/password, Postgres
container name, etc.).

## Tests

- [`tests/syntax-check.sh`](tests/syntax-check.sh) — CI-invoked wrapper for
  the parse-only smoke and focused container-health behavior suite.
- [`tests/playbook.yml`](tests/playbook.yml) — parse-only fixture
  used by syntax-check.
- [`tests/test-container-health.py`](tests/test-container-health.py) — runs
  the production role task on localhost with a stand-in Docker CLI, covering
  accepted, refused, diagnostic, retry, and exhaustion states without
  contacting a Docker daemon.
- **Full 2-node docker-compose integration test:** queued as a
  follow-on. The fixture lives at
  [`tests/run-test.sh`](tests/run-test.sh) (stub today; will mirror
  the shape of `ops/ansible/roles/spock-mesh-peering/tests/run-test.sh`).

## References

- Pattern C plan
- Schema audit — per-table classification matrix
- [ADR 1010 D8](../../../../docs/adr/1010-ory-on-spock-mindful-operation.md)
- Identity architecture §13.5.3
- Implementation tracking: Block 21G in `docs/project/implementation-plan.md`
