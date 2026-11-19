# ADR 1105 — `PUBSUB-ORG-ADMIN`: multi-publisher topics (co-publish), single-owner roots inviolable

| Field | Value |
|---|---|
| Status | Accepted (design 2026-07-31, refined 2026-08-01; grammar + registry + ACL render + resolver + CLI built and ZAGROS live-fired 2026-08-01, then reverted to default-off. Amended 2026-09-04: the class segment is `multipub/`, was `shared/` — see "Amendment" below) |
| Amended by | [ADR 1251](1251-every-source-keeps-its-identity-through-a-pod-carriage-rules-provenance-at-entry-and-pod-local-authority.md) — a single-owner root may also be written by a pod its carriage rules allow |
| Date | 2026-08-01 |
| Repo scope | goat core — machine-plane write authz, org self-administration |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1102](1102-organization-owned-data-sharing.md) (the org authority `data.govern.org` this extends to writes) · [ADR 1074](1074-data-access-role-architecture.md)/[1075](1075-producer-data-labels-and-access-request-loop.md)/[1076](1076-access-language-and-machine-plane-parity.md) (the grammar + grant store + access language) · [ADR 1025](1025-zenoh-dynamic-tenant-authz-slot-pool.md) (the per-UUID slot-pool write ACL this renders into) · [ADR 1039](1039-slot-alias-display-layer.md) (display-only alias — how the org name reaches a human without entering the key) · [ADR 1051](1051-identity-descriptor-standard.md) (born-as vs currently-is) · ADR 1043 (ownership/affiliation never in the write anchor) · [ADR 1071](1071-sponsorship-authorization-at-onboarding.md)/[1072](1072-sponsor-scoped-onboarding-console.md) (at-most-own-scope, fail-closed) · [ADR 1022](1022-zenoh-denial-observability.md) (write-enforcement gated on denial observability) · design: [`pubsub-org-admin-co-publish.md`](../design/pubsub-org-admin-co-publish.md); findings F-156, F-177, F-258, F-272 |

## TL;DR (for a layperson)

By default every publisher writes into **only its own namespace** — and that one-writer-per-namespace rule is a feature, not a limit: it means anyone reading data under a namespace knows, for free, exactly who produced it. This decision keeps that rule **inviolable**. It does *not* let anyone add a second writer to an existing single-owner namespace. Instead it lets an organization **deliberately create a distinct, clearly-marked "multi-publisher topic"** — an object that is *born* multi-writer, is obviously not any one entity's private namespace, and is discoverable as such — and then add several of the org's own publishers to *that*. Provenance on every ordinary namespace stays perfect; "more than one writer" only ever happens on a topic that announces itself as multi-writer.

## Context

The ZAGROS router enforces publishing strictly (ADR 1025): deny-by-default, one pre-cut `slot-w-<anchor>` rule per identity granting write on exactly `<anchor>/**` (+ `volatile/` + `/@adv/`). **Every entity is confined to its own anchor prefix** — verified against the live `config.json5`. This one-writer-per-root property is *load-bearing for provenance*: a subscriber on `<anchor>/**` knows every sample there came from that one identity, with no per-message attribution needed.

The legitimate need is a topic with **more than one publisher inside one org** — a redundant/failover feed, a registry service, a same-org gateway. The wrong way to grant it is to reach into another entity's single-owner root and add a second writer: that silently destroys the provenance guarantee *on that root*, and it is invisible to a subscriber who reasonably assumed one author. An earlier draft of this ADR did exactly that (a write grant onto an arbitrary anchor/topic); it was rejected on review (2026-08-01) for that reason.

Three facts from the code shaped the final design:

- The org data-sharing authority already exists: `data.govern.org:<O>` (ADR 1102), registry-derived, at-most-own-scope, with a nomination surface. ADR 1102 rejected a bespoke parallel governance role.
- Grant rows already exist (ADR 1075) and are read live by the resolver — the natural record for "who may publish to this multi-publisher topic, since when, granted by whom."
- **Ownership must never be encoded in a key** (data-access-roles.md §1, ADR 1043/1051): a key-expr is the write-ACL match, the storage key, and the subscribers' subscription — all freeze-critical — while org ownership is mutable (reorgs, transfers). Encoding the org *name* in the key would make every reorg a fleet event and cannot even be parsed reliably (`example-vendor` vs `examplevendor`).

