# Data-access roles — hierarchical, composable, two-plane conversant

| Field | Value |
|---|---|
| Status | Design canon for [ADR 1074](../adr/1074-data-access-role-architecture.md). SoT: [`ops/permissions/data-access-roles.yaml`](../../ops/permissions/data-access-roles.yaml). v1 enforcement built in panoscope this track; build-out map §6. |
| Scope | The missing dimension rbac-adherence.md §3.3b named: what fabric **data** a principal may see/write, as a role architecture spanning both identity planes (Ory humans ↔ x509 certs) with explicit interchange. |
| Related | [ADR 1073](../adr/1073-rbac-adherence-principle.md) (enforcement discipline this fills with policy) · ADR 1043 (identity grammar; affiliation earmarked for read-scoping) · [ADR 1025](../adr/1025-zenoh-dynamic-tenant-authz-slot-pool.md) (wire posture) · [ADR 1027](../adr/1027-partner-release-set-governance.md) (releasability — the reserved `releaseset` scope) · [ADR 1051](../adr/1051-identity-descriptor-standard.md) (the registry hub the planes join at) · [ADR 1067](../adr/1067-identity-grammar-generated-sot.md) (generated-SoT pattern this follows) |

## 1. The model in one figure

```
                       data.view.all                      ← apex (admin sees all;
                      /      |       \                       belt-and-suspenders: G5)
                     /       |        \
        data.view.org:O   data.view.notional   [reserved: site/echelon/
              |            (default to all        releaseset/coalition]
              |             authenticated)
     data.view.producer:A
              |
        [reserved: topicclass/topic]

  Cross-axis implication (declared, never named-in):
     ADMINS ⇒ data.view.all
     (IAM-ADMIN deliberately carries NO implication — ratified 2026-07-09:
      identity administration needs the access COP, not standing
      all-mission-data view; the matrix grants it panoscope `scoped`)
  Registry-derived (automatic, atomic; evaluation = the resolver, §6 item 1):
     member-of(O) ⇒ data.view.org:O      custody-of(K) ⇒ data.view.producer:K
     cert anchored at K ⇒ data.write.producer:K
```

The three v1 access modes are the ratified baseline requirement: **per real producer** (`data.view.producer:<key>`), **per notional, default-on for everyone** (`data.view.notional`), and **admin sees all** (`data.view.all`). Everything else is the bones for extension.

**Producer keys are what the registry actually holds, in both deployed shapes:** the seed model keys producers by kebab vendor/org prefix (the live ZAGROS population — `org-a-user-1`, `org-b-user-2`), the ADR 1025 slot-pool model by 32-hex anchor. One param charset (`[a-z0-9][a-z0-9-]{0,63}`) covers both; the scope word plus the registry disambiguate. (The first-pass hex-only producer regex could not name a single live ZAGROS producer — checked against real roster rows.)

**Why ownership is never in the qualified name** (`producer:<key>`, not `producer:<org>/<key>`): names must outlive ownership. A producer's key is its cert identity, its write anchor in the *static restart-required* router ACL, its topic prefix, and its storage/history key — all freeze-critical. Ownership is a *mutable* relationship (reorgs, RIP/TOA, re-subordination; orgs nest, so any qualified form encodes a whole mutable parent chain). Encoding it would turn every reorg into a fleet event (cert re-mint + ACL re-cut + namespace migration + subscriber breakage); keeping it in the registry makes a reorg one effective-dated row update. This is the standing ADR 1043 invariant (affiliation NEVER in the write anchor) and the ADR 1051 split (cert = born-as; registry = currently-is) applied to data roles. The accepted price is the resolver dependency: org scoping needs a lookup, not string surgery — which is also mandatory for correctness, since key text cannot be parsed into an org join (`acmecontrols` vs `acme-controls` are distinct orgs; the dash in `org-a-user-1` is not a path separator). Humans get readable names via the alias display layer (ADR 1039), which is likewise banned from every machine path.

## 2. Design requirements (as testable properties)

