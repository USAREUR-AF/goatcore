# ADR 1158 — Host sites stay at the goatnet group rung; only subordinate scopes get their own prefix

- **Status:** Accepted
- **Date:** 2026-09-03
- **Deciders:** operator
- **Extends:** ADR 1043 §1.5
  (the site dimension rides the group prefix — this ADR says *when a site earns
  the second rung*, which 1043 left open)
- **Related:** ADR 1124
  (`HOST:<SUFFIX>` — northbound reach as a rule, not a membership),
  ADR 1116 (gear classes; the `gear: true`
  opt-in per scope),
  ADR 1141 (what a host site *is*),
  [ADR 1122](1122-scale-envelope-systems-vs-peers-pod-tier.md) (the envelope this
  spends groups against),
  F-434,
  F-435,
  F-316,
  `mesh-acl.md` §"A site *inside* an existing goatnet",
  `identity-grammar.md` §1.5,
  `partner-standalone-goatnet.md`

## Context

The first goatnet to hold **two host sites** did not survive that transition
with one naming rule.

ADR 1043 §1.5 established
that the NetBird group prefix carries **goatnet-then-site** in one slot:
`ZAGROS-` spends it on the goatnet, a forward site's `<SITE>-` is the site rung
below. What it never said is **which sites get the second rung**. So each site
decided at stamp time, and three different answers can coexist on one goatnet:

| Machines | Groups held | Rung |
|---|---|---|
| First host site's core nodes | `ZAGROS-INFRA` | goatnet |
| Second host site's core, observability and backup nodes | `ZAGROS-INFRA` | goatnet |
| Second host site's gear | `<SITE>-KID` | site |
| Forward-site gear | `ZAGROS-CONTROL` **+** `<SITE>-KID`/`-POD`/`-P2P` | **both** |

Three patterns for the same question, and the naming reads as if it answers a
different one each time: the first host site was never named at all (it
inherited the goatnet prefix), a forward site got its own prefix, and the
second host site got a site prefix for its gear while its host nodes stayed at
the goatnet prefix.

The provenance is why this ADR writes a rule rather than a cleanup list: a gear
setup key that mints a goatnet-rung class undoes every peer-level correction at
the next enrolment
(F-434).

The confusion is not cosmetic. The *only* thing the second rung buys is
site-localized platform admin — `mesh-acl.md` says so plainly: `<SITE>-PLATFORM-ADMIN
→ <SITE>-INFRA` terminates inside the site and grants no administrative path to
the host site's core nodes. Without a written rule for when that boundary is
real, a prefix gets spent where no boundary exists, and dozens of groups and
policies are minted to express nothing.

## Decision

**A group prefix bounds a `PLATFORM-ADMIN`.** Stamp a subordinate scope when you
can name the person or team whose authority stops at that boundary. If you cannot
name them, do not spend a prefix.

That rule assigns every site:

| Rung | Prefix | Holds | Authority |
|---|---|---|---|
| Goatnet | `<GOATNET>-` | **every host site's** core / observability / backup / breakglass / relay-health nodes; goatnet-wide human roles; the class vocabulary | the platform team |
| Subordinate | `<SITE>-` | forward and tactical sites, exercises, and gear estates attached to a host site | that scope's own admin; northbound reach via the ADR 1124 `HOST:` rule |

**Host sites are goatnet rung, permanently.** A host site is inside the goatnet's
trust domain by construction, not by convention. A goatnet with two host sites
sets `inter_site_spock_topology: explicit` with the other host site as its peer:
each host site is a **Postgres multi-master write peer** of the other and already
holds the write path into every other host site's database. A separate ACL
prefix would assert a boundary the data plane does not have — the definition of
a control that reads as protection and is not.

**A site that needs its own trust domain gets its own goatnet, not a prefix.**
That path already exists (`partner-standalone-goatnet.md`,
Block 72) and is the answer for any site that needs its own trust domain, such as another sovereign's. Keeping it as the
escape hatch is what lets the rule above stay absolute.

