# Membership lifecycle & transfer — moving entities between canonical and satellite groups

| Field | Value |
|---|---|
| Status | Design (ratified with the operator 2026-07-17). Build teed up — membership-lifecycle-plan.md. **On a goatnet governed by [ADR 1189](../adr/1189-netbird-read-only-dashboard-and-warrant-owned-membership.md) membership is declared in `ops/warrant/membership/<scope>/` and applied by `goat warrant apply`; the transfer engine is the converger's mechanism, and holds are entries, not hold records — see [`mesh-change-control.md`](mesh-change-control.md).** |
| Scope | The mechanistic process for moving identities/peers between the canonical role groups and *satellite* groups (event/exercise sets, the probation isolation lane, and the quarantine security hold), in bulk / on individuals / on selected sets, both directions, audited and coverage-gated. |
| SoTs | Mesh groups: [`ops/ansible/netbird_acl_taxonomy.yml`](../../ops/ansible/netbird_acl_taxonomy.yml). Roles reference: mesh-and-ory-roles.md. |
| ADRs | [1088](../adr/1088-membership-transfer-engine.md) (transfer engine) · [1089](../adr/1089-probation-isolation-lifecycle.md) (probation) · [1091](../adr/1091-quarantine-isolation-lane.md) (quarantine) · consumes [1087](../adr/1087-advanced-grant-catalog.md) audit shape · composes with exercise-group-sets |

## The problem

Three needs share one mechanism:

1. **Exercise / activity sets** — shed a selected set of canonical members into a
   satellite group set (`EX-<NAME>-<role>`) for an activity, then absorb them
   back, keep the set, or tear it down (§5 of the ACL hardening plan).
2. **Probation isolation** — an identity that looks inactive (>14 days) is shed
   from its role into an isolation lane, and later restored — automatically on
   the way in, admin-driven on the way out.
3. **Manual admin moves** — reorganizations we can't foresee.

Doing any of these by hand is error-prone and does not scale. The stated
operator intent (2026-07-17): *"the absorption and expelling of entities from
the canonical groups will be something we eventually need to do, and it will
be much better to do it through a mechanistic process than manually."* A
second, concrete benefit: shedding inactive/low-need peers out of the
participant groups **reduces NetBird's signalling load** — the control plane
stops advertising peer-to-peer addressing between entities that don't need it.

## Core concept: the transfer engine (ADR 1088)

A single primitive — **move a set of entities from group A to group B** —
with these invariants, regardless of caller (cron, console, CLI, bulk sweep):

- **Two planes, explicit**: a "member" is a person (Ory identity, `traits.groups`)
  and/or their peers (NetBird group membership). A transfer names which plane(s)
  it touches. Probation and exercises are primarily **mesh** moves; the identity
  keeps its canonical Ory role unless the transfer says otherwise.
- **Targeting**: individual (one handle/peer), select set (an explicit list or a
  filter — e.g. "all DEVELOPER peers last-seen > 14d"), or bulk (a whole group).
- **Direction-symmetric**: shed (canonical→satellite) and absorb (satellite→
  canonical) are the same operation with source/dest swapped.
- **Coverage-gated (fail-closed)**: the same rule that governed every cutover —
  a move may never leave a peer in *no* policy-bearing group. Absorb-then-verify-
  then-remove; the engine refuses a move that would orphan a peer unless the
  destination covers it (probation's isolation group IS a covered destination —
  see ADR 1089).
- **Audited + reversible**: every move emits an `OperatorActionV1` (the `goat
  admin` chokepoint, ADR 1087 shape) naming who/what/why/count; the inverse move
  is always available (ADR 1031 reversibility).
- **Idempotent + resumable**: a bulk move that dies halfway can re-run; entities
  already at the destination are skipped.