1. **Two-plane conversance.** One vocabulary names rights in both planes; each role row declares its enforcement point per plane; the registry is the translation layer (§5).
2. **Orthogonality with declared coupling.** Data-access is its own axis: an observer (no platform rights) can hold `data.view.all`; a platform admin *implies* `data.view.all`; a developer *implies* view of their own data. All three are implication edges in the SoT, never naming conventions.
3. **Atomic parametric roles.** Minting a producer creates `data.view.producer:<anchor>` and its org roll-up *by construction* — the roles are **virtual**, derived from registry rows, so existence is atomic and nobody hand-mints instance roles (the classic sprawl source).
4. **Belt-and-suspenders apex.** `data.view.all` is evaluated BOTH as a wildcard predicate and as the expansion of every sub-role; each mechanism covers the other's failure (a new producer minted mid-session is caught by the wildcard; a wildcard bug is caught by the expansion).
5. **Extension bones without sprawl.** New levels (above org: site/echelon/releaseset/coalition; below producer: topicclass/topic) are *reserved* scope words that land by ADR — the grammar admits them, CI rejects them until ratified.

## 3. Best-practice grounding (what we adopted, from where, and what we rejected)

| Pattern | Source | How it lands here |
|---|---|---|
| Core + hierarchical RBAC (roles, role inheritance) | NIST/ANSI INCITS 359 (RBAC0/RBAC1) | the lattice + implication DAG; seniors inherit juniors, grants never copy permission lists |
| Parametric/relationship roles — a role is a (relation, object) pair, not a hand-minted name | Google Zanzibar / ReBAC (SpiceDB, OpenFGA) | `data.view.producer:<anchor>` is a relation on a registry object; **virtual**, derived not stored — this single choice kills the biggest sprawl vector and gives the atomic auto-establishment requirement for free |
| Role aggregation (a super-role composed from labeled sub-roles at evaluation time) | Kubernetes aggregated ClusterRoles | the belt-and-suspenders apex: `data.view.all` = wildcard ∧ aggregation |
| Attribute-hybrid: enumerable roles for the common shape, attributes for cross-cutting classes | NIST SP 800-162 (ABAC guidance) | `notional` is an attribute-derived class (the producer's `synthetic` self-declaration), not an enumerated org — one role covers a fleet |
| Closed vocabulary, deprecate-never-delete, every value carries governance metadata | SKOS discipline already in `ops/identity/grammar.yaml` (ADR 1067) | grammar regex + `ratified_by` + G6; issued sentences valid forever |
| Role-explosion antidotes: naming grammar, owner per role, periodic recertification, no role without a consumer | enterprise IAM failure literature (role mining/engineering) | G1 (must parse), G8 (named ratifier), **G7 (no role without an enforcement point — our own F-132 lesson formalized: vocabulary that gates nothing is rejected at CI, not discovered by audit three months later)** |
| Policy decision point separated from enforcement points; policy as data | XACML/PDP-PEP model, without the central-server failure mode | the SoT file is the PDP *input*; evaluation is in-process at each PEP (app filter, ACL render) — no central authz service to survive island mode (brokerless discipline applied to authz) |
| Warehouse RBAC discipline: grants only to roles, role hierarchy, enumerable SHOW-GRANTS-style review, future grants (new objects auto-covered), owner-managed access, tag vocabularies, SECURITYADMIN≠SYSADMIN duty separation | Snowflake's access model (adopted as the reference model, 2026-07-09) | all mirrored: org-scope grants / the lattice / grant rows read-not-simulated / virtual roles / producer-approved access / the label lanes / the IAM-ADMIN split. Deliberately NOT mirrored: row-access & masking *policy expressions* (rule-based, un-auditable by enumeration; no per-query engine exists to evaluate them — streams, not tables; per-message grain = the marking-burden failure) and the central query engine itself (brokerless fabric ⇒ the §10 staged enforcement backends) |

**Rejected:** a central PDP service (broker-shaped dependency; island-mode liability); per-human wire certs (humans aren't wire peers; F-181 makes attribute ACLs impossible without a fork); free-form Ory groups as the role store (the sprawl disease itself); storing parametric role instances (drift + orphan roles — derivation makes both impossible).

## 4. The grammar and the sprawl guards

A role is a **sentence** in a closed grammar: `data.<action>.<scope>[:<param>]` — see the SoT for the regex and G1–G8. The grammar is deliberately not the action×scope cross product: `write` pairs only with `producer` (a `data.write.all` would be a sentence with no role row and no enforcement point — G7 applied inside the regex, CI-probed, so undefined vocabulary cannot even ride the transport). The two axes never share a namespace: platform groups stay UPPERCASE legacy (`ADMINS`), data roles are lowercase dotted (`data.view.org:org-b`); both travel in the same Ory `groups` claim — whose identity-schema item pattern admits exactly these two shapes (§5) — mechanically distinguishable, CI-validated against their respective SoTs. A grant that doesn't parse cannot be issued; a role no plane enforces cannot be ratified; a cross-axis link that isn't a declared implication edge is a violation regardless of who asks.

## 5. Two-plane enactment and the interchange logic

The same role means the same right everywhere; *how* it is enforced differs per plane, and each role row declares it:

| Role | Human plane (Ory) | Wire plane (x509) |
|---|---|---|
| `data.view.all` | app viewer-scope **DataAll** (everything but the platform carve-out, SoT `boundaries.platform_only`) | not enforced — reads are flat (ADR 1025); recorded in the SoT's dedicated **`wire_read_posture`** section, deliberately *not* as a grant cell: the wire enforces no view-scoping, and writing "every cert holds view-all" into a role row would normalize what should stay alarming |
| `data.view.org:O` / `producer:K` | app viewer-scope **Scoped** to those keys | not enforced (reads flat) |
| `data.view.notional` | **include_notional** — nodes, edges, samples, AND live ws events of producers self-declaring `synthetic: true` | not enforced |
| `data.write.producer:K` | n/a | **the router ACL slot** — the cert's write anchor is the role credential; the ACL render derives from the same registry rows |

**The interchange is the registry** (the org hub, ADR 1051): `roles_of(email)` = explicit claim grants ∪ implications(membership, custody edges) ∪ defaults; `roles_of(anchor)` = write role ∪ implications(org rollup). Two entry points, one resolver, one hub — no string translation between planes is ever needed because both planes' principals resolve to rows in the same registry, and the roles are those rows read through the grammar. **Claims stay small by design:** they carry only *explicit extra* grants (e.g. `data.view.all` handed to an observer); the automatic majority derives at evaluation. This is the anti-bloat answer to the multi-cert/headless-cert cardinality question — no per-cert human mapping exists anywhere.

**Claim delivery (a transport constraint the initial design missed):** Kratos pattern-validates every `traits.groups` item, and the site identity schema originally admitted only `^[A-Z][A-Z0-9-]{0,40}$` — every lowercase `data.*` value was structurally rejected, making the whole human plane undeliverable. The schema (`ops/ansible/roles/ory/templates/ory/identity-schema.json.j2`) now carries the data grammar as an explicit second alternative, held in lock-step with the SoT pattern by `check-data-access-roles.sh`; redeploying the schema is part of the per-goatnet live-fire. The operator-manifest pipeline still cannot carry `data.*` values (its `ory_site.groups` enum is exact-matched to the platform matrix by `check-ory-groups-enum.sh`, deliberately) — explicit grants ride Kratos identity edits until the manifest `data_access:` field lands (§6 item 2).

## 6. Built now vs. build next (honest map)

**Enforcing today (this PR):** panoscope evaluates the v1 lattice — `data.view.all` (and the ADMINS platform implication) → DataAll/Full; `data.view.org:`/`data.view.producer:` claim values → Scoped keys; `data.view.notional` default-on — the WHOLE synthetic fleet: nodes, edges, samples, and live ws events of `synthetic`-flagged producers (the second pass closed the half-grant where only nodes rendered); everything else → floor. The wire write role is *already* enforced (the slot ACL predates this doc — the row just names it in the shared vocabulary). CI guard validates grammar / G7 / DAG acyclicity / code-constant + boundary drift / Kratos-schema deliverability / SoT↔matrix implication parity.

**Honest v1 gap — org scope on the live population:** seed-model goatnets (ZAGROS today) key topics by *member producer* prefix (`org-a-user-1/**`), not the org rollup (`org-a/**`), so until the resolver lands an org value alone typically matches nothing there; grant explicit `producer:` values in the interim. The SoT's registry-derived implication rows carry `status: resolver-pending` for exactly this reason — declared policy, not yet evaluated by any code.

**Build next, in order:**
1. **Registry resolver** (portal): `GET /api/access/data-roles?identity=<uuid>|email=|anchor=` returning the derived role set — makes org→member-producer expansion real and replaces claim-carried org values as the primary path. The sponsor-door resolve pattern (ADR 1072) is the template. **Built (2026-07-09, ADR 1074 addendum):** the endpoint keys on the identity UUID (email is a resolved alias), expands org→member producers with the vertical parent-containment roll-up below, unions multi-org membership, and serves registry-derived roles only; the parent roll-up landed as a SoT implication row; grant-row evaluation (ADR 1075) is the named extension point, deferred until a grant store exists. **Principal-key discipline (ADR 1051/1069):** the resolver keys on the identity UUID; `email=` is a lookup *alias* it resolves to the id first, never a stored join — emails change, and external subjects (EDIPI, PKI DNs, pairwise OIDC subs) are mapped attributes over the stable anchor, never keys. The existing email-keyed joins (roster `email_hash` reconciliation, COP `roles_of(email)`) are inherited drift from before this discipline was stated; migrate them to id-keyed with email-alias lookup as they are touched. **The resolver must also declare the org-hierarchy rollup rule explicitly:** orgs nest (ADR 1043 `parent` edges), so whether `data.view.org:<parent>` covers child orgs' producers is a policy choice, not an implementation detail. Recommended stance: transitive through vertical `parent` containment only; never through lateral/time-bounded relationship edges (`comrel`, `supports`, …) — the reserved `data-authority` edge (identity-grammar §3.2) is the designed hook for cross-org data rights when that need arrives. Ratify as a SoT implication row + ADR note when the resolver lands; until then org values cover exactly one org node.
2. **Onboarding grant surface**: intake/manifest field for explicit grants (`data_access: [data.view.all]`), operator lever (`goat admin data-access <id> --grant …`), F-207-audited.
3. **Flashlight/stethoscope** adopt the same evaluation (hoist the evaluator to `panoscope-core` at second consumer, per the existing tee-up).
4. **Reserved scopes** land by ADR when needed: `releaseset` (binds to ADR 1027 and becomes the wire-read narrowing via key distribution), `coalition` (the member-lane scope — its eligibility predicate is `affiliation.coalition`, ADR 1077 §"Data-access coupling": a `coalition`/`releaseset` grant to a framework's member release-lane is admissible only for a grantee org whose current standing is `member` of the goatnet's active framework; a `partner`-standing org gets bilateral/coalition lanes only. The predicate is **framework-parameterized** — the same mechanism serves NATO, a regional coalition, or any AOR's framework, since standing is resolved against the goatnet's active framework, not a hardcoded alliance. Standing is nation-derived + effective-dated, so the check reads the current registry value, not a frozen cert — recognition ≠ authorization still holds, the grant is still a deliberate recorded act, standing only bounds *who is eligible* to receive it. The related `affiliation.sector` (mil vs law-enforcement vs civil-government) is the reserved hook for interagency/authorities scoping), `site`/`echelon` (identity-grammar dimensions projected into data scopes), `topicclass`/`topic` below producer.
5. **grammar.yaml integration** (ADR 1067): this SoT becomes a generated dimension alongside the identity vocabularies.

## 7. Worked examples (the defining cases)

- **Observer with data-view-all:** browse-tier human, no platform groups, granted `data.view.all` explicitly at onboarding → panoscope DataAll (sees every producer, real + notional; not platform audit telemetry). Can administer nothing.
- **Platform admin:** `ADMINS` → implication → `data.view.all`, belt-and-suspenders evaluated. Sees everything including platform telemetry (Full).
- **Identity administrator:** `IAM-ADMIN` → NO data implication (ratified narrowing, 2026-07-09): surface entry (`scoped`) + the notional default + whatever `data.*` values they are explicitly granted. Access questions go through the access COP, which is their actual tool; standing all-mission-data view is an identity-owner re-widening decision, not an inheritance.
- **Developer at org O with producer A:** custody edge → `data.view.producer:A`; membership → `data.view.org:O`; default → notional. Sees own org + the synthetic fleet + infra/health. Their *process* bearing cert CN=A holds `data.write.producer:A` at the router.
- **New producer B minted under O:** `data.view.producer:B` exists at mint (virtual, G3); O's members see it with zero grant churn (org roll-up implication) — the atomic-establishment requirement, satisfied by derivation.

## 7a. Nationality narrows an org grant; it never widens one (ADR 1171, proposed)

An org grant names an organisation, and the audience it opens is that organisation's members rolled up through vertical containment. Nothing in that derivation looks at who the members are.

Once a person carries their own citizenship (`identity.nationality`) distinct from their organisation's controlling nation (`affiliation.nation`), the rule is: **access is the intersection of what the organisation may receive and what the person may receive.** An org grant opens a lane; a member's own nationality can only narrow what they get from it, never widen it. A grant to an organisation is never a way to reach a nationality the grant would not have been given to directly.

Two consequences worth stating rather than leaving implicit:

- **A liaison officer does not inherit their host's lanes.** `affiliation.relationship: liaison` means attached from outside; their reads are an explicit grant. Today an attached officer inherits the host organisation's read lanes wholesale, because membership is membership.
- **No new enforcement plane is required.** A read lane already rides the person's own slot uuid (`ops/goat-zenoh-portal/internal/access/partnerblocks.go`), so the fabric can already distinguish two members of one organisation. What is missing is the identity fact that would drive it — not the mechanism.

**Not built; buildable, and the rule is decided.** The tautology that blocked it is gone — an anchor's nationality is the holder's own, not a copy of its organisation's. The rule, ratified 2026-09-07: a holder earns a wrap slot iff they are a member of the grantee org **and** their own nationality is one of the nations that own it. `affiliation.relationship` is not consulted; a Canadian at a US-owned command is excluded because CAN is not USA, and a US officer there is included for the same reason.

What remains is data, not code: `identity.nationality` is blank for every member and `composition` is undeclared on all three registries, so a fail-closed intersection would wrap nothing for anyone today. Scope, the two escape hatches, and the one question still open — whether a multinational org's ownership means every constituting nation — are in `backlog.md` under `grant-audience-nationality-intersection`.

## 8. Command relationships and echelon — the military-reality mapping

Real force employment reassigns units constantly (OPCON/TACON), and echelon determines legitimate breadth. The identity canon already models both — this section declares how data view relates to them, so the stance is policy, not resolver improvisation:

- **Echelon** is the reserved scope between `all` and `org` — the anticipated "levels between admin and org/producer," deferred by design. When ratified by ADR, upper rungs (COMMAND/COMPONENT) project into broad view scopes without holding `all`. Not minted until an enforcement point exists (G7).
- **Comrel (OPCON/TACON) prompts a grant, never derives one.** (Mechanically, that prompt now has a concrete shape: a §9 grant row bounded to the comrel window.) Command edges (`comrel`, `c2-authority` — time-bounded, effective-dated registry attributes per ADR 1051) confer **zero** data view by inference. The gaining commander's real need is met by the owning side's **`data-authority`** issuing an explicit grant — a release-set membership (ADR 1027) or an org/producer grant — **bounded to the comrel window**, with expiry riding the assignment's effective-dated attributes. Three canon reasons: releasability/sovereignty (a partner unit OPCON to another nation's command must not auto-open its national data; recognition ≠ authorization, federated-trust-model.md), the deliberate `comrel`≠`data-authority` edge split (identity-grammar §3.2), and auditability (a granted release leaves a decision record; an inferred one leaves none). Establishing a comrel may *trigger the workflow* (a prompt to the data-authority); it never short-circuits it — that one deliberate step **is** the sovereignty seam.

## 9. Producer data labels + the discover→request→approve loop (ADR 1075 — declared, build-sequenced)

Ratified 2026-07-09, this section adds the data-side half of the model: producers label their streams, everything is discoverable, access to restricted streams is requested and producer-approved. Codified after a dedicated research round (below). SoT: the `data_labels` + `grants` sections of `ops/permissions/data-access-roles.yaml`.

> **Grants carry an action — view or write (ADR 1105).** The grant rows here were originally view-only. A grant row now carries an `action`: `view` (this loop) or `write` — a **co-publish** membership. Its invariant matters: single-owner `<uuid>/**` roots are **inviolable** — a write grant never adds a writer to one. Instead the org's `data.govern.org` admin (the `PUBSUB-ORG-ADMIN` capability, ADR 1102's write facet) **deliberately creates a marked, discoverable, org-scoped MULTI-PUBLISHER topic** (`multipub/<org-pool>/<topic>`) and a write grant adds a publisher to *that* — the grant's `producer` holds the multipub-topic key-base. It is **not** a `data.write.producer` role (a multi-publisher topic is not a producer, and the slash-bearing key is not a valid grammar param) — it is surfaced as a distinct co-publish membership. This is the one grant that *is* pre-cuttable into the static write ACL (one additive `slot-w`-style allow on the multipub-topic key; unlike read pairs, §11). At-most-own-org, fail-closed. Full model: [pubsub-org-admin-co-publish.md](pubsub-org-admin-co-publish.md).

