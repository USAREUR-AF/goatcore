# Environment parity — keeping goatnets aligned after standup

| Field | Value |
|---|---|
| Status | Partly built. The fleet pin table (§3), the self-extending guards and the service-config parity gate (§4), the intent parity report and the repo side of the live join (§5) are built. The cross-goatnet grade and a hosted cross-goatnet aggregator (§5) are designed, not built. Per-mechanism status in §9. |
| Scope | Every goatnet the project deploys to, and every federated mgmt node inside one goatnet. |
| Owns | How goatnets stay aligned after standup, how divergence is detected, and how a deliberate divergence is declared instead of discovered. |
| Related | `instance-variability-model.md` (standup-time variability: the Class A/B/C taxonomy and the new-instance checklist) · [`fleet-management.md`](fleet-management.md) ([ADR 1109](../adr/1109-fleet-management-dimension-and-pull-convergence.md)) · `consistency-foundation.md` ([ADR 1110](../adr/1110-typed-consistency-foundation-and-console.md)) · [`universal-deploy-rollback.md`](universal-deploy-rollback.md) ([ADR 1060](../adr/1060-universal-deploy-rollback-ledger.md)) · [`environment-health-surface.md`](environment-health-surface.md) ([ADR 1036](../adr/1036-environment-health-surface.md)) · [`release-graduation.md`](release-graduation.md) |

**Boundaries.** `instance-variability-model.md` owns *standup-time* variability:
what each new instance must decide for itself and the fail-closed rule for
leaving it undecided. This doc owns the half that model stops short of: staying
aligned afterwards. [`fleet-management.md`](fleet-management.md) names the
fleet-management dimension (identity and inventory, intent, convergence, change
propagation, undo, observation, remediation) and decides that convergence is
pulled. This doc owns **config alignment between goatnets**, the intent half of
that dimension; §3's `ops/pins/fleet-pins.yaml` is the intent substrate that
doc builds on. No mechanism is described in both.

**Why this exists.** Drift is the most common failure shape in a project that
runs several goatnets from one tree, and its dominant form is
*fork-and-edit standup, partially re-threaded*: a new environment is made by
copying another's configuration, and the copy keeps a value that belonged to
its source. Every component reports healthy while the new goatnet quietly uses
the old one's zone, account or identity. Single-property guards each catch one
instance after the fact; none compares environment A against environment B.

The drift fix that demonstrably holds is **generation**: when a hand-synced
pair of files (a service registry and a DNS zone) was replaced by a generator
(`ops/dns/generate-mesh-hosts.py` renders each site's zone from that site's
`group_vars`, with `--check` in CI), that class of drift stopped. This doc
generalises that move.

---

## 1. What drifts

| Shape | Example |
|---|---|
| No cross-environment comparison | Nothing answers "what does each environment run" or "env A has fix X, env B does not" |
| Pins duplicated by hand | The same `*_image` var defined in many `group_vars` files with several distinct values, some floating on `:latest` |
| Guards that enumerate environments | A check with a hand-typed list of site literals; a new environment is silently uncovered |
| Fixes stranded in one fork | One environment's file derives a value from a source of truth, its sibling still hardcodes it |
| Deployed state outside the source of truth | `-e` extra-vars or runtime-appended config that the next render reverts |
| A silence read as a decision | A key one goatnet declares and another omits, inheriting a fail-open or restrictive default nobody chose |
| Docs that describe a guard as enforced while no workflow runs it | A check exists, is cited as CI-enforced, and gates nothing |

---

## 2. Principles

Five rules, ordered by leverage. Everything in §3–§7 applies one of them.

1. **One fact, one home; render the copies.** Where the same fact must appear
   in N places, generate the N places from one committed source of truth and let
   CI fail when regeneration is not clean. Linting the copies is the fallback,
   not the goal: lint catches drift, generation ends it.
2. **Deviation is a first-class, declared state.** Environments may differ: a
   sovereign site on a registry-mirror cascade, a sandbox trialling a newer
   image. What is not allowed is *undeclared* difference. A deviation lives next
   to the fleet value with a `reason:` (and optionally `until:`); the generator
   renders it and the parity report shows it. A declared deviation costs one
   YAML stanza; a silent one fails CI.
