# ADR 1205 — One site of a goatnet, its hub, overrides every other site, authorizes every join and every platform admin, and can sever a site; identity edits run at the identity's owner site

- **Status:** **Accepted** (2026-11-14, operator), together with [ADR 1165](1165-island-mode-multi-writer-every-site-admits.md) as one design. Nothing in a live goatnet changes on merge: each part ships behind the ADR 1165 partition harness and turns on with ADR 1165 step 6. Each decision is marked **In 1.0** or **After 1.0** (a fast follow), by the operator's scope decision.
- **Date:** 2026-10-29; amended 2026-11-14 with D8 to D12 while Proposed, and accepted the same day
- **Deciders:** operator
- **Evidence:** F-590 — two Kratos sites that both write one identity while cut off never converge, because every admin write replaces the identity's credential rows with fresh ids.
- **Amends:** [ADR 1165](1165-island-mode-multi-writer-every-site-admits.md) D3 class 2 (who owns an identity, and how ownership moves) and class 5 (which site is the hub).
- **Related:** [ADR 1193](1193-revocations-are-absorbing-on-replicated-mesh-and-identity-tables.md) (revocation ledger) · ADR 1043 (echelon) · [ADR 1031](1031-credential-revocation-mesh-first-offboarding.md) · [ADR 1182](1182-platform-admin-lifecycle-one-command.md) (platform-admin lifecycle; [`platform-admin-lifecycle.md`](../design/platform-admin-lifecycle.md)) · [ADR 1071](1071-sponsorship-authorization-at-onboarding.md) and [ADR 1072](1072-sponsor-scoped-onboarding-console.md) (sponsor ceiling) · [`island-mode-multi-writer.md`](../design/island-mode-multi-writer.md) §3 · [`island-mode-write-inventory.md`](../design/island-mode-write-inventory.md) C41–C46, K1–K12 · [`goatbuilder-v1-gate.md`](../design/goatbuilder-v1-gate.md) row I4 and "Fast follow after 1.0"

## TL;DR (for a layperson)

Every site of a goatnet keeps working when cut off. One site, the hub, outranks the
others. It is normally the site at the highest command, the most secure and
best-resourced location, and when a goatnet is first built it is the first site.

Only the hub can do four things: make its changes win over another site's, cut a
compromised site out of the goatnet, let a new site join (or a cut-off site back in), and
make someone a platform admin. Every other site refuses any of those that did not come
from the hub, and raises an alert. So a site that is broken into can damage only itself.

The hub can be moved to another site, but only by changing the configuration of every
site, which no single site, the hub included, can do on its own.

Otherwise, a person's record is changed only by the site the person is registered at, so
two sites never edit the same person at the same time.

## The operator's decision

> "yes except there is a higher authority that supercedes others and that is the highest
> echelon node for an evnironment. the highest echelon would reside at the higher command
> at the most secure and well resourced location. and it's actions should take precedence
> over any other node including an emergency act to sever a site if it is compromised.
> only the higher echelon site would have those two asymetric overrides, other wise yes
> the priority should go to the site to which an identity is register and active last"
> (2026-10-29, accepting the F-590 fix with this amendment)

> "there should be site that can overrule other, overwrite others and even disconnect or
> reauthorize the joining of others. that is not necessarily the founder site, but very
> oftern might be, and initial at the first site and then the two site configuration by
> defualt would be that. we need this so that if one site is infiltrated it cannot wreck
> havoc on all sites." (2026-11-14)

> "the limit shoul dbe at the platform admin level ... only hub sites can establsih a new
> platform admin." (2026-11-14)

Accepted with ADR 1165, which the operator chose as the 1.0 target on 2026-11-13 with
"yeah c": island mode starts now, 1.0 requires it, and the cut-apart drill on standing
goatnets proves it. On 2026-11-14 the operator restated it: "I thought we agreedthat all
sites would be island mode fully capable." The scope of this ADR's parts, the same day:
"the major steps that we have described that are related to goatbuilder like hub
authority, phoenix mechanisms, I think these should be planned to be fast follow features
post 1.0", and, on the split below, "this soudns correct".

## Hubs, as declared

| Goatnet | Hub | `goatnet_hub_site` |
|---|---|---|
| zagros-dev (dev) | site alpha | `zagros_dev` |
| zagros-preprod (preprod) | site alpha | `zagros_preprod` |
| zagros (prod) | site alpha | `zagros_a` |

## Decision

