# ADR 1225 — Federation belongs to goatbuilder; one command, one decision

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-09 |
| Deciders | operator |
| Repo scope | goat core (`crates/goat-federation`, `crates/goatbuilder`, `crates/goat-cli`, `ops/scripts/peering-standup.sh`, `ops/federation/standup/`, the federation design documents) |
| Amends | [ADR 1201](1201-two-avenues-scripts-documented-cli-supported.md) rule 2, for the federating-node row only (the script avenue calls the `goatbuilder federate` verbs) · [ADR 1169](1169-partner-audience-membership-via-spire-federation.md) decision 3 (the verb that renders the partner-audience scope is `goatbuilder federate apply`) · [ADR 1170](1170-federation-growth-by-corroborated-party-roster.md) decision 2 (the verb that refuses an uncorroborated counterparty is `goatbuilder federate apply`) |
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
restarts. Everything else the command does, checks and records itself, and it can be run
again safely or picked up where it stopped.

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

An operator who ran the 9 November federation wrote a proposal,
"Federation automation" (9 November 2026), that reached the same entry point from the
operator's side: make goatbuilder the operator entry point for a resumable federation
workflow driven by reviewed configuration, with automated acceptance evidence from each
operator. This decision adopts the proposal's design for configuration ownership, plan and state, and
acceptance (decision 5).

## Decision

### 1. One home: the `goat-federation` library and `goatbuilder federate`

- All federation code lives in the library crate `crates/goat-federation/` (package format,
  lint, compose, diff, verify, render, keyring) and in the goatbuilder driver
  `crates/goatbuilder/src/federate/`. The crate move is built, with no behavior change.
- goatbuilder carries the six verbs with the same flags and exit codes `goat federate` has
  today: `goatbuilder federate init|lint|compose|diff|verify|apply`. `apply` stays a renderer:
  it writes files and deploys nothing. Deploying is a separate, named stage of the driver.
- `goat` keeps no federation code of its own. Until a goatbuilder release carrying the verbs
  is available to partner environments, `goat federate <verb>` keeps working through the
  same library, because partner environments run released `goat federate` today, and prints
  one line on stderr:
  ``goat federate is moving to goatbuilder: run `goatbuilder federate <verb> ...` ``.
  Once that goatbuilder release is available, `goat federate` is removed from `goat`.
- Every federate subcommand name (`init lint compose diff verify apply status`, and any stage
  name added under decision 5) is reserved and can never be a partner name; config
  validation refuses it.

### 2. One command, one decision

`goatbuilder federate <partner>` is the whole procedure. The person makes one decision:
**which partner, and which release sets that partner receives**. It is declared once, in
the deployment's own `goatbuilder.yaml`, as a peering entry (`with`, `environment`, `site`,
`edge_class`, and `offer:` naming the release sets), or given as
`--offer <SET>[,<SET>...]` and proposed back into that file as a reviewable change. Every
other value is read from the deployment's configuration or allocated by the command
(decision 4). Nothing about a deployment is compiled into the binary; a missing value stops
with an error from `ops/errors/catalog.yaml` that names what to supply.

### 3. Two human confirmations, kept on purpose

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
   new approval.

Both stay human because each is a judgement about trust that the software cannot make for
the operator: one about who the partner is, one about what the partner may put into the
core.

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
- issue and install the per-peering certificates through the roles, configure the forwarder
  from the declaration, and run acceptance;
- record intent, progress and evidence separately in the working directory (`state.json`,
  `evidence/`), so a re-run resumes by inspecting actual state and never duplicates a
  resource or removes another partnership.

Each item is designed in the section of [`goatbuilder.md`](../design/goatbuilder.md) that
the code implementing it updates; this ADR does not mark any of them built.

### 5. What is taken from the proposal, and what this program adds

Taken from that operator's proposal, as its specification:

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
- **Stages.** The proposal's `plan`, `deploy`, `test` and `status` are the stages of the one
  command, and each is also runnable alone.
- **Order.** Acceptance first, then the rest in the proposal's five-step order where the changes stack.

What this program adds to the proposal: the library crate and goatbuilder as the only home
of the package verbs (the proposal kept `goat federate` as the package tooling goatbuilder
invokes; see Alternatives); phase 3 measured over SSH from the operator's machine; allocated
per-peering values; certificates installed by the roles; the forwarder applied by the driver
from the declaration; several peerings per party in the directory (below).

### 6. Directory: several peerings per party

Today the directory holds one package per party, and a package carries one tunnel offer, so
a party's second peering's package replaces its first.

**Decision: one package per counterparty.** A party that holds several peerings signs one
package per counterparty and hosts each in its own directory slot,
`/v1/parties/<us>/packages/<counterparty>`. The package format does not change
(`federation-package/v1`: same members, signing, composition and digest), and the single
slot `/v1/parties/<us>/package` and its pointer record keep working unchanged for partners on
released tools.

- **The format already fits.** A package names no counterparty; composition takes two packages
  and sorts them by `sovereign_id` ([`federation-package-spec.md`](../design/federation-package-spec.md)
  §2). One package per peering keeps one `transport_offer` per package, so nothing in the
  negotiated fields changes and no spec version is needed.
- **It matches the allocation.** Decision 4 allocates a separate tunnel interface, listen port
  and /31 link subnet per peering; a package per counterparty carries exactly those values.
- **Existing agreements stay valid.** Only where the bytes live changes, never the bytes. A
  package fetched from a counterparty slot composes to the same canonical bytes and the same
  digest as before.
- **A misdelivered package is caught.** If one side composes with the package meant for a third
  party, the digests differ and the read-aloud comparison (decision 3) fails visibly. The upload
  signature names the slot, so a reader checks under the key read aloud that the party meant
  this package for this counterparty.
- **What the directory learns.** The slot name is the counterparty's id, so a reader of the
  directory can see that two parties hold a peering. The directory still computes nothing about
  who should peer with whom; the slot is the party's own statement, and the catalogue does not
  list slots. A party that does not want the relationship visible exchanges its package out of
  band or from its own host, as today.
- **How it arrives.** The directory gains `GET|PUT /v1/parties/{id}/packages/{counterparty}`
  beside the old route; `ops/federation/standup/directory.py` publishes and looks up per
  counterparty behind an opt-in switch, falling back to the pointer, and refreshes the single
  slot only when it is empty or already holds this same peering's package. Making the switch the
  default is a later, separate change.

Rejected: **one listener serving several partners through one package.** One offer valid for
many partners means one WireGuard key, one endpoint and one `proposed_link_subnet` shared
across peerings, and one /31 cannot serve several links; spec §7 takes the link subnet from
`parties[0]`'s proposal, so two partners' proposals could collide on our side depending only
on sort order. Fixing that changes agreement bytes and digests, needs a v2 specification and
new vectors, and a change in every partner's implementation.

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
  package verbs, as both called `goat federate` before. The script still orchestrates its own
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
