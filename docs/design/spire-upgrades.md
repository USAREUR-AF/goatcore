# SPIRE staged upgrades and restoration

**Status: built locally.** The role validates local version transitions, checks
persisted enrollment, and requires a healthy Workload API. The immutable stage
pairs are in [spire-upgrades](../../ops/pins/spire-upgrades/1.16.0.yml).
**Measured locally:** all five minor upgrades and a stopped-state restore passed
with PostgreSQL 16 on Linux amd64. Fleet rollout is **not built** by this change.

## Version and state contract

The [upstream upgrade guide](https://spiffe.io/docs/latest/maintenance/upgrading/)
requires one minor version per upgrade and servers before agents. Agents must
not exceed their server version and may be one minor behind. The packaged path
is `1.11.2 → 1.12.6 → 1.13.6 → 1.14.9 → 1.15.5 → 1.16.0`.
Final upstream release: [1.16.0](https://github.com/spiffe/spire/releases/tag/v1.16.0).
The fleet table declares that image pair.
Each stage file contains both versioned image digests, suitable for an Ansible
extra-vars file. Both image indexes contain Linux amd64 and arm64 manifests.

The [version preflight](../../ops/ansible/roles/spire/tasks/version-check.yml)
reads candidate versions from isolated containers at the pinned digests.
It accepts digest-only mirror references and checks advertised semantic version tags.
It reads existing versions before configuration changes or Compose deployment. It rejects skipped local minors, image-only downgrades, unreadable
existing runtimes, unsupported agent skew, and agent deployment ahead of the
CA holder. It also refuses to treat persisted keys or agent state without an
existing container as a new installation. Restore the previous runtime first.
The check covers the local pair and, for an agent-only node, its CA holder.
It does not discover omitted, offline, or external agents. The operator must
complete the inventory census before each server stage.

The role uses `agent-data.json` to decide whether enrollment already exists.
File presence prevents unnecessary one-time tokens; it does not prove a valid
SVID. Both paths separately require an agent health check. The agent-only path
requires exactly one CA holder in the play. It validates every exported PEM
certificate with host OpenSSL before replacing the pinned public bundle.
The agent-only role installs the `openssl` package.
Its admin commands use the server's default private socket, matching the server
configuration. Token generation and token-bearing renders suppress Ansible logs.

## Before an authorized rollout

1. Identify the sole active CA holder and every agent in its trust domain.
2. Record each live version, immutable image reference, architecture, and workload identity.
3. Resolve offline agents before the next minor stage.
4. Confirm the target architecture with a local rehearsal on that architecture.
5. Verify upstream artifact provenance under the deployment's supply-chain policy.
6. Rehearse restoration with the deployment's PostgreSQL or SpockHA topology.
7. Reserve a maintenance window for this single-server identity plane.
8. Store a protected, consistent snapshot before each minor stage.

The snapshot must contain the stopped server's logical PostgreSQL database,
server disk keys and state, every agent's state, configuration, and immutable
image references. Stop the agents before the server. Disable automatic restarts
and trust-sync rescue during the snapshot and recovery window. Stop all writers
before the logical dump. Protect backups as credentials. Use mode 0700 directories and mode 0600 files.
Encrypt them under the deployment's backup policy. Exclude tokens, database
passwords, keys, and complete token-attestor IDs from logs.
A database dump alone cannot restore disk-held CA keys.

## Apply one stage

Use the existing [spire-deploy.yml](../../ops/ansible/spire-deploy.yml) playbook.
The first pass reaches the CA holder and its co-located agent. The second pass
reaches all agents and includes the CA holder for delegated bundle and token
operations. A failed pass stops progression. Do not advance to the next stage
until every agent completes the current one.

Set `INVENTORY`, `TARGET`, `CA_HOLDER`, and `RUNTIME_VARS` from the deployment's
approved operator workflow. `RUNTIME_VARS` names a protected vars file; do not
place passwords in command arguments. Set `STAGE` to the next packaged version.
From `ops/ansible`, run the server pass:

```sh
../scripts/ansible-preflight.sh \
  -i "${INVENTORY:?}" spire-deploy.yml \
  -e "target=${TARGET:?}" --limit "${CA_HOLDER:?}" \
  -e "@${RUNTIME_VARS:?}" -e "@../pins/spire-upgrades/${STAGE:?}.yml"
```

1. Verify server health, the CA bundle, and registration entries against the snapshot.
2. Fetch a newly registered SVID through an older remote agent, when one exists.
3. Verify its URI, CN, DNS names, validity, and certificate chain.
4. Confirm that an unregistered workload remains denied.
5. Apply the agent pass with the same stage file:

```sh
../scripts/ansible-preflight.sh \
  -i "${INVENTORY:?}" spire-deploy.yml \
  -e "target=${TARGET:?}" \
  -e "@${RUNTIME_VARS:?}" -e "@../pins/spire-upgrades/${STAGE:?}.yml"
```

6. Verify every agent's health and a fresh workload SVID.
7. Verify reruns do not mint replacement join tokens for persisted agents.
8. Verify trust-sync and any federation handshakes before the next stage.
9. Record completion for every agent in the census.

After the final stage, regenerate the deployment's pins and intent with its
normal generator and offline-CA signature workflow. The public tables declare
no goatnets; their generator refuses an empty deployment table. This repository
therefore has no generated deployment pins or signed intents to update.

## Restore a failed stage

The [rollback ledger](../../ops/ansible/roles/spire/meta/main.yml) classifies SPIRE
as an image with automatic rollback disabled. Its database and disk state
require restoration together. `rollback.sh` must refuse an image-only
swap. Restoration is an operator procedure with its own approval.

1. Stop every agent and disable automatic restarts and rescue timers.
2. Stop the server and prevent all database writers from reconnecting.
3. Preserve the failed database and disk state for diagnosis.
4. Restore the previous stage's database into a separate, empty database.
5. Restore that snapshot's server keys and all agent state into protected, separate paths.
6. Restore the matching configuration and previous immutable image pair.
7. Keep the agents stopped until the restored server passes health checks.
8. Verify its CA bundle and registration entries against the snapshot.
9. Start the restored agents at the snapshot's version.
10. Verify fresh workload issuance and denied-workload controls.
11. Re-enable rescue and restart policies only after those checks pass.

Do not run an old binary against a newly migrated database. The local rehearsal
observed an added `ca_journals.journal_id` column on the 1.16.0 path. The restoration receipt covers a consistent 1.15.5 database and disk snapshot.
The rehearsal restored it after 1.16.0 into a new database. It retained the original upgraded state.

## Verification scope

The local Docker rehearsal invoked every SPIRE image by exact digest and retained
its runtime image ID. It rendered the source server and agent templates, used
synthetic trust, and exercised all five server-first hops. Each mixed-version
issuance used a newly registered identity, so a cached pre-upgrade SVID could
not satisfy it. CA preservation, original registration shape, persisted agent
restart, certificate verification, denied unregistered clients, and restoration
passed. The rehearsal produced 65 successful receipts.

[Enrollment regressions](../../tests/test_spire_enrollment.py) execute the actual
Ansible tasks with a recording Docker boundary. They cover first enrollment,
reruns, unhealthy agents, invalid bundles, export and token refusal, ambiguous
CA holders, unsupported transitions, and orphaned persisted state.
[Pin checks](../../tests/test_spire_pins.py) bind the final stage to fleet and
Compose defaults. These tests do not replace the real Docker rehearsal.

ARM execution and full role deployment against production SpockHA remain unmeasured.
Federation handshakes, expired-agent rescue, long CA rotation, and a complete fleet
identity census also remain unmeasured. Image-index coverage is
not ARM execution. Local health and issuance do not establish live deployment.

A Trivy 0.76.0 scan with the 2026-11-13 vulnerability database found these
Go-package vulnerability instances in the exact image digests:

| Image | 1.11.2 critical / high | 1.16.0 critical / high |
| --- | --- | --- |
| Server | 4 / 59 | 0 / 2 |
| Agent | 2 / 60 | 0 / 1 |

The scan included 245 server packages and 222 agent packages at 1.16.0, compared
with 220 and 208 at 1.11.2. It reported no OS-package targets. The remaining high
findings are `CVE-2026-78661` in `golang.org/x/net` on both images and
`CVE-2026-116587` in `github.com/go-jose/go-jose/v3` on the server. This upgrade
reduces the measured findings; it does not remove every high finding.
