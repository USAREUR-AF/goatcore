# ADR 1240 — A platform-admin grant not made at the hub arrives without the group, and is recorded and alerted

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-14 |
| Deciders | operator |
| Repo scope | goat core |
| Amends | [ADR 1205](1205-the-highest-echelon-site-overrides-and-can-sever-every-other-site.md) D10 (what happens to a replicated grant of `PLATFORM-ADMIN` whose origin is not the hub) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1182](1182-platform-admin-lifecycle-one-command.md) (the `PLATFORM-ADMIN` group is the authority), [ADR 1165](1165-island-mode-multi-writer-every-site-admits.md) D4 (`spock.exception_log`), [`island-mode-multi-writer.md`](../design/island-mode-multi-writer.md) §3, [`goatbuilder-v1-gate.md`](../design/goatbuilder-v1-gate.md) row I4 |

## TL;DR (for a layperson)

Only the goatnet's hub may make someone a platform admin. If another site sends a change that
makes someone a platform admin, every other site still accepts the rest of that change, but
without the platform-admin part. It writes down what it removed, and an alert tells the
operators. ADR 1205 said such a change would be refused whole and logged with replication
errors. Refusing it whole would also throw away the ordinary edits in the same change, so the
sites would stop agreeing about that person.

## Context

ADR 1205 D10 says the guard on Kratos `identities` "refuses a replicated row that adds
`PLATFORM-ADMIN` when the commit's replication origin is not the hub", and that "the refused
row is logged in `spock.exception_log` (ADR 1165 D4) and in a local `goat_ceiling_refused`
table, which raises an alert".

An identity is one row. A replicated change to it carries the whole row: the person's details,
the other groups and the state, as well as `metadata_public.groups`. With
`spock.exception_behaviour = discard` (ADR 1193), refusing the row in the apply discards every
column of it at the receiving site. The receiving site keeps its old copy of the person, and
the sites differ on that identity until someone repairs it by hand. For a new identity, the
receiving site would not hold the person at all. When it was built, the guard took the group
out of the row instead and applied the rest. Nothing then goes to `spock.exception_log`, so
D10's text no longer describes what the code does.

## Decision

This replaces the last sentence of ADR 1205 D10's second bullet ("The refused row is logged
in `spock.exception_log` ...") with the following. The rest of D10 stands.

- **A replicated grant whose origin is not the hub is not applied as a grant.** The guard
  removes `PLATFORM-ADMIN` from `metadata_public.groups` and applies the rest of the row
  unchanged, the other groups included. This covers an UPDATE that adds the group and an
  INSERT that carries it.
- **It records the grant in `goat_ceiling_refused`**, a local table in no replication set: the
  identity, the operation, the sending node, the reason (`not_hub`, `forwarding` or
  `no_hub`), the groups as received and the commit timestamp of the sending site's commit.
  Nothing is written to `spock.exception_log`.
- **It alerts.** `spock-drift-watch` counts the table per peer, and `SpockCeilingRefused`
  (critical) fires on an increase.
- **The hub is matched exactly.** A sending node is the hub only when its name is exactly
  `goat.hub_site` or exactly `<goat.hub_site>_<database>`, never by prefix, suffix or
  pattern.
- **A site that forwards origins trusts no replicated grant.** While any subscription at the
  site has non-empty `forward_origins`, the sending node need not be the node that made the
  commit. Every replicated grant there is treated as not from the hub (reason `forwarding`).
- **A site that names no hub trusts no grant.** With `goat.hub_site` unset, a local grant
  raises an error that names the `ory-narrow-repset` role, which writes the setting. A
  replicated grant is stripped and recorded (reason `no_hub`).

A local grant at a site that is not the hub is still refused with an error, as D10 says.
Removing `PLATFORM-ADMIN` is never touched.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Refuse the whole row, as D10 was written | Discards the row's other changes at every receiving site, and a refused INSERT leaves the person missing there. The sites then differ on that identity until a manual repair, which the partition harness reports as drift. |
| Raise an error in the apply so Spock logs it to `spock.exception_log` as well as stripping | A trigger that raises cannot also return a changed row. The exception log would hold a row that was in fact applied, which misleads whoever reads the resolution queue. |
| Strip the group without recording it | Removes the evidence that a site wrote a grant past its own guard, which is the event the operator most needs to see. |

## Consequences

**Positive.** Sites still agree on everything about an identity except a grant they refused.
Every refusal leaves a record with the evidence needed to judge it: the sending node, the
reason, the groups as received and when the sending site committed. The alert is a metric,
like the other Spock alerts, and not a parse of the replication error log.

**Negative.** After a refusal, the sending site still holds `PLATFORM-ADMIN` on its own copy
of the identity, and the row differs from the other sites until the hub writes that identity
again without the group, which is the hub's remedy. The record's
`committed_at` is filled in after the grant commits, by the guard's next refusal or by
drift-watch's read. If neither runs before Postgres truncates its commit-timestamp data, that
column stays empty.

## Assessor view (SCA-R)

This narrows who can create the most privileged account (AC-2, AC-6(5)). A site that is
broken into cannot make a platform admin that any other site accepts, and the attempt is
recorded at each receiving site (AU-2, AU-12) and raised as an alert (SI-4). The residual risk
is the sending site itself, where the grant stands until the hub acts. The compensating
controls are the alert and the hub's sever (ADR 1205 D4). No waiver is needed.

## Trigger conditions for revisit

- Kratos stores groups anywhere other than `metadata_public.groups`, or the identity row is
  split so that a grant can be refused without refusing other changes.
- Spock gains a way to log a changed-but-applied row to its exception log.
- A refusal is ever recorded with an empty `committed_at` that an investigation needed.
