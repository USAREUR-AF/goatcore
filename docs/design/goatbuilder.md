# goatbuilder — a config-driven build/release CLI that enforces the deploy contract

| Field | Value |
|---|---|
| Status | **Designed; every verb built** (2026-10-28): `crates/goatbuilder` — `validate`, `doctor`, `status`, `deploy`, `release`, `rollback`, `fastlane`, `pins`, `promote`, the output formats and exit codes; the schema frozen and gated in `ops/goatbuilder/`. `deploy`/`release`/`rollback`/`fastlane` match their shell drivers under parity gates in CI (`goatbuilder-pr.yml`), each gate a dry-run comparison. `pins` and `promote` built (2026-10-28) under parity gates that compare rendered bytes and printed lines, and `release`/`fastlane` make their edits through them; `standup`, `join-node`, `federate` built the same day over their phase drivers; the developer verbs `verify`, `diff` (one component or the whole environment), `history`, `plan`/`apply` and `adopt` built 2026-10-29, the first three measured read-only on zagros-dev; `survey` (intra- and inter-goatnet consistency, every goatnet read in parallel) built and measured across zagros-dev, zagros and zagros-prod 2026-10-29; `rollback` measured live on zagros-dev 2026-10-29 (herd v0.2.1 → v0.2.0 → v0.2.1); `update` (self-update from `get.<site>`, as `goat update`) built 2026-10-30; `completion <shell>` (+ a hidden `_complete` for live site/component/goatnet candidates), `docs --markdown` (the command reference, walked from the clap tree, gated against the committed copy) and the `goat`-parity globals `--color`/`--verbose`/`--timeout` built 2026-10-30; `tutor` designed. Built-and-run status per verb: [goatbuilder-conformance.md](goatbuilder-conformance.md) §"What is built" |
| Decision record | [ADR 1198](../adr/1198-goatbuilder-config-driven-deploy-release-cli.md) |
| Reference | [the goatbuilder reference](../reference/goatbuilder/README.md) — every verb as a task, the contract, secrets, transports, errors, testing; the source of the goatbuilder book |
| Distribution and hosting | [goatbuilder-distribution.md](goatbuilder-distribution.md) — one signed build, two homes (public releases; `get.<site>` inside each goatnet), and how the first copy escapes the catch-22 |
| Audience | goat engineers, and the external goat dev teams who will run their own repos and environments |

## What this is

`goatbuilder` is the operator command-line for **building and shipping a goat environment** — cutting a release, deploying a component to an environment, promoting it along the promotion path, reverting it, standing up a new environment. It is the build-side sibling of `goat`: `goat` operates a running fabric (who is on it, is data flowing, what may they do); `goatbuilder` produces and changes the thing `goat` then operates.

It rests on one model — **declare, converge, verify** — and one written contract that binds every verb to that model. Everything repo-specific (which components exist, which environments and machine classes, how each is converged, how a change is allowed to spread) is data in a config file; the model, the contract and its enforcement are the binary. The same released binary therefore drives this repository and any other goat team's repository, with none of ours compiled in.

Today the same behaviour exists spread across a dozen shell and Python drivers — `ops/scripts/rc.sh`, `ops/scripts/deploy-component.sh`, `ops/scripts/fastlane.sh`, `ops/release/graduate.py`, `ops/scripts/pr-land.sh`, `ops/scripts/ansible-preflight.sh`, `ops/pins/generate-pins.py`, `ops/scripts/rollback.sh`, `ops/scripts/cross-env-apply.sh`, `ops/scripts/goat-standup.sh` — each holding one piece of a contract nobody wrote down. `goatbuilder` writes it down, enforces it in one place, and replaces those drivers one verb at a time.

## Why one tool, and why separate from `goat`

An invariant that lives in no single component is not enforced. F-586 is the shape of that failure: `rc.sh` declared a release pin in one worktree and merged it, then deployed from a *different* checkout that need not carry it, while the staleness guard that would have caught the mismatch was switched off for feature-branch checkouts. Three scripts each owned a fragment of "a deploy applies the declared ref"; the defect was the gap between them. Every additional driver that shares implicit contracts widens that gap.

`goatbuilder` is a **separate binary from `goat`**, not a subcommand, for three reasons that do not change over time:

- **Different audience.** `goat` is for anyone on a goatnet; `goatbuilder` is for the people who release and operate environments. One binary for both blurs who a command is for and what blast radius it carries.
- **Different lifecycle.** `goat` tracks the fabric's runtime; `goatbuilder` tracks CI, image builds, declared state, the promotion path and git. Coupling their release cadences helps neither.
- **Different trust surface.** `goatbuilder` holds the authority to change every environment, and in the pull model it holds the signing seat. Keeping that out of the binary every participant runs is least-privilege by construction.

This is the seam `goat-cli` already draws between runtime surfaces and admin surfaces (goat-cli.md §"goat admin — operator-only by design"); `goatbuilder` moves the build/release half into its own tool rather than growing a second admin tree inside `goat`.

## The model: declare, converge, verify

Every change to an environment is three distinct acts, and `goatbuilder` keeps them distinct:

