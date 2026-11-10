# Release graduation — how "what should run" changes, rung by rung

| Field | Value |
|---|---|
| Status | **Built (repo-side) — ladder + tool + CI invariants live in the tree; first graduation not yet exercised.** The dev rung is `planned` (no dev goatnet exists 2026-08-09); the live ladder is preprod (zagros) → prod (zagros_prod). Live convergence verification through env-doors/CF-7 is designed-not-built (§7). |
| Anchor ADR | [ADR 1120](../adr/1120-release-graduation-ladder.md) |
| Composes with | `consistency-foundation.md` ([ADR 1110](../adr/1110-typed-consistency-foundation-and-console.md)) — answers "is each env running what it's supposed to?"; **this doc defines how "what it's supposed to" changes.** `environment-parity.md` §5 (the two-fact model this stays inside) · [`universal-deploy-rollback.md`](universal-deploy-rollback.md) ([ADR 1060](../adr/1060-universal-deploy-rollback-ledger.md)) · [`env-doors.md`](env-doors.md) ([ADR 1117](../adr/1117-per-env-doors-cross-env-management.md)) · [ADR 1115](../adr/1115-goatnet-e2e-acceptance-suite.md) (the validation gate's eventual mechanism) · [`goatnet-deployment-doctrine.md`](goatnet-deployment-doctrine.md) |
| Tracking | `release-graduation` in `in-flight.md` |

> **60-second summary:** Changes graduate along a committed ladder of goatnets
> — dev (planned) → preprod (zagros) → prod (zagros_prod) — captured as data in
> `ops/release/ladder.yaml`, never as tribal knowledge. The unit of graduation
> is a component's pin value in `ops/pins/fleet-pins.yaml`: **a release
> candidate is a declared deviation on the preprod rung; graduation folds that
> deviation into the `fleet:` value of record.** Build once, promote the same
> digest — never rebuild per env. Every promotion writes a record under
> `ops/release/records/` naming its reason, evidence and approver;
> `ops/release/graduate.py` guides the steps and grades the cross-rung state;
> CI (`graduate.py check`, in the fleet-pins job) refuses undeclared skew. The
> observation half is a **projection** of the facts the consistency foundation
> already maintains — no new collector, no new fact kind, per
> environment-parity §5's projection rule.

## 1. The problem this solves

The consistency foundation made "is every env running what it's supposed to?"
answerable the same way twice. What remained unanswerable was the *change*
question: when a new image/config exists, **which envs should get it, in what
order, after what proof, and where is that captured?** Until now the answer
lived in operator judgment per deploy — fine at two goatnets, unworkable at
three-plus, and invisible in review either way. The failure modes it invites
are both live classes in this repo's history: an untested value reaching prod
first (the F-298 shape — a concurrent session deploying a stale branch), and
preprod silently running ahead of prod forever with nobody able to say whether
that's a release in flight or drift.

## 2. The model

**Rungs are goatnets; the ladder is data.** `ops/release/ladder.yaml` names the
rungs in graduation order, each rung's goatnet (matching `fleet-pins.yaml`
`goatnets:` keys), and the gates per hop. The dev rung is declared now with
`status: planned` so the model survives its standup unchanged; until then
preprod absorbs the dev role, stated in the ladder as a declared temporary
widening, not an accident.

**The unit of graduation is the pin.** What a goatnet "should run" is already
one artifact — the resolved pin table (IntentV1). Therefore:

- **Candidate** = a `deviations.<preprod-goatnet>.value` entry on the component
  in `fleet-pins.yaml`, reason `RC (<date>): … — record <file>`. Preprod
  running ahead is *visible, reasoned skew* — exactly what deviations exist to
  express, and every existing consistency surface already renders it.
- **Graduation** = fold: the candidate ref becomes `fleet:`, the RC deviation
  is deleted. Prod converges to the new value of record on its next deploy.
- **Build once, promote the digest.** The ref that graduates is byte-identical
  to the ref that soaked (multi-arch *index* digest, F-245). Rebuilding "the
  same thing" for prod would make preprod's soak evidence about a different
  artifact.

**Context decides the path, and the context is committed.** Graduation classes
(`ladder` default, `env-local`, `exempt`) live in the ladder file with reasons;
components absent from a rung are already declared `absent` deviations.
**Hotfix is a path, not a class**: any component may go expedited-to-prod as a
declared prod deviation + a hotfix record with a named approver — and it *owes
a backfill*, rendered as `hotfix-backfill-owed` — whether or not the rungs agree — until the same ref graduates
normally. Rapid is allowed; silent is not.

## 3. The record — graduation systematically captured

