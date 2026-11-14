# 10. Standing up, proving, joining, tearing down and federating

Five verbs work on whole sites rather than one component. `standup`,
`join-node`, `teardown` and `federate` run numbered, idempotent phases with a
fail-closed gate at each end. Each prints the exact command of every phase with
`--dry-run` and resumes with `--from <phase>`. A phase marked `[MANUAL]` stops
the run and says which `--from` resumes it. `assay` reads only: it proves what
the other four built.

Phase kinds, as every plan prints them:

| Kind | Means |
|---|---|
| `GATE` | a fail-closed check; the run stops if it fails |
| `T` | a tool on the operator's machine |
| `A` | an Ansible play |
| `MANUAL` | a person's step; the run stops and names the resume point |

## `standup`

**Purpose.** Take a site from nothing to verified: the phases of
[goatnet-standup.md](../../operations/goatnet-standup.md) in the doctrine's
order, the observability host as a parallel leg, the rescue nodes and the
cross-goatnet surfaces last
([ADR 1183](../../adr/1183-rescue-nodes-are-part-of-the-site-baseline.md)).

**Use it** to build a new goatnet's founder site, or a new joiner site of an
existing goatnet.

## `federate`

**Purpose.** Federate this goatnet with another sovereign goatnet. Today one
run takes the peering through the fourteen phases below; it stops at phase 5
(`CONFIRM`) and phase 10 (`MANUAL`) and resumes with `--from`. The table is
rendered from the phase catalog (`ops/phases/catalog.yaml`); `goatbuilder
explain federate <phase>` prints one phase's whole entry.

<!-- BEGIN generated: phase-catalog federate summary -->

| Phase | Kind | What it does |
|---|---|---|
| 1 | GATE | preflight, fail-closed |
| 2 | A | prepare what the package measures: the federation interface and the SPIRE bundle |
| 3 | T | `init` — our half, measured |
| 4 | T | `lint` |
| 5 | **CONFIRM** | exchange through the directory; read the key and CA fingerprint **aloud** |
| 6 | T | `compose` |
| 7 | T | `verify` |
| 8 | T | validate or mint the peering leaves |
| 9 | T | `apply --confirm` |
| 10 | MANUAL | admit the edge on our core |
| 11 | A | open the transport the AGREEMENT selected |
| 12 | A | deploy the edge, then the outbound republisher (`peering-zenoh-edge.yml`, then `peering-republisher.yml` with `--limit <peering>_core`) |
| 13 | A | federate the SPIRE trust domains (ADR 1157 step 2) |
| 14 | GATE | acceptance: carries, refuses, and round-trips |

<!-- END generated: phase-catalog federate summary -->

Phase 1 needs `FED_DIRECTORY_URL`, `FED_COUNTERPARTY_ID` and `FED_SIGNING_KEY`
(`FED_NO_DIRECTORY=1` only for a counterparty that can reach no directory).

```bash
goatbuilder federate <partner> --env <goatnet> --dry-run
goatbuilder federate allocate <partner>                       # the peering's values and files, for review
goatbuilder federate <partner> --env <goatnet>                # at a terminal: the whole procedure
```

At a terminal the run asks at phases 5, 6 and 10: the partner's verifying key,
typed as it is read aloud, and their CA fingerprint confirmed at phase 5; the
agreement digest confirmed at phase 6; the plan's hash and then what our core
admits at phase 10. A run that stops resumes where it stopped.
`--as` says which party we are, `--edge-group` names the Ansible group of the
edge (default `<partner>_edge`), and `--work` the working directory (default
`$FED_WORK`, else `/var/lib/goat-federation/<partner>`). `--no-confirm` keeps
them by hand: the run stops at 5 and 10 and resumes with `--from 6` and
`--from 11`. **Script:** `ops/scripts/peering-standup.sh`.
Status: measured (run up-4b1adc18cc3b-20261111T102642).

**The stages.** `goatbuilder federate <partner>` runs phases 1 to 9 (measure,
exchange, compose, mint, render); then come the stages, each also runnable
alone: `goatbuilder federate plan <partner>` (phase 1's checks, and the plan of
what is in the working directory), `deploy` (phases 10 to 13, after approval bound to the plan's hash), `test`
(phase 14: acceptance, whose checks read PASS, FAIL, BLOCKED, SKIP or NOT
APPLICABLE) and `status`. The offered release sets are owned by
`federation.toml` (`[data] release`); the peering's declaration may only narrow
them. The procedure:
[sovereign-federation-runbook.md](../../operations/sovereign-federation-runbook.md),
step 1. Phase 5 publishes one directory package per counterparty, so each of
an environment's peerings keeps its own package; `FED_DIRECTORY_PER_COUNTERPARTY=0`
returns to one package per party (the single slot and pointer), for a directory
that has no per-counterparty slots.