1. **Declare.** Record the desired state — *this component, for this class at this site in this environment, at this immutable ref* — in the **source of record**, which is git: the pin and intent files under `ops/pins/` and `ops/consistency/intent/`, authored and reviewed as a merge. Then **publish** that declaration in the form the environment's converger consumes. For a push converger the publication is the merged files themselves; for a pull converger it is a small **signed index** per environment — one exact version per class and a serial that only increases — pushed to the registry chain. Authoring and distribution are two different jobs: git is where humans decide, the publication is how the decision travels. Declaring changes nothing on any host.
2. **Converge.** Something makes the actual state match the declared state. That something is a **converger** — today an operator-driven push (Ansible from the operator's machine), in the pull model an on-host agent that, on its own jittered timer, pulls the index from the nearest registry that answers, verifies its signature, reads only its own class's version, pulls that release by digest, applies it to itself, health-checks, and promotes ([ADR 1109](../adr/1109-fleet-management-dimension-and-pull-convergence.md): convergence is pulled; the pull chain is lab-demonstrated in *GitOps Reconciliation at the Edge*, draft, the companion to the scale plan). The converger is the only act that touches a host.
3. **Verify.** Read back each machine's own record of what it is running — today the per-host deploy ledger ([ADR 1060](../adr/1060-universal-deploy-rollback-ledger.md)), in the pull model the machine's one-line status — and compare it to what was declared. Verification trusts the machine's record, never the converger's report of itself.

Desired state is declared and reconciled; imperative action against a host is for debugging and emergencies, and any lasting change is captured back into the declared state. That principle is why the tool's own job is only ever to **declare** and to **verify**: it records and publishes the desired state and confirms convergence, and it is never itself the thing that reaches a host.

### The converger interface

A converger is anything that makes actual match declared, and `goatbuilder` speaks to every converger through one narrow interface:

| Call | Meaning |
|---|---|
| `publish(env, site, class, component, ref)` | the declaration has been published in the form this converger consumes |
| `observe(env, site, class, component, deadline)` | report convergence per machine — *converged*, *pending*, or *unknown* — until the deadline |
| `read_record(machine, component)` | the machine's own record: digest, ordering, action, timestamp — or *unreadable* |

A push converger implements `publish` as a no-op beyond the merge, `observe` by running the play, and `read_record` from the ledger. A pull converger implements `publish` by signing and pushing the index, `observe` by watching each machine's status as the agents reconcile — with the deadline derived from the agents' poll interval and jitter — and `read_record` from the machine's status line. Neither implementation is visible to the verbs, which only ever declare and then verify against `read_record`. This is the property that makes the push→pull shift a configuration value rather than a change to the tool, and it is what a future converger — a different orchestrator, a different host platform — must satisfy to plug in.

The interface is deliberately asynchronous. In the pull model, **disconnection is a normal operating state**: an islanded environment keeps running what it last pulled and catches up on its next timer tick after a path home reappears. A verb therefore never blocks on "the whole fleet is done"; it reports the set that has converged, the set still pending, and the set it cannot read, and the operator decides what that means for the rollout.

## The deploy contract

Every declarative verb **guarantees** the following, and refuses or reconciles — with a named reason and the exact recovery command — rather than proceeding when a guarantee cannot be met. The clauses are ordered by the phase they bind. Each is testable against a single run or a single query, and each is one that a current script enforces in part or not at all; writing them down is what lets one code path enforce them uniformly.

**Declare**

| | Guarantee | Canon |
|---|---|---|
| **C1** | **Immutable, content-addressed ref.** A deploy applies exactly the ref the declaration names, and that ref is a digest — never a mutable tag, never a value inferred from the operator's checkout. | F-586 was a C1 violation; images are pinned by digest fleet-wide |
| **C2** | **Signed and ordered.** Every declaration is signed at the source and verified wherever it is consumed, so a copy from any registry is as good as any other — trust is in the content, not the path. Every declaration is ordered, and a consumer never accepts one older than the last it accepted: git ancestry orders the pins, a serial that only increases orders the index. A replayed old declaration is therefore harmless, and a revert is a *new* declaration at an old digest. No flag or option disables verification; a digest supplied by the operator is an additional check, never a substitute for the signature. | signed intents today (`intent-sign`); the signed index and single shared serial counter of the pull chain; bring-your-own-PKI signing under the organization's CA |
| **C3** | **Fresh source.** The declaration a verb reads is the source of record's *current* value. A stale local copy is fast-forwarded, or the verb refuses and prints the recovery command; it never acts on stale input silently. | `ansible-preflight.sh`'s guard, made unconditional rather than branch-dependent — **built** 2026-10-29 for real runs as `ops/scripts/fresh-source.sh`, one check both avenues run before the first host (GB-064 behind, GB-065 trunk unreachable unless `--offline`); [goatbuilder-conformance.md](goatbuilder-conformance.md) §"C3 on real hosts" |
| **C4** | **Attributed.** Every declaration names the actor who made it and is recorded where the next operator can read it. | the release ladder's `--ack`; the operator-action audit |

**Converge**

| | Guarantee | Canon |
|---|---|---|
| **C5** | **Idempotent and resumable.** Re-running the same verb against the same declaration makes no change and is safe to repeat. A run that stops part-way resumes from where it stopped. | `rc.sh --from <rung>` is the resume primitive; a re-run play reports `changed=0` |
| **C6** | **Least-privilege transport.** Each action opens the minimum credential and transport it needs, declared per environment, and never holds a blanket grant open across unrelated steps. In the pull model no seat reaches into a machine at all; the agent reaches out. | the per-goatnet door model ([ADR 1117](../adr/1117-per-env-doors-cross-env-management.md)) |

**Verify**

| | Guarantee | Canon |
|---|---|---|
| **C7** | **Verified-done, per machine; unknown is not done.** A machine is converged only when its own record names the declared digest *and* marks the run finished. An unreadable record is *unknown* and is never counted as converged. A machine that has not yet pulled is *pending* — a normal state in the pull model, reported as such, never silently folded into "done." | F-530: a digest match alone read three unfinished rows as converged; [ADR 1115](../adr/1115-goatnet-e2e-acceptance-suite.md) decision 3: UNKNOWN is not a pass |
| **C8** | **Inspectable.** Declared state, actual state and the drift between them are readable at any moment by read-only verbs that mutate nothing — in push mode and in pull mode alike. What an environment *should* run is one small signed file; what a machine *is* running is one record. | [ADR 1125](../adr/1125-fleet-reconciliation-join-and-graduation-sets.md): one row per declared fact |

**Cross-cutting**

| | Guarantee | Canon |
|---|---|---|
| **C9** | **Reversible.** Every declarative verb has a declared reverse, and the reverse is itself a conforming declaration: a rollback declares the prior digest as a new declaration and travels the same converge-and-verify path with the same guarantees. There is no privileged "undo" that bypasses the contract, and because every machine keeps its previous version one step away, a revert converges even on a disconnected environment. | [ADR 1060](../adr/1060-universal-deploy-rollback-ledger.md); `ops/scripts/rollback.sh`; rollout and rollback are the same act with different versions |
| **C10** | **Machine contract.** Every verb is non-interactive, returns a predictable typed exit code, and renders its result in a format selected by one flag — a table for a person, versioned JSON or YAML for a program, NDJSON for a stream — so automation never parses prose. Declarative (state-mutating) verbs are distinct from read-only verbs. | `goat-cli`'s structured `E_*` error UX and `--json` purity |

The contract is enforced once, in the binary, and **verified by a conformance suite** (below) — not by each operator's memory of which flag to pass or which directory to run from.

## Architecture

### One Rust binary, config-driven

A single statically-linked Rust binary with `clap` subcommands, on `goat-cli`'s build and distribution model (goat-cli.md §Architecture). It carries no repo-specific facts. Every error it prints is interpreted — what happened, why, what to do, where to read — through the catalog it shares with `goat` (`crates/goat-errors`, [error-interpretation.md](error-interpretation.md), ADR 1199); `goatbuilder explain` takes a code or any pasted text. Everything repo-specific is read from a config file at the repo root (`goatbuilder.yaml`), so the same released binary drives this repository and any other.

### The config model

The config describes a repository's **topology**, four levels deep: an **environment** is a goatnet; a **site** is a host site within it (ADR 1141's four-node baseline is one site) — the first site is the goatnet's founder, which originates the account, the CA and the declared ACL, and any other is a joiner that attaches to the founder's ([ADR 1084](../adr/1084-block-64r-mgmt-repset-and-two-node-federation.md), [goatnet-standup.md](../operations/goatnet-standup.md) §"Second host site"); a **class** is a machine class within a site; a **machine** is one host. The unit of release and of pull adoption is one class at one site — the paper's unit. The config also names the promotion path a change travels and any sovereign **peerings**, and `goatbuilder` supplies the model, the contract and the verbs over that topology. A sketch, not yet frozen; the names are illustrative, and a real repository names its own:

```yaml
schema: goatbuilder/v1
preset: full                          # or: lite — see "A goatlite profile"
repo:
  source: example-org/example-repo    # the release source of record
  image_namespace: ghcr.io/example-org
  signing: cosign                     # C2: signed at the source, under the org's CA
components:
  example-service:
    image_workflow: example-service-image.yml
    declared_in: ops/pins/fleet-pins.yaml   # C1/C3: where the declaration lives in git
    ledger: example-service                  # C7: today's per-machine record
environments:                         # an environment is a goatnet
  - name: dev
    converger: push-ansible           # the default every site and class inherits
    transport: direct                 # SSH over the mesh — the routine path; a door only when not meshed
    sites:                            # host sites: the first is the founder, any other a joiner
      - name: alpha
        role: founder
        classes: { core: {}, obs: {}, edge: {} }
      - name: bravo
        role: joiner
        joins: alpha
        classes: { core: {}, pod: { converger: pull-agent } }   # adoption is per class at a site
  - { name: preprod, converger: push-ansible, transport: direct }   # no sites block = one founder site
  - { name: prod,    converger: push-ansible, transport: ssm }      # its SSM door: used only when the operator is not on its mesh
peerings:                             # sovereign peering, the four identical steps of ADR 1157
  - { with: org-1, environment: dev, site: alpha, edge_class: edge,
      tunnel: wireguard, trust: spire-federation, ingress_prefix: from/org-1/ }
promotion:                            # the path a change travels; one gate mechanism between stages
  path:
    - { name: dev,     kind: rung, environment: dev }
    - { name: preprod, kind: rung, environment: preprod, gates: [ { id: validation-evidence } ] }
    - { name: prod,    kind: rung, environment: prod,    gates: [ { id: soak, min_days: 1 }, { id: operator-ack } ] }
  # the gate vocabulary is the release ladder's own (ops/release/ladder.yaml; ADR 1120)
  # a pull-converged environment may add stages of other kinds on the same path —
  # a ring (canary → stable, one shared counter) or a geographic tier (region, theater)
```

A team points `goatbuilder` at their own config — their components, their environments, their convergers — and gets the same contract-enforcing behaviour with nothing of ours baked in. The schema is versioned (`schema:`) and validated on every run; an unknown or malformed field fails closed, as the pins and intents already do. A **preset** supplies defaults that an explicit field overrides, which is the mechanism goatlite (below) uses. The converger is set per environment and overridable **per class**, because the pull chain's unit of release is one class in one environment, and adoption moves one class at a time. That override is a working seam, not a theoretical one: `octo-pod` (`octo-pod`) is an appliance class that already converges itself from a verified release — its own non-goal is being a fleet control plane — so `goatbuilder` declares and publishes for the `pod` class, the pod applies locally, and `read_record` is its `status --json` plus the version it records before every swap. The schema and the config file are frozen in `ops/goatbuilder/` and gated in CI ([goatbuilder-conformance.md](goatbuilder-conformance.md)).

