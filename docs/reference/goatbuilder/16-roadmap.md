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
| Federation in goatbuilder: one command, one decision | **measured** (run up-4b1adc18cc3b-20261111T102642): [ADR 1225](../../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) (Proposed); modules in the table below. Measured: the federation end-to-end test drives both sides with goatbuilder federate alone (run up-4b1adc18cc3b-20261111T102642) | `goatbuilder federate <partner>` is the whole procedure: one decision (partner and offered release sets), two human confirmations, everything else done, checked, recorded and resumable; `goat` keeps no federation code | [goatbuilder.md](../../design/goatbuilder.md) §"Federation modules and their seams" |

## Federation in goatbuilder: modules

As of 2026-11-09. One row per module of [ADR 1225](../../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md), in merge order. A module is one pull request with one purpose and reverts alone. A stage graduates only when its proving scenarios pass in the federation test environment on its branch and nothing that passes on `main` regresses. Each switch of default is its own pull request. States: not started, in progress, built, tests pass, graduated. Nothing in this table is on `main`.

| Stage | Module | Branch | Stacked on | Proving scenarios | State | How to revert |
|---|---|---|---|---|---|---|
| 0 | Test environment baseline on main | `track/fed-e2e-env` | main | the baseline matrix | in progress | revert its branch; run.sh stays |
| — | Three-party harness and role fixes | `fix/federation-three-party` | main | `three-party-shared-edge` | in progress | revert alone |
| 1 | Acceptance cannot pass on skipped checks | `ws/w7-1-acceptance-skips` goatcore#274 | `integration/federation-turnkey` | `acceptance-strict-no-skip` + baseline | in progress | one revert; skips pass again unless FED_ACCEPTANCE_STRICT is set |
| 2 | Evidence schema and outcome words | `ws/w7-2-acceptance-evidence` | Stage 1 | baseline; schema tests | in progress | one revert; acceptance falls back to Stage 1 output |
| 2.x | Agreement-derived checks, bilateral exchange, cleanup, publishing `exchanging` | `ws/w7-federate-acceptance` (then smaller PRs) | Stage 2 | to be named | in progress | each one revert |
| 3 | Library crate (pure move) + CI coverage kept | `ws/w1-federation-crate` goatcore#242 | `integration/federation-turnkey` | baseline equal to main | tests pass (cargo test, clippy, parity, phase-numbering); reviewed; e2e matrix not yet reported | revert; later stages first |
| — | Decision record ADR 1225 and design docs | `ws/w0-adr-federation-in-goatbuilder` goatcore#245 | Stage 3 | doc checks | tests pass (doc and ADR gates); under review | one revert |
| 4 | Verbs into the library (pure move) | `ws/w2-verbs-move` | Stage 3 | baseline equal to main | built, tests pass | one revert |
| 5 | goatbuilder verbs beside goat's | `ws/w2b-goatbuilder-verbs` | Stage 4 | baseline; identical-output tests | in progress | one revert |
| 5.1 | Driver runs verbs in process (opt-in) | `ws/w2c-driver-in-process` | Stage 5 | `goatbuilder-only-both-sides` with opt-in | in progress | one revert |
| 5.2 | `goat federate` prints the notice | `ws/w2d-goat-federate-notice` | Stage 5 | baseline | in progress | one revert |
| 6.1 | Phase 8 resolves the CA key itself | to come | Stage 6 | to be named | not started | one revert; hand key file still wins |
| 6 | Planner: `goatbuilder federate plan` | `ws/w6a-federate-plan` | Stage 3 | to be named | in progress | one revert; preflight.py stays phase 1 |
| 7 | Edge role installs peering certificates (opt-in `goat_zenoh_edge_install_certs`) | `ws/w5-edge-certificates` (2 commits) goatcore#246 | Stage 3 | `edge-installs-certificates` (requested) + baseline with switch off | built; role test 18/18; under review | one revert; manual placement still works |
| 8 | Allocated peering values, proposed for review | `ws/w4-peering-allocation` | Stage 3 | second peering, three-party | in progress | one revert; hand-written files still work |
| 9 | Phase 3 measured over SSH | `ws/w3-measure-over-ssh` | Stage 5 | phase 3 from outside the edge | in progress | one revert; on-host init still works |
| 10 | Forwarder applied from the declaration | `ws/w8-peering-forwarder` | Stage 8 | second peering through the forwarder | in progress | one revert |
| 11 | Directory: one package per counterparty (opt-in `--per-counterparty`) | `ws/w9-directory-multi-peering` 7af5b8a goatcore#248 + saltlick `ws/w9-package-per-counterparty` 7cde26d saltlick#10 (saltlick merges first) | Stage 3 | two counterparties (requested) | built; unit tests pass; existing agreement digest unchanged (test); under review | one revert per repo; single package stays default |
| 12 | Deploy, approval bound to plan, journal, locks, resume, status | `ws/w10-federate-surface` (to be split) | Stages 5, 6 | to be named | in progress | each one revert |
| 13+ | Default switches, each alone | `ws/w2e-switch-driver-in-process`, `ws/w2f-switch-standup-to-goatbuilder`, others | their module | the module's scenarios | not started | one revert each |
| 14 | Teaching material | `ws/w12-federation-teaching` (goatcore, goatstuff, hello-galaxy-*, goatdocs) | after the switches | doc checks | in progress | one revert per repo |
| gate | Both sides driven by `goatbuilder federate` alone | `track/federate-in-goatbuilder` (integration) | everything | `goatbuilder-only-both-sides` | not started | — |