## Decision

**Single-owner `<uuid>/**` roots are inviolable. Multi-publisher writing happens only on a deliberately-created, discoverable, org-scoped multi-publisher class — `multipub/<org-pool>/<topic-id>/**` — whose authorized publisher set is recorded as write grant rows. `PUBSUB-ORG-ADMIN` is the operator-facing name for the write facet of `data.govern.org:<O>`.**

1. **No new authority; no new grammar token.** The authority is the write facet of `data.govern.org:<O>` (ADR 1102). `PUBSUB-ORG-ADMIN` names the capability across the role vocabulary, portal, CLI, and ACL rendering (owner-confirmed, 2026-07-31). It is a distinct, more-scrutinized, separately-audited action from the view grants the same admin issues — a separation of duties at the *action* level, not a second role.

2. **A multi-publisher topic is a first-class, marked, discoverable registry object — the `multipub/` class.**
   - Key-expr namespace: `multipub/<org-pool>/<topic-id>/**` (+ `volatile/` + `/@adv/` mirrors). The reserved `multipub/` segment is immutable and structural — the key *itself* tells any reader "do not assume one writer here," distinct from every opaque `<uuid>/**` single-owner root.
   - `<org-pool>` is a **stable, opaque, per-org handle** minted once (`org_multipub_pools`), resolved to the org — and, for humans, to the org's display name via the alias layer (ADR 1039). It is a *born-under* marker (ADR 1051): if a multi-publisher topic ever moves orgs, the registry re-points ownership and the key never changes. The org **name** is never in the key (the rejected anti-pattern above); the org is discernible by resolving the handle, which every discovery surface does.
   - `<topic-id>` is a stable opaque id; the human-legible topic name is a registry alias (display-only, ADR 1039), never in a machine path.
   - Creating a multi-publisher topic is a **deliberate** `PUBSUB-ORG-ADMIN` act (registration, justification-recorded), not a side effect of a grant.

3. **The guardrail is mechanistic: a co-publish grant can target only a registered multi-publisher topic.** Grant issuance refuses any target that is not a registered multi-publisher topic (`multipub/`-class) owned by an org the caller governs; the admin *cannot* point a co-publish grant at an ordinary anchor root. At-most-own-org, fail-closed: the multi-publisher topic's owning org and the publisher X must both resolve to an org the caller holds `data.govern.org:<O>` for. Cross-org co-publish is refused.

4. **The publisher set is grant rows (ADR 1075), action `write`.** A co-publish grant is `{grantee: principal:<X>, producer: <multipub-topic key-base>, action: write, granted_by, justification, effective, status}` — reusing the grant store (`no_third_shim`), so "who may publish here, since when, granted by whom" is a row read, effective-dated, revoke-not-delete, F-207-audited. It is **not** a `data.write.producer:<…>` sentence — the target is a multi-publisher topic, not a producer, and a `multipub/<pool>/<topic>` key-expr is deliberately not grammar-valid as a producer param. It is surfaced as **co-publish membership** of a multi-publisher topic, distinct from a producer write role.

5. **Enforcement is one additive ACL allow on the multi-publisher namespace, static and pre-cut.** For each in-effect write grant, `config.json5.j2` renders a `copub-w-<publisher>-<n>` allow (`put/delete/liveliness_token`, ingress) on `multipub/<pool>/<topic>/**` (+ mirrors), bound in an extra policy on the publisher's own `slot-<publisher>` subject. It never edits the publisher's own `slot-w` rule and never touches any `<uuid>/**` root. Render input is the default-off group_var `zenoh_copublish_grants`; a multi-publisher topic also renders its own rocksdb store (`zenoh_multipub_topics`), so it persists like any producer namespace. Static/restart-required; ungranted keys denied silently; ZAGROS-only.