**Promotion is one mechanism over typed stages.** A change travels an ordered path, and the tool enforces a gate between consecutive stages — a soak, a health signal, an operator ack. Today the stages are environments (the rungs of the release ladder, [ADR 1120](../adr/1120-release-graduation-ladder.md)). A pull-converged environment can add stages of two more kinds on the same path: a **ring** (a canary population takes a change first, and promoting copies the exact versions canary proved to stable, with one shared ordering so a machine switching rings never sees ordering run backwards) and a **tier** (the geographic seams — region, theater — along which a change is allowed to spread). Rung, ring and tier are kinds of stage, not three systems; the gate logic does not know which kind it is gating.

### Verb taxonomy — declarative and read-only (C10)

| Verb (illustrative) | Kind | Phase | Replaces |
|---|---|---|---|
| `goatbuilder release <component> --tag … --ack …` | declarative | declare + publish, then converge → verify per stage | `rc.sh`; in the pull model, the CI job that packs, signs, pushes and bumps the index |
| `goatbuilder deploy <component> --env <e>` | declarative | converge → verify of what is declared | `deploy-component.sh` |
| `goatbuilder promote <component> --to <stage>` | declarative | declare → converge → verify along the promotion path | `graduate.py`, `fastlane.sh` |
| `goatbuilder rollback <component> --env <e>` | declarative | declare the prior digest → converge → verify (C9) | `rollback.sh` |
| `goatbuilder pins {check,regenerate}` | declarative | declare | `generate-pins.py` |
| `goatbuilder standup <env> --site <s>` | declarative | all — declares the site's whole topology (founder or joiner), converges the doctrine's phases in order, verifies every record; the one-hour target | `goat-standup.sh` |
| `goatbuilder join-node <env> --site <s> --class <c>` | declarative | declares one more machine of a class at a live site → converge → verify | node-addition.md by hand |
| `goatbuilder federate <with>` | declarative | declares a sovereign peering → converges ADR 1157's four steps → verifies each | the peering runbook by hand |
| `goatbuilder federate init\|lint\|compose\|diff\|verify\|apply` | read-only (`init --out`, `compose --out` and `apply --confirm` write the files they name) | the six federation verbs, in process from the `goat-federation` library, with `goat federate <verb>`'s flags and exit codes | `goat federate <verb>`, which runs the same library and names this command on stderr |
| `goatbuilder status <component>` | read-only | verify: converged / pending / unknown per machine | `graduate.py status` |
| `goatbuilder verify <component> --env <e>` | read-only | verify | `deploy-component.sh --verify-only` |
| `goatbuilder diff <component> --env <e>` | read-only | verify: declared vs actual (C8) | — |
| `goatbuilder survey [--env <e>…\|--all]` | read-only | verify, across goatnets: declared vs ledger vs container per host, hosts that disagree, the goat CLI and the served payloads, and the ladder's grade for every difference between rungs, every goatnet read in parallel (C8) | `graduate.py status` plus `deploy-component.sh --verify-only` per component per goatnet |
| `goatbuilder plan …` | read-only | writes a saved plan of what a declarative verb would do | `--dry-run` across the above |
| `goatbuilder apply <plan>` | declarative | runs exactly the reviewed plan; refuses if the declaration moved | — |
| `goatbuilder history <component> --env <e>` | read-only | the records as a timeline (C8) | reading the ledger by hand |
| `goatbuilder update [--site <s>] [--check] [--force]` | the tool itself | replaces this binary with the release `get.<site>.<zone>/goatbuilder/` serves: the `.sha256`, `SHA256SUMS` and (with cosign on PATH) its signature checked before anything is touched, the swap atomic; `--check` reads only. A behind-version line on a terminal at most once a day; `doctor` grades the same comparison ([goatbuilder-distribution.md](goatbuilder-distribution.md)) | the landing page's install sequence by hand |
| `goatbuilder doctor` | read-only | preflight: deps, checkout freshness, transports, registries | `rc.sh`'s and `ansible-preflight.sh`'s preflights |
| `goatbuilder validate` | read-only | the config against the versioned schema, offline | — |
| `goatbuilder adopt --env <e>` | declarative | declares what an existing environment's records say it runs | — |
| `goatbuilder tutor` | read-only, on `--mock` copies | **built** (2026-10-28): eleven lessons, each a short text and the exact commands to type against a throwaway fixture copy (or, read-only, this checkout); the tutor checks each run's exit and rows before the next unlocks; `--list`, `tutor <n>`, `--reset`; lessons needing a verb this build lacks are listed and skipped | — |

Read-only verbs never mutate and are safe to run anywhere, including against a pull-converged environment the operator does not drive. Declarative verbs enforce C1–C10 before acting; `plan` shows exactly what one would do without doing it.

### Enforcement in one place

The contract lives in one module every declarative verb calls, not in each verb. C1 resolves the ref from the declaration in git, never from the working tree. C2 signs on publish and verifies on read, and refuses a declaration older than the last accepted. C3 reconciles or refuses the input against the source of record. C7 reads each machine's record through the converger interface and gates completion on it, reporting *unknown* and *pending* honestly. C9 means `rollback` is the same module with the prior digest as input. This is the structural fix for the F-586 class: one place knows "a deploy applies the declared ref," and it cannot be bypassed by running from a different directory or on a different branch.

### Convergers: push today, pull as the direction of travel