One YAML per promotion at `ops/release/records/<component>/<date>-<ref12>.yaml`:
component, class, candidate ref, prior fleet ref, opened date, reason,
`evidence.validation` + `evidence.preprod_convergence` (filled as they exist),
state (`candidate` / `graduated` / `abandoned`), graduation date, operator ack.

**Soak waivers are a field, not prose.** An operator who moves a ref before its
window elapses records that decision as:

```yaml
soak_waiver:
  by: <who>            # null ONLY when the record genuinely cannot attribute it
  reason: <why>        # required — C2 refuses a waiver with no reason
```

It is a **record of a decision, not an override**: `gate_failures()` is
unchanged, so `graduate` still refuses on the unmet gate and the readiness view
still shows the window as unmet. What changes is that a waived soak stops
rendering identically to a window nobody has looked at — the state is
`soak-waived`, distinct from `soaking` and from `met`, and it carries who and
why. A waiver naming nobody renders as **unknown attribution**, deliberately
worse-looking than an attributed one. The waived graduations of 2026-08-10/11/12
(`id-admin-console/20260811-f15011626670.yaml`,
`id-admin-console/20260812-fe780ae86caa.yaml`,
`portal/20260811-cbf7afeb408f.yaml`, and the two same-day
`id-admin-console` records that moved both rungs) were backfilled into this
field from their own `operator_ack` prose; `test_readiness.py` fails if a
record says a soak was waived and carries no field.
**Runtime health is a field, captured at the soak's end.** The acceptance rows
show that each service answers; they do not show how it behaved under a day of
real load. `ops/release/rung-health.py <component> --goatnet <goatnet>
--write-record <record>` reads that from the hosts of the rung and writes it
into the record:

```yaml
runtime_health:
  preprod:                       # the rung it was read on (the hop's lower rung)
    captured: '2026-08-12T10:00:00Z'
    deployed: '2026-08-10T09:00:00Z'   # the newest container's Created time
    digests: [sha256:…]          # what was running — must include the candidate's
    restarts: 0                  # docker RestartCount, summed over the containers
    oom_kills: 0                 # kernel OOM kills charged to the containers' cgroups
    peak_memory_fraction: 0.42   # highest memory.peak / container limit
    containers: [...]            # the per-container readings behind the totals
runtime_health_override:         # only when an operator graduates past a failure
  preprod: {by: <who>, reason: <why>, recorded: <date>, failures: [...]}
```

A count the reading could not obtain is `null`, never `0`, and fails the gate.
Unlike a soak waiver, an override with a named `by` and a `reason` satisfies the
gate: it is the operator's decision to ship past a measured failure, and the
failures it overrode are kept beside it.

Records are the durable answer to "how did this reach prod?" — greppable,
reviewable in the PR that made the change, and cheap enough that skipping one
is never justified. They are governance metadata, not a deploy mechanism: the
deploy ledger (ADR 1060) remains the only truth about what ran.

## 4. Gates per hop (data in the ladder, semantics in the tool)

| Gate | preprod→prod | What it checks |
|---|---|---|
| `candidate-record` | ✓ | an open record exists for this exact ref |
| `soak` | ✓ (`min_days: 1`) | days since the record opened — rapid by default, never zero; an operator may WAIVE it in the record (§3), which is recorded and rendered, and still refused by the tool |
| `convergence-evidence` | ✓ | record names how preprod was *seen* converged on the candidate (goat health consistency band / console render / M-doc) |
| `validation-evidence` | ✓ | record names what validated the change — the ADR 1115 acceptance suite when live; CI + targeted tests until then |
| `validation-evidence` (dev→preprod, planned) | — | same presence check on the dev hop; for goat-id-console the named validator is the browser-driven intake smoke (`ops/scripts/onboard-intake-smoke.sh`, manifest row `intake-form-browser-submit`), run against zagros-dev after the candidate deploy. The candidate is a release-candidate tag and the rows are the ones the change affects — [release-candidate-validation.md](release-candidate-validation.md) ([ADR 1173](../adr/1173-release-candidate-tags-trigger-affected-flow-validation.md)) |
| `operator-ack` | ✓ | a named human approves the prod hop (`graduate --ack`) |
| `runtime-health` | any hop that declares it | the record's `runtime_health.<lower rung>` block (§3) was captured no earlier than the soak's end, on the candidate digest, and shows **0 restarts, 0 OOM kills, and peak memory ≤ `max_memory_fraction` of the container limit** (default 0.8). Refused unless `runtime_health_override.<lower rung>` names who and why |

The runtime-health gate is configured per hop in `ladder.yaml`, and applies to
every hop that lists it, dev→preprod as well as preprod→prod:

```yaml
- id: runtime-health
  max_memory_fraction: 0.8      # peak memory ceiling, fraction of the limit, in (0, 1]
  min_days: 1                   # default: the hop's soak min_days, else 1
  require_memory_limit: true    # a container with no limit fails (default true)
  records_opened_from: '<date>' # optional: C10 leaves older open candidates to `graduate`
```

It is graded on the hop's **lower** rung — the goatnet the candidate soaked
on. `graduate` enforces it on preprod→prod. The dev→preprod move is a pin edit,
not a `graduate`, so `check` C10 holds that hop: a candidate already running on
preprod must pass the gate for dev. `graduate --override-runtime-health REASON
--ack NAME` records an override and proceeds; for a dev override, or a set
member, the same block is written into the record in the PR.

Evidence gates are **presence** checks: the tool requires the record to name
evidence; it cannot verify the evidence is true. That signature is the
operator's. The dev→preprod hop (planned) carries record + validation gates
only — soak and ack weight belongs at the prod boundary.

## 5. The tool — guide and observe, deliberately not actuate

`ops/release/graduate.py`:

| Command | Does |
|---|---|
| `status` | cross-rung matrix per component, graded: `aligned` / `candidate` / `hotfix-backfill-owed` / `env-local` / `legacy-declared` / `UNDECLARED-SKEW`; every adjacent hop (dev→preprod, preprod→prod) is graded and the component carries its worst hop, so a deviation on the middle rung alone is seen; lists open records |
| `open <c> --ref --reason` | writes the candidate record; **prints** the exact preprod deviation edit |
| `graduate <c> --ack` | verifies every hop gate, marks the record graduated, **prints** the fold-to-fleet edit; with no ladder candidate open it closes the component's open hotfix record instead — a fastlane's backfill. `--override-runtime-health REASON` writes the override (§3) and graduates past a failing runtime-health gate |
| `hotfix <c> --ref --reason --ack` | writes the hotfix record; prints the prod deviation edit |
| `check` | CI invariants C1–C10 (§6); C9 fails an open hotfix record older than three days with no validation evidence |

`ops/release/rung-health.py <component> --goatnet <goatnet> [--write-record
<record>]` is the reading behind the runtime-health gate. It is read-only on
the host: one `sudo -n python3` per host reads `docker inspect`, the
container's cgroup (`memory.peak`, `memory.events`, cgroup v1 equivalents) and
the kernel journal since the container was created. Hosts and container names
come from the component's connection map (`hosts:` in
`ops/ansible/roles/<component>/deploy-history.yml`, narrowed to `site:
<goatnet>`), reached directly, through `GOAT_ROWS_SSH_JUMP`, or through the
goatnet's door — the same paths `rollback.sh` and the acceptance rows use. It
prints the block as JSON and a one-line verdict computed by the gate's own
function; exit 0 pass, 1 fail, 2 unknown. `--host` and `--container` read a
component that has no connection map.

It **prints pin edits instead of making them**, on purpose: `fleet-pins.yaml`'s
comments are load-bearing review context that YAML round-tripping would
destroy, and the edit belongs in the PR diff where a reviewer sees it next to
the record. The pair `printed edit + check` is self-closing: a record without
its pin edit (or vice versa) fails C3–C6. First run against the real tree:
11 components, all `aligned`, zero grandfathering needed.

## 6. CI invariants (fleet-pins job, unfiltered)

C1 ladder well-formed, rungs name real goatnets, gates/classes known, every
override and allowlist row carries a reason · C2 records parse, name real
components, digest-pinned refs, hotfix ⇒ ack, a `soak_waiver:` carries a reason
and an explicit `by` (`null` allowed, absent not), a `runtime_health_override:`
names a non-empty `by` and `reason` per rung · C3 preprod-ahead skew ⇒ open
candidate record or shrink-only `legacy_skew_allowlist` row · C4 prod-ahead
skew ⇒ hotfix record · C5 latest graduated record's ref = current fleet value
(the fold actually happened) · C6 no record stays open for a ref its rung no
longer runs (stale candidates get abandoned, not forgotten) · C10 an open
ladder candidate running on the upper rung of a live hop that lists
`runtime-health` passes that gate for the lower rung, or carries an override
(records opened before the gate's `records_opened_from` are left to
`graduate`). Current-tree, not diff-scoped: a violation cannot hide behind an
unrelated PR.

## 7. Observation, and the line not crossed