**Three closed lanes, topic-namespace grain** (never per-message — the marking-burden evidence):

| Lane | Values | Default | Role |
|---|---|---|---|
| `fidelity` | notional · simulated · real | *unlabeled* (rendered honestly) | advisory + display; subsumes `synthetic: true` (notional), additive migration |
| `context` | experiment · exercise · operational | *unlabeled* | advisory; `operational` exists on every goatnet for parity but belongs on operational fabrics — elsewhere it surfaces a loud wrong-fabric flag |
| `access` | community · org · controlled | portal default `data_label_access_default` | **not an access control** ([ADR 1208](../adr/1208-sovereign-environments-release-namespaces-and-grant-encryption.md)): a stored catalog value that nothing reads to allow or refuse access; undefined for now, possibly relevant to a future consumer data persistence system |

**The loop:** every topic's catalog metadata (existence, labels, producer, org) is visible to every authenticated viewer — ADR 1028 already decided visibility is advisory, never access. Any viewer requests; the **producer approves** (custodian humans/org — the §8 data-authority principle); approval mints a **grant row** in the registry: `{grantee: org|principal, producer, granted_by, justification, effective [from,to), status}` — F-207-audited, expiry-able (the comrel-window pattern), org-scope preferred at the coalition boundary. The data-roles resolver evaluates rows; surfaces cache registry snapshots with declared staleness; who-can-see-X is answered by *reading rows*.

