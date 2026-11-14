# Release-candidate validation — a tag decides when the expensive tests run, and which

**Status:** Designed 2026-09-08; the flow map, the affected-flows tool, the cheap-suite job and the nominator check are built; the dev-rung activation and the mesh-side runner wiring are the next increments · **ADR:** [1173](../adr/1173-release-candidate-tags-trigger-affected-flow-validation.md) · **Composes with:** [release-graduation.md](release-graduation.md) (the ladder this feeds), [goatnet-acceptance-suite.md](goatnet-acceptance-suite.md) (the live rows), [three-stage-candidate-onboarding.md](three-stage-candidate-onboarding.md) §"Acceptance gate" (the first covered flow)

## What this is

A release candidate is a git tag. Cutting it is the one act that starts the expensive validation of a component: the candidate image is built from the tag, the flows the change affects are computed from the diff, the cheap browser suites for those flows run in CI at once, and the live acceptance rows for them run on the dev goatnet from a mesh-connected controller. A passing live run is the validation evidence the release ladder's dev→preprod hop requires. Before the tag exists, only the cheap suites run, on pull requests.

## The tag

`<component>/v<major>.<minor>.<patch>-rc.<n>`, on a commit that is on `main`. Examples: `goat-id-console/v1.4.0-rc.1`, `brief/v0.9.0-rc.2`.

- The component prefix is the one the component's image workflow already triggers on (`tags: ['goat-id-console/v*']` in `.github/workflows/goat-id-console-image.yml`). The tag therefore builds the image once, tagged `v1.4.0-rc.1` beside its `sha-<short>` tag. The digest that graduates is that digest.
- `-rc.<n>` counts attempts. A candidate that fails is abandoned by deleting its tag; the fix is tagged `-rc.<n+1>`.
- A tag whose commit is not on `main`, or whose pusher is not a nominator, is deleted by the workflow and reported (§"Who may nominate").

## Which flows a change affects

`ops/validation/flow-map.yaml` is the map. One entry per flow:

```yaml
- id: candidate-intake
  what: a candidate opens the invite link and submits the public intake form
  paths: [ops/goat-id-console/web/static/onboard.html, ops/goat-id-console/internal/handlers/onboard_public.go, ...]
  cheap: [{dir: ops/goat-id-console/web/e2e, run: npm ci && npx playwright install --with-deps chromium && npm test}]
  live: [intake-form-browser-submit]
```

`ops/validation/affected-flows.py` intersects a diff with the map:

```
affected-flows.py --component id-admin-console --head goat-id-console/v1.4.0-rc.1
```

The base of the diff is the commit the fleet runs for that component: `ops/pins/fleet-pins.yaml`'s `fleet:` value carries the image's `sha-<short>` tag, which is the git short sha the image workflow built from. So the answer is "everything that changed since what the fleet runs", which is what a release candidate is. A pin that names only a digest carries no commit; the base is then the component's previous `<component>/v*` tag, and with none the empty tree, so every mapped flow counts as affected.

**What a candidate carries is written down where it is reviewed.** The tag names only the component and version (`<component>/v<semver>-rc.N`), the semver convention. `rc.sh` (and `goatbuilder release`) list the commits since the component's previous tag that touch the paths its image workflow builds from. That list goes into the dev pin PR body and the preprod PR body, and its one-line form (`N change(s) since <tag>: #a, #b`) into the candidate record's `reason`.

The tool prints each affected flow with the files that made it so, the live rows to run, and any changed files that match no flow. `--json` gives the workflow the same. `--lint` fails when a flow names a suite directory or a manifest row that does not exist, and `--self-test` proves a hit and a miss offline.

Four flows are mapped today:

| Flow | Cheap suite (PR + tag) | Live rows (tag only) |
| --- | --- | --- |
| `candidate-intake` | `ops/goat-id-console/web/e2e` (Playwright, mocked `/api/onboard/*`) | `intake-form-browser-submit` |
| `onboarding-review-and-mint` | none (Go tests only; the admin pages sit behind Ory login) | none yet |
| `portal-topic-registration` | `ops/goat-zenoh-portal/web/e2e` | none yet |
| `panoscope-frontend` | `ops/panoscope/frontend` (typecheck + build) | none yet |

"None yet" is printed, not hidden. Adding a suite or a row means adding it to the map in the same PR; the lint runs in the release-candidate workflow and in the PR check.

## What runs where