Graduation **status is a projection**: `status` reads the pin table, the
ladder, and the records — all committed facts — and computes grades at read
time. It introduces no collector, no new fact shape, no comparison semantics of
its own (environment-parity §5). Whether an env *converged* on what graduation
says it should run stays the consistency foundation's question, read through
`goat health --domain consistency`, the CF-6 console, and the CF-7 cross-
goatnet aggregator (run through the env-doors, ADR 1117).

**Designed-not-built:** `status --live` — joining the grades with each rung's
actual convergence via the CF-7 collect path, so "candidate soaking on
preprod" upgrades from repo fact to observed fact in one view. It composes
from existing pieces (doors + collect + this tool's grades) and lands as a
projection, not a service.

## 8. The readiness document, and the console view over it

`status` answers the ladder's question at a terminal. The same question asked
by someone who is not in a checkout — *what is merged but not deployed, what is
soaking and how much longer, what is ready to graduate, and what blocks the
rest* — is answered by the **Release readiness** view at `/release` on the
consistency console (and on the CF-7 aggregator; same binary, same page, same
fence). The gap it closes is real and dated: 2026-08-11, three merged PRs with
images built and nothing pinned, invisible to everyone until someone read the
yaml by hand.

**One computation, two surfaces.** The view computes nothing. `graduate.py
readiness` emits a `release-readiness-v1` JSON document from the same
`build_status()` grading `status` prints and the same `gate_failures()` the
`graduate` subcommand refuses on, so the page cannot call a component ready
that the tool would refuse. Per component the document carries: the graded
cross-rung row with each deviation's stated reason; member-level deviations
(carried, and labelled as node-parity's axis, not the ladder's); the open
candidate with its soak clock and its failing gates verbatim; the last
graduated record; the ADR 1060 deploy-ledger join; and the pinned build's
commit and date.

**"Is there a build nothing pins?" — answered per component, from the tree.**
The gap that motivated this page is a merge that built an image nobody pinned.
Each component declares its image workflow in `fleet-pins.yaml`
(`image_workflow: .github/workflows/<name>.yml`), and that workflow's
`on.push.paths` filter is *already* the authoritative statement of what source
produces a new image — the same filter GitHub evaluates on every push. So
`readiness` counts, per rung, the commits between the **pinned build's commit**
and the generating HEAD **that touch those paths**, and names the newest one
(sha, date, subject). Above zero reads: *a newer image for this component almost
certainly exists, and this rung does not run it.* Zero reads: *the deployed build
is current for this component's sources.* The stronger claim — **no pin in the
table names those builds** — is made once, at component level, and only when
every live rung is behind and every rung's reading is usable; one rung ahead (a
candidate) or one rung unknown withdraws it. The declaration is never inferred
from a name convention (two workflows can build from the same paths —
`zenoh-pqc` and `zenoh-pqc-fips` do), which is the `ledger_component` rule
applied to a fourth join key.

**The ledger join is by DIGEST.** `fleet-pins.yaml` stores the literal
`{{ ghcr_org }}` template and `deploy-history.yml` stores the resolved org, so
a whole-ref comparison matches nothing. Joining on `@sha256:` gives the one
merged-but-not-deployed signal the tree can actually answer: **a pinned digest
that appears in no deploy-history row was never recorded as deployed** — and
the converse, a newest ledger row whose digest no current pin names.

**Deploy-time derived, never committed.** The document stamps the generating
checkout's HEAD and a generation time, so a committed copy would drift on every
merge and no regenerate-diff gate could pass. The `consistency-console` role
renders it from the deploying tree and places it beside the intent artifact;
the console re-reads it from disk on every render. Its consequence is stated on
the page rather than hidden: **the document is a snapshot as old as the deploy
that placed it**, and its age and source commit render at the top — and the page
**degrades as it ages** rather than printing an old number in the same voice as
a fresh one: `fresh` under 7 days, `stale` at 7, `very stale` at 30, each with a
severity edge and the same instruction (redeploy the console; nothing else
refreshes it). The windows are the console's deploy rhythm, not a ladder gate:
this page's components graduated several times a week through 2026-08, so a week
without a redeploy has certainly missed pins and records. A stamp from the
future reads `clock skew`, a document with no stamp `age unstatable` — both
degraded, neither fresh.

**What the view deliberately does not know**, listed on the page itself:

