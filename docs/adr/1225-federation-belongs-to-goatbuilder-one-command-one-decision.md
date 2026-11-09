# ADR 1225 — Federation belongs to goatbuilder; one command, one decision

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-09 |
| Deciders | operator |
| Repo scope | goat core (`crates/goat-federation`, `crates/goatbuilder`, `crates/goat-cli`, `ops/scripts/peering-standup.sh`, `ops/federation/standup/`, the federation design documents) |
| Amends | [ADR 1201](1201-two-avenues-scripts-documented-cli-supported.md) rule 2, for the federating-node row only (the script avenue calls the `goatbuilder federate` verbs, from the script's switch-of-default PR) · [ADR 1169](1169-partner-audience-membership-via-spire-federation.md) decision 3 (the verb that renders the partner-audience scope is `goatbuilder federate apply`) · [ADR 1170](1170-federation-growth-by-corroborated-party-roster.md) decision 2 (the verb that refuses an uncorroborated counterparty is `goatbuilder federate apply`) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1198](1198-goatbuilder-config-driven-deploy-release-cli.md) decision 3 (goatbuilder builds environments, goat operates the fabric) · [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) (the peering this command stands up) · [ADR 1202](1202-goatnet-tutorial-program.md) (tutorial 3 wraps `goatbuilder federate`) · [ADR 1208](1208-sovereign-environments-release-namespaces-and-grant-encryption.md) (release sets, the thing offered) · [ADR 1218](1218-peering-forwarder-when-neither-edge-accepts-inbound.md) (the forwarder) · [ADR 1219](1219-peering-edge-on-the-mesh-reaches-only-the-core-router.md) (only platform admins reach the peering edge) · [ADR 1199](1199-one-error-catalog-interpreted-by-both-clis.md) (the error catalog) · [`goatbuilder.md`](../design/goatbuilder.md) · [`goatbuilder-conformance.md`](../design/goatbuilder-conformance.md) · [`federation-package-spec.md`](../design/federation-package-spec.md) · [`peering-directory-service.md`](../design/peering-directory-service.md) |

## TL;DR (for a layperson)

Joining two independent goat environments together ("federating" them) took two tools on
two machines, nineteen settings typed by hand, files copied between hosts and test scripts
pasted into a terminal. From now on it is one command, `goatbuilder federate <partner>`, run
by the platform administrator who already runs goatbuilder to build the environment. The
administrator decides one thing: which partner, and which release sets that partner
receives. The command asks for two confirmations that stay human on purpose: reading the
partner's key fingerprints aloud, and approving what the core will admit before it
restarts, which includes reviewing and merging the resulting configuration change in the
deployment's own repository. Everything else the command does, checks and records itself,
and it can be run again safely or picked up where it stopped. Once the link works and both
sides' acceptance passes, the two administrators make one more deliberate choice: to mark
the partnership as exchanging data.

## Context

Federation is the procedure that turns two sovereign goatnets into peers: each side builds
a signed package describing itself, the two packages compose into one agreement with one
digest, and each side renders and deploys its edge router, tunnel, trust bundle and access
rules from that agreement ([ADR 1157](1157-cross-sovereign-federation-edge-terminated.md)).
Until this decision the procedure was split. The package verbs (`init`, `lint`, `compose`,
`diff`, `verify`, `apply`) lived in `goat`, the everyday operator CLI. The fourteen-phase
driver lived in `goatbuilder federate <with>` and in its script twin
`ops/scripts/peering-standup.sh`, and both shelled out to `goat federate`.

The first federation between two separately administered environments, run on 9 November
2026, showed what that split costs. Nineteen environment variables were exported by hand
before phase 1. In phase 3 the `goat` binary and a signing key were copied to the edge, the
core was reached over SSH for Docker access, and the package was copied back. Verified
keys were typed into a file; the CA signing key was fetched by hand; the pull request, the
router dry run and the router restart were done by hand; per-peering values (tunnel
interface, listen ports, link subnet, edge listen port, republisher metrics port) were
chosen by hand; six configuration files per peering were written and five certificate files
copied and installed by hand; the forwarder's port mapping was changed by hand; and the
acceptance checks ran as pasted scripts. Three failures that a preflight should have caught
stopped the run: the tunnel listen port was already used by NetBird on the edge, the
committed slot list was out of date against the registry, and the core's receive store was
wider than the edge grants and was refused.

Four facts say that `goat` is the wrong home for this code:

1. **The client writes a deployment repository's files.** `apply` renders
   `ops/ansible/group_vars/…`, `ops/federation/partners/…` and `ops/federation/acl/…`
   (`crates/goat-federation/src/render.rs`, moved from `crates/goat-cli/src/fed/`). Writing an
   environment's configuration is goatbuilder's job ([ADR 1198](1198-goatbuilder-config-driven-deploy-release-cli.md)
   decision 3), not the fabric client's.
2. **The client guesses deployment facts.** `goat federate init` has to find the core router
   container by name pattern and label ([`federation-router-discovery.md`](../design/federation-router-discovery.md),
   issue #238), because the fabric client does not hold the deployment's declaration.
   goatbuilder does: the site, its classes and its inventory groups are in `goatbuilder.yaml`.
3. **One federation needed two tools on two machines.** The driver ran on the operator's
   machine; `goat federate init` had to run where the core router is, so phase 3 copied a
   binary and a signing key to the edge and the package back.
4. **Only the platform admin federates, and that person runs goatbuilder.** Only platform
   admins reach a peering edge ([ADR 1219](1219-peering-edge-on-the-mesh-reaches-only-the-core-router.md)),
   and the same person stands the environment up with goatbuilder. Every `goat` user carrying
   the federation verbs serves no one who can use them.

The operator who ran the first sovereign federation, in a proposal of 9 November 2026
("Federation automation"), reached the same entry point from the
operator's side: make goatbuilder the operator entry point for a resumable federation
workflow driven by reviewed configuration, with automated acceptance evidence from each
operator. This decision is the best of both: it keeps this program's structure and adopts the
proposal as the specification for configuration ownership, plan and state, acceptance, and
`apply` staying a renderer (decision 5).

## Decision

### 1. One home: the `goat-federation` library and `goatbuilder federate`

- All federation code lives in the library crate `crates/goat-federation/` (package format,
  lint, compose, diff, verify, render, keyring) and in the goatbuilder driver
  `crates/goatbuilder/src/federate/`. The crate move is built, with no behavior change.
- goatbuilder carries the six verbs with the same flags and exit codes `goat federate` has
  today: `goatbuilder federate init|lint|compose|diff|verify|apply`. `apply` stays a renderer:
  it writes files and deploys nothing. Deploying is a separate, named stage (decision 5).
- The new home arrives beside the old one. The driver keeps spawning `goat federate` by
  default, and `ops/scripts/peering-standup.sh` keeps calling `goat federate`, until the test
  environment shows the goatbuilder verbs pass the same scenarios; each of those two switches
  of default is its own small pull request (decision 7).
- `goat` keeps no federation code of its own. Until a goatbuilder release carrying the verbs
  is available to partner environments, `goat federate <verb>` keeps working through the
  same library, because partner environments run released `goat federate` today, and prints
  one line on stderr:
  ``goat federate is moving to goatbuilder: run `goatbuilder federate <verb> ...` ``.
  Once that goatbuilder release is available, `goat federate` is removed from `goat`.
- Every federate subcommand name (`init lint compose diff verify apply plan deploy test
  status`) is reserved and can never be a partner name; config validation refuses it.

### 2. One command, one decision

`goatbuilder federate <partner>` is the whole procedure. The person makes one decision:
**which partner, and which release sets that partner receives**. The offered release sets
are owned by the deployment's `federation.toml`, which `init` already reads (decision 5,
"Where this departs from the proposal"). The partner is a peering entry in the deployment's
own `goatbuilder.yaml` (`with`, `environment`, `site`, `edge_class`); that entry's `offer:`, or
`--offer <SET>[,<SET>...]`, may only narrow the sets `federation.toml` offers, is validated as
a subset, and is never a second owner. Every other value is read from the deployment's configuration or allocated by the command
(decision 4). Nothing about a deployment is compiled into the binary; a missing value stops
with an error from `ops/errors/catalog.yaml` that names what to supply.

### 3. Two human confirmations, kept on purpose, and one deliberate act afterwards

1. **Identity.** The two operators read the partner's verifying key and CA fingerprint aloud
   over a channel they already trust, and compare the agreement digest. The command shows
   the values and records the confirmation; it never accepts a key on its own. The reason:
   the directory, or any channel the command can reach, is a cache of the binding and never
   its source ([`federation-package-spec.md`](../design/federation-package-spec.md) §5.1,
   [ADR 1170](1170-federation-growth-by-corroborated-party-roster.md)). A first-contact binding
   that a machine accepts by itself is a binding an attacker who controls that channel can
   forge.
2. **Admission.** Before the core's router restarts, the operator approves what the core will
   admit: the access-control fragment and the receive scopes rendered from the agreement. The
   reason: this is the one act that changes what the sovereign's own core accepts from
   outside, it restarts a production router, and only this party can change this party's
   core. The approval is bound to the hash of exactly what it approves; changed inputs need a
   new approval. The review and merge of the allocated values and the rendered configuration
   in the deployment's own repository is part of this confirmation: it is the deployment's
   existing change process, and the admission approval covers it.

Both stay human because each is a judgement about trust that the software cannot make for
the operator: one about who the partner is, one about what the partner may put into the
core.

A third human act comes after the federation works, not during it: **publishing
`exchanging`**. Both operators do it, through a reviewable change, after the required
bilateral acceptance scope passes. It is deliberate because it is each sovereign's statement
that the partnership carries data, and acceptance evidence alone never makes it.

### 4. Everything else is done, checked, recorded, idempotent and resumable

Everything outside decisions 2 and 3 is the command's job:

- read every input from the deployment's configuration, with no hand-exported environment;
- build our package on the core host over SSH, with no binary or key copied to the edge;
- fetch the partner's package from the directory, and write verified keys into the keyring
  only after confirmation 1;
- allocate the per-peering values (tunnel interface, listen ports, link subnet, edge listen
  port, republisher metrics port, certificate directory), stably across re-runs and without
  collision between peerings on one host, and propose them as a reviewable change in the
  deployment's configuration;
- check every prerequisite before changing anything (ports already in use, the slot list
  against the registry, receive scopes against the edge's grants, certificate names and
  expiry, conflicts with existing peerings), and recheck the live ones immediately before
  changing anything;
- mint the per-peering certificates from the agreement in phase 8 with the existing script
  and the operator-held CA key, resolving that key itself (no hand fetch), and install the
  minted files through the edge role; configure the forwarder from the declaration; and run
  acceptance;
- record intent, progress and evidence separately in the working directory (`state.json`,
  `evidence/`), so a re-run resumes by inspecting actual state and never duplicates a
  resource or removes another partnership.

Each item is designed in the section of [`goatbuilder.md`](../design/goatbuilder.md) that
the code implementing it updates; this ADR does not mark any of them built.

### 5. What is taken from the proposal, and what this program adds

Taken from the proposal of the operator who ran the first sovereign federation (9 November
2026), as its specification:

- **Configuration ownership** (the proposal's section 1). The verified agreement determines the permitted
  flows. Local configuration may narrow them or demand stronger checks and never expands them.
  One owner per field; no second hand-kept ACL or lane list.
- **Plan and state** (the proposal's section 2). A plan before any change, carrying versions, the
  agreement digest, rendered-configuration hashes, resource ownership and restart
  requirements; approval bound to that plan's hash; live prerequisites rechecked immediately
  before any change; intent, progress and evidence kept as three separate things; a run
  journal; resume by inspecting actual state; per-peering locks and a host-level allocation
  check.
- **Acceptance as a contract** (the proposal's sections 3 and 4). Checks generated from the agreement's
  actual flows, including send-only, receive-only and asymmetric agreements; a versioned check
  and result schema; the outcomes PASS, FAIL, BLOCKED, SKIP and NOT APPLICABLE with the
  proposal's meanings, where a required check cannot be waived into PASS; persistent reception proven
  with no diagnostic subscriber; refusal shown by the receiving edge's allow and deny records
  beside an allowed control; the republisher boundary as its own check; existing partnerships
  checked before and after; bilateral evidence with each operator running locally on their own
  systems, a session manifest, result bundles exchanged by file, peer-reported results kept
  apart from local observations, and no automatic bilateral gate until report signing and
  replay rules exist; probes time-limited, run-owned and cleaned up; `exchanging` published
  only through a reviewable change after the required scope passes on both sides.
- **`apply` stays a renderer, and the stages are the proposal's.** `goatbuilder federate plan |
  deploy | test | status` are the stages of the one command, each also runnable alone:
  `plan` validates and explains the local change, `deploy` executes an approved plan through
  the existing roles, `test` starts or resumes a bounded acceptance session, and `status`
  shows deployment health and the last acceptance evidence with its age and scope.
  `goatbuilder federate <partner>` runs them in order, with the two human stops of decision 3.
- **Order.** Acceptance first, then the rest in the proposal's five-step order where the changes stack.

What this program adds to the proposal: the library crate and goatbuilder as the only home
of the package verbs (the proposal kept `goat federate` as the package tooling goatbuilder
invokes; see Alternatives); phase 3 measured over SSH from the operator's machine; allocated
per-peering values; certificates installed by the roles; the forwarder applied by the driver
from the declaration; several peerings per party in the directory (below).

**Where this departs from the proposal, or settles what it left open:**

- **Certificates.** The program does not add automatic certificate issuance, which the
  proposal judged too risky for a first release. Phase 8 still mints from the agreement with
  the existing script and the operator-held CA key. The program automates resolving that key
  (stage 6.1) and installing the minted files on the edge (stage 7).
- **Offer intent.** Decided now: `federation.toml` owns the offered release sets. That is the
  proposal's placement and what `init` already reads. A `peerings:` entry in `goatbuilder.yaml`
  may only narrow them, is validated as a subset, and is never a second owner.
- **Acceptance policy.** Derived from the agreement's flows. Local configuration may only
  tighten it, so it needs no separate decision. NOT APPLICABLE is valid only for scope the
  agreement excludes.

### 6. Directory: several peerings per party

Today the directory holds one package per party, and a package carries one tunnel offer, so
a party's second peering's package replaces its first.

**Decision: one package per counterparty.** A party that holds several peerings will sign
one package per counterparty and host each in its own directory slot,
`/v1/parties/<us>/packages/<counterparty>`. The package format does not change
(`federation-package/v1`: same members, signing, composition and digest), and the single
slot `/v1/parties/<us>/package` and its pointer record keep working unchanged for partners on
released tools. None of this is on `main` yet; it arrives as stage 11, opt-in.

- **The format already fits.** A package names no counterparty; composition takes two packages
  and sorts them by `sovereign_id` ([`federation-package-spec.md`](../design/federation-package-spec.md)
  §2). One package per peering keeps one `transport_offer` per package, so nothing in the
  negotiated fields changes and no spec version is needed.
- **It matches the allocation.** Decision 4 allocates a separate tunnel interface, listen port
  and /31 link subnet per peering; a package per counterparty carries exactly those values.
- **Existing agreements stay valid.** Only where the bytes live changes, never the bytes. A
  package fetched from a counterparty slot composes to the same canonical bytes and the same
  digest as before.
- **A misdelivered package is caught by the slot's upload signature, not by the digest.** The
  agreement carries neither the tunnel endpoint nor the tunnel public key, and takes the link
  subnet from only one party, so two of one party's packages that differ only in those members
  compose to the same agreement and the same digest: the read-aloud comparison (decision 3) does
  not tell them apart. The upload signature names the slot (this party, this counterparty), so
  a reader MUST verify it, under the key read aloud, before using a package fetched from a slot;
  the directory step does, and places nothing when it does not verify.
- **What the directory learns.** The slot name is the counterparty's id, so a reader of the
  directory will be able to see that two parties hold a peering. The directory still computes
  nothing about who should peer with whom; the slot is the party's own statement, and the
  catalogue will not list slots. A party that does not want the relationship visible exchanges its package out of
  band or from its own host, as today.
- **How it arrives.** The directory service gains `GET|PUT /v1/parties/{id}/packages/{counterparty}`
  beside the old route and merges first; `ops/federation/standup/directory.py` then publishes
  and looks up per counterparty behind an opt-in switch, falling back to the pointer, and
  refreshes the single slot only when it is empty or already holds this same peering's
  package. Making the switch the
  default is a later, separate change.

Rejected: **one listener serving several partners through one package.** One offer valid for
many partners means one WireGuard key, one endpoint and one `proposed_link_subnet` shared
across peerings, and one /31 cannot serve several links; spec §7 takes the link subnet from
`parties[0]`'s proposal, so two partners' proposals could collide on our side depending only
on sort order. Fixing that changes agreement bytes and digests, needs a v2 specification and
new vectors, and a change in every partner's implementation.

### 7. Modules, seams and the graduated ladder

The program is built as modules. One pull request is one module with one purpose; a code move
and a behavior change never share a pull request. Each module can be reverted alone, and its
pull request says what stops working, what keeps working and which scenarios change outcome
when it is reverted. New behavior arrives beside the existing behavior; the existing avenue
stays the default until the federation test environment shows the new one passes the same
scenarios, and **each switch of default is its own small pull request**, so undoing it is one
revert. This ADR records each module's owner, what it stands on and how it reverts alone.
Interfaces, artifacts and the paragraph a later change reads are in
[`goatbuilder.md`](../design/goatbuilder.md) §"Federation modules and their seams", which each
module's pull request updates, so a later change to one module needs no reading of the others.

A stage graduates only when the scenarios named for it pass in the test environment on that
branch and no scenario that passes on `main` regresses. The ladder is the merge order:

| Stage | Module | Owns | Stands on | Revert alone |
|---|---|---|---|---|
| 0 | Test-environment baseline | the scenario matrix on `main` | — | the harness keeps running as today |
| 1 | Acceptance cannot pass on skipped checks | the exit status of `ops/federation/standup/acceptance.sh` | — | skips pass again unless `FED_ACCEPTANCE_STRICT` is set |
| 2 | Evidence schema and outcome words | the versioned check and result schema | 1 | acceptance prints stage 1 output |
| 2.x | Acceptance from the agreement | agreement-derived checks, bilateral exchange by file, probe cleanup, publishing `exchanging` | 2 | each part reverts to the fixed checks |
| 3 | Library crate, pure move | `crates/goat-federation` | — | the code returns to `goat-cli` (later stages first) |
| 4 | Verbs into the library, pure move | the six verb implementations | 3 | they return to `goat-cli` |
| 5 | goatbuilder verbs beside `goat`'s | `goatbuilder federate init\|lint\|compose\|diff\|verify\|apply` | 4 | the driver and script keep using `goat federate`, still the default |
| 5.1 | Driver runs the verbs in process, opt-in | the driver's opt-in to call the verbs in process | 5 | the driver spawns `goat federate` |
| 5.2 | Notice in `goat federate` | the one-line notice naming the goatbuilder command | 5 | `goat federate` prints no notice |
| 6 | Planner and preflight | `goatbuilder federate plan` | 3 | `preflight.py` stays phase 1 |
| 6.1 | Phase 8 resolves the CA key | locating the operator-held CA key for minting | 6 | a hand-supplied key file is used, as today |
| 7 | Edge role installs the minted certificates | installing the per-peering certificate files on the edge, opt-in | 3 | the files are placed by hand |
| 8 | Allocated peering values | per-peering values, proposed as a reviewable change | 3 | per-peering files are written by hand |
| 9 | Phase 3 over SSH | building our package on the core from the operator's machine | 5 | phase 3 runs on the host |
| 10 | Forwarder from the declaration | the forwarder's port mapping | 8 | the mapping is set by hand |
| 11 | Directory, one package per counterparty | per-counterparty slots, opt-in (decision 6) | 3 | the single slot stays |
| 12 | The one-command surface | `deploy`, approval bound to the plan, journal, locks, resume, `test`, `status` | 5, 6 | phases run as today |
| 13+ | Switches of default, each alone | one default each: the driver in process; the script on goatbuilder; the directory per counterparty; removal of `goat federate` | its module | one revert each |
| 14 | Teaching material | the runbook, tutorials and guides | 13+ | the earlier text returns |

Two seams cross stages. Until stage 8 lands, stages 6 and 7 take the per-peering values as
parameters, and their call sites are wired to the allocation when stage 8 merges. Stage 9
runs our package's `init` through the stage 5 verbs, so it stands on stage 5.

## Alternatives considered

- **Keep `goat federate` as the package tooling goatbuilder invokes** (the proposal's
  position). It keeps the protocol tooling in the binary every operator already has, and a
  party without goatbuilder could still build and check a package. Rejected because of the
  four facts above: the tool still writes deployment files, still guesses deployment facts,
  still has to run on the core host, and serves only the platform admin, who runs goatbuilder.
  The library keeps the protocol code in one reusable place, and the transition shim keeps
  released `goat` working for partners until a goatbuilder release reaches them.
- **Copy the verbs into goatbuilder and leave them in `goat` as well.** No transition step for
  anyone. Rejected: two implementations of a signed-package protocol drift, and a drift there
  is a security defect, not a convenience gap.
- **Make the command fully unattended.** No confirmations at all, keys accepted from the
  directory and the core restarted on success. Rejected for the reasons in decision 3: the
  directory is never the source of a first-contact binding, and changing what a sovereign's
  core admits is that sovereign's decision.
- **A long-running federation controller service.** Continuous reconciliation of every
  peering. Rejected for the first version, as the proposal also rejected it: a new
  always-running service with authority over two sovereigns' edges adds more risk than a
  re-runnable, resumable command.

## Consequences

**Positive.**

- One tool, run from one machine, by the one person who may federate. Phase 3 no longer copies
  a binary or a signing key anywhere.
- The deployment's configuration is the only source of a peering's values; the command
  allocates, checks and records them, so the hand-chosen values and the three failures of the
  9 November run become preflight refusals with catalog codes.
- `goat` loses code that most of its users could never run.

**Negative.**

- Partner environments running released `goat` need the transition: `goat federate` stays,
  through the same library, until a goatbuilder release with the verbs is available to them,
  and the one-line notice has to be maintained until then.
- The script avenue changes shape. **Amendment to [ADR 1201](1201-two-avenues-scripts-documented-cli-supported.md)
  rule 2, federating-node row only:** both avenues stay, the driver (`goatbuilder federate`)
  and `ops/scripts/peering-standup.sh`, and both call the `goatbuilder federate <verb>`
  package verbs, as both called `goat federate` before. This holds from the script's own
  switch-of-default pull request (decision 7); until then the script calls `goat federate` and
  needs no goatbuilder binary. The script still orchestrates its own
  phases and is still documented and unsupported; it now depends on the goatbuilder binary for
  the package verbs only, because a second implementation of the package protocol is the
  drift the previous alternative rejects.
- The parity gate `ops/goatbuilder/parity-federate.sh` keeps the two avenues one behavior,
  and has to follow every change to the plan.
- The federation end-to-end test (`ops/federation-e2e/run.sh`, workflow
  `.github/workflows/federation-e2e.yml`) becomes the gate for this decision: it passes when it
  drives both sides with `goatbuilder federate` alone. Until it does, the program's design
  sections carry `Status: WIP — gate: the federation end-to-end test drives both sides with
  goatbuilder federate alone`.
- **Amendment to [ADR 1169](1169-partner-audience-membership-via-spire-federation.md)
  decision 3 and [ADR 1170](1170-federation-growth-by-corroborated-party-roster.md) decision
  2:** the verb that renders the partner-audience scope, and the verb that refuses an
  uncorroborated counterparty, is `goatbuilder federate apply`. Their decisions are otherwise
  unchanged.

## Assessor view (SCA-R)

The decision narrows where federation authority sits: the package signing, the agreement and
the rendered access rules are handled by one binary run by platform admins (AC-6 least
privilege, CM-5 access restrictions for change), instead of by the fabric client every
operator carries. The two human confirmations are the controls an assessor should look for:
identity corroboration out of band before a first-contact binding (IA-3, IA-5, SC-12), and an
approval bound to the hash of the exact admission change before a core router restarts (CM-3,
CM-4). Intent, progress and evidence are recorded separately, and acceptance evidence is tied
to one run and one agreement (AU-2, AU-3, CA-2). The residual risk is the transition period,
when the same protocol is reachable from two binaries; the compensating control is that both
call the same library, so there is one implementation to assess.

## Trigger conditions for revisit

- A goatbuilder release carrying the verbs is available to partner environments: remove
  `goat federate` from `goat` (a consequence of this decision, not a revisit).
- A party needs to build or check a package without running goatbuilder (for example, a
  partner that federates with a non-goat implementation of the package spec): consider a
  standalone package tool built on the same library.
- Report signing and replay rules for bilateral acceptance are built: revisit whether a
  bilateral acceptance gate may run automatically.
- A deployment runs enough peerings that a re-runnable command no longer keeps them converged:
  revisit the rejected controller.