3. **Guards derive their coverage; they never enumerate it.** A check that
   iterates environments discovers them from the tree (`group_vars/*/`,
   `terraform-*/`), so creating an environment extends every guard to it.
4. **Parity is observed, not assumed.** Repo intent (pins, flags) and live state
   (on-host deploy ledgers) are both machine-readable; a parity report joins
   them. The report states what it could not read rather than rendering green.
5. **A new environment is filled in, never forked.** Class-A values come from
   the new-instance checklist; shared values come from generators or shared
   roles. Copying another environment's `group_vars` wholesale, or its database
   end-state, is not a standup method.

---

## 3. Mechanism 1 — the fleet pin table (`ops/pins/fleet-pins.yaml`)

One committed file declares, per component, the image every goatnet runs:

```yaml
# ops/pins/fleet-pins.yaml — the source of truth for every *_image pin.
goatnets:
  alpha: [alpha_hosts, alpha_b_hosts]
components:
  netbird-mgmt:
    var: netbird_mgmt_image
    fleet: "ghcr.io/{{ ghcr_org }}/netbird-mgmt@sha256:<index-digest>"
    deviations:
      alpha_b_hosts:
        value: "{{ netbird_mgmt_image_tiers | select | first }}"
        reason: "island-mode registry-mirror cascade"
```

- **Pins are keyed per goatnet, not per host group.** The table declares each
  goatnet's member host groups once; `ops/pins/generate-pins.py` renders a
  wholly generated `group_vars/<member>/pins.yml` into every member from the
  same rows, so two federated nodes of one goatnet cannot carry different pins
  by accident. A per-member deviation is declarable with the same grammar. A
  member that does not run a component declares `absent: true` with a reason
  instead of receiving an inert pin, because an inert pin implies a service that
  is not there and trips guards that read the key as "this env deploys it".
- **Precedence is unchanged.** The generated file sits at the same `group_vars`
  level hand pins occupy; migrating a pin removes the superseded key from
  `vars.yml`, since a leftover hand pin in the same directory would shadow the
  generated one.
- **`--check`** regenerates and fails CI on any difference, and fails on any
  `*_image` or release-tag var defined outside a generated `pins.yml`. Pins not
  yet migrated sit on a shrink-only allowlist (the `check-rollback-coverage.sh`
  ratchet pattern).
- **Schema rules.** A deviation without a `reason:` is a schema error. A
  `:latest` or bare-tag value is an error anywhere except inside a deviation
  that says why; otherwise pins are multi-arch **index** digests.
- **A host outside the table.** A component whose role defaults carry no pin
  reads it from the generated `pins.yml` on a member of a listed goatnet, and
  from `ops/ansible/group_vars/all.yml` (which carries the row's `fleet:` value
  and is bumped with it) everywhere else. A host group with neither fails the
  play on an undefined image.
- **Rollout facts live on the same row.** `owns_state`, `rollback`,
  `restore_from` and `coupled_with` are declared per component here, and the
  rollout preflight reads them.

What this buys: a fleet-wide bump is a one-line reviewable diff that cannot miss
an environment; split and floating pins become schema errors or declared
deviations; a new environment gets its pins by being listed, not by copying.

Public goatcore ships the table with `goatnets: {}`; each environment overlay
lists its own goatnets and carries the generated `pins.yml` files.

## 4. Mechanism 2 — guards that extend themselves