The `goat admin members move` engine implements mesh moves, quarantine
with `--strip-others` and marker flags (ADR 1091), and Ory group swaps with
boolean trait markers. See the [CLI reference](../goat-cli-book/src/reference.md#goat-admin-members-move).

The identity console uses that engine for a single device's mesh preview
and rechecks the plan at confirmation. In an enforcing scope it prepares a
declaration for review and deployment through the warrant workflow. See the
[console workflow](../../ops/goat-id-console/README.md#move-a-peer-between-mesh-groups).

## Satellite groups

A **satellite** is any non-canonical group an entity is temporarily parked in:

- **Exercise satellites** — `EX-<NAME>-<role>` (their own rules, per
  exercise-group-sets). Overlap or
  isolation from the persistent ecosystem is chosen by which groups a peer holds.
- **Probation satellites** — `PROBATION-<role>` (ADR 1089). Isolation, not
  reach; the `<role>` suffix is retained *only* so restore returns the entity to
  its exact prior role.
- **Quarantine** — `QUARANTINED` (ADR 1091). The security hold, flat and
  admin-placed; restore content lives in the hold record (below) and the
  transfer-engine audit record, not the group name.

Which lane a hold belongs to is decided by how it was entered, never by
reading the group: **dormancy (ADR 1136, the sweep) ⇒ `PROBATION-<role>`;
security (ADR 1091, an admin) ⇒ `QUARANTINED`.** The sweep parking dormant
peers in `QUARANTINED` was a defect (ADR 1136 D1, clarified 2026-09-11; fix
#1040).

Satellites are declared in the taxonomy like any group (never hand-made); the
engine creates an exercise set at stamp time and the probation + quarantine
sets once, fleet-standard.

## Probation lifecycle (ADR 1089) — a policy on top of the engine

- **Detect** (cron, sibling of identity-reconcile): an identity is *dormant* when
  its last activity — max of NetBird peer last-seen and Kratos last session — is
  older than the threshold (default 14 days; per-goatnet). Dormant ≠ offboarded;
  it is a reversible holding state.
- **Shed** (engine): move the identity's peers from `ZAGROS-<role>` into
  `ZAGROS-PROBATION-<role>`. The identity keeps its Ory role record (so restore is
  exact). The mesh move is the enforcement; the v1 hold record plus live mesh
  state is the server-trusted classification contract. No Ory trait is required
  for probation classification.
- **Isolation posture** (the point): a `PROBATION-*` peer reaches **no** other
  peer and **no** application — except the narrow path needed to be *told* it is
  probationed and the Ory endpoints needed to maintain its identity. One rule:
  `PROBATION-* → INFRA` on `:443` + DNS only. Everything
  the peer's browser requests, at any application hostname, is served the
  **single probation explainer page** by Traefik ForwardAuth. NetBird management,
  signal and Ory identity routes remain outside the console's failure domain so
  the peer retains control-plane connectivity and the site's login/session
  infrastructure does not depend on the explainer. Application routers remain
  gated, so an Ory session cannot restore application or mesh access for a held
  peer. Classification is server-side and
  exact: connection-derived source mesh IP → one live NetBird peer → one
  same-site dormancy hold record whose id, group and live state all agree. It is
  never inferred from a group substring or a browser-supplied identity header.
  Generic denial is deliberately broader: any live group matching the shared,
  prefix-independent hold grammar is denied, including a hold retained across
  a site rename.
  Unknown, stale, releasing, orphan, duplicate or inconsistent states receive
  the same generic denial as every security hold. Once classified as held,
  non-document, asset and API requests are also denied rather than forwarded;
  only the HTML explainer and its separately revalidated petition endpoint are
  admitted. Peer lookup is an immutable IP-indexed snapshot refreshed in the
  background after 30 seconds, so ordinary request cost does not grow with fleet
  size. Ordinary allow and deny decisions keep using the last-known-good
  identity binding during a management-API outage. Showing the explainer or
  accepting a petition requires a binding refreshed within 60 seconds; after
  that age a would-be explainer synchronously refreshes, retries failures after
  5 seconds, and fails closed until a current binding is available. A newly
  enrolled address fails closed until that refresh completes. The
  local hold-record path is statted on every decision; an immutable,
  site-and-peer-indexed snapshot is rebuilt synchronously when an append or
  atomic replacement changes its identity, size or modification time. This
  keeps steady-state lookup independent of ledger size without delaying a new
  hold behind a TTL. The explainer states the hold start date and its age from
  the record's `entered_at`; it never shows live NetBird last-seen because a
  member viewing the page is online. It also offers a **petition** control.
  Activation is a separate, default-off per-goatnet switch. It is limited to
  legacy-record scopes: the deploy refuses it when the active warrant
  declaration says `converge: enforce`, because those holds are warrant entries
  and ADR 1189 supersedes this record source. `converge: report` remains a
  legacy-record scope. Before activation, Traefik calls a running-console
  readiness endpoint that validates the process's actual NetBird credentials
  and hold-record path, rather than only checking the desired Ansible values.
  The console deployment repeats that readiness check after every later
  recreation while the gate is enabled.
- **Petition → separate lane**: the petition posts into a dedicated queue in the
  onboarding-admin console (goat-id-console) — *not* the normal onboarding lane —
  so reactivations are triaged distinctly from new joins. The form token binds
  the peer id and current hold timestamp; the POST re-classifies the source and
  the store permits one row per exact hold, making retries idempotent. The admin
  queue is read-only until #1044 adds the guarded release action. This increment
  has no close path; a hold can also end through the sweep's release or revoke,
  so the queue may list requests whose hold has ended. The admin confirms the
  current state through the row's **View mesh hold** link; closing those rows
  belongs to #1044.
- **Reactivate** (engine, admin-driven): any `IAM-ADMIN` or `PLATFORM-ADMIN`
  approves; the engine absorbs `PROBATION-<role>` → `<role>` and the peer is a
  full participant again. Fully reversible; auto on the way in,
  human on the way out (never auto-delete — composes with ADR 1068 anomaly
  probation's hold/verify/repair-or-purge stance, of which this is the mesh-reach
  expression).

## Quarantine (ADR 1091) — the security sibling, deliberately inverted

Where probation is dormancy-driven and self-service-recoverable, **quarantine**
is the operator's containment lane for suspected or confirmed threat-actor
behavior. Three graduated dispositions; the group implements only the middle:

1. **Observe in place** — access untouched; watch via the microscope lens
   (ADR 1090 watch missions) + flow-log attention. No group change.
2. **Quarantine and observe** — a transfer-engine move into `QUARANTINED`, out
   of **all** other groups (roles, P2P, labels). Reach collapses to
   `→ INFRA :443 + DNS` — every remaining move lands on the logged L7 gates
   (the observation point *is* the reach we leave); **no fabric ports** (a
   compromised slot cert must not keep publishing); INFRA reaches the peer
   any-port for inspection.
3. **Sever** — ADR 1031 mesh-first revoke, unchanged.

The deliberate inversions of probation: **no explainer page, no petition, no
self-service exit** (a suspect must not be told they are being watched); flat
rather than role-suffixed (entry is rare and human; the audit record names the
prior groups exactly, and a role suffix would leak signal); exit only by
operator restore or sever. The route-override classifier treats every security
hold and security hold-group state as deny-as-normal, never the probation
explainer; it does not trust an Ory marker to distinguish the lanes.
Privileged accounts (Ory PLATFORM-ADMIN / IAM-ADMIN) are never merely
quarantined — sever and rotate immediately.

That is an enforced boundary, not operator guidance. A security transition
into or out of `QUARANTINED` first evaluates all three privilege facts: live
Ory `PLATFORM-ADMIN`, `IAM-ADMIN`, or legacy `ADMINS`; live mesh
`PLATFORM-ADMIN`; and the declared mesh baseline. Every possible person owner
must have a deployable email hash that resolves to exactly one live Ory
identity, and every possible owner's Ory roles and declared device and handset
baselines must be ordinary. The peer's own live and resolved baseline must also
be ordinary. An ambiguous join is allowed only when all of those facts prove
every candidate ordinary; a privileged or unprovable candidate refuses the
operation before any NetBird, Ory, or warrant-file write. The candidate grammar
also accepts the pre-v1.1.0 `peer-H-H-<scope>-<date>...`
form, because those legacy peers must remain reachable by security controls;
the second segment must equal `H`, so arbitrary multi-token orgs do not match.
The deployed converger gets owner joins from the generated baseline; the direct
command supplements it from active local operator manifests when a baseline
row is absent, for ownership only and never as a group baseline. Machines and
systems with no human login use only their live mesh membership and declared
baseline.
A uniquely owned privileged subject is directed to ADR 1091 severance and
credential rotation. An ambiguous join with a privileged possible owner is
reported as unproven, so the operator is never told to sever an ordinary user.
No refusal prints identity details.

This guard applies only to the security lane. A `PROBATION-*` hold is dormancy,
so an administrator's valid dormancy release remains subject to the ordinary
release consistency checks and is not mistaken for quarantine close-out.

### Why probation is worth the complexity

It converts "inactive account quietly retaining full mesh reach" (a standing
lateral-attack surface and a signalling cost) into an explicit, isolated,
self-service-recoverable state — without deleting anyone or losing their role.
The mesh shrinks to entities that are actually participating.

## Hold record — the restore source of truth

Every hold, whichever lane, writes **one JSON line** describing what was done
and what to undo: [`ops/lifecycle/schema/hold-record.v1.schema.json`](../../ops/lifecycle/schema/hold-record.v1.schema.json),
fixture [`ops/lifecycle/fixtures/hold-records.jsonl`](../../ops/lifecycle/fixtures/hold-records.jsonl),
validator `ops/lifecycle/validate-hold-records.py` (run by
`tests/lifecycle-hold-record.bats`).

| Field | Meaning |
| --- | --- |
| `v` | `1`. A reader that does not know the version refuses the line. |
| `lane` | `dormancy` or `security`. Decides who may release and how. |
| `site` | goatnet id, lowercase (`zagros`). |
| `peer_id` / `peer_name` | NetBird id (the key) and the display name at hold time. |
| `handle` | parsed from the peer name by the peer-name grammar; `null` when it does not parse. |
| `entered_at` | UTC, `YYYY-MM-DDTHH:MM:SSZ`. |
| `hold_group` | the group moved into — `<SITE>-PROBATION-<role>` or `<SITE>-QUARANTINED`, where `<SITE>` is the taxonomy `site_prefix` (`ZAGROS`, `ZAGROS-PROD`); the schema refuses a lane/group cross and the validator refuses a prefix that is not `site` upper-cased. |
| `prior_groups` | **the restore content**: group names held before, minus `All` and the hold group. May be empty. |
| `actor` | `sweep`, or the operator's handle. |
| `reason` | optional, ≤200 chars. Minimal for security holds. |

Where it lives on a mgmt host is declared once, in the `peerview` role
(`lifecycle_hold_records`, default `/var/lib/peerview/hold-records.jsonl`,
handed to the sweep as `LIFECYCLE_HOLD_RECORDS`). Writers: the sweep's
dormancy lane (#1040) and, when built, the console's quarantine action.
Readers: the console's Access banner (#1041), Holds page (#1042), probation
explainer/petition classifier (#1038) and Release action (#1044), and
`lifecycle-sweep.py --lane release`.

Dormancy hold-group choice follows one rule:

- exactly one canonical role group that has a `PROBATION-` mirror ⇒ that mirror;
- any gear-class group (ADR 1116) ⇒ `<SITE>-PROBATION-GEAR`;
- anything else (two role groups, no role group, or a role with no mirror) ⇒
  refuse and leave the peer in place.

The rows apply in order: a peer holding one mirrored role group *and* a gear
class group parks in the role's mirror, and its gear group is restored with the
rest of `prior_groups`. Its dormancy clock is still the gear clock, because
classification and hold-group choice are separate questions. For this rule the
role groups are the warrant's mirrored set — `PLATFORM-ADMIN`, `DEVELOPER`,
`OBSERVER`, `MONITOR`, `MOON`, `POD` — so `MONITOR`, `MOON` and `POD` park in
their own mirror even though the dormancy clock classifies them as gear.

Invariants every reader may rely on:

- **The group name is the restore *target*; `prior_groups` is the restore
  *content*.** A release restores exactly that list. It can never grant more
  than was held, because it never reads anything else.
- **No record ⇒ security hold.** A peer found in any hold group with no line
  for it offers no self-service or one-click exit.
- **One line per held peer.** Release removes the line; a later hold replaces
  it under the same lock by dropping any existing line for that `peer_id`
  before writing the new one. Two lines for one `peer_id` is a writer bug and
  the validator refuses it.
- **Lane and group agree.** `dormancy` only with a `PROBATION-*` group,
  `security` only with `QUARANTINED`; contradictory live state (peer in a
  hold group *and* any other group, or in two hold groups) is rendered
  "inconsistent" and refused for release, never resolved silently. The
  sweep's release refuses a `security` record outright (no override flag)
  and refuses a dormancy record when `QUARANTINED` sits on top of the
  `PROBATION-*` group, because NetBird ACLs are additive and a restore would
  hand a contained peer its full reach. A record is this peer's by
  `peer_id`; a name match is accepted only for a re-enrolled peer whose old
  id is gone and whose name no other live peer shares; the ledger fallback
  applies the same rule to its rows.
- **Writers serialize on an exclusive `flock` of a sidecar lock file
  (`<path>.lock`); removal is temp-file + atomic rename under that lock.**
  Every writer uses the record helper to open the file only after the
  sidecar lock is held. A hold appends under the lock; a release rewrites
  the surviving lines beside the file and atomically replaces it
  (`os.replace`). The sweep (#1040) and the console release (#1044) both
  implement against this sentence.
- **No secrets, no personal data beyond the handle.** Every ADMIN can read the
  file.
- **A bad line is skipped, never fatal, and always counted.** Not a JSON
  object, not `v: 1`, or a required v1 field missing ⇒ every reader skips
  the line, keeps reading, never acts on it, and reports the count (the
  sweep's report and stderr; the console's Access section with line
  numbers). No reader refuses the whole file for one bad line; writers carry
  bad lines through unchanged for the validator to name. The peer behind a
  bad line has no record and so reads as a security hold until repaired.
  Decided 2026-09-21 across #1040 and #1041; `ops/lifecycle/README.md` is
  the operative wording.
- **A stranded hold has an exit; inconsistent state that needs hands fails
  the apply run; a group-choice refusal does neither.** The hold order is
  ledger, record, add, verify, remove — intent before the first group
  change — so a crash leaves a peer in its hold group and its prior groups
  with its record — inconsistent live state, listed under `refused` whether or
  not the peer is connected. Inconsistent means *any* group beside a hold
  group (role or not — a stranded `PROBATION-GEAR` + `SENSOR` counts), or
  more than one hold group at once. The next `--apply --lane quarantine` finishes
  the removal when the peer is in its record's hold group and holds nothing
  the record's `prior_groups` do not list; any other hold-group-plus-role-
  group state it refuses and exits 1. A peer the group-choice rule could
  not place is left in its full groups by design and does not change the
  exit code. The report-only timer run exits 0 and publishes every refusal
  in `refused` (its guard reads non-zero as an unreadable source). A record
  whose hold group the peer no longer holds is removed by `--lane release`,
  said out loud as `STALE RECORD`. Before a hold acts, the sweep re-reads
  the peer and re-runs the group-choice rule, refusing if the answer changed
  since the candidate list was built. Every group PUT re-reads the group
  immediately before writing (NetBird's update is a whole-list replace, and
  a stale list reverts another writer's change). A release rewrites the
  record with `phase: releasing` before its first group write, so a
  half-done release is never mistaken for a stranded hold; an orphan record
  (peer gone from the mesh) is counted by the report and removed by
  `--lane release --peer <id>`. The resume path, like release, acts on
  `dormancy` records only, and the revoke lane skips a peer under a
  `security` record or in `QUARANTINED`. A line whose `lane` and
  `hold_group` disagree, or whose `phase` is unknown, is a bad line to the
  sweep and the validator; the console's reader checks required fields and
  `v` (the difference is stated in `ops/lifecycle/README.md`). The report
  names `stale` records (live peer not in its record's hold group). Release
  re-reads the peer before its checks, as the hold does. `--limit` is one
  budget across holds and resumes, counted in attempts; group-choice
  refusals never spend it.

The tab-separated ledger the sweep writes today (`/var/lib/peerview/
lifecycle-sweep.log`: time, lane, name, id, prior groups) already carries the
same restore content and keeps being written until the record is proven in
use; `--lane release` reads the record first and falls back to the ledger.
The fallback is gated: a peer with no record is a security hold by the
invariant above, and `QUARANTINED` is also ADR 1091's containment group, so
the sweep refuses to release from the ledger unless the operator passes
`--acknowledge-quarantined-is-dormancy`, attesting that the hold is a
pre-change dormancy hold; membership in `QUARANTINED` is checked before the
attestation, so a repeat release on a peer already back in its role group is
a plain "nothing to release". Until those pre-change holds drain, the console
reads each of them as a security hold; that is the honest reading of "no
record", and the release attestation is how an operator resolves one.

## Boundaries (what this is NOT)

- Not offboarding/revocation (ADR 1031) — probation is reversible and keeps the
  identity; revocation is deliberate removal.
- Not affiliation/echelon — those are registry identity data, never groups
  (identity-grammar). A satellite is a *reach* construct, not an identity fact.
- Not a data-access change — moving groups changes L4 reach + service-gate
  admission, never the `data.*` grammar (ADR 1074), which is a separate axis.