**Built 2026-10-28 — the push converger.** `goatbuilder deploy --env <goatnet> --component <c>` (`crates/goatbuilder/src/push.rs`) makes every decision `ops/scripts/deploy-component.sh` makes, in the same order from the same files — the pins, the play through its alias and its target mode, the secrets the play templates, the transport, the per-group run through `ansible-preflight.sh`, the ledger read-back requiring the digest and `action: deploy` — and the parity gate `ops/goatbuilder/parity-deploy.sh` (CI) runs both with `--dry-run` for every component on every live goatnet and refuses any difference. Its first full run found two silent-exit defects in the script, which were fixed; the script stays the deploy that `rc.sh` calls until `release` lands in this binary, and is deleted then.

The converger is a configuration value because the contract never mentions *how* a ref reaches a host. `release` and `promote` declare and `publish`; `deploy` takes the current declaration and sees it through. Today `deploy` calls `observe` on the push converger (which runs the Ansible play and reads the ledgers) and verifies. Under `converger: pull-agent`, `publish` has signed and pushed the index, and the same `deploy` calls `observe` on the pull converger (which watches each agent's status as it reconciles on its own timer) and verifies the same way. The middle calls are dispatched to a different implementation; the verbs, the contract and the verification are untouched. One class in one environment can move to pull while everything else stays on push, and a converger for a platform that does not yet exist plugs in by implementing the three calls.

The pull shape this is designed to receive — taken as the likely direction, not as fixed — is the one lab-demonstrated in *GitOps Reconciliation at the Edge*: desired state signed once and published as ordinary registry artifacts; a chain of registries, one per storage tier, that pull from an ordered list of upstreams so a change crosses each network link exactly once and an islanded store keeps serving; an agent on every machine that pulls from the nearest store that answers, verifies the content rather than trusting the registry, applies its own class's release locally, health-checks, and keeps the previous version one step away. Two of its properties shape the contract directly: trust is in the signed content (C2), and failure produces quiet rather than a storm — a machine that cannot reach a store keeps what it has and retries next tick (C7's *pending*). What that work has not yet built — two-organization federation, the CI job, transport hardening — are the parts of the road not yet driven, and nothing in `goatbuilder` depends on them landing in a particular order.

**Transports: the mesh first, a door only when not meshed, and never one person's network.** A push converger reaches hosts over the mesh (`direct`: SSH with an inventory, minutes) whenever the operator's machine is on the environment's profile; the environment's `transport` names its **door** for when it is not — an SSM port-forward through the environment's cloud mesh peer (`ssm`), open to anyone holding the provider credentials. The door is bootstrap and break-glass grade, not a routine path: SSM sessions idle out at about 20 minutes, its agent and file copy have failed in four recorded ways, and a long play over a fragile path dies mid-run — the hours-then-retry that the golden-path buildout paid for. So `doctor` reports which transport a run will use and refuses to start a long play over a door when the mesh is available, and `plan` names the transport in its output. The pull converger removes the question: the agent reaches out from the host, so no laptop, door or tunnel is in the routine path at all ([env-doors.md](env-doors.md)).

An address on one operator's own network is not a transport. Tailscale is the bad-day path onto a break-glass node or management host when the mesh is down (ADR 1114 §4, interim), and an environment whose only wired address is there is deployable from one machine. `doctor` reads the two files that decide who can deploy — the environment's row in the mesh inventory (`ops/ansible/inventory/mgmt-hosts-mesh.ini`) and its entry in the door registry (`ops/ansible/inventory/doors.yaml`) — and reports `env:<name>:reach` as deployable over the mesh, through the door, or both; it refuses an environment with neither, or whose only door is not `ssm`, naming the row and the door to declare.

### Founders, joiners, host-site nodes and sovereign peering

