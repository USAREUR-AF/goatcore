# ADR 1084 — Block 64R: mgmt-layer repset membership + two-node goatnet federation (sandbox first)

| Field | Value |
|---|---|
| Status | Accepted 2026-07-15. D3 amended by [ADR 1165](1165-island-mode-multi-writer-every-site-admits.md) (Accepted): single-homing of admin surfaces is lifted one write class at a time, as each class passes the partition harness; the intended placement is every site. |
| Deciders | maintainers |
| Block | 64R (closes the backlog gate opened by 64Q) |
| Supersedes / amends | Amends ha-multi-site.md §5.6/§5.9; consumes ADR 0801 (Spock), ADR 1009/1010 (Ory on Spock), F-004 |

## Context

The inter-site Spock substrate (Block 64Q, `spock-mesh-peering`) wires subscriptions
idempotently, and the production mesh has run those edges since 2026-05 — but the
default repset on every live edge carries only `spock_test`. The membership decision for
the netbird mgmt-layer tables was deferred as Block 64R, gated on F-004 (no-PK rejection),
concurrent-write semantics, and ADR 0833 A→B promotion. Meanwhile every component leg was
individually validated: signal federation forward (CGNAT Phase 3 gate PASS 2026-04-27,
M-008), Kratos-on-Spock (a dev-environment repro, zero slot transitions),
the netbird schema audit (GREEN), and the `skip-migrations` patched mgmt image.

The forcing case: a goatnet whose single central host has large compute headroom but
sits behind one site uplink that carries all ingress (relay fan-in, signal, fabric,
every web surface). A second host at a different site with an independent uplink
relieves that only if both hosts serve identity + mesh state — i.e. netbird and Ory
databases must be active-active.

A fresh F-004 audit against the **live** schema (30 tables; the realized schema has
grown since the 2026-04 audit) found: all mgmt-state tables have PKs; the only
no-PK tables (`extra_settings`, `network_addresses`) are empty; the only SERIALs sit in
tables excluded anyway (`installations.id`, reverseproxy `targets.id`).

## Decision

### D1 — Curated `netbird_mgmt` repset (20 tables in, 10 out)

A named repset `netbird_mgmt` in the `netbird` database (never `repset_add_all_tables`,
per F-004). Membership is an explicit, versioned list:

**INCLUDE (20):** `accounts`, `account_onboardings`, `group_peers`, `groups`, `jobs`,
`name_server_groups`, `network_resources`, `network_routers`, `networks`, `peers`,
`personal_access_tokens`, `policies`, `policy_rules`, `posture_checks`, `records`,
`routes`, `setup_keys`, `user_invites`, `users`, `zones`.

`jobs` is deliberately IN: a job row is created by the admin node but delivered by
whichever mgmt instance holds the target peer's sync stream — replication is required
for cross-node delivery, and status writes are naturally single-writer-per-row (only
the stream-owning node updates a given job).

**EXCLUDE (10):**

| Table | Reason |
|---|---|
| `extra_settings`, `network_addresses` | No PK — F-004 hard exclude (both empty live) |
| `installations` | Single-row, SERIAL PK |
| `services`, `targets`, `domains`, `proxies`, `proxy_access_tokens` | Reverseproxy feature unused (0 rows); `targets.id` SERIAL |
| `access_log_entries` | Append-heavy access log; per-site by the `netbird_events` precedent |
| `bootstrap_idempotency` | Instance-local request-idempotency markers; has a PK, so it can join later if cross-node retry dedup proves needed |

`netbird_events` (separate DB) stays per-site — unchanged from §5.6. `netbird_idp` is
not replicated (empty; Ory is the IdP).

### D2 — Ory rides the existing `ory_narrow` repset

`kratos` + `hydra` databases replicate via the already-built `ory-narrow-repset` role
(10 tables per the schema-audit matrix). No new Ory decision here; ADR 1009 shared-`nid`
pre-seed and ADR 1010 D1–D6 disciplines apply as written for the inter-site case.

### D3 — Write-routing: single-homed admin surfaces, any-node enrollment

Concurrent-write safety comes from topology, not middleware: all admin mutation surfaces
(portal, id-console, dashboard, `goat admin`) remain single-homed on one node — every
policy/group/key/identity mutation therefore originates on one writer. The second node
serves enrollment, peer sync, signal, and Ory login flows: enrollment writes are
row-disjoint (new peer rows, xid-keyed); login-flow tables are excluded from the repset;
`accounts.serial` concurrent increments resolve LWW with a benign lost increment
(§5.1 — peers converge on next sync). Promotion of admin surfaces to the second node is
an operator action (re-point, not re-engineer).

### D4 — Mechanism universal, adoption per-goatnet; production NOT flipped here

`spock-mesh-peering` grows multi-database + repset-membership support behind a new
`spock_mesh_databases` list (default preserves today's single-`netbird`/`default`-repset
behavior exactly). The sandbox goatnet adopts first (2-node). The production mesh keeps its
narrow repset until the remaining prod-scale gates clear (ADR 0833 A→B subscription
promotion, N-way concurrent-write review) — flipping prod is a separate, later decision.

### D5 — Drift-watch from day one

`spock-drift-watch` inclusion list widens to the `netbird_mgmt` critical tables
(`accounts`, `users`, `peers`, `groups`, `setup_keys`) alongside the existing Ory
entries, and `spock.resolutions` growth alerts (Spock issue #349 over-trigger pattern)
ship with the first federated deploy.

## Alternatives considered

- **`repset_add_all_tables`** — rejected: F-004 no-PK rejection is permanent Spock
  behavior; bulk-add also silently couples us to future schema growth.
- **Stay single-host, move the host to the better uplink** — relieves bandwidth but
  keeps the availability SPOF and abandons the validated federation substrate.
- **Replicate `netbird_events` via snowflake sequence swap** — deferred, unchanged; audit
  stays site-local.

## Consequences

- First deployment anywhere of netbird mgmt tables under Spock multi-master (no public
  prior art — §5.2 caveat stands); the sandbox goatnet is the right blast radius for
  first adoption.
- Netbird AutoMigrate must run on exactly one node per upgrade (§5.8.1); the
  `skip-migrations` image + marker-file entrypoint is now load-bearing on both nodes.
- The repset list is versioned config: a netbird upgrade that adds tables requires a
  deliberate membership decision (CI drift check compares live schema to the list).
