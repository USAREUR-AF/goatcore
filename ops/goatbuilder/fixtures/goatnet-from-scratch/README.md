# goatnet-from-scratch — the worked example behind the goatnet tutorials

The goatnet the tutorials build, proven step by step, so a reader can diff their
own run against a known-good one. Design: `docs/design/goatnet-tutorials.md`
(ADR 1202).

| Directory | Tutorial | What it holds |
|---|---|---|
| `01-founder/` | 1 — a new goatnet | the goatnet `zagros` with its founder site `alpha` (core ×2, obs, breakglass) and five components; `records/` empty |
| `02-first-federated-node/` | 2 — the first federated node | the same goatnet with the joiner site `beta` attached to `alpha`; `records/` and `declared/publications.jsonl` seeded with tutorial 1's end state, so serials continue from 6 |
| `03-boundary-node/` | 3 — a boundary node | the same goatnet with an `edge` class at `alpha` and one `peerings:` entry, `hello-ally`; `goatbuilder --mock 03-boundary-node federate hello-ally --dry-run` plans the peering against that declaration. No `records/` or `converger.yaml`: `federate --dry-run` plans, it does not converge a mock |
| `checkpoints/` | all three | the recorded output of each tutorial's rehearsal, normalised (milliseconds and scratch paths) |

Each directory is a `goatbuilder --mock` fixture holding a `goatbuilder.yaml`
(the declaration, valid against the v1 schema). The two standup tutorials (1 and
2) add `declared/pins.yaml` (digest-pinned refs) and `converger.yaml` (how each
machine behaves under the scripted converger, and the standup's classes and
phase order); tutorial 3 is a `federate` plan, so it has the declaration and its
`peerings:` entry but no converger. `diff 01-founder/goatbuilder.yaml
02-first-federated-node/goatbuilder.yaml` is exactly what tutorial 2 declares,
and `diff 02-first-federated-node/goatbuilder.yaml 03-boundary-node/goatbuilder.yaml`
is the `edge` class and the peering tutorial 3 adds.

## Status

| Proof | Status |
|---|---|
| Tutorial 1 on the fixture | **measured** — `standup` 5 components on 4 machines, `status` green; `checkpoints/01-new-goatnet.txt` |
| Tutorial 2 on the fixture | **measured** — joiner 4 components on 3 machines including the founder-side mirror on the founder's machines, `status` green across both sites; `checkpoints/02-first-federated-node.txt` |
| Kept true on every pull request | **built** — `ops/tutorials/check.sh` runs each tutorial's rehearsal commands as written and diffs against `checkpoints/`; the Rust tests `goatnet_from_scratch_*` in `crates/goatbuilder/src/standup.rs` |
| The Hello Galaxy repository equal to this directory | **built** — `ops/tutorials/sync-goatnet-from-scratch.sh`, run by hand; no CI joins the two repositories |
| Tutorial 1 or 2 on real hardware | **not run** — the first timed founder standup on hardware is the one-hour goatnet's measurement |
| Tutorial 3, the boundary node | **measured** on the fixture — `federate --mock 03-boundary-node --dry-run` plans all fourteen peering phases from the fixture's declared `hello-ally` peering; `checkpoints/03-boundary-node.txt`. Its live `Done` step needs a counterparty, so it is not stood up on a mock |

## The Hello Galaxy repository

The same step directories and checkpoints are the repository
`USAREUR-AF/hello-galaxy-goatenv` (created 2026-10-29), for a reader
who clones the worked example alone; its `./run` rehearses the three tutorials
and compares them with the checkpoints. This directory stays because
`goatbuilder tutor`, the `goatnet_from_scratch_*` tests and `ops/tutorials/check.sh` read
it on every pull request here.

Tutorial 3 here is the light `federate` rehearsal. A reader who wants
cross-environment federation run in full — a second goatnet, real package
exchange, a live gate — clones `hello-galaxy-cross-env-federation`,
which runs the same flow end to end.

A change to a fixture or a re-recorded checkpoint is copied to the Hello Galaxy
repository in the same piece of work:

```text
ops/tutorials/sync-goatnet-from-scratch.sh <clone> --goatbuilder <bin>           # compare; exit 1 on any difference
ops/tutorials/sync-goatnet-from-scratch.sh <clone> --apply --goatbuilder <bin>   # copy the steps, re-record the checkpoints, set GOATBUILDER_REF
```

`<bin>` is a goatbuilder built at this checkout. The script copies only the files
this directory holds; a file only the clone has (its `checkpoints/base-images.txt`)
is kept and named. It never copies a checkpoint: the clone rehearses tutorial 3 from
a copied fixture, so its checkpoint names other paths. The clone's own `./run`
checks the checkpoints and, with `--apply`, re-records them (`./run <bin> --record`,
with `GOAT_REPO` set to this checkout). Without `--goatbuilder` the checkpoints are
not compared, and the script exits 3.
