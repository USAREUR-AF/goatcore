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
| Federation in goatbuilder: one command, one decision | **built**, in goatbuilder 0.5.1; **measured**: the federation end-to-end test drives both sides with goatbuilder federate alone (run up-4b1adc18cc3b-20261111T102642). [ADR 1225](../../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) (Proposed). What 0.5.1 carries and what remains: the section below | `goatbuilder federate <partner>` is the whole procedure: one decision (partner and offered release sets), the run's prompts at phases 5, 6 and 10, everything else done, checked, recorded and resumable; `goat federate` runs the same library and names the goatbuilder command | [goatbuilder.md](../../design/goatbuilder.md) §"Federation modules and their seams" |

## Federation in goatbuilder: what 0.5.1 carries

goatbuilder 0.5.1 is the first published release with federation in it (0.5.0 was tagged and not
published). Everything in the first list is **built** and on `main`; the procedure is
[sovereign-federation-runbook.md](../../operations/sovereign-federation-runbook.md), step 1.

- `goatbuilder federate <partner>`: phases 1 to 14 in one run at a terminal, prompting at phases
  5, 6 and 10, resuming where it stopped. Without a terminal, the same run stops at 10 and
  `plan`, `deploy` and `test` carry it on.
- `goatbuilder federate plan`, `deploy`, `test`, `status` and `backout`, each also run alone.
- `goatbuilder federate allocate [<peering>]`: per-peering values that never collide or move,
  and the files a person used to write (the peering's inventory, both deployment halves and its
  `federation.toml`), beside peerings that still run from hand-written files.
- The six package verbs, `init`, `lint`, `compose`, `diff`, `verify` and `apply`, in goatbuilder.
- Values from the declarations: `secrets.federation`, `environments[].peering_directory`, the
  site inventory, the core and edge declarations; the `FED_*` variables stay overrides.
- Admission on a core deployed by the goat-zenoh role, from the role's own declarations.
- Acceptance that cannot pass on a check that did not run, with a versioned evidence bundle, and
  a bilateral session exchanged by file (`ops/federation/standup/acceptance_session.py`).

What remains:

| Item | Status |
|---|---|
| A peering with a real counterparty, between two goatnets with different owners | **not built** as a measured run: every measured run had both sovereigns ours |
| The bilateral acceptance session driven by goatbuilder itself, instead of `acceptance_session.py` by hand | **not built** |
| One directory package per counterparty as the default | **built**, opt-in (`FED_DIRECTORY_PER_COUNTERPARTY=1`); the default switch is **not built** |
| `status` measuring a deployed peering's live health | **not built**: it reads the run journal and the evidence |
| The core certificate's name narrowing to a declared `environments[].inventory` | **not built**: it reads every site inventory |
| The non-English `federate` help | **not built**: the other catalogs carry the earlier text and fall back cleanly |