**D1 — Each goatnet declares one hub, its highest-echelon site.** *In 1.0.* The declaration is
`goatnet_hub_site`, the Spock node name of that site, set once in the goatnet's
group_vars. ADR 1165 class 5 work (creates and deletes of global derived state after
heal) runs at this site. The `spock-hub-directives` role refuses to run on a site with
no `goatnet_hub_site`, so no site carries out directives until the operator has named
the hub.

**D2 — Hub directives are the hub's only channel for acting on another site.** *In 1.0.* A
directive is a row in `goat_hub_directives` in the `netbird` database (repset
`netbird_mgmt`, present at every Spock site): `id` uuid, `kind`, `subject`, `body` jsonb,
`at`, `actor`. A trigger refuses UPDATE and DELETE. Each site runs
`spock-hub-directives`, a 30-second systemd timer that applies directives it has not yet
applied and records each in a local table that is in no repset. The timer accepts a
directive only when its commit came from the hub: a local commit at the hub itself, or,
elsewhere, a commit whose replication origin (`pg_xact_commit_timestamp_origin(xmin)`) is
the hub's Spock `node_id`, which Spock's apply records on every commit it applies.
`goatnet_hub_site` is read from the site's own configuration, never from the database.
A directive written at any other site is refused at every site and raised as an alert,
so the two overrides stay with the hub even when another site's database is
compromised.

**D3 — Override 1: the hub's actions take precedence.** *In 1.0.* The hub may change any record.
When the record is owned by another site, the hub writes a directive, and the owner site
carries the change out with its own local write. While the owner is cut off, the
directive waits at the hub. After heal it applies after the owner's own cut-off edits
have arrived, so the hub's version is the last write and is the one both sites keep.
There is still only one writer per record, so the F-590 divergence cannot occur. A hub
revocation needs no directive: the hub writes to the ADR 1193 ledger like any site, and
a reinstatement from another site cancels only the revoke events it names. The hub
therefore undoes a hostile reinstatement by revoking again.

**D4 — Override 2: the hub can sever a site.** *In 1.0: the `sever_site` and `restore_site` directives. After 1.0: the sever-and-restore drill on a standing goatnet.* `sever_site` names a Spock node. On
applying it, every site disables its own subscriptions from that node in every Spock
database, so nothing the severed site writes afterwards reaches anyone. The hub's admin
then revokes the severed site's management-host mesh peers through the ADR 1193 ledger
(`goat_revoke('netbird.peer.deleted', …)` at the hub), so other sites' NetBird drops them
from every network map; the applier does not do this, because nothing declares which
peers belong to a site. Writes the severed site made before the directive arrived stay
applied, and are what `spock.exception_log` and the drift watch show from then on. A
severed site comes back only by being re-seeded from the hub (the existing
`*-seed-from-peer.yml` playbooks) and re-subscribed, followed by a `restore_site`
directive. Its queued changes are never replayed.

A directive whose commit origin a site cannot yet read (its commit timestamp is not
available) is left pending and counted in `goat_hub_directives_pending`; a site records
`refused_origin` only for a commit it read as coming from a node other than the hub.

**D5 — An identity's owner is the site it was registered at or last made active at.** *In 1.0.*
The owner site is held in the identity's `metadata_admin.owner_site`, written by the
creating site at registration. It moves only by a `transfer_owner` directive from the
hub, which covers re-registration and reactivation at another site. An identity with no
`owner_site` (every identity created before this ADR) is owned by the hub.

**D6 — Who writes a person's identity.** *In 1.0.*

| Act | Owner site | Hub, not owner | Any other site |
|---|---|---|---|
| Edit traits, metadata, credentials (C42, C43, C45, C46, K2, K12) | writes through local Kratos | directive, carried out by the owner | refused, naming the owner |
| Revoke: deactivate, delete, session revoke (C44, K4, K11) | ADR 1193 ledger | ADR 1193 ledger | ADR 1193 ledger |
| Reinstate | ledger, naming the revokes it cancels | ledger | ledger |
| Create (C41, K1, K9) | — | — | allowed; the creating site is the owner |

A non-owner revocation is written through the ledger functions, never as a Kratos
`PATCH /state`, because that `PATCH` rewrites the credential rows.

**D7 — Two sites creating the same email is a collision, resolved by the hub.** *In 1.0.* Spock
logs the second site's credential identifier as a `23505` in `spock.exception_log`. The
console lists it in the ADR 1165 D4 resolution queue. The hub's admin picks the identity
that survives. Two directives resolve it, each carried out by an owner site:
`identity_delete` for the losing identity, then `identity_resave` for the surviving one,
which re-issues its credential rows so they reach the site that discarded them.