6. **Discoverability is a first-class deliverable, not a nicety.** Every discovery surface renders a multi-publisher topic with its `MULTI-PUBLISHER` marker, its owning org (handle → name), and its declared publisher set: the portal catalog + `GET /api/access/multipub-topics`, `goat access co-publish list-topics` / `goat access topic`, and the COP note. So an outsider discovers, in one look, "multi-publisher topic • owned by Acme • writers X/Y/Z" — the discernibility the one-writer default otherwise gives for free.

7. **Assignment and backfill reuse ADR 1102.** The `PUBSUB-ORG-ADMIN` capability = holding `data.govern.org:<O>`, already retroactively bootstrapped (`member-of(O) ⇒ data.govern.org:O`, narrowing to nominated admins). No separate migration; onboarding assigns the capability by nominating a govern-admin. Nothing is pre-created — default-unchanged until an admin creates a multi-publisher topic and grants a publisher.

## Alternatives considered

- **Write grant onto an arbitrary anchor/topic (the earlier draft).** Rejected 2026-08-01 — it adds a second writer to an existing single-owner root, silently destroying that root's provenance guarantee. The whole value of one-writer-per-root is what this ADR now protects.
- **A separate org authority (`data.copublish.org` + its own nomination surface).** Rejected — re-creates the "bespoke governance-role beside the grammar" ADR 1102 rejected; the write facet of `data.govern.org` covers it, separation preserved at the action level.
- **The org name in the multi-publisher key (`multipub/acme/<topic>/**`).** Rejected — the codebase's own anti-pattern: mutable ownership frozen into a freeze-critical key (reorg = fleet event) and not reliably parseable. The stable opaque `<org-pool>` handle + registry/alias resolution gives org-discernibility without it.
- **A dedicated multi-writer *anchor* (a UUID slot) with the marker only in the registry.** Viable, but the key looks identical to a single-owner root, so discernibility rests entirely on a registry lookup. The reserved `multipub/` class makes the distinction structural — a raw-key reader sees it immediately — which is the stated goal.
- **Surfacing co-publish as `data.write.producer:<multipub-key>`.** Rejected — conflates a multi-publisher topic with a producer, and the slash-bearing key-expr is not a valid grammar param; co-publish is its own membership concept.
- **Live ACL reload / portal live-append on ZAGROS.** Impossible/unsafe; the group_var is the render input, the portal is the record + operator workflow.

## Consequences

- **Provenance is preserved everywhere it mattered:** ordinary `<uuid>/**` roots are never opened; "more than one writer" is confined to a class that announces itself. Attribution on a multi-publisher topic is knowingly many-to-one — a subscriber reads `SourceInfo`, not the key — and the best-practices doc (Hello Galaxy `hello-galaxy-zenoh-patterns`, `docs/co-publish.md`) covers when to create a multi-publisher topic, when not, and the audit/accountability guidance that ships with the capability.
- New registry objects: `org_multipub_pools` (org → stable pool handle) and `multipub_topics` (the marked, org-scoped, discoverable topic). The resolver surfaces a principal's co-publish memberships; the COP invariant note becomes "publish own anchor only — the sole multi-writer case is an explicitly-declared `multipub/` topic (ADR 1105)."
- Every create/grant/revoke emits `OperatorActionV1`: `pubsub.multipub-topic.create` / `pubsub.write-scope.set` / `.revoke`.
- **Built (2026-08-01 track):** the `org_multipub_pools`/`multipub_topics` registry + CRUD; topic create/list endpoints; the reworked at-most-own-org grant gate (target must be a registered multi-publisher topic); the resolver co-publish-membership surfacing + COP note; the additive `multipub/`-targeted ACL render + per-topic storage + `render-check.py` guardrail checks (single-owner roots never a target); `goat access co-publish` (create-topic/list-topics/grant/revoke/export) with audit; the SoT/vocabulary rows; the best-practices doc.
- **Live-fired on ZAGROS 2026-08-01** (track `zagros-copublish-livefire`, portal pin `sha-3ccf51b`), then reverted to the default-off steady state. All four gold-standard properties held on the live router: a granted co-publisher's sample landed, an ungranted writer was silently denied, a co-publisher still could not write another peer's own root, and ordinary own-namespace publishing was unaffected. **Still deploy-pending:** the portal→group_var materialization (v1 renders from the group_var; auto-export is the named seam). Panoscope's `MULTI-PUBLISHER` badge is a sibling-surface follow-on.
- **B2.1 interaction:** co-publish is write-side, at-most-own-org, on the `multipub/` class; the partner read partition + release lanes are read-side and cross-org. They share the ACL template but never a rule set; no conflict.


