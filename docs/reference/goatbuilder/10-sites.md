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
run takes the peering through the fourteen phases below; it stops at the two
`MANUAL` phases and resumes with `--from`.

| Phase | Kind | What it does |
|---|---|---|
| 1 | GATE | peering preflight, including the peering directory |
| 2 | A | prepare what the package measures: the federation interface and the SPIRE bundle |
| 3 | T | our half, read from the running system |
| 4 | T | lint our package against the specification |
| 5 | MANUAL | exchange packages and CA certificates through the peering directory; read the verifying key and CA fingerprint aloud and record both keys |
| 6 | T | compose one agreement with one digest |
| 7 | T | verify the agreement from our side |
| 8 | T | mint the peering leaf certificates from the agreement |
| 9 | T | render the access rules and configuration; the only writer |
| 10 | MANUAL | review what was rendered; admit the edge on our core |
| 11 | A | open the transport the agreement chose, or refuse when no play stands it up |
| 12 | A | deploy the edge router, then the republisher on our core host |
| 13 | A | federate the SPIRE trust domains |
| 14 | GATE | acceptance, once per offered release set: it carries, it refuses, it round-trips |

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
step 1. One switch has not landed: one directory package per counterparty
stays opt-in (`FED_DIRECTORY_PER_COUNTERPARTY=1`, or `--per-counterparty` on
`ops/federation/standup/directory.py`), so the directory holds one package per
party by default.
