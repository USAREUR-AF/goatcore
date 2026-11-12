# Goatnet deployment doctrine — the single holistic path, and how compliance is proven

**Read this before doing any goatnet standup, deployment, or config work — human
or AI.** It is the set of principles that keep goatnet deployment on ONE
holistic path instead of splitting into per-environment forks and one-off
scripts.

Each principle is paired with the **mechanism that proves compliance** — a check
you can run, that CI runs, or that fails a standup. A principle without an
enforcing mechanism is marked **TO BUILD** honestly; do not treat it as
enforced. The point is *provable* compliance: you should not be able to
accidentally leave the path without something going red.

**How this stays discoverable:** CLAUDE.md's Site-deployments pointer leads here
and to [`goatnet-standup.md`](../operations/goatnet-standup.md); `check-deployment-doctrine.sh`
(CI) asserts the load-bearing artifacts still exist and are referenced, so the
doctrine cannot silently rot or be bypassed.

---

## P1 — One source of truth, one entrypoint. Never a second "THE procedure."

The procedure is [`docs/operations/goatnet-standup.md`](../operations/goatnet-standup.md)
(the doc). Its entrypoint comes in two avenues that
[ADR 1201](../adr/1201-two-avenues-scripts-documented-cli-supported.md) keeps
functional — the supported `goatbuilder` CLI (`goatbuilder standup`) and the
documented `ops/scripts/goat-standup.sh` — both running the same phases from the
same config (P12). The ~20 other
bringup/standup docs are *why* / *reference* / *per-env instance* — never a
parallel procedure. A new standup adds an execution *instance* or a *reference*,
and it must point back to the single source. Do not fork the path into a new
"how to stand up X" that a reader could mistake for canonical.

**Proven by:** `ops/ci/check-deployment-doctrine.sh` — fails if the canonical
doc or entrypoint is missing/unreferenced, or if a new `*-bringup.md` /
`*standup*.md` under `docs/operations/` neither is the canonical doc nor points
back to it (a silent second procedure).

## P2 — Absence is data, never a comment and never silence.

Every intentional omission — a service, a group_vars file, a cloud component, a
service on a standby node — is a **declared exclusion with a reason** in a spec a
checker reads. A missing-and-undeclared thing FAILS and forces a decision. "An
absence with no declared home cannot be reported"; a decision recorded only in a
code comment is invisible and does not count.

**Proven by:** `.groupvars-parity.yaml` (file-inventory), `ops/pins/cloud-parity.yaml`
(cloud tree), `ops/standby/service-classes.yaml` (`check-standby-cutover-gated.sh`).
Each fails on an undeclared absence; each strips comments before matching so a
check can't green on the very comment that documents the absence.

## P3 — A config surface is a FILE SET, not a `vars.yml`.

A new environment's config is the reference environment's **file inventory**
minus recorded exclusions, re-threaded — not one hand-written vars file.

**Proven by:** `standup-config-preflight.py` file-inventory check against
`.groupvars-parity.yaml`.

## P4 — Coupled facts derive or assert, before a host is touched.

Two values that must move together (image pin ↔ variant flag; cert ↔ key;
mesh-IP ↔ every IP-pinned artifact) are either derived from one source or
asserted as a pair by preflight. The coupling must not live in a human's head or
a comment. Mint IP-pinned artifacts only after the *final* enrollment; any
re-enroll triggers the re-render checklist.

A **topology** rule is a coupled fact too, and the same sentence applies: a
constraint stated in an ADR and echoed in a rollback ledger is stated in two
places and enforced in neither. ADR 1084 §D3 single-homes every admin-mutation
surface (portal, id-admin-console, netbird-dashboard, the onboarding engine) on
one node per goatnet. `tasks/assert-admin-write-host.yml` is now the first
pre-task of all five plays: it skips any host whose `router_role` is not the
declared writer, and refuses outright when a multi-host play declares none.
Promotion to the second node stays the operator action §D3 says it is —
`-e admin_write_role=B` **moves** the single home; there is deliberately no flag
that turns the guard off.
Which components may be single-homed at all, and why only those, is P13.

**Proven by:** `standup-config-preflight.py` (coupled-fact asserts) + the
cert-mint orphan-key guard + `tasks/assert-admin-write-host.yml` (single-homed
admin surfaces). Some couplings remain **TO BUILD** (derive `fips_variant` from
the pinned image name).