**Grants name the audience.** A grant row records who may receive a producer's data; the key service seals to that audience (§10 M2).

**Research grounding (deep-research round 2026-07-09; every claim adversarially verified 3-0 against primary sources unless noted):**

| Design choice | Verified evidence |
|---|---|
| Tiny closed lanes, machine-validated | NIST SP 800-162: the finite well-known marking set is the tractable pattern; ABAC attribute/policy management cost can exceed benefit; ABAC adoption failed its own forecasts (NCCoE SP 1800-3a; Gartner 70%-by-2020 never happened) |
| Default open; label only restriction; expect defects | DODIG-2013-142: 70% of 220 documents had marking discrepancies; GAO-06-706: 84% of 111 had marking errors; labelers ratchet UP under doubt (>60% encounter conflicts, majority pick the higher label); SP 800-178/NGAC: universal mandatory marking is the burden, label-the-restricted the economy |
| Friction on restrictive application | DODIG: the sharing damage came from erroneous *restrictive dissemination markings*; CDSE doctrine: NOFORN misuse obstructs allied collaboration; the Navy's fix was a justification gate |
| Grants = enumerable relations, producer-administered | SP 800-178: rule-formula policy is NP-complete to review and its delegation model cannot govern federated administration; relation representations review efficiently and delegate per-element |
| No central PDP; cached snapshots at surfaces | SP 800-162 §3.3.1: local attribute caching is the disconnected/tactical pattern, staleness the accepted declared risk; XACML's centralized pipeline degrades with policy count |
| Org-scope grants at the coalition boundary | SP 800-162's one proven ABAC benefit: the unanticipated external user; RAND RR-302: AMN interoperability required *relaxing* restrictive regimes; fragmenting across many security domains reduces interoperability |

