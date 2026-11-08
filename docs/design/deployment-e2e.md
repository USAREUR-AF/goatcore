# Deployment e2e — the deployment engineering proved end to end, before a person runs it

| Field | Value |
|---|---|
| Status | **Built** 2026-10-28: the hermetic tier (`ops/e2e/hermetic.sh`, CI `deploy-e2e.yml`), the run record, the secrets-coverage and deps-named gates, the rehearsal tier; the live canary **built, not yet run** (its image builds on the first push to main; `--bump` points the dev rung's goatnet at it) |
| Decision | [ADR 1200](../adr/1200-deployment-e2e-three-tiers.md) |
| What it tests | `ops/scripts/{deploy-component,rc,rollback,fastlane,pr-land,ansible-preflight}.sh`, the rollback-ledger role, `ops/release/graduate.py`, `ops/pins/generate-pins.py`, the `goatbuilder` binary — unmodified |
| Audience | whoever changes a driver, and whoever has to trust one before releasing |

## What this is

Three tiers of evidence that a deploy, a release, a rollback and a fastlane work,
each answering a different question, each with the host's ledger as the only proof:

| Tier | Question | Runs | Hosts |
|---|---|---|---|
| **Hermetic** | do the drivers, as committed, move a component through dev → preprod → prod and back? | every PR touching them; nightly | three containers on the CI runner that look like management hosts |
| **Live canary** | does a deploy work on a real goatnet today? | nightly, from an operator laptop (`ops/e2e/live-canary.sh`) | the management host of the release ladder's dev rung |
| **Release rehearsal** | will the next step of every open candidate run, before a person runs it? | daily, in the CI of the repository that holds the deployment's candidate records and ladder | none; dry runs |

Two gates sit beside them, because the 2026-10-28 releases failed on things no
tier would have moved: **secrets-coverage** (every variable a deploy templates
has a reference line a deployer can copy) and **deps-named** (every tool the
release path invokes is named by its preflight).

## The corpus: what two releases on 2026-10-28 ran into, and which gate now holds it

The first two external releases through `rc.sh` — microscope/v0.1.0-rc.2 (six
retries) and panoscope/v0.1.0-rc.1 (tag to preprod in one day, with an operator
log kept by hand) — are the regression corpus. Every item has a gate or a fix:

| # | What happened | Where it is held now |
|---|---|---|
| 1 | the checkout was 16–20 commits behind origin; `ansible-preflight.sh` refused | hermetic `behind-origin-refused` / `behind-origin-fast-forward-then-ok`; `refuse-feature-branch-checkout` |
| 2a | `rc.sh --dry-run` never ran the secrets check, so a real run tagged and pinned before refusing on `NOT SET` | `rc.sh secrets_preflight` before the tag (dry run prints it); hermetic `release-dry-run-runs-secrets-check`; `goatbuilder release` mirrors it |
| 2b | with no live rows mapped, `rc.sh` died at `line 242: : No such file` (an unset evidence path) | `rc.sh rows_on` computes the path first; hermetic `release-tag-dev-preprod` (no rows mapped for the canary) |
| 3 | `direnv exec . VAR=… cmd` fails (`command 'VAR=…' not found`) | the first-run page and every refusal show `direnv exec . env VAR=… op run …` |
| 4 | 1Password signed out read as missing secrets | `goatbuilder doctor` row (track/error-interpreter) |
| 5 | a variable a deploy templates had no reference line for a deployer to copy | `ops/ci/check-secrets-coverage.py` |
| 7 | `crane` not installed, and `rc.sh` discarded its stderr: "image not found" for an image that existed | `ops/scripts/image-digest` (ADR 1204) uses any of crane, skopeo or docker and keeps the tool's own error; `rc.sh` refuses up front when none is present, `goatbuilder doctor` names it; `check-deps-named.py` |
| 9 | the smoke exited 2 (no Playwright toolchain) and `rc.sh` said "the candidate is dead, delete the tag" | `rc.sh browser_toolchain_check` before the rows, exit 2 = **UNTESTED**, never FAIL; mirrored in `release.rs`; hermetic `release-dry-run-names-browser-toolchain` |
| 13 | the preprod rows ran from the laptop, which had to be meshed to zagros and hold its contexts — against the door model | `goatnet-runners.yaml goat_via: door:<env>`: the rows run `goat` on the rung's management host through the door (`env-door.sh --host-target` through `--ssh-target`); the host must hold the contexts (zagros-dev does; zagros and zagros-prod need theirs minted) |
| 14 | the laptop's zagros profile lacked `onboard_endpoint`; the mint got a bare 405 | moot under 13; the 405 is catalogued (track/error-interpreter) |
| 15 | every retry merged a +3/−3 PR of signature files only | #2046 re-signs only a changed intent and lands nothing when only signatures would change; the deviation edit (`rc.sh set_deviation`, `pins::edit_set_deviation`) now replaces a re-declared deviation where it stands — it used to move it to the top of the map, so every re-run after a later rung's deviation reordered the table and opened a PR; hermetic `release-rerun-dev-step-opens-no-pr` re-runs from dev after the ladder reached preprod and asserts "nothing to land" and no new dev-pin PR |
| 16 | `--from preprod` rebuilt the record's dev evidence from a variable only the dev step sets | `rc.sh` reads the evidence file from disk on a resume |
| 17 | "had to install a couple random things" mid-run | `RC_DEPS` + `check-deps-named.py`: every binary the release path invokes is named before step 1, with the install |
| 18 | both operators kept a run log by hand | the run record (`ops/e2e/rec.py`, `report.schema.json`): every step's command, exit, duration and the ledger rows read, rendered as one page in the CI job summary |
| 19 | microscope's dev deploy landed and "Failed at the Gate" — the gate being the browser row with no toolchain (item 9) | as 9 |

## Hermetic

`ops/e2e/hermetic.sh` builds a world in a temporary directory and runs the real drivers in it:

| Piece | What stands in | Why it is honest |
|---|---|---|
| source of record | a fresh clone of the commit under test, with a bare remote at `$WORK/origin.git` as `origin` | `rc.sh` tags, pushes, lands PRs and fast-forwards against it exactly as against GitHub; the clone's `main` moves |
| declarations | `ops/e2e/overlay.py`: three goatnets `e2e_dev`/`e2e_preprod`/`e2e_prod` (one host each), two components `e2e-canary` and `e2e-canary-secret`, the ladder's rungs pointed at them (the soak gate unchanged; the harness dates the candidate record back a day, a declared step), a throwaway CA (`ca-init`) whose SPKI is committed and whose key sits where `rc.sh` looks (`$HOME/.goat/state/e2e-ca/`), intents generated and signed | the same table shape, the same generator, the same signature gate (`generate-pins.py --check`, `graduate.py check`), nothing of the real goatnets in the clone |
| hosts | `ops/e2e/host/`: Ubuntu 24.04 + sshd (key-only sudoer `goat`), python3, the docker CLI and compose plugin over the runner's socket | the push converger reaches them over ssh, `become`s, pulls, composes, and the rollback-ledger role writes `/var/lib/goat/<component>/deploy-history.jsonl` on them |
| registry | `registry:2` with a self-signed `ghcr.io` certificate; the runner resolves `ghcr.io` to it (`/etc/hosts`, `/etc/docker/certs.d/ghcr.io/`) | every pin, ledger row and compose file names `ghcr.io/usareur-af/e2e-canary@sha256:…` exactly as on a goatnet; the digest is the registry's |
| gh, crane, aws-vault | `ops/e2e/stubs/`: `gh` answers the calls the drivers make and performs a real squash merge into the bare remote; `crane digest` reads the local registry; `aws-vault exec … --` runs the command with no credentials | every call recorded to the run record; the header of each stub says exactly what it answers |
| the canary | `ops/ansible/roles/e2e-canary` (busybox + one file; compose; the ledger meta-dependency) and `e2e-canary-secret` (templates `E2E_CANARY_TOKEN`; deployed through an alias play, F-587) | the fleet's role shape with nothing a canary does not need |

The cases, in order (`ops/e2e/hermetic.sh`, each a row in the record):

| Group | Cases | Proof |
|---|---|---|
| dry runs and refusals | validate; missing secret refused naming the variable; alias target mode; feature-branch checkout refused; whole-ladder dry run; browser toolchain named; secrets check in the dry run; fastlane dry run; goatbuilder validate/deploy/release dry runs; the first-run page's dry-run lines | exit codes and the exact words |
| the push converger | deploy to dev; the ledger row is `action: deploy` with the declared digest; a re-run changes nothing (row count); the secret canary through the alias; `--verify-only`; goatbuilder deploy to preprod and verify | the row read back over ssh |
| the checkout rules | a commit pushed to origin the deployer has not pulled: refused; fast-forward, then ok | the preflight's refusal text |
| the ladder | `rc.sh` tag → dev pin PR (stub-merged) → dev deploy → preprod record + pin → preprod deploy → evidence PR; `--from prod`: graduate, fold into `fleet:`, prod deploy, every rung verified; a re-run of the landed dev step opens no dev-pin PR (item 15); a stale open candidate abandoned by the next candidate's preprod step (C6, #2039) | every rung's ledger names the candidate digest |
| fastlane and rollback | fastlane to all three; `rollback.sh --list/--status`; one step back from the on-host ledger | the hotfix digest everywhere, then the rollback row on dev |
| the page as test | `ops/e2e/doc-as-test.py` runs the fenced blocks of `docs/operations/goatbuilder-first-run.md` §"A deploy, with its proof" with the declared substitutions | a drifted page fails |
| site-cache client | the `site-cache-client` role applied to the dev host with an answering port, then a closed one (goatbuilder-speed.md sink 6) | the probe apt runs prints the cache URL, then `DIRECT`; `apt-config dump` names the probe |
| teardown | `teardown-service-stop.yml` stops the dev canary by compose service name, then `teardown-compose.yml` brings its project down (goatbuilder-speed.md §"Teardown") | each play's own assertion that nothing runs; two `teardown.jsonl` rows read back over ssh; the deploy ledger's row count unchanged |

`--no-hosts` runs the first group only, which is what a laptop without Docker
can run; the rest needs Linux and a Docker daemon (container addresses must be
reachable), which is the CI runner.

## The run record

Every step of every tier goes through `ops/e2e/rec.py`: one JSON object per
step in `report.jsonl` (`ops/e2e/report.schema.json`: name, command, exit,
expected exit, duration, started, the log file, the tail, why it failed, whether
the failure was expected), the ledger rows read at the end in `ledgers.jsonl`,
and the stubs' call logs. `ops/e2e/report.py` renders one page of Markdown and
judges the run; CI uploads the directory as an artifact and appends the page to
the job summary. An operator running the live canary gets the same directory.

## Live canary

`ops/e2e/live-canary.sh`, from a checkout on main or an assembled tree, on a
laptop that reaches the goatnet (mesh or door). The goatnet is the release
ladder's `dev` rung (`ops/release/ladder.yaml`) unless `--env` names another, and
the host whose ledger the rollback steps read is that goatnet's first management
host in `ops/ansible/inventory/doors.yaml` unless `--host` names another; both are
read by `ops/scripts/goatnet-facts.py`. The run: `goatbuilder doctor`, a dry run, the live deploy of
`e2e-canary`, `--verify-only`, `rollback --list`, then a real rollback one step back (proved by the host ledger's newest row reading `action: rollback` and naming the prior ref) and the roll forward through `deploy`, verified again — all recorded. The rollback alternates avenue by day (even days `goatbuilder rollback`, odd days `rollback.sh`; `LIVE_CANARY_AVENUE` overrides, ADR 1201) and needs two refs in the host's ledger, which the second `--bump` provides. The canary
templates no secret, so no `op run`. `--bump` first points the goatnet's deviation
at the newest image `e2e-canary-image.yml` built from main
(`ghcr.io/<ghcr_org>/e2e-canary:sha-<main>`, `ghcr_org` from the goatnet's
group_vars), regenerates, re-signs its intent with the operator's CA key and lands
the pin through `pr-land.sh`. That is a single-repository checkout's path: an
assembled tree takes its pins and intents from the overlay, so there `--bump` is
refused and the canary pin lands on the overlay like any other pin. In `ops/pins/fleet-pins.yaml` the component is
`fleet: null`, absent on every goatnet until the first `--bump`. No GitHub runner
reaches a goatnet (`ops/validation/goatnet-runners.yaml`), so this is the
operator's nightly command, not a workflow.

## Release rehearsal

`ops/e2e/rehearsal.sh`, daily in the CI of the repository that carries the
deployment's release records and live ladder rungs (this repository carries
neither, so `deploy-e2e.yml` here runs the hermetic tier, the goat suite and
the gates): for every open ladder candidate (a record
in state `candidate`) the next step is `--from prod`; for every component whose
dev rung carries an rc deviation with no open record, `--from dev`. Both
`rc.sh --dry-run` and `goatbuilder release --dry-run` rehearse it, and the run
says whether the machine holds node and npm for a browser row. A refusal shows
up in the daily record, not on the day a person runs the step.

## Rows for a rung the laptop is not meshed to

`ops/validation/goatnet-runners.yaml` carries `goat_via: door:<env>` for zagros
and zagros-prod: `rc.sh` and `goatbuilder release` open the door
(`door-open.sh`), resolve `env-door.sh <env> --host-target` to the management
host (`user@<mesh address>`) and `--ssh-target` to the door (`user@127.0.0.1:port`,
passed as `GOAT_ROWS_SSH_JUMP`), and `onboard-intake-smoke.sh` runs its `goat`
legs on the host through the door. The rows run as
the non-person identities the values file names, never as an operator: the
intake rows as `acceptance` (`goat_sponsor_context`, [ADR 1207](../adr/1207-acceptance-rows-run-as-a-non-person-sponsor-identity.md)),
the fabric rows as `assay-probe` (`probe_context`) where the goatnet declares
it. The zagros-dev and zagros hosts hold `acceptance`; the zagros-prod host holds
none, so its rows read **UNTESTED**. A row that cannot run (exit 2) is never a
dead candidate. Operator keys never go on a host.

## Both avenues, three golden paths

The operator's rule, recorded as [ADR 1201](../adr/1201-two-avenues-scripts-documented-cli-supported.md):
both avenues — the shell scripts and the `goatbuilder` CLI — stay functional for
the three golden paths (founder standup, federating node, deployments across
environments); the CLI is the supported path, the scripts documented. Every tier
therefore runs both, and a failure on either avenue is a failure of the run:

| Golden path | Scripts avenue | CLI avenue | Hermetic coverage today |
|---|---|---|---|
| deployments across envs | `deploy-component.sh`, `rc.sh`, `rollback.sh`, `fastlane.sh` | `goatbuilder deploy`, `release`, `rollback`, `fastlane` | **built**: the ladder tag → prod, a fastlane and a rollback run live through each avenue in turn (rc.1 and the first hotfix through the scripts; rc.3 and the second hotfix through the CLI); rc.2 exercises the open-candidate case |
| founder standup | `goat-standup.sh` | `goatbuilder standup` (#2022) | **not built**: a founder standup brings up the whole management stack (NetBird, Ory, Zenoh, Postgres, Traefik) on a host; the container host of this harness holds sshd, python3 and the docker CLI, which is what a deploy needs and not what a standup needs. Next increment: a heavier host image and the standup's phases through both avenues, with `goatbuilder standup --mock` on the conformance fixture as the hermetic founder path until then |
| federating node | `node-addition.md` + `peering-standup.sh` | `goatbuilder join-node` + `federate` | **not built**: needs the founder first; same increment |

## Suites

One harness, two suites. The tiers (hermetic on every pull request, a live tier on
zagros-dev, the daily rehearsal), the run record (`ops/e2e/rec.py`,
`report.schema.json`, `report.py`), the prerequisites scan and doc-as-test are
shared; what differs is the system each suite exercises.

| Suite | System under test | Hermetic | Live |
|---|---|---|---|
| goatbuilder | the deploy machinery: drivers, ladder, records, ledgers on hosts | `ops/e2e/hermetic.sh` (container hosts) | `ops/e2e/live-canary.sh` |
| goat | a running goatnet as an operator meets it: portal, onboarding console, Zenoh router | `ops/e2e/suites/goat/stack.sh` + `run.sh` (job `goat-suite`) | `ops/e2e/suites/goat/live.sh` |

The two compose: goatbuilder builds an environment, goat operates it. The goat
suite's tiers are the T1s, T2 and T3 of [goat-cli-e2e-testing.md](goat-cli-e2e-testing.md).

**The goat suite's local stack** runs on one machine: the real portal
(`ops/goat-zenoh-portal`, registry in Postgres, the router config template the ACL
tests use, its own fakes for ACL reload and certificate signing), the real
onboarding console (`ops/goat-id-console`, stores in Postgres), the console's identity
stand-in for Kratos, Hydra and NetBird management (`web/e2e/identity-fixture.mjs`),
and, where Docker is present, the Zenoh 1.9.0 router (digest-pinned) with an mTLS
listener and the REST plugin. Minting a person (a Kratos identity and a NetBird
key) is not in the stack; the live tier covers the intake path on zagros-dev.

**Classes of escape the goat suite is designed to catch.** Derived from the
classes of defect that past goat, portal and console fixes fell into; the cases
come from what each command promises, and this table checks that no class is blind.

| Class | What the suite exercises |
|---|---|
| Profile and context state the unit tests never set up | a missing context, an unreadable token, a wrong token, `doctor --all` over a real profile |
| Contract drift between the CLI and the portal or console | every mutation read back from the service's own record; the `--json` shapes parsed |
| A partial or failed answer shown as an empty or successful one | a stopped console must fail the command, never print an empty queue |
| Lifecycle side effects across steps | invite → list → revoke; operator-key request → reject, with nothing minted |
| Authorization and tier boundaries | operator-gated verbs against the service's own key check |
| Defaulted or edge inputs | an invite minted with no lifetime is not a one-hour invite |
| The served artifact differs from the built one | T2: the binaries on get.&lt;site&gt; match their checksums and advertised version, and round-trip |
| The real fabric path | the mTLS router session and a publish received (local router; live with a slot certificate) |

## The same pattern for a federation

`ops/federation-e2e/run.sh` runs two sovereigns generated fresh, the real federation driver on both
avenues, a real WireGuard link, and reads the proof from the receiving side's storage and ACL trace.
Its hosts run systemd and their own dockerd, because the edge role bind-mounts host paths and reads
the container's PID, which a socket shared with the runner would break.

## What is not here

- A live canary on preprod or prod: the dev rung is the question "does it work today"; the ladder answers the rest.
- A hermetic run on macOS: container addresses are not reachable from the host there; `--no-hosts` covers the refusal and dry-run cases.
- The `--record` flag inside `goatbuilder`: the wrapper records every run the same way; the flag is a later convenience.
