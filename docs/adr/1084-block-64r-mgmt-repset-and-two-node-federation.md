# ADR 1084 — Block 64R: mgmt-layer repset membership + two-node goatnet federation (sandbox first)

| Field | Value |
|---|---|
| Status | Accepted 2026-07-15. D3 amended by [ADR 1165](1165-island-mode-multi-writer-every-site-admits.md) (Accepted): single-homing of admin surfaces is lifted one write class at a time, as each class passes the partition harness; the intended placement is every site. D6 added for the tables NetBird 0.83.x creates. |
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

### D6 — Tables added by NetBird 0.83.x: all twelve stay out

NetBird 0.83.2 creates twelve tables 0.71.4 did not. None joins `netbird_mgmt`; the
INCLUDE list stays at 20.

| Table | Holds | Decision |
|---|---|---|
| `management_instances` | One row per running management process: a random id, the hostname, a heartbeat every minute stamped with the database clock | Exclude: instance-local runtime state, by the `bootstrap_idempotency` precedent |
| `peer_connections` | One row per peer: the management instance holding its sync stream, a session fencing token, last seen. Replaces `peers.peer_status_connected/last_seen/session_started_at`, which 0.83.0 stops writing (upstream #8426) | Exclude, together with `management_instances` (below) |
| `agent_network_providers`, `agent_network_policies`, `agent_network_guardrails`, `agent_network_settings`, `agent_network_budget_rules` | Agent Network configuration, keyed by string ids or account id | Exclude while the feature is unused (0 rows), by the reverseproxy precedent. A goatnet that turns Agent Network on adds these five to the list first: they are account configuration, like `policies` |
| `agent_network_consumption`, `agent_network_access_log`, `agent_network_access_log_group`, `agent_network_request_usage`, `agent_network_request_usage_group` | Per-request logs and usage, and budget consumption counters per time window | Exclude: append-heavy logs and counters, per site by the `access_log_entries` precedent |

**Why `peer_connections` and `management_instances` move together, and stay out.**
`peer_connections.instance_id` names a `management_instances` row, and every management
instance sweeps connections whose instance row is missing or has not heartbeated for
five minutes (upstream `ListStaleInstances`, `SweepInstanceChunk`). Replicating
connections without instances makes each site release the peers another site holds, and
the release replicates back and disconnects them at the site that holds them. Replicating
both compares one site's heartbeat with another site's database clock, and during a
partition each site releases the other's peers after five minutes and deletes its
instance row; on heal those writes cross, the other site finds its row gone and
re-registers under a new id, and the session-fenced upserts are applied last-writer-wins
by Spock regardless of their fence. Upstream built these tables for instances sharing one
database.

**Consequences.**

- Liveness is per site. A site's API, dashboard and SQL readers show a peer as connected
  only while that site's management holds the peer's sync stream; the goatnet-wide view
  is the union across sites. goatcore's SQL readers (`mgmt-host-bootstrap list-peers`,
  `peer-dormancy-report.sh`, `rosenpass-remediate.sh`, `rosenpass-compliance-report.py`,
  `relay-peer-names.py`) say so. Before 0.83.0 liveness rode the replicated `peers` row,
  last-writer-wins.
- A peer created at one site has its connection row there only; another site creates
  its own on the peer's first connect (an upsert). A peer deleted at one site leaves its
  connection row at the others; every reader joins from `peers`, so it is never shown.
- Peer inactivity expiration (`settings_peer_inactivity_expiration_enabled`) would let a
  site expire a peer held by another site. The account bootstrap sets it false; it stays
  false on any multi-site goatnet.
- Schema parity. A database upgraded to 0.83.2 keeps `peers.peer_status_connected`,
  `peer_status_last_seen` and `peer_status_session_started_at` (no longer written) and
  `setup_keys.user_id` (from the dropped `setupkey-user-id-stamp` patch); a database
  created at 0.83.2 has none of them. The joiner's data-only seed
  (`site-seed-from-peer.yml`) and Spock apply both need the joiner to carry every column
  the founder replicates, so a joiner of an upgraded goatnet adds those four columns
  before its seed.

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
  deliberate membership decision (CI drift check compares live schema to the list). The
  decisions for 0.83.x are D6.