| Not known | Why, and what is shown instead |
|---|---|
| Whether an image was actually built and pushed | This tool contacts no CI and no registry. It reports whether commits since the pinned build touched the paths that component's image workflow builds on — a strong "a newer image exists that this rung does not run", not a confirmed one. |
| Components whose build inputs the tree does not declare | No `image_workflow:` in `fleet-pins.yaml`, a workflow with no `on.push.paths` filter (it builds on every push), or a filter this tool will not evaluate — `paths-ignore`, a negated pattern. Those rows read **unknown with the reason**, never 0 commits. |
| CI run state | No committed artifact carries it, and this tool contacts nothing — inventing a GitHub Actions dependency for a page that must render on an island is the wrong trade. The newest ledger row's PR is the last build with in-tree evidence, shown as exactly that. |
| Whether a recorded soak waiver was justified | A `soak_waiver:` block is an operator's decision written down, verified by nothing. `graduate` still refuses on the unmet gate; the page shows the waiver as its own state and never as a met window. |

Two claims left this table when they stopped being true: *"images built on main
but pinned nowhere"* (now answered per component from the image workflows' own
`paths:` filters) and *"whether a soak was waived"* (now a record field, §3).
The `commits_on_head_since` all-commits number stays on the page, still labelled
as counting everything — it is now the secondary reading, under the attributed
one.

Two things the soak clock is honest about: it is measured from the record's
`opened` date, the only date a record carries, so it is a **record clock, not a
deploy clock** — a candidate opened before it reached preprod reads as having
soaked longer than it ran; and a record whose `opened` is unusable renders
"soak unknown" with the reason, never a zeroed countdown.

## 9. Break-glass and the doctrine

An emergency prod change that cannot wait for any record is the deployment
doctrine's territory, not this doc's — but the morning after, the change is
retroactively recorded as a hotfix (backfill owed) or rolled back; `check`
turning red on undeclared prod skew is the reminder mechanism, not a
punishment. Rollback itself is ADR 1060's job and needs no record: reverting
to the prior `fleet:` value is returning *to* the value of record.

## 10. What would make this wrong

Named revisit triggers: a third live rung whose gates differ per component
class (the flat `hops:` list gains per-class gate overrides); records
accumulating unfilled evidence fields (the presence gate is being satisfied
with junk — tighten to schema'd evidence types); the printed-edit workflow
producing frequent C5 failures (adopt comment-preserving YAML editing rather
than asking humans to be more careful).

## 9. Cadence: the daily review-and-act (ADR 1177)

Nothing graduates on its own. A soak ending is not an event the system acts on;
it is a fact a person reads and acts on, and this section is how that reading
is guaranteed to happen.

**The tool.** `ops/release/graduate.py review [--ack <name>]` lists every open
record sorted by what a person must do, each with the exact command:

| Block | Means | Action |
|---|---|---|
| ACT NOW | every hop gate passes; only the ack is missing | read the record and the live state, then run the printed `graduate` command |
| SOAKING | the window has not elapsed | fill the evidence it names before the runnable date it prints |
| BLOCKED | evidence owed or a gate failing; for a fastlane debt, the day `check` C9 goes red | do the `do:` line, re-run the review |
| ATTENTION | undeclared skew, or a `check` failure in the tree | fix the record or pin it names first |

The default view is a table that fits the terminal, one line per item:
STATE · COMPONENT · KIND (candidate: graduating deploys to prod; fastlane: a
debt prod already runs) · AGE · DUE (the soak's end, the day CI goes red, or
`never` for a grandfathered debt) · NEXT, the one clause to do. The summary
line counts how many items would change prod if acted on. `review --detail
[component]` gives the four lines behind any row: **what** it is in the
record's own words, **if left** (drift, a candidate quietly lost, CI red on a
named day, or "only this review will ever show it"), **if acted** (what the
command does, a production deploy called a PROD DEPLOY in capitals), and the
**command**. `make review` is the same thing for a person who does not know
the script's name.

`ops/scripts/daily-review.sh` wraps it: it warns when the checkout is behind
`origin/main` (a stale tree reviews yesterday's ladder), then prints the
review, the developer's tracks (`ops/work/work list`) and open PRs. `--json`
gives a session the same document.

**The ceremony.** Every developer, every working day:

1. **Morning.** Run `ops/scripts/daily-review.sh --ack "<name>"`. Act on ACT
   NOW. Put each SOAKING runnable date in the day. Anything that cannot be
   acted on today is given an owner on the bus.
2. **Evening.** Run it again. A record opened today must show under SOAKING
   with its evidence filled, or its command must have been run. Then the
   close-out (`ops/scripts/closeout-check.sh`, open items classified).

A Claude session does the same through the `/daily-review` skill at start and
before close-out; it runs a graduation only on the operator's explicit
instruction in that session.

**Why not a timer.** The soak is a window in which a person looks. Ending it by
clock would put a change on production unwatched, which is the failure the
ladder exists to prevent. `check` C9 stays as the backstop for a missed
review, never as the mechanism.

