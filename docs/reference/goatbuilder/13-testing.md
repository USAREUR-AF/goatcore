# 13. Testing and assurance

Four kinds of evidence hold goatbuilder and the scripts to the contract: the
conformance tests on fixtures, the parity gates between the two avenues, the
deployment e2e tiers, and the gates on the config, the pins and the error
catalog.

## Conformance on fixtures

A fixture is a directory that stands in for an environment: a config, the
declared pins and a scripted converger whose machines behave in fixed ways.
`--mock <dir>` runs any verb against it with no registry and no transport.

| Machine behaviour in `converger.yaml` | Tests |
|---|---|
| `converge-now` | the record carries the declared ref after one observation |
| `after: N` | pending until the Nth observation, then converged: resumable (C5) |
| `never` | stays pending: never reported done (C7) |
| `unreadable` | the record cannot be read: unknown, never done (C7) |

| Fixture | Used for |
|---|---|
| `ops/goatbuilder/fixtures/basic` | two components on dev and preprod; the conformance tests |
| `ops/goatbuilder/fixtures/tutor` | the tutor's deploy, drift and rollback lessons, with a `data` artifact |
| `ops/goatbuilder/fixtures/standup` | a founder site with core, obs and break-glass classes |
| `ops/goatbuilder/fixtures/goatnet-from-scratch` | the tutor's release lesson |
| `ops/goatbuilder/fixtures/required-inputs` | the required-input scan |

Every clause, C1 to C10, is built as a test in `crates/goatbuilder` on a
throwaway copy of a fixture, as
[goatbuilder-conformance.md](../../design/goatbuilder-conformance.md) specifies.

## Parity gates

Each verb with a script equivalent has a gate that runs both sides in dry run
and fails on any difference. All run in `.github/workflows/goatbuilder-pr.yml`
on any change to the binary or to the script.

| Gate | Compares |
|---|---|
| `parity-deploy.sh` | groups, transport, play, target flag, ledger, declared ref and digest, for every component on every environment |
| `parity-verify.sh` | the same facts for `verify` against `deploy-component.sh --verify-only` |
| `parity-release.sh` | every step and command of `release` and `rc.sh`, from `tag`, `dev` and `prod` |
| `parity-rollback.sh` | `--list` and `--status`, line for line, for every component with a ledger |
| `parity-history.sh` | `history` against `rollback.sh --list` |
| `parity-fastlane.sh` | the dry run, line for line |
| `parity-pins.sh` | `--check` output, then every regenerated `pins.yml` byte for byte and every intent |
| `parity-promote.sh` | every read-only subcommand line for line, then the mutations on scratch copies |
| `parity-standup.sh` | every phase, command and phase-12 `needs:` line, on three sites |
| `parity-teardown.sh` | both burn-down plans, full and services-only |
| `parity-federate.sh` | every phase and command of one peering |
| `parity-fresh-source.sh` | the fresh-source check's verdict and words: behind, ahead, unreachable and offline |

## Deployment e2e: three tiers



Each tier answers a different question, with the host's ledger as the only
proof ([ADR 1200](../../adr/1200-deployment-e2e-three-tiers.md),
[deployment-e2e.md](../../design/deployment-e2e.md)).

| Tier | Question | Runs | Hosts | Status |
|---|---|---|---|---|
| Hermetic | do the drivers, as committed, move a component through dev, preprod and prod and back? | every pull request touching them, and nightly: `ops/e2e/hermetic.sh` in `deploy-e2e.yml` | three containers on the CI runner shaped like management hosts | built |
| Live canary | does a deploy work on a real goatnet today? | nightly, from an operator machine: `ops/e2e/live-canary.sh` | a dev goatnet's management host, deploying the `e2e-canary` component | built; `--bump` points the dev goatnet at a new canary image |
| Release rehearsal | will the next step of every open candidate run, before a person runs it? | daily, in CI: `ops/e2e/rehearsal.sh` | none; dry runs | built |

Every run writes a record with `ops/e2e/rec.py`, checked against
`ops/e2e/report.schema.json`. The goat suite is in `ops/e2e/suites/goat/`.

Two gates sit beside the tiers. **secrets-coverage** checks that every variable
a deploy templates has a reference line a deployer can copy. **deps-named**
checks that every tool the release path invokes is named by its preflight.

## Static gates

| Gate | Workflow | Refuses |
|---|---|---|
| config schema and generation | local Python suite; `gen-config.py --check` in an assembled deployment | a config that does not validate; unsafe declaration migration; generated-output drift. No public-core workflow currently runs these assembled checks |
| pins | `goatbuilder pins --check` | drift, an unsigned or tampered intent |
| ladder | `goatbuilder promote check` | a malformed ladder, a record without its pin edit, a stale hotfix |
| error catalog | `error-catalog.yml` | an incomplete entry, an uncited or missing code, a stale reference |
| translations | `goatbuilder-pr.yml`, `goat-cli-pr.yml` (`ops/scripts/translate-ftl.py --check`) | an English message missing from any shipped language |
| phase-12 graph | `ops/goatbuilder/standup-dag.py --check` | a cycle or an unknown need |
