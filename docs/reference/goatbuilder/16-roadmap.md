# 16. Roadmap

Everything in chapters 1 to 15 is on `main`. This chapter lists the items
around them and the status of each, **built**, **designed** or **not built**,
so a reader can tell them apart.

| Item | Status | What it adds | Design |
|---|---|---|---|
| goatlite | **built**: `preset: lite`, run once on hardware; never for production | a `lite` preset and a smaller verb set over the same binary and contract | [goatlite.md](../../design/goatlite.md) |
| Zero-SaaS recipes | **built**: the store-generic new-site secret generator, the local registry, the local mail sink, alert channels off when empty | a goatnet with no hosted registry, mail or alerting account | [pluggable-dependencies.md](../../design/pluggable-dependencies.md) |
| Tutorials 2 and 3 on hardware | **built** and rehearsed on the fixture; **not yet run** on real machines | the founder-and-joiner pair, and a boundary node granting one topic to one partner, run on real machines | [goatnet-tutorials.md](../../design/goatnet-tutorials.md) |
| A `boundary` machine class | **not built**: tutorial 3 needs none, because the boundary node is the router `goatbuilder federate` deploys | `join-node --class boundary` installing a partner egress node | [goatnet-tutorials.md](../../design/goatnet-tutorials.md) |
| Zero-SaaS standup, measured | **not built** | a goatnet stood up end to end on the `env-file` posture with no hosted account | [pluggable-dependencies.md](../../design/pluggable-dependencies.md) |
| Forge adapter | **designed** | the release path's pull requests, CI waits and admin nomination on a forge other than GitHub | [pluggable-dependencies.md](../../design/pluggable-dependencies.md) |
| OpenBao injector | **designed**; not in this repository | `ops/secrets/openbao/run`, so the `openbao` store resolves `bao://` references | [pluggable-dependencies.md](../../design/pluggable-dependencies.md) |
| `status` on real hosts | **designed** | `status` reads real hosts through the push converger, as `verify` and `diff` do today | [goatbuilder-conformance.md](../../design/goatbuilder-conformance.md) |
| A pull converger | **designed** | `converger: pull-agent`: an agent on each machine applies its class's signed release and `deploy` observes it; push and pull mix per class | [goatbuilder.md](../../design/goatbuilder.md) |
| A semantic error matcher | **designed** | a fourth matcher after the lexical one | [error-interpretation.md](../../design/error-interpretation.md) |
| Federation in goatbuilder: one command, one decision | **designed**: [ADR 1225](../../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) (Proposed); modules in the table below. Status: WIP — gate: the federation end-to-end test drives both sides with goatbuilder federate alone | `goatbuilder federate <partner>` is the whole procedure: one decision (partner and offered release sets), two human confirmations, everything else done, checked, recorded and resumable; `goat` keeps no federation code | [goatbuilder.md](../../design/goatbuilder.md) §"Federation modules and their seams" |

## Federation in goatbuilder: modules

One row per module of [ADR 1225](../../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md), in merge order. A module is one pull request with one purpose and reverts alone. A stage graduates only when its proving scenarios pass in the federation test environment on its branch and nothing that passes on `main` regresses. Each switch of default is its own pull request. States: not started, in progress, built, tests pass, graduated. Nothing in this table is on `main`.

| Stage | Module | Branch | Stacked on | Proving scenarios | State | How to revert |
|---|---|---|---|---|---|---|
| 0 | Test-environment baseline | `track/fed-e2e-env` | main | the baseline matrix | in progress | revert its branch; `run.sh` stays as today |
| 1 | Acceptance cannot pass on skipped checks | `ws/w7-1-acceptance-skips` | main | `acceptance-strict-no-skip`, baseline | in progress | one revert; skips pass again unless `FED_ACCEPTANCE_STRICT` is set |
| 2 | Evidence schema and outcome words | `ws/w7-2-acceptance-evidence` | stage 1 | baseline; schema tests | in progress | one revert; acceptance prints stage 1 output |
| 2.x | Checks from the agreement, bilateral exchange, cleanup, publishing `exchanging` | `ws/w7-federate-acceptance`, then smaller branches | stage 2 | to be named | in progress | each part one revert |
| 3 | Library crate (pure move) | `ws/w1-federation-crate` | main | baseline equal to main | tests pass | one revert, after later stages |
| — | Decision record (ADR 1225) and design documents | `ws/w0-adr-federation-in-goatbuilder` | stage 3 | doc checks | tests pass | one revert |
| 4 | Verbs into the library (pure move) | `ws/w2a-verbs-into-library` | stage 3 | baseline equal to main | in progress | one revert |
| 5 | goatbuilder verbs beside `goat`'s | `ws/w2b-goatbuilder-verbs` | stage 4 | baseline; identical-output tests | in progress | one revert |
| 5.1 | Driver runs the verbs in process (opt-in) | `ws/w2c-driver-in-process` | stage 5 | `goatbuilder-only-both-sides` with the opt-in | in progress | one revert |
| 5.2 | `goat federate` prints the notice | `ws/w2d-goat-federate-notice` | stage 5 | baseline | in progress | one revert |
| 6 | Planner: `goatbuilder federate plan` | `ws/w6a-federate-plan` | stage 3 | to be named | in progress | one revert; `preflight.py` stays phase 1 |
| 7 | Edge roles install peering certificates | `ws/w5-edge-certificates` | stage 3 | to be named | in progress | one revert; manual placement still works |
| 8 | Allocated peering values, proposed for review | `ws/w4-peering-allocation` | stage 3 | second peering, three-party | in progress | one revert; hand-written files still work |
| 9 | Phase 3 over SSH | `ws/w3-measure-over-ssh` | stage 5 | phase 3 from outside the edge | in progress | one revert; on-host `init` still works |
| 10 | Forwarder from the declaration | `ws/w8-peering-forwarder` | stage 8 | second peering through the forwarder | in progress | one revert |
| 11 | Directory: several packages per party | `ws/w9-directory-multi-peering` (and the directory service) | stage 3 | two counterparties | in progress | one revert per repository; the single package stays default |
| 12 | Deploy, approval bound to the plan, journal, locks, resume, status | `ws/w10-federate-surface` (to be split) | stages 5 and 6 | to be named | in progress | each part one revert |
| 13+ | Switches of default, each alone | `ws/w2e-switch-driver-in-process`, `ws/w2f-switch-standup-to-goatbuilder`, others | their module | the module's scenarios | not started | one revert each |
| 14 | Teaching material | `ws/w12-federation-teaching` | the switches | doc checks | in progress | one revert per repository |
| gate | Both sides driven by `goatbuilder federate` alone | `track/federate-in-goatbuilder` (integration) | every stage | `goatbuilder-only-both-sides` | not started | — |