| Guard | What it asserts | How it covers a new environment |
|---|---|---|
| `ops/ci/check-site-config-bleed.sh` | No site's literals (zones, mesh addresses) appear in shared templates | The literal set is derived from every environment's own `group_vars` (`--list` prints it) |
| `ops/ci/check-env-coverage.sh` | Every `provider "aws"` block in every terraform tree sets `allowed_account_ids`; every guard claiming full-tree coverage declares an `# env-discovery:` glob that expands to every `group_vars/<env>/` dir, and is wired into a workflow | Trees and env dirs are discovered, never listed |
| `ops/ci/check-service-config-parity.py` | For every role deployed on two or more goatnets, a key one declares is declared by the others **where omitting it lands on a fail-open default** (`""`, `false`, `off`, or another environment's name) | Goatnets and roles are discovered from the tree |
| `ops/ci/check-group-vars-parity.sh` | Pairs of keys duplicated because two roles deploy one mechanism hold the same value | Reads every env dir |
| `ops/ci/check-zenoh-pinning.sh` | The zenoh version of record is pinned everywhere | Full tree |

`check-env-coverage.sh` is the meta-check: it makes "we added an environment"
and "every guard covers it" the same event.

**The service-config parity gate compares declaration, never value.** Two
goatnets are supposed to hold different URLs, secrets and IDs, so the question
it asks is the one that fails in practice: *did this environment decide, or
inherit a silence?* Fail-safe defaults (ports, timeouts, retention windows) are
ignored, because a gate that reports them trains people to skip it. Two
properties to keep when extending it:

- It groups keys by **which role declares the default**, never by matching a
  key's prefix to a role directory name; keys and role directories are often
  named differently.
- Its waiver file, `ops/ci/service-config-parity-waivers.yaml`, is a **ratchet,
  not an amnesty**: pre-existing gaps are waived explicitly as untriaged with an
  `until:` and the list may only shrink. Security-shaped rows (a fence that
  defaults off, an identity layer that silently turns off) are triaged first.

**A restrictive default is as dangerous to leave undeclared as a fail-open
one.** Inheriting it fails closed and therefore looks safe: a front door whose
default admits only admin groups, left undeclared on one goatnet, quietly turns
away the ordinary members a self-service page was built for. The value is
printed on every deploy and reads as normal, because a default looks identical
to a choice. The fix is the same — declare it on every goatnet that runs the
service, and bring the key under the consistency intent so the next goatnet
cannot inherit it silently.

## 5. Mechanism 3 — the parity report

`ops/scripts/parity-report.sh` (a thin wrapper over `ops/pins/report.py`), in
two stages.

**Stage 1 — intent parity (repo only).** Renders a cross-env matrix from
`fleet-pins.yaml` plus a curated set of `group_vars` booleans
(`ops/pins/parity-capabilities.yaml`, each row with a `why:` and a
`reason:`-required `deviations:` block): component or capability × environment,
with per-node columns inside each goatnet, deviations shown with their reasons,
and unmigrated pins as UNGOVERNED rows. It answers "what is *supposed* to be
where". It contacts no host and says so on every render.

*What a green capability row does not mean.* The stage-1 grade is computed
**per goatnet, across that goatnet's members**. A cross-goatnet difference is
rendered (every goatnet's section sits in one document) but never graded
`SKEW`, and a single-member goatnet always grades `match` because it agrees with
itself. Declared cross-goatnet deviations make the values and the reason
visible, but a reader must compare the goatnet sections, and nothing fails if
they differ. A cross-goatnet grade in the shared grading library is designed,
not built; until it lands, a green matrix is not cross-goatnet agreement.

**Stage 2 — live parity (joins deploy truth).** For reachable environments,
reads the on-host `/var/lib/goat/<component>/deploy-history.jsonl` tips and
diffs them against intent, including the intra-goatnet node-skew view (§7).
Unreachable hosts render as `unreadable`, never green. This is a *pull* report
run at deploy time and on the reconciliation cadence, not a resident scanner
(§8).

**Currency.** Every hosted surface declares its currency model, `tracks-main`
or `pinned-release`, and the report prints it.

**Continuous tier — the consistency domain.** Beyond the pull report,
consistency is an observable on the existing environment-health surface
(ADR 1036): each node self-publishes a small `ConsistencyV1` envelope (component
→ running ref from its own deploy ledger, config render hashes, the generation
it last converged against); the passive reader rolls up per goatnet; and a
read-only aggregator joins each goatnet's rollup with the git source of truth
for a cross-goatnet view. The aggregator may run anywhere, but it is a renderer,
never an authority: goatnets stay fully operational without it, and nothing in
an enforcement path reads it. It renders at the usual three altitudes:
`goat health`, the panoscope band, the stethoscope target. The mechanisms (the
typed intent artifact, the single grading library, the envelope, the console)
are specified in `consistency-foundation.md` (ADR 1110); this section keeps the
model and the maturation contract.

### The model — one fact pair, one comparison, many projections

There are two fact kinds, and only two:

- **Intent** — a versioned *generation*: the resolved output of
  `fleet-pins.yaml` plus the governed `group_vars` renders for one goatnet,
  identified by the commit that produced it, signed when it travels
  (`ops/consistency/intent/<goatnet>/intent.json` in each overlay). Nothing else
  may claim to know what should run.
- **Convergence record** — one self-published fact per node: *"I converged
  against generation G at ledger tip T; components {ref → digest}; render hash
  H."* Its only source is the node's own deploy ledger (ADR 1060). Nothing else
  may claim to know what runs.

Everything an operator sees is **one comparison function** applied to that
pair, computed at read time, never stored, never re-derived differently per
surface. The stage-1 render, the stage-2 join, the `goat health` band,
panoscope, stethoscope and the cross-goatnet view are *projections* of the same
pair at different scopes. A projection may choose which records it sees and how
it renders; it may not introduce its own collector, fact shape or comparison
semantics. A projection with no private state can be deleted without losing a
fact, which is what lets the stages below retire cleanly.

**The grammar** is defined once, in `ops/consistency/grammar.yaml`, with its
mapping onto environment-health colours:

| Grade | Meaning |
|---|---|
| `match` | Every member resolves to the same value and every reporting node agrees |
| `declared` | Members differ, and the difference is a declared deviation with a reason |
| `SKEW` | Members differ without a declaration, or a node is behind; every observed value is explained by some generation |
| `unaccountable` | A node reports a ref, hash or render that no generation in the tree explains |
| `unreadable` | Something was read but could not be evaluated |
| `indeterminate` | There was nothing to read |

`SKEW` and `unaccountable` demand opposite first actions: re-running the play
fixes a behind node and **destroys the evidence** on an unaccountable one.
`unreadable` and `indeterminate` can never reach `match`.

### Maturation contract

The stages are scaffolding with named retirement criteria, not layers of a
permanent stack.

| Artifact | Role | Retires when | What survives |
|---|---|---|---|
| A committed markdown render of the stage-1 report | Bootstrap-era view and reviewable PR diff | Retired: `report.py` renders on demand, and the CI gate moved onto `intent.json` (`generate-pins.py --check` schema-validates it and refuses a body whose `content_hash` does not match) | `report.py`'s resolution semantics, now the intent-provider library under `ops/consistency/` |
| Stage-2 pull report (ledger join) | First live truth | `ConsistencyV1` self-publishing covers the same hosts | Its ledger-reading code is the node publisher's data source: same parse, different trigger |
| Cross-goatnet view (read-only aggregator) | Rear-echelon comparison | Never load-bearing (§8); may be rebuilt or discarded freely | Nothing needs to; it is the per-goatnet console binary with a wider input set, so it cannot grow its own semantics |

Two rules make this binding:

1. **At most one authoritative consistency surface per question at any time.**
   A change that lands a successor surface retires or visibly downgrades its
   predecessor **in the same change** (the same-change discipline of
   [ADR 1016](../adr/1016-complexity-budget-substrate-encapsulation.md) D7).
2. **The grammar is versioned once and reused everywhere.** A stage may *add* a
   grade only by amending this section; no stage may redefine one.

## 6. Standup rules for a new goatnet

These gates are topology-independent and cost nothing to adopt on the first
day.

**Before first apply:**

1. The new environment's guards are green by construction: `check-env-coverage.sh`
   and the derived `check-site-config-bleed.sh` literal set already cover it.
2. Its pins are rows: the goatnet is added to `fleet-pins.yaml` (members listed,
   deviations declared inline) and its `pins.yml` files are generated.
3. Docs that would misdirect the standup operator are fixed first.

**During standup (fork-free):**

- Class-A values come from the new-instance checklist; pins from the table;
  per-service `group_vars` start from the *shared role defaults* plus the
  checklist, never from another environment's directory. Where a copy is
  genuinely the fastest path for a long file, it is diffed against its source
  and every retained line is either re-threaded or listed.
- Never fork another goatnet's database end-state. A joiner site gets its data
  through the guarded data-only seed (`ops/ansible/site-seed-from-peer.yml`)
  and replication, never a copy of the founder's database.
- A new terraform tree has `allowed_account_ids` on every provider block before
  its first apply (CI-asserted by `check-env-coverage.sh`).

**Go-live gate (all four, no partial credit):**

1. **Fresh-peer probe** — enrol a factory-fresh peer and verify end to end: DNS
   resolves from the peer, the relay path works, a representative service is
   reachable by name, a publish lands and is readable. An environment whose
   every component is "up" can still give a fresh peer nothing; this is the
   test that tells them apart.
2. Parity gates green for the new environment: `check-env-coverage.sh`,
   `check-site-config-bleed.sh`, `check-portal-cert-identity-override.sh`,
   `generate-pins.py --check`, `check-service-config-parity.py`.
3. `ops/scripts/rollback-coverage.sh <site> <host>` reports a gap of zero
   (ADR 1060).
4. The environment's validation row is written with its evidence, so it is born
   inside the tracking surface.

## 7. Staying in parity

- **Propagation-or-deviation.** A change to a fleet-shared fact (a pin row, a
  shared role, a shared template) applies everywhere by construction once the
  fact has one home. For facts still living in per-env copies, the change either
  updates every copy or adds a declared deviation. Reviewers enforce the tail;
  the structure enforces the migrated case, which is why migration goes
  worst-first.
- **Persistent state.** Any `-e` extra-var or runtime mutation that changes
  durable topology lands in `group_vars` (or the relevant source of truth) in
  the same change that first applies it. Runtime-accreted state (for example
  ACL rules appended by a portal) is either re-derivable at render or explicitly
  merged by the role, never silently clobberable.
- **Node parity inside a goatnet.** Federated mgmt nodes of one goatnet are the
  same fabric: pins render identically into every member (§3); non-secret
  Class-B keys get an equality check across member `group_vars` dirs; deploy
  plays target the goatnet's member set, not one host, so a routine refresh
  cannot leave a federation peer a version behind; and stage 2 diffs the
  per-host deploy ledgers within a goatnet, so one node re-rendered with its
  peer left behind is a rendered skew row.
- **Reconciliation cadence.** Folded into the existing complexity-budget pass:
  run the live report against every environment, triage deviations whose
  `until:` has passed, re-run `rollback-coverage.sh` per environment, refresh
  the validation inventory.
- **Currency declaration.** A new hosted surface declares `tracks-main` or
  `pinned-release` when it is created (§5).

## 8. Non-goals

- **No mega-unification of `group_vars`.** Large key overlaps between
  environments are real, but collapsing them wholesale is a high-risk rewrite of
  live config for a generalisation the pin table and derived guards deliver more
  cheaply. Keys migrate into generated files opportunistically, pins first.
- **No live terraform state migration.** Shared meta-modules stay forward-only;
  new trees use them, live trees are not moved.
- **No authoritative consistency service.** No service that *owns* consistency
  state or sits in an enforcement path: that re-introduces a central authority
  and breaks island mode. The in-grain form is the §5 continuous tier:
  reveal-only per-node self-reports, a passive rollup, and a read-only
  cross-goatnet renderer. Observability yes; authority no.
- **No identical-environments mandate.** The goal is *no undeclared
  differences*, not *no differences*. A registry-mirror cascade, a cloudless
  LAN site and a sandbox ahead of production are all legitimate, as declared
  deviations with reasons.

## 9. Status by mechanism

| Mechanism | Where | Status |
|---|---|---|
| Fleet pin table, generator, `--check` | `ops/pins/fleet-pins.yaml`, `ops/pins/generate-pins.py` | Built |
| Self-extending guards | `ops/ci/check-env-coverage.sh`, `ops/ci/check-site-config-bleed.sh` | Built |
| Service-config parity gate | `ops/ci/check-service-config-parity.py` + waivers ratchet | Built |
| Stage 1 intent report | `ops/pins/report.py`, `ops/scripts/parity-report.sh`, `ops/pins/parity-capabilities.yaml` | Built |
| Typed intent artifact | `ops/consistency/intent/<goatnet>/intent.json` (per overlay) | Built |
| Stage 2 live join | `ConsistencyReportV1` on `ops/consistency/report/<host_id>`, rollup on `ops/consistency/health/<site>`, `goat health --domain consistency` | Built repo-side |
| Cross-goatnet grade | the shared grading library | Designed, not built |
| Cross-goatnet aggregator | `ops/consistency/aggregator/serve.sh` | Built as a read-only console run on the operator's laptop; a hosted deployment is designed, not built |

The guards and the generator compare environments, and goatcore alone holds
none: they run against an assembled tree (goatcore plus an environment
overlay), from the overlay's checkout.
