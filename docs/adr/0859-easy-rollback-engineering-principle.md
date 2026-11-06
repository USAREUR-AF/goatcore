# ADR 0859 — Easy rollback as an engineering principle

**Status:** Proposed 2026-05-19. **Extended by [ADR 1060 — universal deploy-rollback ledger](1060-universal-deploy-rollback-ledger.md)** (2026-06-23), which generalizes this per-service opt-in into a background capability every deploying role gets automatically: on-host runtime ledger as source of truth (covers group_vars/env pins), artifact-type taxonomy with fail-closed refusal, and a coverage ratchet. This ADR's three substrate elements stay valid as the image-rollback mechanism.
**Anchor design:** [`docs/design/easy-rollback.md`](../design/easy-rollback.md) → generalized in [`docs/design/universal-deploy-rollback.md`](../design/universal-deploy-rollback.md).

## Context

A bad deploy (a config split-brain between an inventory group and its group_vars) can crash-loop a service and serve `502 Bad Gateway` until someone debugs it by hand. The gap is *structural*, not a one-off:

> "let's enable easy rollback to latest good deployment for all services so that we can rapidly fix anything that we break as we keep changing this … let's make it an engineering principle"

The substrate already supports many flavors of recovery (Ansible roles are idempotent, compose files are checked in, image SHAs persist on host), but each service's recovery has its own ad-hoc dance. Eleven tier-0 services × eleven different dances ≈ no dance. Under deploy pressure, this looks like outage time.

## Decision

**Adopt easy rollback as a per-service engineering invariant:**

> Every service we deploy can be rolled back to its last 3 known-good image SHAs with one command — by any operator, from any session, without consulting another doc.

Three concrete substrate elements:

1. **In-repo per-service ledger** at `ops/ansible/roles/<service>/deploy-history.yml` — last 8 SHAs newest-first, with PR + summary + `do_not_roll_to` flag for known-bad images.
2. **On-host per-service ledger** at `/var/lib/goat/<service>/deploy-history.jsonl` — append-only NDJSON written by both the Ansible role (on apply) and the rollback script (on rollback).
3. **One-command rollback** at `ops/scripts/rollback.sh <service> [N]` — flips the compose file on-host to the N-th-most-recent SHA, force-recreates the container, appends a rollback row to the on-host ledger.

CI substrate: `.github/workflows/deploy-history-check.yml` requires that any change to a role's `<role>_image` default also appends the new SHA to the corresponding `deploy-history.yml` (and prunes >8 oldest rows).

Tier-0 sweep: panoscope, goat-zenoh-portal, zenoh-router, audit-subscriber, isitup, brief, dataflows, airdefenders, netbird-dashboard, netbird-mgmt, oauth2-proxy-dashboard. Adopted in parallel per-service follow-on tracks after this principle PR lands.

## Scope (in / out)

**In scope:** container-image rollback for tier-0 services. Idempotent, no destructive side-effects, no schema migration.

**Out of scope:**
- Config / env / group_vars rollback — composes with this but belongs to [ADR 0224 progressive config rollout with auto-revert](0224-progressive-config-rollout-with-auto-revert.md).
- Schema rollback (DB) — needs forward-fix or down-migration; conflating into "rollback" creates an unsafe impression.
- Repo rollback (`git revert`, force-push) — already covered by [ADR 0109 trunk-based posture](0109-git-collaboration-trunk-based.md).
- Cross-service coordinated rollback ("roll back A only if B is on version X") — v2 if a real case appears.

## Alternatives considered

- **"Operators run docker compose manually."** Status quo. Per-service dance differs; under pressure operators make irreversible mistakes (wrong SHA, image not pulled locally, sed typo). Rejected.
- **Blue-green deploys as the rollback mechanism.** Right answer for high-criticality production services; over-engineered for the trial-fabric tier. Defer to per-service when it earns the complexity. Easy rollback is a strictly weaker invariant that we can always have.
- **Floating `:latest` tag with `docker image rollback` style flip.** Docker has no first-party rollback primitive; `:latest` is mutable and erases the rollback target. A role pulling `:latest` (no SHA pin) has already overwritten the prior-good image by the time an operator wants to roll back. The first step of the tier-0 sweep is converting every `:latest` to a SHA pin. Rejected as a primary mechanism; preserved only as a non-rollback-friendly default.

## Consequences

**Positive:**
- 502 → fixed in <60s by any operator with a shell, regardless of session context.
- Eliminates "what was the last good SHA" as a coordination problem between agents and operators.
- Per-service deploy history becomes a first-class artifact in the repo — useful for incident review without spelunking `gh run list`.
- Removes the floating-`:latest` footgun by making SHA pinning a precondition for the rollback substrate.

**Negative / cost:**
- Eleven roles to update (image-pin conversion + role wire-up + initial ledger seed). Estimated ~30 min per service; ~5-6h cumulative across the tier-0 sweep.
- New CI workflow to maintain (`deploy-history-check.yml`).
- New invariant to enforce in code review: image bumps require ledger appends.

## Composition

- [ADR 0224 progressive config rollout with auto-revert](0224-progressive-config-rollout-with-auto-revert.md) — sibling pattern for config rollback.
- [ADR 0317 per-service failure mode declaration](0317-per-service-failure-mode-declaration.md) — rollback is the operator response when fail-open turned out wrong.
- [Block 66 PR governance](https://github.com/usareur-af/goatdocs/blob/main/docs/design/pr-governance.md) — `deploy-history-check.yml` is a branch-protection candidate once stable.

## Tracker

[`docs/design/easy-rollback.md`](../design/easy-rollback.md). Track: `easy-rollback-engineering-principle` for the principle declaration + tier-0 sweep coordination.