**Naming form for new subordinate scopes: `<ISO3>-<letter>`** (for example
`XYZ-K`), matching the opaque-short-code convention already used for relay IDs.
An existing scope with an older name is grandfathered — a rename forks the
policy and relies on the orphan pass to catch the leftover (`mesh-acl.md`,
F-349), which is not worth paying for a naming aesthetic on a live site.

## Consequences

**`HOST:INFRA` means the whole backbone, not the nearest host site.** ADR 1124
resolves `HOST:` to the `site_prefix` of the scope's declared `host_site`, and
every subordinate scope declares the goatnet as its `host_site` — so every one
resolves to `<GOATNET>-INFRA`, which holds the core nodes of every host site. A
forward site's gear northbound rule therefore reaches the core nodes of host
sites other than its own on tcp 443/53/5353/7447. That is intended under
one-backbone-one-trust-domain, and it is written here because **it widens with
every host site added and nothing will announce it**.

**Per-host-site northbound narrowing is not available, and a label will not buy
it.** NetBird groups only union reach, so adding a location group beside
`<GOATNET>-INFRA` narrows nothing — the taxonomy's own label-group rule says the
same thing. Narrowing would require making host sites first-class scopes, which is
what this ADR declines; if a future scope genuinely must reach only its local host
site, that is a change to ADR 1124's resolution, not a new prefix rung.

**For backbone nodes the site dimension lives in the cert URI-SAN `site=` and the
hostname, not in a mesh group.** This is consistent with the grammar rather than a
gap in it: affiliation is already identity-only for the same reason. A group's job
is reach; a group that grants no distinct reach is a label, and the taxonomy
already forbids a label standing in for coverage.

**Each subordinate scope costs ~30 groups and ~50 policies.** At the ADR 1122
envelope this is the line item to budget, and it is the arithmetic reason the
rule refuses to spend a prefix on a boundary nobody administers.

**Teardown stops being guarded by a comment.** `netbird-acl-cutover.yml`'s
`mode=teardown` asserted only that the target is a declared scope row, while the
taxonomy comment said "never run this against an operational row". `kind:`
becomes required on every row and the teardown refuses `kind: operational`
mechanically.

**This rule does not by itself fix an existing mesh.** The fix lands on the
**setup key** before the peer: a gear key that mints a goatnet-rung class undoes
peer-level corrections at the next enrolment. An empty `<SCOPE>-INFRA` leaves the
site-admin rule granting nothing.

**The worked example.** A host site whose core nodes sit at the goatnet rung,
whose gear sits at its site rung, and where no machine holds both, is already
correct under this rule.

## Alternatives considered

**Every site gets its own prefix, host sites included.** Uniform, and no judgment
call at stamp time. Rejected on two counts. Mechanically, ADR 1124's `HOST:`
qualification is deliberately one-directional (subordinate → host) and cannot
express peer scopes, so N host sites need N² symmetric rules or a new mechanism
invented for the occasion. Substantively, it would draw an administrative boundary
between machines that are multi-master write peers of each other's databases — the
rules would then be written to re-unite exactly what the prefixes separated.

**All sites collapse to the goatnet rung; no subordinate prefixes at all.** This
is the other half of the operator's original question. Rejected: it discards the
only thing the rung buys — a site admin bounded to their own site — and it
re-creates the F-311 shape, because a forward gear box holding `<GOATNET>-KID`
becomes an any-port destination of `infra → all`, `p2p → all` and `platform-admin
→ gear`.

**Rename the goatnet prefix to free the slot for a site rung.** Renames every
group on a live production goatnet, forks every policy, and buys a naming
aesthetic. The slot is not scarce — the rule above is what was missing, not room.

**Leave it undocumented and decide per site.** This is the status quo, and the
table in Context is what it produces.