## Amendment — 2026-09-04: the class segment is `multipub/`, not `shared/`

**The decision above is unchanged. Only the reserved segment's spelling changes:
`shared/<org-pool>/<topic-id>` becomes `multipub/<org-pool>/<topic-id>`, and the
registry tables `org_shared_pools`/`shared_topics` become
`org_multipub_pools`/`multipub_topics`.**

**Why.** This ADR's stated reason for reserving a segment at all is that it is
*structural* — "the key *itself* tells any reader 'do not assume one writer
here'". `shared/` defeated that, because by 2026-09 the repo was using "shared"
for two unrelated things:

| Sense | Where | What a reader must conclude |
|---|---|---|
| Sharing **write rights** among our own orgs | this ADR, the co-publish class | several of *our* orgs deliberately co-publish here |
| Sharing **data** across an organisational boundary | cross-sovereign federation (ADR 1157) | one writer, *not* ours, origin unverified |

A raw-key reader who cannot separate those two has lost exactly the property
the reserved segment exists to provide. `multipub/` names the one thing this
class actually asserts — more than one publisher — and collides with nothing.
The federation side of the same collision was resolved independently and in the
same direction: its ingress prefix is `from/<sov>/`, never `shared/`.

**Why it was safe to do as a rename rather than a migration.** Measured against
both live goatnets on 2026-09-04 before any edit:

| Surface | zagros | zagros-prod |
|---|---|---|
| `org_shared_pools` rows | 1 (the live-fire pool) | 0 |
| `shared_topics` rows | 1, status `retired` | 0 |
| Write grants targeting the class | 0 | 0 |
| `shared/` in the rendered router ACL | 0 | 0 |
| `zenoh_copublish_enabled` | false | false |

Nothing was serving the class on either goatnet: the live-fire had been reverted
a month earlier, its topic retired, and no grant survived. The one live-fire
artifact — a `multipub_topics` row, its pool row, and an orphaned rocksdb store
holding a single test sample — was swept as debris in the same change rather
than migrated. **The audit event recording its creation was deliberately left
untouched**: `data.shared-topic.create` on target
`shared/f028d43c79196128/4cdbbfdc23d9c2f8` is an immutable record of an act that
genuinely happened under the old name, and rewriting history to match a later
rename is the wrong instinct (ADR 0811). A reader who meets that event should
read it as this class under its former spelling.

Had a live topic or an in-effect write grant existed, this would have been a
data migration needing its own plan and an operator window — not this
amendment.

**Consequences.**

- `POST`/`GET /api/access/shared-topics` become
  `POST`/`GET /api/access/multipub-topics`. This is a breaking API change and is
  taken deliberately: the only client is `goat access co-publish`, renamed in
  the same change, and no other caller exists.
- The audit action `pubsub.shared-topic.create` becomes
  `pubsub.multipub-topic.create` (`data.shared-topic.create` →
  `data.multipub-topic.create` on the portal side).
- The group_var `zenoh_shared_topics` becomes `zenoh_multipub_topics`; the
  rendered store id `st-shared-<pool>-<topic>` becomes
  `st-multipub-<pool>-<topic>`, derived from the key as before.
- Every goatnet's `zenoh_copublish_enabled` was already `false` with empty
  inputs, so the rendered ACL is byte-identical across the rename.
- The revert-leaves-the-store behaviour found while verifying the above is
  tracked separately as F-447;
  it is a lazy-storage retention gap (ADR 1101), not a co-publish defect.
