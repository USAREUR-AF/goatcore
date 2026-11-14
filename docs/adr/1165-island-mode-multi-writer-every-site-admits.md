# ADR 1165 — Every site writes: portal and console move to Spock-Postgres, and island mode is built by making concurrent writes disjoint

- **Status:** **Accepted** (2026-11-14, operator), together with [ADR 1205](1205-the-highest-echelon-site-overrides-and-can-sever-every-other-site.md) as one design. Nothing in a live goatnet changes on merge. The decision sets the design that the island-mode program builds to; each step ships separately behind the gates in [`island-mode-multi-writer.md`](../design/island-mode-multi-writer.md) §8. ADR 1205's "Acceptance" section gives each part's scope and state.
- **Date:** 2026-10-29; accepted 2026-11-14
- **Deciders:** operator
- **Evidence:** the portal and id console keep all admin state in three per-host SQLite files (`/opt/goat-zenoh-portal/registry.db`, `/var/lib/goat-id-console/onboard.db`, `/var/lib/goat-id-console/orgs.db`) plus an in-memory invite store, so [ADR 1084](1084-block-64r-mgmt-repset-and-two-node-federation.md) D3 pins every admin write to one site. Onboarding a person also writes a NetBird setup key and user, through the onboard-runner running `goat admin onboard --native`, and a Kratos identity; D3 pins those as well. A cut-off site can therefore not admit anyone.
- **Amends:** [ADR 1084](1084-block-64r-mgmt-repset-and-two-node-federation.md) D3 — single-homed admin surfaces are lifted one write class at a time, as each class passes the partition harness.
- **Amended by:** [ADR 1205](1205-the-highest-echelon-site-overrides-and-can-sever-every-other-site.md) (Accepted with this ADR) — D3 class 2 (an identity is edited only at its owner site) and class 5 (the hub is the goatnet's highest-echelon site); and, from 2026-11-14, the hub's authority over joins and over who becomes a platform admin, which every class's writes are checked against.
- **Builds on:** ADR 1193, revocations are absorbing on replicated mesh and identity tables (Proposed; PR #1538, not yet merged) — its revocation ledger is class 3 for NetBird and Kratos, its partition test is the harness base, and its Spock apply settings are D4's.
- **Related:** [ADR 0801](0801-postgresql-spock-multi-master.md) · [ADR 1010](1010-ory-on-spock-mindful-operation.md) · [ADR 0206](0206-island-mode-by-construction.md) · [ADR 1164](1164-per-site-spire-trust-domains.md) · [ADR 1025](1025-zenoh-dynamic-tenant-authz-slot-pool.md) · `ha-multi-site.md` · [`hub-spoke-admit-split.md`](../design/hub-spoke-admit-split.md) §2 · program `island-mode-program.md`

## TL;DR (for a layperson)

Each site of a goatnet should keep working on its own when cut off — admitting people,
enrolling machines, changing access — and merge cleanly when the link returns. Today the
admin tools keep their records in files on one machine, so only one site can change
anything. This decision moves those records into the database every site already
replicates, and then makes it safe for every site to write at once. Safety comes from
designing the writes so two sites almost never touch the same record. The rare case
that cannot be designed away, such as the same person enrolled at two sites at once, is
caught and shown to an admin, never silently dropped.

## Acceptance

On 2026-11-13 the operator chose island mode as part of 1.0, with "yeah c": the program
starts now, 1.0 requires it, and the cut-apart drill on standing goatnets proves it. On
2026-11-14: "I thought we agreedthat all sites would be island mode fully capable." Every
site of a goatnet is to be fully capable on its own; no service is kept at one site as a
matter of design. The operator accepted this ADR on 2026-11-14: "and accept and merge and
move on with all here". ADR 1205 is accepted with it.

## Decision

**D1 — One engine.** The portal registry, the console's onboarding and org stores, the
console invite store and the peerview hold records move to Postgres on the management
host's existing `postgres-a`, each store in its own database. SQLite is removed from both
Go modules once the move is proven.

**D2 — Engine swap and schema change are separate steps, each with its own rollback.**
Step A swaps the engine with the tables unchanged, blue-green per goatnet (zagros-dev →
zagros → zagros-prod), keeping the SQLite files read-only for four weeks with a tested
Postgres → SQLite export as the rollback. Step B reshapes the schema on Postgres under a
versioned migration table; its rollback is pgBackRest point-in-time restore.

**D3 — Every write is in one of five classes, across all four stores at once** (portal,
console, NetBird, Kratos):

| Class | Rule | Conflict possible |
|---|---|---|
| New rows with site-unique or derived ids | ids are xid/UUID, or derived from the inputs so two sites creating the same thing write one row | no |
| Changes to an owned record | the owner (a site, or the org or person the record belongs to) is the only writer; a slot claim is owned by the site whose range of the pre-cut pool holds it; an explicit, logged takeover moves ownership when the owner is unreachable | no |
| Revocations | any site may revoke; a revocation is an inserted row in an append-only ledger, and the effective state is "revoked if any site revoked" until an attributed reinstatement that names the revoke events it cancels. NetBird and Kratos use the ADR 1193 ledger, extended to group membership, user role, holds, sessions and consent; portal and console follow the same pattern | no — set union, no clock |
| Rows unique by value | allowed at any site; a collision lands in a resolution queue and an admin resolves it (orgs through the existing org merge) | rarely, and never silently |
| Global derived state | while cut off a site regenerates only by updating existing ids; creates and deletes run at the hub after heal, and the hub resets NetBird's network serial above both sites | converges by regeneration |

Last-write-wins therefore decides almost nothing, and site clock drift stops mattering. Org merge is the one write refused while a site is cut off. The per-path classification is `docs/design/island-mode-write-inventory.md`.

**D4 — Failure is visible.** Every Spock node runs the ADR 1193 apply settings
(`spock.exception_behaviour = discard`, `spock.save_resolutions = on`), so a failed row is
logged and the rest of the transaction applies, and the console shows
`spock.exception_log` and `spock.resolutions` as the resolution queue. `spock-drift-watch` (ADR 1084 D5) widens
to every replicated table of the new databases and alerts on any count or hash mismatch.

**D5 — A partition harness gates every class.** A two-node Spock harness in CI, grown
from ADR 1193's `ops/postgres/spock/tests/partition-revoke/run.sh`,
partitions the nodes, runs every write path of a class on both sides, heals, and
asserts convergence, no unresolved exception, and no slot or key issued twice. ADR 1084
D3 is lifted for a class only when its harness tests pass; any later change to a write
path must keep them passing.

**D6 — Schema changes are additive.** Every migration after version 1 is
expand-then-contract and applied through the coordinated-upgrade pattern, since Spock
does not replicate DDL.

**D7 — Preconditions before any store moves onto `postgres-a`:** the F-581 disk and
WAL-archive guards on every goatnet, a live zagros-prod standby, exception logging on, and
the harness in CI, and ADR 1193 merged and rolled out. Moving admin state onto `postgres-a` makes it the one database admin
depends on.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep D3 single-homing (status quo) | A cut-off site cannot admit anyone; fails the island-mode goal of ADR 0206 |
| Hub-and-spoke admit/revoke split (`hub-spoke-admit-split.md`) | Admits fail during a partition. Its revoke rule (any site, most restrictive wins) is adopted here as class 3 |
| Streaming replication with one writer (`ha-multi-site.md` §17 fallback) | Same loss of island admits; stays the fallback if Spock fails its §17 bar |
| Plain last-write-wins on every table | Two sites can issue the same slot, which is a certificate identity; the merge drops one with no error |
| Custom merge logic or CRDT columns | Adds code on the identity path where disjoint writes remove the need |
| Separate Postgres instance per store | More to run and back up, no isolation gained against the host failing |
| Dual-write SQLite and Postgres during the swap | Two sources of truth to reconcile; blue-green with a tested export is simpler |

## Consequences

- Every site can onboard people, enrol machines and change access while cut off, once
  all five classes have passed the harness.
- `postgres-a` carries NetBird, Ory, portal and console; its health is a precondition, not
  an afterthought.
- Any new write path must declare its class and carry a harness test.
- A value collision becomes an admin task in the console instead of a silent loss.
- Ten scripts that open the SQLite files directly move to the API or to Postgres.
