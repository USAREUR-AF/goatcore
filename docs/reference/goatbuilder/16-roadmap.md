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
| Federation in goatbuilder: one command, one decision | **designed**: [ADR 1225](../../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) (Proposed). Status: WIP — gate: the federation end-to-end test drives both sides with goatbuilder federate alone | `goatbuilder federate <partner>` is the whole procedure: one decision (partner and offered release sets), two human confirmations, everything else done, checked, recorded and resumable; `goat` keeps no federation code | [goatbuilder.md](../../design/goatbuilder.md) |
| Federation library crate | **built**, not yet on `main`; no behavior change | the package code moves from `goat-cli` into `crates/goat-federation`, used by both binaries | [federation-package-spec.md](../../design/federation-package-spec.md) |
| The six package verbs in goatbuilder | **designed** | `goatbuilder federate init\|lint\|compose\|diff\|verify\|apply` with today's flags and exit codes; `goat federate` runs the same library and names the goatbuilder command until a goatbuilder release reaches partner environments | [goatbuilder-conformance.md](../../design/goatbuilder-conformance.md) |
| Our package built on the core over SSH | **designed** | phase 3 runs from the operator's machine; no binary or signing key copied to the edge | [goatbuilder.md](../../design/goatbuilder.md) |
| Allocated per-peering values | **designed** | tunnel interface, listen ports, link subnet, edge listen port and metrics port allocated stably per peering and proposed as a reviewable change | [goatbuilder.md](../../design/goatbuilder.md) |
| Edge certificates installed by the roles | **designed** | the per-peering certificates issued and installed by the edge roles, not copied by hand | [goatbuilder.md](../../design/goatbuilder.md) |
| Federation preflight | **designed** | ports in use, the slot list against the registry, receive scopes against the edge's grants, certificate expiry and conflicts with existing peerings refused before any change | [goatbuilder.md](../../design/goatbuilder.md) |
| Acceptance as a contract | **designed** | checks generated from the agreement's flows; PASS, FAIL, BLOCKED, SKIP, NOT APPLICABLE; a required check cannot become PASS; bilateral evidence exchanged by file | [goatbuilder.md](../../design/goatbuilder.md) |
| Forwarder from the declaration | **designed** | the peering forwarder configured by the driver from the peering entry | [peering-forwarder.md](../../design/peering-forwarder.md) |
| Several peerings per party in the directory | **designed**: [ADR 1225](../../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) §6 | one package per counterparty, each in its own directory slot; the package format unchanged | [peering-directory-service.md](../../design/peering-directory-service.md) |
| Federation plan, state and resume | **designed** | a plan with hashes before any change, approval bound to it, a run journal, resume by inspecting actual state | [goatbuilder.md](../../design/goatbuilder.md) |
| Federation end-to-end through goatbuilder alone | **not built** | the federation end-to-end test drives both sides with `goatbuilder federate` and nothing else | [goatbuilder-conformance.md](../../design/goatbuilder-conformance.md) |