**Honesty ledger:** the panel *refuted* (0-3) the attractive claim that permissive defaults *outperformed* fine-grained restriction on AMN — the community default rests on ZAGROS's sandbox purpose + labeling economy, not that claim. And the research produced **no surviving verified evidence** for: cloud tag-explosion at scale, data-mesh approval-workflow bottlenecks, DDS Security field lessons, IDS/Gaia-X deployment reality. Those transfer by analogy only; open questions recorded in ADR 1075. The design compensates structurally: closed lanes cannot explode; enumerable expiry-able grants counter stale entitlements; the request loop reuses the proven sponsor-approval shape (ADR 1071/1072) rather than a new workflow engine.

**Built vs next:** the portal half is now BUILT-IN-CODE (2026-07-09, `ops/goat-zenoh-portal`), deploy-pending. Built: producer label lanes with the justification-on-restriction gate (`PUT/GET /api/orgs/{vendor}/labels`, optional block at registration, `synthetic→notional` migration); the per-goatnet `data_label_access_default` knob (portal flag + ansible role default community); the discover→request→approve loop and preemptive grants (`/api/access/requests*`, `/api/access/grants*`, `/api/access/directory`) with producer-custodian-or-operator gating; the grant store (effective-dated, revoke-not-delete, F-207-audited); resolver grant-evaluation folded into `GET /api/access/data-roles` (in-effect rows join a principal, org grants expand to members); the restrictive-share drift counter (`GET /api/access/label-drift`); and a minimal UI at `/data-access.html`. Next: the ZAGROS portal redeploy live-fire (follow-ons §1) and panoscope label badges incl. the wrong-fabric flag (§0c.6, a sibling surface). Until the redeploy, the lanes are enforced in code but not yet live-fired — stated per lane in the SoT (G7 discipline).