| When | Where | What | Cost |
| --- | --- | --- | --- |
| Pull request touching a suite's paths | GitHub runner | that suite, mocked backend (`id-console-web-e2e.yml`, `portal-web-e2e.yml`, `panoscope-frontend-test.yml`) | minutes, no goatnet |
| Release-candidate tag | GitHub runner (`rc-validate.yml`) | nominator check; image build (existing workflow); `affected-flows.py`; every affected flow's cheap suite; a job summary with the live commands | minutes, no goatnet |
| After the candidate is deployed to zagros-dev | the operator laptop, or later the zagros-dev management host, mesh-connected | `run-acceptance.py --env zagros-dev --only <rows>` | mints invites, drives the goatnet; minutes to tens of minutes |

The live half runs from the mesh because that is the path people traverse and because the GitHub runners hold no goatnet credential. The two candidate-intake rows (`intake-form-browser-submit`, `review-gate-accepts-open-field-invite`) run as the goatnet's non-person sponsor identity `goat-acceptance` ([ADR 1207](../adr/1207-acceptance-rows-run-as-a-non-person-sponsor-identity.md)): sponsorship capped at `browse`, no operator power. Its goat context is `acceptance` (`goat_sponsor_context` in the generated values file) on the management host the rows run `goat` on. The smoke mints, vouches and then withdraws its own vouch, so no row needs an operator key on any host.

### Rows for a rung the laptop is not meshed to