The golden paths landed in September 2026 — the founder-and-joiner standup walkthrough (#1362), the declared ACL applied at standup (ADR 1185, #1326), the rescue node built by the standup path (#1233), and the service-identity registry gated at standup, deploy and CI (#1166) — are not a separate concern from this tool; they are what `standup`, `join-node` and `federate` declare, converge and verify. Each is already a declaration in the repository, and the config above gives each its home.

| Golden path | Declared where today | In `goatbuilder` |
|---|---|---|
| **Founder standup** — a site that originates the goatnet: the NetBird account baseline, the CA, the SPIRE server, the declared ACL enforced (phase 12, founder only) | `group_vars/<site>_hosts/` with `goatnet_founder: true`; `goat-standup.sh <site>` | `standup <env> --site <founder>`: the site's `role: founder`; the founder-only phases run because the config says so, not because an operator remembers which ones |
| **Joiner standup** — a second host site attaching to the founder's account and ACL, in three passes that cannot be reordered | `goatnet_founder: false` + `join_founder_site`; node-addition.md §3–4 | `standup <env> --site <joiner>`: `role: joiner`, `joins: <founder>`; the validator refuses a joiner with no founder to join, before any pass runs |
| **Host-site node classes** — core pair, obs pair, break-glass and network-jump, converged when the site declares them (ADR 1141, ADR 1183) | `has_breakglass_node`, `has_network_jump_node` and the class plays | the site's `classes`; a class the site does not declare is not converged, and `doctor` says so |
| **Sovereign peering** — the identical four steps on each side: a dedicated tunnel, SPIRE-federated trust, an agreed identity, one dedicated edge router per partner; recognition, never authorization ([ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md), cross-sovereign-federation.md, [federated-trust-model.md](federated-trust-model.md)) | `spire_federates_with`, the partner's router profile, the `from/<sov>/` ingress prefix | `peerings:` — one entry per partner naming our environment, site and edge class; `federate <with>` declares it, converges the four steps and verifies each (tunnel up, bundle synced, SVID CN as agreed, the router profile trusting only that bundle). Cutting a peering is the reverse declaration (C9) |
| **Registry-driven router configuration** — router configs generated from one registry, deployed by automation, proven to peer across routers on a laptop in one command (`hello-galaxy-zagros-pod`) | the registry and its renderer | the same shape as every declaration here: the registry is the source of record, the rendered config is its publication, and the smoke test is a conformance fixture |

Status of those paths, in the contract's words: **built, and worked once** — by the end of the September buildout each path had run, fixed in flight as it went; **a fresh, unattended full run has not been proven.** That proof is what `standup --mock` rehearses on the conformance fixture and what the first timed real run measures; until then the one-hour target is a target, not a claim.

What this adds to the model is only the site level and the peering object; the verbs, the contract and the converger interface are unchanged. `federate` is a declarative verb like any other — which is the point: a sovereign peering becomes something that is declared, reviewed, converged and verified rather than a procedure carried in a runbook.

### A goatlite profile (future)

> Designed 2026-10-31: [goatlite.md](goatlite.md) and [ADR 1213](../adr/1213-goatlite-preset-one-box-one-file.md). The paragraph below is the reservation this section made; the design doc is canonical.

A future **goatlite** — a much easier-to-deploy, easier-to-manage, easier-to-use variant of goat — is a separate effort, prepared for here so the CLI never needs re-architecting for it. goatlite is a **preset over the same contract and the same binary, not a fork**: `preset: lite` supplies opinionated, batteries-included defaults — a single environment, one converger, the minimum required fields — so a lite deployment is a handful of config lines, while C1–C10 hold unchanged. The preset layer exists in the schema from the first version; goatlite adds a preset and a smaller default verb set, not a second tool with its own contract.

### Standing up a goatnet in an hour

A fresh goatnet, from nothing to verified, within one hour is a stated target of this tool — the operator's "phoenix engineering ability to rapidly stand up a fresh goat net within a single hour," the routine teardown-and-rebuild that [phoenix-architecture.md](phoenix-architecture.md) frames as an engineering practice rather than a crisis. `standup` is where `goatbuilder` has to collapse the most complexity, so it is designed as a first-class verb even though it is the last to be ported.

What exists today is the doctrine, not the hour. [goatnet-standup.md](../operations/goatnet-standup.md) drives a site through sixteen numbered phases (0–15) with `goat-standup.sh <site>` — one driver, idempotent phases, a fail-closed gate at each end, `--from <phase>` to resume — under [goatnet-deployment-doctrine.md](goatnet-deployment-doctrine.md) and the [staged-delivery pattern](https://github.com/usareur-af/goatdocs/blob/main/docs/design/staged-delivery-pattern.md), on the four-node host-site baseline (ADR 1141) with the rescue nodes converged in the last phase ([ADR 1183](../adr/1183-rescue-nodes-are-part-of-the-site-baseline.md)). `goatbuilder standup` subsumes that driver behind the contract, and the hour comes from four things the contract already gives:

- **One declaration for the whole site.** The config declares every component for every class; `standup` declares them all at once (C1), so the site's shape is reviewed in one file rather than discovered phase by phase.
- **Every precondition up front.** `doctor` checks credentials, DNS, registry reach, transports and the checkout before phase 1, so the hour is not spent finding what phase 7 needed.
- **Resumable by phase, observed per machine.** A phase that fails re-runs from where it stopped (C5), and progress is a per-machine stream (C7, `-o ndjson`) rather than a log read afterwards.
- **Rehearsed before hardware.** The same config runs under `--mock` in minutes against the conformance fixture ([goatbuilder-conformance.md](goatbuilder-conformance.md)), so the first real run is in effect the second run.

Target: declared → verified in 60 minutes or less on the four-node baseline; where the time goes, the per-phase budget and the ranked levers are in [goatbuilder-speed.md](goatbuilder-speed.md). Status: **built; rehearsed on the fixture; unmeasured on hardware** — `standup --mock` converges the three-class fixture site in milliseconds and `standup <env> --site <s>` plans and runs `goat-standup.sh`'s phases under the parity gate (2026-10-28); the first `standup` run on hardware records its time, and that number, not this sentence, is the claim.

### Conformance suite

The suite is specified clause by clause, with its fixture and parity gate, in [goatbuilder-conformance.md](goatbuilder-conformance.md); the config-schema gate and every clause C1–C10 are built as tests on the fixture (2026-10-29). The contract is testable, and the tests are the specification. A conformance suite exercises each clause against a fixture topology with a fixture converger: C1 by pointing a deploy at a stale tree and asserting it still applies the declared digest; C2 by presenting an older declaration and asserting refusal, and a tampered artifact and asserting refusal; C3 by making the local copy stale and asserting a fast-forward or a refusal, never a silent act; C4 by asserting the declaration and the record carry the actor, and that a declaration without one is refused; C5 by running the same verb twice and asserting the second is a no-op; C6 by asserting a deploy opens only its own environment's transport, and none under a pull agent; C7 by withholding a machine's record and asserting *unknown*, and by holding a machine offline and asserting *pending*, never done; C8 by asserting `diff` reads declared-versus-actual without writing; C9 by rolling back and asserting the record names the prior digest under a newer declaration. A converger, a transport or an environment kind is admitted by passing the suite, which is how generalization stays honest: a config the suite passes behaves to contract on any repository.

### Operator experience — borrowed, each for a reason

`goat-cli` already solved the operator-experience problems that matter here, and the best infrastructure CLIs solved the rest. `goatbuilder` takes each of the following because it serves a contract clause or a named need — and takes nothing that would add a second state store, a plugin system or a formatter.

| Borrowed | From | Why it is here |
|---|---|---|
| `doctor` — probe the deps, the checkout's freshness, and each environment's transport and registry before anything is cut | `goat doctor` | the preflight the first microscope release lacked: C2, C3 and C6 checked up front, each failure printed with its recovery command |
| `validate` — check the config against the versioned schema, offline | terraform `validate` | fail-closed config; the first conformance test every repository runs, with no environment needed |
| validation in two places on purpose — a fast, friendly error at the command line, and the gate in the enforcement module that cannot be bypassed | `octo-pod` (its declare tool validates before writing; the playbook validates again as the gate) | the fast error serves the operator, the second gate serves the contract; neither can be reached around by running the other |
| every command the tool runs is echoed, secrets redacted | `octo-pod`'s `run()` | the operator sees exactly what ran while the migration still shells out to `pr-land.sh` and `ansible-playbook`; nothing is invisible and nothing leaks |
| `plan` writes a saved plan; `apply <plan>` applies exactly what was reviewed, and refuses if the declaration has moved since | terraform `plan` / `apply` | the push-world form of "git is where humans decide": what the reviewer saw is what runs (C1, C3) |
| `--frozen` — refuse any run that would need to fast-forward | cargo `--locked` | strict C3 for CI and unattended runs, where a silent fast-forward is itself a surprise |
| `diff`, `history` — declared versus actual; the records as a browsable timeline | kubectl `diff` / `rollout history`, helm `history` | C8 over records that already exist (ADR 1060); `rollback` reads the same history |
| `survey` — one read of every goatnet, in parallel, that says what differs inside each and between them | Argo CD's application list (sync and health per app per cluster), `kubectl get` across contexts | C8 for the whole ladder at once: a goatnet trailing the one below it shows as a graded difference, not a surprise found one component at a time |
| `--narrate` — show the cascade a declarative verb will cause: the git change, the publication, the machines to be observed, the records expected | `goat --narrate` | makes `plan` and `apply` legible without reading the implementation; the pull model's "audit is a file read," in advance |
| `-o table\|json\|yaml\|ndjson` and `--fields` — one flag selects the rendering; JSON carries its own output-schema version | kubectl `-o`, gh `--json`, terraform `-json`, aws `--output` | C10 made usable: a person reads a table, a pipeline reads versioned JSON without scraping, and `observe`'s per-machine convergence is an NDJSON stream — one line per machine as it converges, pends or reads unknown |
| confirm by default; `--yes` for automation | terraform `-auto-approve` | a human confirms a mutation, a machine passes `--yes` (C4, C10) |
| `--mock <fixture>` — run any verb against a fixture topology and a fixture converger: no hosts, no registry, deterministic | `goat-cli`'s in-process sandbox simulator | one artifact serves three needs — the conformance suite, golden tests in CI, and training — the same keystone that let `goat-cli` test its whole onboarding flow with zero infrastructure |
| `tutor` — vimtutor-style lessons completed by running the real verbs on `--mock` | `goat tutor` | the onboarding gap the first external deployers hit; a release is learned end to end without touching an environment |
| `?` / `compose` — help and command construction as one navigable surface | `goat compose` | long verbs with many flags (`release`, `promote`) are built rather than memorized |
| `completion <shell>` + live candidates through a hidden `_complete` hook | `goat completion` | shells complete the verbs statically and the sites, components and goatnets a checkout declares dynamically — the same two-layer pattern `goat` uses |
| `docs --markdown` — the whole command reference, walked from the live clap tree, gated in CI against the committed copy | `goat docs --markdown` | the reference cannot drift from `--help`; the hosted book renders the same bytes |
| `--color auto\|always\|never`, `--verbose`/`-v`, `--timeout` (`GOATBUILDER_TIMEOUT`) | `goat`'s globals | color honours `NO_COLOR`/`CLICOLOR_FORCE` and touches only the glyph and verdict; `-v` logs the external commands and `update`'s fetch to stderr so `--json` stays clean; `--timeout` fails fast on an unreachable surface |
| `adopt` — bring an existing environment under declared state by reading its records, deploying nothing | terraform `import` | the dual-run migration and goatlite onboarding both begin from environments that already exist |

Not borrowed, deliberately: a state backend or lock service (git and the machines' records are the state; a plan file is the only artifact), workspaces (environments are config), modules or a provider plugin system (the converger interface is the one seam), and a formatter.

## Conventions adopted from current practice

The contract converges with the current state of deployment tooling, which is evidence it is the right shape rather than a local invention:

- **Idempotent re-deploy makes no change**, and imperative commands are reserved for debugging with lasting changes captured back to the declared repo ([University of Cambridge deployment standard](https://techdesign.uis.cam.ac.uk/en/latest/standards/deployment/); [declarative vs imperative resource management](https://notes.kodekloud.com/docs/Prep-Course-Certified-Cloud-Native-Platform-Engineering-Associate-CNPA/Domain-1-Platform-Engineering-Core-Fundamentals/Declarative-vs-Imperative-Resource-Management-Concepts/page)).
- **A CLI for humans and automation is non-interactive, emits JSON, is idempotent, uses predictable exit codes, and separates declarative verbs from read-only ones** ([Harmony application-lifecycle-CLI ADR, draft](https://git.nationtech.io/NationTech/harmony/src/branch/master/docs/adr/026-application-lifecycle-cli.md)).
- **An `apply` runs a plan of upsert operations that reconciles in place**, callable from other automation ([Stalwart declarative deployments](https://stalw.art/docs/configuration/declarative-deployments/)).
- **Config separated from source, environments as directories not branches, images pinned by digest not `latest`, drift detected continuously** ([ArgoCD multi-environment repo structure](https://oneuptime.com/blog/post/2026-02-26-argocd-git-repo-structure-multi-environment/view); [GitOps best practices](https://www.pulumi.com/blog/gitops-best-practices-i-wish-i-had-known-before/)).
- **Pull-based convergence so no central seat holds environment credentials**, signing in exactly one place and verification everywhere content is consumed, a monotonic serial as the replay guard, canary rings with one shared counter ([ADR 1109](../adr/1109-fleet-management-dimension-and-pull-convergence.md); *GitOps Reconciliation at the Edge*, draft; [GitOps best practices for Rancher](https://oneuptime.com/blog/post/2026-03-20-gitops-best-practices-rancher/view)).

These are practitioner and standards sources, not a single authority; the contract above is where they agree with goat's own ADRs (1060, 1109, 1115, 1117, 1120, 1125, 1173).

## Migration — two avenues, both live; the CLI supported, the scripts documented

Both avenues stay functional for the three golden paths — founder standup, federating node, deployments across environments — and neither is deleted ([ADR 1201](../adr/1201-two-avenues-scripts-documented-cli-supported.md), amending ADR 1198's "subsume and delete"). The operator's rule, verbatim: "keep both avenues functional … both the scripts method and the cli method and eventually we will document how scripts can work, but support only the cli."

1. **Dual-run.** `goatbuilder <verb>` is built to pass the conformance suite and to produce the same result as the script beside it. The script never depends on the binary.
2. **Parity gate.** A CI check runs the binary and the script against the same input and asserts identical declaration and record outcomes, the way `registry-parity` already gates image refs. A failure on either side fails the check; the fix lands on both sides in the same change.
3. **Support moves, surface stays.** The runbooks, one-pagers, hosted book and error catalog teach the binary. Each script's page says what it does and that it is unsupported. The CLAUDE.md pointer names the binary as the driver and the script as the documented alternative.

The order followed blast radius and churn: `release` and `deploy` first (the F-586 surface, highest churn), then `rollback`, `fastlane`, `promote`, `pins`, and last `standup`, `join-node`, `federate` — last because they are the largest surface, first-class because the one-hour goatnet is their target. The push→pull adoption follows the same discipline at a finer grain: one machine class moves onto the pull converger and the push path is retired for that class in the same change, then the next — the push path is a converger, not an avenue, and that retirement stands.

## What this is NOT

- **Not a second `goat`.** It does not operate the running fabric. `goat doctor`, `goat who`, `goat flow`, `goat admin` stay in `goat`.
- **Not a converger.** It declares and verifies; it never reaches a host itself. Ansible today and the pull agent tomorrow are convergers it drives through the interface above.
- **Not the registry chain or the agent.** The pull chain's registries and the on-machine agent are the pull converger's own parts; `goatbuilder` publishes signed declarations into that chain and reads records back from it.
- **Not a new CI system.** It runs inside the existing GitHub Actions, Ansible and declared-state machinery as the single operator-facing entry point over them; in the pull model its `release` verb *is* the CI release job.
- **Not a wrapper.** A verb that cannot replace and delete its script is not shipped. Wrapping would leave the contract split across the scripts, which is the defect.
- **Not repo-bound, and not program-bound.** No `USAREUR-AF/goatcore` path, no goatnet or environment name, no program or goatnet-family name (`zagros`, `zagros_dev`, `zagros_prod`), and no transport is compiled in. The binary knows only the abstractions — components, environments, classes, convergers, stages — as the config names them; `zagros` is one deployment's config value, not a concept the tool carries. A team with no connection to ZAGROS points it at their own config and nothing in the binary refers to ours.

## Phasing

This document fixes the model (declare, converge, verify), the converger interface, the contract (C1–C10), the separation from `goat`, the config-driven generalization with its preset layer and per-class convergers, promotion as one mechanism over typed stages, and the two-avenue migration (ADR 1201). The config schema is frozen and gated (`ops/goatbuilder/`), and the binary's first increment is built: `crates/goatbuilder` with `validate`, `doctor`, `-o table|json|yaml` and the typed exit codes, on goat-cli's build and CI model (`goatbuilder-pr.yml`). `doctor` already enforces two of the contract's lessons in code — the checkout rule and the playbook-alias target mode. Built since, each behind its parity gate and recorded verb by verb in [goatbuilder-conformance.md](goatbuilder-conformance.md) §"What is built": the mock fixture and scripted converger, `deploy`, `release`, `rollback`, `fastlane`, `pins`, `promote` (the drivers' table and record edits typed; a re-run whose pin is on main lands nothing), and (2026-10-28) `standup` — the rehearsal on the fixture and the real path over `goat-standup.sh`'s phases — `join-node` from the runbook, and `federate` over `peering-standup.sh`'s phases. Every verb the design names is built. The scripts stay, documented and working beside their verbs, each held to its verb by a parity gate ([ADR 1201](../adr/1201-two-avenues-scripts-documented-cli-supported.md)).

## Open questions

- Does `goatbuilder` embed the git and Ansible steps directly, or shell out to `pr-land.sh` / `ansible-playbook` during the migration and absorb them later? (Leaning: shell out first behind the converger interface, absorb per verb, so migration is incremental.)
- Where does the config schema's source of truth live for an external team — in their repo, or a published JSON Schema the binary ships and validates against? (Leaning: shipped with the binary and versioned by `schema:`, so the contract travels with the tool.)
- How does `goatbuilder` represent a second organization's signing root for federated pull — a trust list in config, or the federated-peering model's own anchor? (Leaning: the peering model's anchor, so "does this artifact chain to my root" is the same question as "does this peer"; that half of the pull work is not yet built, and this stays open until it is.)
