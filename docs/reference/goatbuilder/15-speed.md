# 15. Speed and the one-hour goatnet

The target is a founder site declared to verified in sixty minutes, one new
node in fifteen, and a burn-down to nothing, records kept, in fifteen. The
budget below says where each minute goes; most of it is modelled, and the first
timed standup on hardware is the measurement.

| Field | Value |
|---|---|
| Targets | founder site ≤ 60 min · one new node ≤ 15 min · burn-down ≤ 15 min |
| Design | [goatbuilder-speed.md](../../design/goatbuilder-speed.md) |
| Status | budget written; levers 1 to 6 built in both avenues (lever 5 WIP until a seeded host is timed); the hour not yet measured on hardware |

## The budget

| Phase | Minutes | Basis |
|---|---|---|
| 1 gates, preflight, required-input scan, door | 3 | measured: the scan takes 6–13 s |
| 2 host baseline and disk encryption, hosts together | 10 | modelled, with lever 5 |
| 3–4 secrets, leaf certificates, name snapshot | 2 | modelled |
| 5–6 management stack, relay, identity | 15 | modelled |
| 7 public TLS and relay tier | 5 | modelled |
| 8 enrolment, scripted | 3 | modelled |
| 9 router and workload identity | 5 | modelled |
| 10–11 dashboard, OIDC clients, first admin and token | 4 | a person, at phase 11 |
| 12 services in six waves | 8 | modelled, with lever 4 |
| 13 acceptance | 3 | modelled |
| 14 rescue nodes and cross-goatnet surfaces | 2 | modelled |
| **Total** | **60** | |

## The levers

| # | Where time went | Lever | Status |
|---|---|---|---|
| 1 | defects found during the run, one stop each | `doctor --site` and phase 1's gate scan every play the run reaches for required inputs and secrets, in one pass | built; measured at 91–132 requirements per site in 6–13 s |
| 2 | a new SSH connection per Ansible task | connection multiplexing for direct hosts | built; measured 6.4 s to 1.5 s per call |
| 3 | human pauses | the bot-token pause removed; phase 8 enrolment scripted | both built; phase 8 not yet run on hardware |
| 4 | fourteen phase-12 steps one after another | `--parallel` runs them in six dependency waves | built; modelled at about 8 min |
| 5 | host baseline and disk encryption, 10–20 min per host | an installer seed per node class (`node-seed.py`) | built, WIP until a seeded host is timed; modelled at 3–5 min per host |
| 6 | image pulls per role | a pre-pull of every pinned digest, and a package cache and Docker Hub mirror on the observability host | built; not yet measured |

Phase 11 stays a person: the first NetBird token needs the first
administrator's browser login.

## Measuring a run

Both avenues print the time each phase took. On a fixture, `standup --mock`
reports declared-to-verified time for the whole rehearsal.

```bash
goatbuilder standup --mock ops/goatbuilder/fixtures/standup dev --site alpha --ack "<your name>"
```

Measured on the fixture: five components on four machines in 9 ms, and a second
run writes no record.

## The required-input scan

`ops/scripts/standup-required-inputs.py` reads every play the site's standup
reaches and lists every input it requires: role assertions, `| mandatory`
filters, and the secrets the plays template. `goatbuilder doctor --site <site>`
prints it as rows; phase 1 runs it as a gate.

| Code | Meaning |
|---|---|
| GB-060 | an input the run will reach is required and not set |
| GB-061 | a secret the run reads is one the posture cannot supply |
| GB-062 | an input may be required: its condition cannot be decided before the run (a warning) |
| GB-063 | a play's host limit selects no host, so the play does nothing (a warning) |