## 10. The two planes share one policy — and the machine plane has a staged plan

The grants and labels above are **plane-neutral policy**. What differs per plane is the enforcement backend, and the machine plane's backends arrive in stages (decided 2026-07-09: the model accounts and plans for both planes even though they are not built in one pass). SoT: the `wire_read_posture.plan` section.

**The plane × operation matrix — never collapse these four cells into one word:**

| | WRITE | READ |
|---|---|---|
| **Machine (x509, wire)** | **Enforced per-identity, live** — the router ACL lets a cert publish only under its own producer key (ADR 1025; predates this track; now named `data.write.producer:<key>` in the shared grammar) | **Membership-gated, not per-stream** — any *enrolled* cert subscribes to any non-carved stream; one deny carve exists (`ops/audit/operator/**`, M-068). Per-stream scoping = the M1/M2 stages below |
| **Human (HTTP surfaces)** | n/a (humans don't publish; processes do) | **Enforced per-stream, built this track, live-verified** |

Saying "the wire is open" without the word **read**, or "the developer is governed" without the word **write**, is exactly the collapse that manufactures contradictions. Three cells are enforced today; machine-read is the single staged cell, and it is never open to the world — only to enrolled members.

**Be precise about who the surface layer binds.** Wire readers are enrolled machine principals only — mesh membership plus an issued cert, both revocable, the population enumerable from the registry, universal observers badged on the COP. Humans are browser-only *unless issued a cert*. So:

- For the **browser-only majority** (most onboarded humans — analysts, leadership, most partner staff), surface enforcement is the real and complete boundary: they have no other path to fabric data.
- For a **cert-holding developer**, surface scoping governs her *screen*, not her *code*: her process can subscribe to any non-carved stream today. For her, the binding controls are admission + revocation + observability (M0), the on-demand wire carves (M1), and eventually keying (M2). Any narrative that implies her scoped browser view also constrains her cert is wrong and must not be written.

**The stages — one policy, three backends:**

| Stage | Mechanism | Status |
|---|---|---|
| **M0 — now** | Admission control: mesh + per-identity certs, mesh-first instant revocation; enumerable reader population; universal-observer badges; audit trail | live |
| **M1 — on demand** | Static wire read-carves for a named prefix: a restricted prefix readable only by enumerated grant-holding CNs. Proven mechanism — `ops/audit/operator/**` is deny-carved on the wire today; ADR 1025 anticipated "a handful of fixed tiers." Restart-per-change ⇒ only for slow-changing, genuinely sensitive streams | available; unused beyond the audit carve |
| **M2 — end state** | Payload encryption / release-set keying — **mechanism fully worked in grant-keyed-encryption.md + [ADR 1094](../adr/1094-grant-keyed-payload-encryption.md)**: envelope encryption (one AES-GCM data key per release-set/generation, wrapped per member to their SPIFFE/cert key, ADR 0412), the wrapped-key bundle published over Zenoh (`keys/<releaseset>/<gen>` — no key server in the read path); a **per-site key-distribution service off the data path** resolves the audience from these same grant rows (resolver, §6 item 1), rotates the data key, wraps, and publishes; revocation = rotation. **The grant rows and label lanes are M2's policy input — nothing built now is redone.** Build plan M2.0→M2.5 in the design doc §8 | built (SDK + `grant-keyd`, `ops/grant-crypto/`); a publisher seals a stream with `goat pub --sealed` or the SDK |

**One sovereign, one environment ([ADR 1208](../adr/1208-sovereign-environments-release-namespaces-and-grant-encryption.md)).** The environment boundary is the machine-plane trust boundary. Each sovereign's data runs in its own environment; sharing across sovereigns goes through `release/<set>/**` namespaces over federated boundary nodes. Inside an environment, M2 is the per-reader control, and it is a standard production control whoever shares the environment.

## 11. The access language (ADR 1076 — declared, build-sequenced)

One functional verb set for every access question and change, exposed identically as `goat access <verb>` and portal API endpoints (JSON-first, CLI = thin client). Extends the built `goat access who|topic|why` (ADR 1026) — the Snowflake-shaped discipline (grants-to-roles, enumerable review, owner-managed) with the four capabilities the model it mirrors lacks:

- **`show grants` / `grant` / `revoke` / `label set`** — enumerate and mutate the rows (producer-approved or operator path, justification enforced on restriction, F-207-audited, deprecate-never-delete).
- **`can <principal> read|write <key> [--as-of <date>]`** — verdict + the four-layer why-chain; time is a first-class axis because rows are effective-dated (ADR 1051): grant history is queryable, not log archaeology.
- **`simulate --grant ... --then can ...`** — rehearse a change against a copy of the rows; pure function over data, possible precisely because grants are enumerable relations, not rules.
- **`diff --since` / `graph --json|--dot`** — change feeds and graph export; the COP web surface (ADR 1026 R.3) is a client of the same API, not a privileged UI.

**Machine-plane parity (ADR 1076): one policy source and one language, consulted by both planes — but the enforcement differs materially, not merely in transport.** What is genuinely shared is the policy: the same grant rows and vocabulary govern both planes (a grant row is read by the human surface AND the machine key service — no second store, the `no_third_shim` invariant). What is NOT identical is the enforcement, and the design must not claim otherwise: (a) **granularity** — the human surface scopes per stream arbitrarily; the machine end-state (M2) scopes by encryption *audience* (release-set), coarser, unable to express arbitrary per-pair distinctions without an audience per pair; (b) **revocation timing** — human is instant at the next request, machine is bounded by key-rotation latency and cannot claw back ciphertext+key already delivered; (c) **mechanism** — human is an active per-request checkpoint, machine is possession-of-key; (d) **today**, before M2, machine reads are unscoped (enrolled ⇒ read-all minus the audit carve), so parity is a committed destination, not a present state. Machines move toward that destination via M1 carves, the M1.5 SDK compliance-floor (binds well-behaved code only — never call it security), and **M2 grant-keyed encryption — a committed roadmap item, not a contingency** (the two-non-releasable-orgs tripwire remains the hard sequencing floor). Why not per-request checks in the router: it is a stock component with a static, restart-required, exact-CN ACL, and read grants are relational (consumer×producer) — the pre-cut trick that made writes dynamic cannot pre-cut pairs. Key possession is the machine plane's per-request check.

## 12. Posture guidance, preemptive grants, and the Access COP (ratified 2026-07-09)

**Sharing posture ([ADR 1208](../adr/1208-sovereign-environments-release-namespaces-and-grant-encryption.md)).** Within an environment, reads are open to its enrolled members, because the router ACL is static; data for particular receivers is sealed to them with grant-keyed encryption. Across sovereigns, data moves only by deliberate release.

**Grants flow both directions.** Consumers discover streams in the catalog and request access; **publishers browse the same registry the other way** — a searchable directory of orgs and people (the roster the COP already serves) — and **preemptively grant** before anyone asks. The `grant` verb (ADR 1076) has no pending-request precondition by design; the approval screen offers "grant to…" with org/person search, not just a request inbox. Both directions mint the identical grant row.

**The Access COP — a third picture, not a feature of the existing two.** Panoscope shows *live data* (who publishes what, now). The id-admin console shows *identities and platform rights*. Neither shows **access control in relation to publishers, orgs, entities, and the live data** — who can see what, drawn over what is actually flowing. The Access COP is that join: the org tree with its people and producers, grant edges (with expiry/justification on hover), label lanes as visual state, and live stream activity overlaid from the same feed panoscope consumes — so "Acme was granted this stream and is actively consuming it" and "this by-permission stream has seven grants but no traffic" are both one glance. It extends the ADR 1026 "See it" web surface (which planned the access graph but not the live-data overlay) and consumes the ADR 1076 `graph`/`show`/`why` API — a client of the language, never a fourth policy surface. Build home + detailed design get their own doc at pickup (follow-ons §0e).
