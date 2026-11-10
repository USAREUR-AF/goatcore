# `spock-mesh-peering` — inter-site Spock auto-mesh

Block 64Q.2. Idempotent role that wires inter-site Spock multi-master
subscriptions across every participating site in the mgmt-host fleet.
Replaces the manual `spock.sub_create()` ceremony that scaled linearly
with new sites and quadratically with cluster size.

Companion to [`postgres-streaming-replication`](../postgres-streaming-replication/)
(intra-site A↔B WAL streaming, single-WAL stream) and
[`spock-drift-watch`](../spock-drift-watch/) (interim L2a divergence
detector). This role covers the inter-site multi-master subscription
graph; intra-site replication is a different mechanism per ADR 0801.

## What it does

For every site marked `participates_in_spock: true` in inventory, runs
once per site (against the site's primary Orin: router_role=A for A/B
pairs, or the only host for single-Orin sites). On each invocation:

1. **Pre-flight.** Resolves the cluster-shared replicator password from
   `<SITE_UPPER>_REPLICATOR_PASS` env var. Ensures the local `replicator`
   PostgreSQL role exists with the right password (idempotent — no-op
   when `postgres-streaming-replication` already created it on full-
   profile A/B sites).

2. **Discovery.** Walks `groups['mgmt_hosts']` and builds a peer registry:
   one entry per distinct `site_id` other than the local one, keyed to
   the peer's primary inventory hostname. A/B pair sites contribute their
   `router_role=A` host only (per ADR 0833 — only Orin-A participates in
   inter-site Spock).

3. **Topology filter.** Honors `inter_site_spock_topology`:
   - `none` (default) → role short-circuits via `meta: end_play`.
   - `full_mesh` → wire bidirectional subs to every discovered peer.
   - `explicit` → wire only the peers in `inter_site_spock_peers`.

4. **Local-side wiring.** Ensures the local Spock node exists. For each
   peer, ensures `sub_<peer>_to_<local>` exists on the local postgres,
   pulling from the peer's wg-cp0 publisher endpoint via the cluster-
   shared `replicator` user.

5. **Remote-side wiring.** For each peer, `delegate_to` the peer's primary
   host and ensures `sub_<local>_to_<peer>` exists on the peer's postgres,
   pulling from the local site. This makes a single site-bootstrap.yml
   run for a NEW site bring it fully into the mesh — no follow-up
   operator step against existing peers.

6. **Verify.** Polls `spock.sub_show_status()` until both directions of
   every newly-created subscription report `replicating` (default budget
   60 s, 2 s polling).

All catalog mutations are guarded by `EXISTS` checks — re-runs are
silent no-ops on healthy edges.

## Naming convention

Both conventions match the live edges that landed via the manual
ceremony, so a future operator can switch a site from manual-mode to
auto-mesh by simply flipping `inter_site_spock_topology` to `full_mesh`
without renaming any subscription.

| Site shape | Spock node name | Subscription name |
|---|---|---|
| A/B pair (vega) | `vega_a` | `sub_<provider>_to_<subscriber>` |
| Single-Orin (nova) | `nova` | `sub_<provider>_to_<subscriber>` |

## Invariants

- **Local site MUST be the role's target host.** Phase 7B in
  `site-bootstrap.yml` selects the primary Orin of the local site
  (`groups[<site>_hosts][0]`). The role uses `delegate_to` for peer-
  side work; it never multi-targets directly.
- **Cluster-shared replicator password.** Every site's
  `<SITE>_REPLICATOR_PASS` env var carries the same value. Without that
  invariant, peer-side `sub_create` would fail with
  `password authentication failed`.
- **Default `inter_site_spock_topology: none`.** The role MUST NOT touch
  existing live state on first deploy. Operator opts in
  per-site after the 64Q.4 replicator-DSN migration retrofits the
  existing edges to use `user=replicator`.
- **No data sync at sub_create time.** `synchronize_data := false` —
  this role only wires the subscription graph; table population is
  controlled by `spock.repset_add_table` (separate Block 64Q.5 / 64R
  decision).

## Inputs

Required (per-site group_vars):

| Var | Where set | Purpose |
|---|---|---|
| `participates_in_spock` | `group_vars/<site>_hosts/vars.yml` | Gate. False sites short-circuit. |
| `intra_site_pair` | same | Drives node-name suffix derivation. |
| `site_id` | same | Site identity. |
| `inter_site_spock_topology` | same | `none` (default) / `full_mesh` / `explicit`. |
| `inter_site_spock_peers` | same (when `explicit`) | List of site_ids to mesh with. |
| `signal_uri` | `inventory/mgmt-hosts.ini` per host | wg-cp0 publisher host comes from `signal_uri.split(':')[0]`. |
| `router_role` | same | `A` flags the primary side at A/B pair sites. |

Required (operator env, per site, cluster-shared value):

| Env var | Purpose |
|---|---|
| `<SITE_UPPER>_REPLICATOR_PASS` | Subscription DSN password. Cluster-shared: the same value at every site. |

Tunables (defaults in `defaults/main.yml`):

