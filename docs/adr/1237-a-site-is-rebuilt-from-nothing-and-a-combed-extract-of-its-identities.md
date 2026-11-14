# ADR 1237 — A site is rebuilt from nothing and a combed extract of its identities

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-14 |
| Deciders | operator |
| Repo scope | goat core (goatbuilder, the identity, mesh and workload-identity planes) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 0208](0208-phoenix-native-peer-discipline.md) (phoenix-native peers), [`phoenix-architecture.md`](../design/phoenix-architecture.md) (the April exploration), [ADR 1205](1205-the-highest-echelon-site-overrides-and-can-sever-every-other-site.md) (the hub; sever and restore), [ADR 1165](1165-island-mode-multi-writer-every-site-admits.md) (island mode), [ADR 1193](1193-revocations-are-absorbing-on-replicated-mesh-and-identity-tables.md) (the revocation ledger), [ADR 1031](1031-credential-revocation-mesh-first-offboarding.md) (offboarding), [ADR 1065](1065-graph-engine-property-graph-read-model.md) (the actor graph), [ADR 1088](1088-membership-transfer-engine.md) and [ADR 1089](1089-probation-isolation-lifecycle.md) (membership lifecycle), gate rows C1 and I5 of [`goatbuilder-v1-gate.md`](../design/goatbuilder-v1-gate.md) |

## TL;DR (for a layperson)

When a site has to be rebuilt, for example after it may have been compromised, we do not copy
its old disks back. We take out only a short, signed list of who and what belonged there: the
people, the machines and the services, with their roles, and only references to their keys.
Then we build the site again from nothing with the same commands that built it the first time,
and bring the list in. Nothing on the list is trusted just because it was there before: each
person, machine and service is checked again before it exists on the new site. Routine
low-privilege entries are checked automatically; anyone with high privilege, or anything that
holds a key, needs a named person to approve it, and that person sees every warning sign for it.
A site the hub has cut off comes back only this way.

## Context

ADR 0208 made phoenix-native peers a discipline: every component can be torn down and rebuilt,
because its runtime state is re-derived from its cryptographic identity, its declared role, its
discovery channel and its supply-chain code. The April exploration
([`phoenix-architecture.md`](../design/phoenix-architecture.md)) split two workstreams: that
discipline, which is required, and a scheduled rebuild cadence, which was left undecided.

What neither settled is the one thing a rebuild cannot re-derive: **which people, machines and
services belong to the site.** Everything else is declared (group_vars, the pin table, the
inventory) or regenerates on start. Those identities were created at run time, by onboarding,
enrolment and registration, and live in the identity, mesh and workload-identity stores.

Today the only way to bring a site back is to copy those stores whole. A joiner is seeded
data-only from a peer (`ops/ansible/site-seed-from-peer.yml`), and a severed site returns by
being re-seeded from the hub and re-subscribed, then a `restore_site` directive (ADR 1205 D4).
A whole copy carries everything: an account no one can explain, a privilege raised by hand, an
identity created at the site while it was severed. If the site was compromised, the copy can
carry the compromise back.

The operator's direction, 2026-11-14:

> "we need a new adr that establishes a phoenix engineering design pattern. it should be easy to
> extract only the essential identities of people, machines, and processes from any given site
> and/or tactical pod site and to rebuild the entire site from scratch, the same for entire
> environmentes comprised of multiple host nodes at multiple sites. the rebuild process should
> include a combing and revalidation of all accounts, calling out any indicators for concern and
> providing some kind of validation for each element, probably manual for higher privilege
> entities and machine level for limited least privs that are routine."

## Decision

### D1 — The extract: one declared, minimal, signed set per site, and the only thing carried

One command, `goatbuilder extract <site>` (not built), writes one file, schema
`goat-site-extract/v1`. It holds the identities the site owns that no configuration declares,
and nothing else:

| Kind | Carried | By reference only |
|---|---|---|
| People | identity id, handle, roles, affiliation, owner site (ADR 1205 D5), sponsor, created at and by which site, last active, last credential change | public credential identifiers (a passkey's id and public key). No password, hash or recovery code |
| Machines | enrolment identity (slot or anchor), class, mesh groups, owner site, created at, last seen | the mesh public key and the fabric certificate's serial and fingerprint. Never a private key |
| Processes | workload identity (SPIFFE id and selectors), OAuth clients (id, scopes, redirect names), service users (name, policy name) | the name of the variable or file that holds each secret. Never the secret |

Not carried: sessions, tokens, caches, logs, audit objects, counters, replication state, derived
tables, or anything the deployment's configuration already declares (a service user named in
group_vars is rebuilt from group_vars). The file is signed by the person who ran the extract,
with their hardware-held operator key, and names them.

**Reason.** A rebuild can re-derive everything except membership. Carrying membership alone
keeps the carried set small enough to read, and leaves no channel through which state that is
not an identity returns. Secrets and hashes are left behind because a copy from a possibly
compromised site may hold the attacker's; a public key can be compared against the hub's record
without trusting it.

### D2 — For a severed site, the extract is taken at the hub

A severed site's own extract is never the source. The extract is taken at the hub, from the
hub's copy of the records the site owns (the replicated identity and mesh tables). If the
severed site can still produce its own extract, that file is evidence only: every element in it
that the hub's copy does not hold, or holds differently, is listed as an indicator of concern
(D4).

**Reason.** Anything the severed site wrote after the sever never reached the hub (ADR 1205 D4).
The hub's copy is the last state the goatnet accepted from that site.

### D3 — The rebuild: from nothing, by the commands that built it, plus an import

A site is rebuilt by a burn-down (`goatbuilder teardown`), then `goatbuilder standup` (the hub) or
`goatbuilder join-node` (any other site), as gate row C1 measures them, then
`goatbuilder import <site> --extract <file>` (not built). No step restores a disk, a database
dump or a volume. An environment is rebuilt site by site, **hub first**: the hub from its own
extract, then each other site from the extract the rebuilt hub produces for it. A tactical pod is
a site, rebuilt the same way with `join-node`.

**Reason.** One path, the one already used to build sites, means the rebuild is exercised every
time a site is built, and has no separate restore procedure to drift. Hub first, because every
other site's extract and its approval to rejoin come from the hub.

### D4 — The comb: every element is revalidated before it exists on the rebuilt site

The import restores nothing directly. For each element it runs the checks of its tier, records
the result, and creates the element only once the result is a pass. An element that fails is not
created, and is listed with its reasons.

| Tier | Who it covers | Validation |
|---|---|---|
| Routine | people, machines and processes whose roles and grants are within the least-privilege set of their class | machine checks: declared or sponsored by a declared identity; still enrolled at the hub; its credentials unexpired and not revoked (ADR 1193); seen within the class's dormancy threshold (ADR 1089); grants within the class's ceiling |
| Privileged | platform admins, organization admins, and anything that holds a key, a signing right or an ADR 1205 override | the machine checks, then approval by a named person on the goatnet's declared approver list (`goatnet_rebuild_approvers`), who is neither the element itself nor the person who ran the extract. The approval is signed and recorded with the element |

Every element carries its **indicators of concern**, computed by the import and shown to the
approver (and, for a routine element, failing it):

| Indicator | Read from |
|---|---|
| No declaration or sponsor explains it | configuration, the sponsor's own record |
| A privilege above its sponsor's or its class's ceiling | the actor graph (ADR 1065), the class |
| Created at a severed site, or at a site that is not its owner | `created_site`, `owner_site`, the severed-site list |
| In the severed site's own extract but not in the hub's copy, or different there | D2 |
| Unused for longer than its class's dormancy threshold | last active, last seen |
| A credential, key or group changed shortly before the extract or the sever | last credential change, the membership transfer record (ADR 1088) |
| Revoked (ADR 1193) | ADR 1193 ledger |

A person or machine that is not carried, or fails the comb, is not deleted anywhere: it simply
does not exist on the rebuilt site. It returns by onboarding or enrolment as if new.