A row runs `goat` where the context is. `ops/validation/goatnet-runners.yaml`
names, per goatnet, the host that holds the context (`goat_via`): zagros-dev's
management host over the mesh, and for zagros and zagros-prod `door:<env>` — the
driver opens the door (`door-open.sh`), takes the management host's address from
`env-door.sh <env> --host-target` (`user@<mesh address>`) and the door from
`--ssh-target` (`user@127.0.0.1:port`), and every ssh the rows make goes to the
host through the door: the smoke runs its `goat` legs there. The laptop
needs neither that goatnet's mesh nor its contexts, which is the door model
([env-doors.md](env-doors.md)). The host must hold the contexts the rows name, all
non-person: `acceptance` for the intake rows, and `assay-probe`
(`probe_context`, where the goatnet declares it) for the rows that read the
fabric. The runners file names no context; operator keys never go on a host. The driver passes the door as
`GOAT_ROWS_SSH_JUMP`; `goat` rows, host rows (`cmd-assert` with a `host:`) and
`onboard-intake-smoke.sh` all jump through it with a ProxyCommand (#2100). The
door address is never the ssh destination itself: it is a tunnel port whose host
key changes between goatnets, so only the host behind it is verified. A row that
cannot run (the runner exits 2: a tool or a context missing) is reported
**UNTESTED** and stops the run — never a failed row, never a dead candidate.

## Evidence and the ladder

The ladder's dev→preprod hop carries a `validation-evidence` gate (`ops/release/ladder.yaml`, `status: planned`). The preprod candidate record (`ops/release/records/<component>/<date>-<ref12>.yaml`) names the release candidate's live run as its `evidence.validation`: the tag, the rows, the result file (`~/.goat/state/zagros-dev-acceptance/results.jsonl`, one line per row) and the date. `graduate.py` already refuses a candidate whose `evidence.validation` is empty.

Activating the dev rung is one edit to `ladder.yaml` (`goatnet: zagros_dev`, hop `status: live`) and is done after the first candidate has gone through this path by hand, so the tool's C1 well-formedness check meets a rung that has been exercised.

## Who may nominate

People holding the **admin** role on the repository, and nobody else. That is GitHub's own permission model: the workflow reads the tag pusher's role with the token every workflow already has (`repos/{owner}/{repo}/collaborators/{user}/permission`), so the check needs no secret, no organisation read and no team lookup. Every other role may open pull requests and may not cut candidates; a candidate tag from a non-admin is deleted by the workflow and the run says so.

Adding a nominator means granting that person admin on the repository, deliberately. The server-side half is the repository ruleset `release-candidate-tags` (active since 2026-09-09): creation, update and deletion of `refs/tags/*/v*-rc.*` are restricted with a bypass for the repository admin role only, so GitHub refuses a non-admin push before `rc-validate.yml` runs. The workflow check remains as the second layer and the record of who nominated.

`ops/scripts/rc.sh` runs the same role check before it cuts a tag, so a non-admin operator is refused locally rather than by a deleted tag.

An earlier draft bound nomination to CODEOWNERS team membership and needed an organisation-read credential in the workflow to check it. The first candidate tag was deleted twice on that credential's absence (2026-09-09). The role check replaced it.

## One driver

`ops/scripts/rc.sh <component> --version X.Y.Z --ack "<name>"` performs the whole path a candidate takes to preprod, the way `fastlane.sh` performs a fix's path to every goatnet: cut the tag on main, wait for `rc-validate` and the image build, resolve the digest under both registries, land the dev deviation (`pr-land.sh`: commit, checks, merge), deploy the dev goatnet and verify every host's deploy ledger (`deploy-component.sh`), run the live rows the change affects (`affected-flows.py` → `run-acceptance.py`, with the goatnet's runner settings from `ops/validation/goatnet-runners.yaml`), open the candidate record with that evidence, land the preprod deviation, deploy and verify preprod, run the rows there, record convergence. It stops at the soak (ADR 1120: at least one day on preprod, tool-enforced, a waiver is a record and not an override). After the window the same driver performs the prod fold: `rc.sh <component> --tag <candidate tag> --from prod --ack "<name>"` graduates the record, makes the candidate the fleet value and deletes the per-rung deviations that carried it, abandons as superseded the prod hotfix record whose ref the fleet value was, regenerates and re-signs pins and intents, lands the PR, deploys and ledger-verifies prod, and re-verifies the dev and preprod ledgers so every rung reads one value. `--dry-run` prints every step; `--from` resumes one. A failing live row ends the candidate: fix on main, cut rc.N+1.

## Running a candidate, step by step

Run this from a normal clone checked out on `main` — not a per-track worktree (the prerequisites below say why). One time, install the Python deps the pin/intent render needs:

```
pip install pyyaml jsonschema        # or pipx, or a venv
```

Then, for each candidate:

```
cd <your goat clone>
git checkout main && git pull --ff-only origin main
ops/scripts/rc.sh <component> --tag <component>/vX.Y.Z-rc.N --from dev --ack "Your Name"
```

`rc.sh` fast-forwards your checkout after each pin merge, so the deploy that follows is never stale. If it stops, it names what to fix — a missing dep, a non-`main` checkout, a failed live row; fix that and **re-run the same command**, which resumes and is safe to repeat. The run takes the candidate through the dev rung (deploy + live rows) to the preprod rung and opens the candidate record. After the preprod soak (ADR 1120), fold it to prod with the same driver: `rc.sh <component> --tag <that tag> --from prod --ack "Your Name"`.

## Prerequisites, and re-running after a failure

- **Secrets.** A deploy templates each component's secrets for a goatnet out of **your environment** (`group_vars/<group>/vars.yml` binds `<component>_…: lookup('env', NAME)`); a fresh clone has none. `deploy-component.sh` derives the full set for the component and goatnet (`ops/scripts/deploy-secrets-needed.py`) and refuses **before any host is touched**, naming every unset variable. The references (pointers, no values) are in the private `goat-credentials` repository, `refs/goat.refs`; put the named lines in `.envrc.local.d/<file>.refs` and run the same command under `op run --env-file=.envrc.local.d/<file>.refs -- …` (that repository's `local-secrets-pattern.md` §"A deployer's first run"). The values exist only inside that one process.
- **Python deps.** The pin and intent render (`ops/pins/generate-pins.py`, `ops/consistency/.../intent.py`) need `jsonschema` and `PyYAML`. Install them once — `pip install jsonschema pyyaml` (or a pipx/venv) — before running `rc.sh`. The driver now preflights this in step 1 and fails fast with the install command, rather than stopping halfway through a deploy after the tag and image already exist.
- **Run from a `main` checkout.** `rc.sh` authors the pin in a scratch worktree, merges it to `origin/main`, and then deploys from **the checkout you invoked it in** — Ansible reads the pins and group_vars from that working copy. Only a `main`/`master` checkout is fast-forwarded to carry the just-merged pin (and `ansible-preflight.sh` enforces its staleness guard only there); a feature-branch worktree both skips that guard and need not hold the merged pin, so it can silently deploy a stale ref and still verify green. `rc.sh` now refuses at preflight if it is not run from `main`/`master`, and fast-forwards that checkout after every merge. Keep the clone you run releases from on a current `main`.
- **Or run from an assembled tree.** A team that keeps its goatnets' pins, intents and release records in an overlay repository deploys from the tree the overlay's `assemble.sh` builds (the `.goat-assembled` marker). `rc.sh` and `goatbuilder release` recognise the marker and run every step after the tag there (`--from dev|rows|preprod|preprod-rows|prod`, with `--tag`). Its scratch tree is a second assembled tree, built from a fresh overlay worktree on `origin/main`, so every tool a step calls is the pinned one. Each step's edits land as a pull request on the overlay repository, carrying exactly the files the step changed. After a landing the scratch tree is assembled again from the merged overlay and the deploy runs from it, so `fresh-source.sh` sees a current overlay. The nominator check reads the overlay repository's admins. Cutting the tag stays with a checkout of the repository whose image workflow builds the component.
- **Re-running is safe.** `rc.sh` is resumable: re-running with the same `--tag` validates the existing tag as-is (step 2 prints "tag exists on origin — validating it as-is") rather than erroring, and the deploy and verify steps are idempotent. So after fixing a missing dep or a transient failure, just run the same command again. `--from <step>` resumes at one step; `--dry-run` prints every step without acting.
- **A re-run whose pin already merged lands nothing.** The regenerate step re-signs only an intent whose body changed (an ECDSA signature is fresh every time, so re-signing an unchanged body is a diff of `.sig` files and nothing else), and the landing step compares the worktree with `origin/main` first: when they differ in signatures alone, or not at all, it prints `pins already current on main; nothing to land` and opens no pull request. `pr-land.sh` holds the same rule for every driver, and `goatbuilder release` / `fastlane` apply it in process.
- **The stale-tree-after-merge pitfall (fixed, and how to escape it).** `rc.sh` lands each pin/record PR to `origin/main` and then deploys **from your local checkout**, which `deploy-component.sh` → `ansible-preflight.sh` requires to be current — Ansible reads role defaults (image pins, group_vars) from the working tree. The driver now fast-forwards your local `main` to `origin/main` inside `land()` after every merge, so the deploy that follows is never stale. If you ever still see `ansible-preflight: REFUSE — local main is N behind origin/main` (for example a pre-fix checkout, or you ran from a branch), the escape is exactly what the message prints: `GOAT_ALLOW_MASTER_GIT_MUTATE=1 git -C <repo> pull --ff-only origin main`, then re-run the same `rc.sh … --from <rung>` — the pin PR already landed, so the resume just deploys it.
- **A re-run on a later day lands nothing in `goatbuilder release`.** The dev rung's deviation reason carries the day it was declared. `pins::edit_set_deviation` leaves a goatnet that already declares the candidate's value as it stands, reason and date included, so re-running the dev step after midnight UTC changes no byte of `fleet-pins.yaml` and opens no dev-pin PR (built; `release::tests::a_dev_step_rerun_on_a_later_day_leaves_fleet_pins_byte_identical`).

### Before a person runs a step

Every day `ops/e2e/rehearsal.sh` dry-runs the next step of every open candidate
through both drivers, and every PR that touches a driver runs the whole ladder
hermetically against container hosts ([deployment-e2e.md](deployment-e2e.md),
ADR 1200). `rc.sh` names every tool it will need and every secret each rung
templates before it tags; a dry run prints both. A re-run after a failure is
the same command.

## Failure handling

- **`<goatnet>: deploy FAILED`** means the Ansible deploy playbook hit a fatal task on that goatnet's hosts (its PLAY RECAP showed `failed=` or `unreachable=`); the first `fatal:` lines and the full log path are printed right after. It is **not** the `deploying ghcr.io/…/<image>` line above it (that is the resolved ref the run is about to deploy), and **not** the ledger check (which prints `NOT converged`). Read the named log's `fatal:` lines for the cause — a common one is an image pull/auth failure against `ghcr.io/usareur-af` (the only image namespace since ADR 1195); fix it and re-run the same command.
- A cheap suite fails on the tag: the candidate is dead. Delete the tag, fix on `main`, tag `-rc.<n+1>`.
- A live row fails on zagros-dev: same. The result file keeps the FAIL line; nothing is rewritten.
- A live row reports UNKNOWN (the probe could not run): not a pass. The gate counts only PASS ([ADR 1115](../adr/1115-goatnet-e2e-acceptance-suite.md) decision 3).
- The map matches nothing for a change: the workflow prints the unmapped files; a person decides whether a flow is missing from the map. That is a map defect to fix in the same candidate, not a reason to skip.

## Build order

1. Built 2026-09-08: `flow-map.yaml`, `affected-flows.py` (with `--lint`, `--self-test`), `run-acceptance.py --only a,b`, `rc-validate.yml` (nominator check, affected computation, cheap suites, live-command summary), the first covered flow.
2. Done 2026-09-09: the first candidate, `goat-id-console/v1.0.0-rc.2`, cut by hand (rc.1 was deleted twice by the nominator check before #915), deployed to zagros-dev as the dev-rung deviation (#917), both onboarding rows PASS through `run-acceptance.py` (evidence `ops/validation/evidence/goat-id-console-v1.0.0-rc.2-zagros-dev.jsonl`), `ladder.yaml` dev rung and dev→preprod hop activated, preprod candidate record opened. Found on the way and fixed: the runner printed `GATE: PASS` over zero selected rows; it now refuses an unknown `--only` id and reports NOTHING PASSED when no row reached PASS.
3. Then: a browser suite and a live row for `onboarding-review-and-mint` (needs an automated Ory login for a test admin), and rows for the portal and panoscope flows.
4. Then: the mesh-side runner moves from the operator laptop to the zagros-dev management host, fired by the deploy ledger ([goatnet-acceptance-suite.md](goatnet-acceptance-suite.md) §Triggering v2).