## P5 — A default every environment overrides is a bug in the default.

If every goatnet overrides a role default to the same value, fix the default and
delete the overrides. Three separate standup failures were "the skeleton forgot
the override everyone carries" (sidecar port, `signal_proto`, the surface
network/dial defaults).

**Proven by:** **TO BUILD** — the universal-override sweep + a CI guard against
re-introduction (remediation plan class C3). Until built, this is a review
discipline, not enforced.

## P6 — Honesty grammar. Nothing-heard is never green.

Every health surface and every check reports `PASS | FAIL | UNKNOWN | N/A |
SKIPPED`. A probe that cannot run is UNKNOWN, not PASS. A gate counts only PASS.
And every new check is **proven RED against a known-broken fixture before its
green is trusted** — a checker that greens on its own excuse is worse than none.

**Proven by:** the acceptance suite's honesty grammar + `run-acceptance.py
--self-test` (red-first per probe type). Applies to `EnvHealthV1`/ConsistencyV1
surfaces too (ADR 1034/1110).

## P7 — Prove function, not process. Resolved-in-code ≠ resolved-in-prod.

"Running" is not "working": a healthy container can have dead storages, a clean
render can carry empty auth, an alert dir can be empty. Every provisioned aspect
gets an **outside-in** acceptance test (probed from the mesh-connected
controller, the path a real peer takes — self-report is what lied). A change is
not done until re-probed live.

**Proven by:** the acceptance suite (`run-acceptance.py --all-eligible`, the
phase-13 gate). Each row cites the defect it catches.

## P8 — Never-exercised is untested. Prove from-fresh.

A role/var/path that has only run on its birth environment is unproven
elsewhere. "Works on the existing env" that relies on hand-applied host state
(the pg_hba admit rule that made Phase 6A impossible from-fresh) is a defect, not
a baseline.

**Proven by:** **TO BUILD** — an offline per-env render+vars lint (render every
template + resolve every referenced var against the target env's group_vars,
before any host). Remediation plan class C1. Until built, the from-fresh
acceptance run is the backstop.

## P9 — IDs unique; MERGED is not proof; verify content on main.

ADR numbers (and finding IDs) are unique. A PR showing MERGED is **not** proof
the intended content landed — a mis-created PR can merge an empty or wrong diff.
After any merge, verify the content is actually on main
(`gh api …/contents/<path>?ref=main`), and always create PRs with an explicit
`--head <branch>`.

**Proven by:** `ops/ci/check-adr-number-uniqueness.sh`; the verify-on-main
practice (operator discipline, backed by the finding-ID race guards).

## P10 — Every gap feeds the mechanism, not just the fix.

When a standup hits a defect, you do two things: fix the instance, and add its
*class* to an enforcing mechanism — a preflight check or an acceptance row — so
the next standup cannot hit it. This is the loop that converts "followable" into
"certified turnkey". File every gap against
`standup-remediation-plan.md` with its
class (C1–C14).

**Proven by:** the remediation plan governs it; a standup that ends without
folding its new gaps into preflight/acceptance has not closed out.

---

## P11 — Every declared host is built by the driver; independent hosts build in parallel.

A host class the single path does not build is a host class the single path
cannot *re*-build, and one the acceptance gate only half-sees. The obs host
lived for a year as "a separate operator-fired play, not a phase": it got
re-created by hand after each Docker data-root rebuild and the Forgejo it
carries — the pub/sub state-of-record primary — did not, twice
(F-504).
So: when a site declares a host (`has_obs_host: true`, a config surface, an
inventory row), `goat-standup.sh` builds it, and everything on it that does not
depend on the core host runs **concurrently** with the core phases — a
background leg with its own log, joined at the first phase that needs it. A
declared-but-absent host is skipped with the reason, never an error.
"Identified and prepped" is decided by the driver from the config and the
inventory, not by an operator remembering a second play.

**Proven by:** `goat-standup.sh <site> --plan` prints the obs leg (`∥`) for a
site that declares one; `--obs-only` re-runs it; the gate's obs and
state-of-record rows grade the result.

---

## P12 — Two avenues, both functional: the CLI is supported, the scripts are documented.

Every job that builds or ships a goatnet can be done two ways, and
[ADR 1201](../adr/1201-two-avenues-scripts-documented-cli-supported.md) keeps
both working: the `goatbuilder` CLI and the shell and Python scripts that have
always done it. The CLI is the supported avenue — the runbooks, the one-pagers
and the error catalog teach it (`goatbuilder standup`, `join-node`, `federate`,
`deploy`, `release`, `rollback`, `fastlane`, `promote`, `pins`). The scripts
stay in the repository, documented and exercised by the same gates, and are the
alternative (`goat-standup.sh`, node-addition.md's plays, `peering-standup.sh`,
`rc.sh`, `deploy-component.sh`, `rollback.sh`, `fastlane.sh`, `graduate.py`,
`generate-pins.py`). Neither avenue is deleted; a script is never removed
because its verb exists. Both read the same pins, ladder, records and intents,
and the scripts' embedded edits and the CLI's typed edits produce
byte-identical files — two avenues, never two behaviours.

**Proven by:** the parity gates (`ops/goatbuilder/parity-*.sh`) and every tier
of the deployment e2e harness ([`deployment-e2e.md`](deployment-e2e.md)) run
both avenues; a failure on either fails the check. A script runs on its own and
never depends on the `goatbuilder` binary.

---

## P13 — Every site runs every surface it needs to operate cut off.

A goatnet keeps working at a site that has lost its uplink
([ADR 0206](../adr/0206-island-mode-by-construction.md)), and island mode puts
writes at every site, one write class at a time
([ADR 1165](../adr/1165-island-mode-multi-writer-every-site-admits.md), which
amends [ADR 1084](../adr/1084-block-64r-mgmt-repset-and-two-node-federation.md)
§D3). So the default placement of a component is **every site of its goatnet**.
A second or joining site that lacks a surface is a site that cannot run that
surface while cut off.

Four placements, and the only reasons for each:

| Placement | Applies to | Declared as |
|---|---|---|
| Every site | Read-only and per-site surfaces: viewers, inspection pages, download and docs surfaces, node-scoped publishers | `read_only: true` on the component in `ops/pins/fleet-pins.yaml` for a read-only surface; no `absent` deviation on any live site |
| One node per goatnet (single-homed) | An admin write class that ADR 1165 has not yet lifted: the surfaces ADR 1084 §D3 names (portal, id-console, dashboard, `goat admin`) and the engines that write the same stores | `single_homed_role: "<router_role>"` on the component; its play runs `tasks/assert-admin-write-host.yml` |
| One node per site | A per-site roll-up publisher that writes one key for the whole site: the fabric, mesh, time and presence health roll-ups (`ops/<domain>/health/<site>`) and the single lazy crawler. Every site runs one; a site whose host group holds two nodes runs it on the node that runs the router, because a second copy is a second writer to the same key | `single_homed_role: "<router_role>"` on the component; its play runs `tasks/assert-admin-write-host.yml` with a `single_home_reason` |
| Absent at a named site, for a stated reason that is not single-homing | A dormant standby that routes nothing until promotion (ADR 1111 `route` trigger); a one-per-trust-domain singleton such as a certificate authority (ADR 1111 `singleton` trigger); a site that does not run the component yet, naming what it lacks | `absent: true` deviation whose `reason:` says which |

A read-only surface is never single-homed. When ADR 1165 lifts a write class,
its components lose `single_homed_role` and run at every site.

A surface that runs at a site with no local Zenoh router of its own (a passive
node whose router copy is stopped) points its endpoint at the active router over
the mesh and does not join a local router network; the role carries the switch,
the host's vars set it.

**Proven by:** `resolve.check_placement` in
`ops/consistency/python/goat_consistency/resolve.py`, run by
`ops/pins/generate-pins.py --check` on every pull request: it refuses a
deviation whose reason claims single-homing (`single-homed`, `D3`) unless the
component declares `single_homed_role`, and refuses `single_homed_role` on a
component declared `read_only: true`. Tests:
`ops/consistency/python/tests/test_placement.py`.

---

## The honesty invariant

The single path is **not yet certified turnkey** — it is proven correct offline
and on paper, pending a live **cold-read drill** (a non-author, no AI, this doc
set only, zero improvisation). `goatnet-standup.md` carries this warning at the
top. Do not present the path as certified turnkey until the drill has passed;
do not remove the warning until it has.