- `spock_mesh_database` (default `netbird`)
- `spock_mesh_replication_sets` (default `[default]`)
- `spock_mesh_replicating_timeout_seconds` (default 60)
- `spock_mesh_databases` — Block 64R (ADR 1084) multi-database list; default
  is one entry built from the two legacy vars above, preserving pre-64R
  behavior exactly.

## Multi-database + curated repsets (Block 64R, ADR 1084)

The whole grants → repset → node → subscription → verify pass runs once per
entry in `spock_mesh_databases` (`tasks/per-database.yml`; Spock catalogs are
per-database). An entry's optional `repset` key creates a named repset locally
and curates membership per-table — never `repset_add_all_tables`, and
always `synchronize_data := false`: adding a table with sync=true pushes the
*adder's* data at every subscriber, which on a fresh empty joiner would
truncate the established node. Initial data seed for a fresh joiner is an
explicit bootstrap step (guarded dump/restore), never a repset side effect.

ZAGROS 2-node shape (the first adopter — see `group_vars/zagros_hosts/vars.yml`):

```yaml
spock_mesh_databases:
  - name: netbird
    replication_sets: [netbird_mgmt]
    repset:
      name: netbird_mgmt
      tables: "{{ spock_mesh_netbird_mgmt_tables }}"   # ADR 1084 curated 20
```

Do NOT add kratos/hydra entries here — the `ory-narrow-repset` role (Phase
7D, `ory_spock_enabled`) owns Ory federation end-to-end, including its own
`ory_sub_*` subscriptions; a second subscription pulling `ory_narrow` from
this role would double-apply every replicated Ory row (PK conflicts).

The production mesh stays on the default (narrow `default` repset) —
prod adoption is separately gated (ADR 0833 + N-way review); do NOT set
`spock_mesh_databases` in prod group_vars as part of this block.

## Operational guidance

### Day-1 — adding a NEW site to an existing mesh

1. New site's `group_vars/<site>_hosts/vars.yml` declares
   `participates_in_spock: true` + `inter_site_spock_topology: full_mesh`.
2. Operator exports `<SITE>_REPLICATOR_PASS` (the cluster-shared value) in
   `.envrc.local`.
3. Operator runs `site-bootstrap.yml -e site_id=<new-site>`.
4. Phase 7B fires the role; mesh closure happens automatically (local
   subs + delegate_to peer subs). All edges report `replicating` before
   the play completes.

### Re-running site-bootstrap.yml on an established site

No-op for the role. Existence checks short-circuit every catalog
mutation; verify-step finds existing edges already `replicating`. Safe.

### Migrating an existing site from manual ceremony to auto-mesh

The role's `tasks/main.yml` carries the
`grant spock-schema permissions to replicator` task that gives the
replicator role enough access to call `spock.node_info()` /
`spock.sub_create()` (per the permission error class the migration
surfaced).

For a NEW site joining the existing mesh:

1. Ensure the cluster-shared `<SITE>_REPLICATOR_PASS` env var is set
   on the operator workstation.
2. Set `inter_site_spock_topology: full_mesh` in the new site's
   `group_vars/<site>_hosts/vars.yml`.
3. Run `site-bootstrap.yml -e site_id=<new-site>`. The role's Phase
   7B creates the replicator role + grants the spock-schema perms +
   creates the local spock node + creates the directional subs (from
   the new site, and mirrored on existing peers via `delegate_to`).

For a site that came up under the legacy `user=postgres` DSN pattern
and needs to be retrofit (the 64Q.4 ceremony):

1. Drop each existing sub via `spock.sub_drop()`.
2. Re-run `site-bootstrap.yml` — role re-creates with the canonical
   `user=replicator` DSN.

Alternative: run the operator-fired retrofit SQL directly (the path
the closure took because it didn't require fanning the
site-bootstrap chain across every site). The role's GRANT task is what
makes either path work without the `permission denied for schema
spock` failure that blocked the pre-Block-64Q.4 attempts.

### Removing a site from the mesh

This role does NOT remove subscriptions. Operator-fired:
`spock.sub_drop('sub_X_to_Y')` on each side, plus `spock.node_drop`
when the site itself is decommissioned. Per the HA dev validation runbook,
§"Spock node DSN caching" — drop subs first, then nodes.

## Tests

Local 2-node compose harness in [`tests/`](tests/) — see
[`tests/README.md`](tests/README.md) for the runbook. Spins up two
postgres-spock containers + a synthetic inventory + invokes the role
against the fixture; verifies bidirectional `replicating` outcome and
re-run idempotency.

CI smoke (parse-only): `tests/syntax-check.sh`.

## Related ADRs / docs

- ADR 0801 — Spock multi-master decision.
- ADR 0833 — Spock target Orin-A direct (only A side participates in inter-site replication on A/B pair sites).
- ADR 1009 + ADR 1010 — Ory-on-Spock mindful operation.
- `docs/design/ha-multi-site.md` §"Inter-site Spock auto-mesh" — design.
- `docs/project/implementation-plan.md` Block 64Q — implementation tracking.
- F-052 — Spock subscription endpoint targets publisher postgres directly, not VIP.