**D8 — The first site is the hub by default; moving the hub takes every site's
configuration.** *In 1.0.* `goatbuilder standup` for a goatnet's first site writes
`goatnet_hub_site` as that site's Spock node name into the site's group_vars, beside
`goatnet_founder: true`. A site that joins inherits it: its standup copies
`goatnet_hub_site` from the site it joins (`join_founder_site`), and the config
preflight refuses a joining site that declares no hub or a different one. The hub need
not stay the first site. Moving it is one change made everywhere: `goatnet_hub_site` set
to the new hub in every site's configuration, and the hub-directives role and the D10
guard re-applied at each site.

The property this gives: **a site's belief about which site is the hub comes only from
its own configuration, outside every database.** The applier reads it from its
environment file and the D10 guard from a Postgres setting (`goat.hub_site`), both
written from group_vars by Ansible. No replicated row is read for it, and no directive
kind changes it. A compromised site, the hub included, can change what that one site
believes, but not what any other site believes. It cannot make itself the hub for the
others, and it cannot move the hub away. Moving the hub takes whoever deploys every
site's configuration, never one site.

*Reason:* the first site exists before any other, and the operator named it the default.
D2's origin check is only as strong as the place the hub's name is read from.

**D9 — A site joins, and a severed site rejoins, only on the hub's authorization.** *After 1.0.* A
new directive kind, `admit_site`, names the joining site's Spock node, with a reason
and an expiry. The hub's admin writes it at the hub. It reaches every site the way every
directive does, and each site's applier records it in its local table after the D2
origin check. For a rejoin after a sever, the authorization is a `restore_site` from the
hub that is later than the site's last `sever_site`.

Each existing site checks the authorization where it lets the new site in: the
`spock-mesh-peering` role, run by `spock-mesh-wire.yml`, reads the site's local table
before it creates a subscription from a node (`roles/spock-mesh-peering/tasks/per-database.yml:367`
and `:409`), and refuses with no applied, unexpired `admit_site` or later `restore_site`
for that node. `goatbuilder standup` and `join-node` for a joining site ask first: before
their Spock step they read whether the site they join has applied the authorization, and
if not they stop and print the command the hub's admin runs. A severed site returns only
by rebuild, re-seed from the hub and the hub's `restore_site` (D4).

A site cannot admit another site for the rest of the goatnet. Every subscription is
created with `forward_origins` empty, so a site that wires a node into itself alone takes
that node's writes into its own database and passes none of them on. This ADR makes
empty `forward_origins` a requirement: the role refuses a goatnet that sets
`spock_mesh_forward_origins`.

*Reason:* a site that could bring in another site could bring in an attacker's node with
it; the operator asked for the hub to "disconnect or reauthorize the joining of others".

**D10 — Only the hub makes a platform admin.** *In 1.0* (gate row I4). A platform admin is a person whose Kratos
identity holds the `PLATFORM-ADMIN` group (`metadata_public.groups`), which replicates in
`ory_narrow`. That group is the authority; every other plane follows it (ADR 1182 item 3).
Making a platform admin, by creating an identity with that group or adding it to one,
happens only at the hub. It is refused twice everywhere else:

- **At the site that writes it.** Every grant path refuses at a site that is not the hub
  and names the hub: `goat admin platform-admin grant` (ADR 1182 item 1), the console's
  group change and user create, `goat admin role set`, and the onboarding plays.
- **At every other site, when the write arrives.** A guard trigger on Kratos
  `identities`, installed beside the ADR 1193 guard, refuses a replicated row that adds
  `PLATFORM-ADMIN` when the commit's replication origin is not the hub (read as D2 reads
  it). It also refuses such a local commit at a site that is not the hub. The refused
  row is logged in `spock.exception_log` (ADR 1165 D4) and in a local
  `goat_ceiling_refused` table, which raises an alert.

NetBird's `admin` role follows the group at each site through the NetBird admin
reconciler, which promotes only the people its own site's Kratos names (ADR 1182 item 7).
A NetBird admin that no site's Kratos entitles, such as one a compromised site wrote into
`users.role` directly, is reported by every other site's reconciler as a stray admin
(`netbird_admin_reconcile_stray_admins`); D10 adds the alert on it. A trigger does not
guard `users.role` itself, because every site's reconciler writes it locally and each
promotion replicates to the others.