**Reason.** Copying an account forward makes its earlier admission the only check it ever had.
The comb checks each one again against the present declarations, with the cost of the check
matched to what the account can do: machine checks scale to every routine account, and a
person's judgement is spent only where a mistake grants control.

### D5 — A severed site returns only through this pattern

A site severed under ADR 1205 D4 returns by D2 to D4: its extract is taken at the hub, the site is
burnt down and joined again, and the import combs every element. Only then does the hub issue
`restore_site`. The rebuilt site is seeded from the hub as any new joiner is; its old queued
changes are never replayed, as ADR 1205 already states. This replaces re-seeding the severed
site in place as the way back.

**Reason.** Re-seeding in place keeps the severed site's disks, hosts and anything left on them.
A severed site is presumed compromised; rebuilding from nothing is the only return that does not
depend on that presumption being wrong.

### D6 — What this is not

- **Not a backup restore.** No data other than the extract is carried. Application data,
  audit records and stores are restored, if at all, by their own retention and offload designs.
- **Not a scheduled teardown.** When to rebuild stays the operator's choice, per site. The April
  exploration's second workstream, a rebuild cadence, remains undecided.

**Reason.** Both are different decisions with different risks; folding them in would make this
ADR carry data it was written to leave behind.

### D7 — Gate scope: proposed, the operator's call

Proposed: only what the sever-and-restore drill needs gates 1.0. That is D1 to D5 for one
joiner site, on the two-site goatnet of gate row I5: sever it, extract at the hub, burn it down,
join it again, import with the comb, restore, and the sites agree. Environment rebuild, tactical
pods, and the comb's full indicator list beyond what that drill exercises do not gate 1.0. Which
parts gate 1.0 is the operator's decision, recorded in the gate document when made.

**Reason.** The drill is the one case where the pattern is required, not optional: ADR 1205
already lets the hub sever a site, and without D5 its only way back is the in-place re-seed this
ADR replaces.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Restore the site's stores from a backup or a peer copy | Carries every account and every change, including any made by an intruder, with no check |
| Carry everything and comb it afterwards | Elements would exist, and could act, before they are checked |
| Machine checks for every element, no person | A platform admin or an override holder that passes every machine check can still be the wrong person to hold it |
| A person approves every element | Does not scale to the routine accounts of a site, and spends attention where a mistake grants little |
| Carry password hashes and private keys so people and machines need not re-enrol | The copy may hold an attacker's credential; public halves are enough to compare, and the rest is re-issued |
| The severed site produces its own extract | Its own records are what is in doubt (D2) |

## Consequences

- **People with only a password set a new one.** No password or hash is carried, so after a
  rebuild such a person uses the recovery flow. A person with a passkey keeps it if the comb
  passes.
- **A machine may need to re-enrol.** It keeps its key only when the comb validates it against
  the hub's record; otherwise it enrols again.
- **New commands and records are needed:** `goatbuilder extract`, `goatbuilder import`, the
  extract schema, the per-element validation record, and `goatnet_rebuild_approvers` in each
  goatnet's configuration. None is built.
- **Identities need provenance.** The indicators need `created_site`, `owner_site`, sponsor and
  last credential change on every carried kind. ADR 1205 D5 provides `owner_site` for people;
  machines and processes do not record all of these yet.
- **A rebuild takes longer than a re-seed**, by the comb, and by the approvals, which wait for a
  named person.

## Assessor view (SCA-R)

The pattern implements reconstitution to a known state (NIST 800-53 CP-10) without trusting the
state being replaced, and an account review at every reconstitution (AC-2, AC-2(3) for accounts
unused past a threshold). Privileged accounts get a separate, named, recorded approval (AC-6,
AC-2(7)), by someone other than the account and the person who extracted it (AC-5). The extract
and each approval are signed and name their signer (AU-10). An assessor can ask for a rebuilt
site's extract and its validation record and check that every identity present on the site has a
passing record.

## Trigger conditions for revisit

- The sever-and-restore drill shows that the comb's machine checks pass an element a person
  would reject, or reject routine elements often enough that approvers wave them through.
- A store gains an identity kind the extract does not cover.
- The operator decides a rebuild cadence (D6).