So a site that is broken into, and whose own software is bypassed, still cannot make a
platform admin that any other site accepts. Lowering or removing a platform admin is a
revocation (ADR 1165 class 3) and is allowed at any site. Everything below platform admin
stays as ADR 1165 has it.

The sponsor ceiling (ADRs 1071, 1072) gains one rule: at a site that is not the hub, the
highest tier anyone can sponsor or approve is `iam-admin`, one below `platform-admin`,
whatever the sponsor's own tier. Operator keys, uncapped in ADR 1071, are capped the
same way there.

*Reason:* a platform admin can do everything else, so one compromised site that can make
one compromises every site. The operator put the limit at this level.

**D11 — After a sever, the hub lists what the severed site wrote and revokes it.** *After 1.0.* This
is a step of the sever procedure. The hub lists every row, in every replicated database,
whose current version the severed node committed since a stated time, by the commit's
replication origin (read as D2 reads it), and its admin revokes each through the ADR 1193
ledger or a directive. The list holds only rows that still exist: a delete, or a version
later overwritten, shows only in `spock.exception_log` and drift-watch. Postgres keeps a
commit's origin only while it keeps that commit's timestamp, so the list is taken at the
sever, not weeks later.

*Reason:* the writes a severed site made before the directive arrived stay applied (D4).

**D12 — The gate.** *In 1.0:* gate row I4 of `goatbuilder-v1-gate.md` holds a write
class off at every site until a platform-admin grant made at a non-hub site is refused
at every other site, proven in the cut-apart partition harness; and the hub-directive
applier is applied by a play (no play applies it today; PR #596 adds it to
`site-bootstrap.yml`). *After 1.0:* the sever-and-restore drill on a standing two-site
goatnet, with the operator present (the hub severs a site, the site's later writes reach
no one, and the site returns only by rebuild, re-seed and the hub's `restore_site`),
listed under "Fast follow after 1.0" in the gate file with D9 and D11.

## Acceptance (with ADR 1165)

ADR 1165 decides that every site writes, and how concurrent writes are kept apart. This
ADR decides which site's word is final, and limits what any one site can do to the
others. Neither is safe without the other: every site writing with no hub lets one
compromised site corrupt every other, and a hub with no multi-writer design leaves
cut-off sites unable to work. The operator accepted them together on 2026-11-14.

| Part | Scope | State |
|---|---|---|
| D1, D2, D4: hub declaration, directives, sever and restore directives | In 1.0 | built; proven by the partition harness scenarios `hub-directive-from-non-hub-refused` and `hub-sever-site-stops-its-writes`; applied by no play (PR #596) |
| D3, D5 to D7: precedence through owner sites, identity ownership, collisions | In 1.0 | built in the applier and the harness Kratos scenarios; the console and CLI refusals of a non-owner edit are not built |
| D8: the default hub at standup and its inheritance | In 1.0 | designed, not built |
| D10: the platform-admin ceiling and its guard | In 1.0, gate row I4 | designed, not built |
| D9: `admit_site` and the join checks | After 1.0 | designed, not built |
| D11: the post-sever list | After 1.0 | designed, not built |
| D4's live drill: sever and restore on a standing goatnet | After 1.0 | not measured |

## Alternatives considered

| Alternative | Why not |
|---|---|
| Every site equal, last write wins on identities (ADR 1165 as written) | Two writers to one identity never converge |
| The hub writes other sites' identities directly | Two writers again whenever the hub and the owner both act while cut off |
| A hub-precedence conflict handler inside Spock | Spock resolves per row by timestamp only; a custom handler is code on the replication apply path |
| Hub authority checked by a `site` column on the row | Any site can write any value into a column; only the replication origin shows where a commit came from |
| Signed directives | No signature verification in Postgres without an extension; the replication origin gives the same guarantee for free |
| Ownership moving on every login | Logins happen at every site, so ownership would change while cut off, with two sites each believing they are the owner |
| The hub chosen by a vote among sites | In a two-site goatnet one compromised site is half the vote; configuration outside every database keeps the choice out of any site's reach |
| The hub's name held in a replicated table | Any site can write any row; the hub would be whatever a compromised site last wrote |
| A join authorized by a shared secret in group_vars | A compromised site holding the secret admits anyone; the hub's directive is checked by origin, which a site cannot forge |
| Platform admin refused only by the console | A compromised site bypasses its own console; only the check at every other site, on apply, holds |
