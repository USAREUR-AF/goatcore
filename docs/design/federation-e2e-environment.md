# Federation e2e environment

Status: WIP — gate: the full scenario matrix runs from `ops/federation-e2e/env` against `main` with every scenario built, and the fast subset runs in `.github/workflows/federation-e2e.yml`.

The federation e2e environment runs synthetic sovereign environments on one Linux machine and federates them through the real driver, so that any goatcore ref can be measured against a fixed list of scenarios. Each scenario states its pass condition and ends in one of five outcome words. The environment extends `ops/federation-e2e/run.sh`, the two-sovereign end-to-end test; run.sh keeps working and now calls the same engine.

| Part | Status |
|---|---|
| Engine (`ops/federation-e2e/lib/engine.sh`): hosts, sovereigns, the fourteen phases, three parties | Built |
| Several runs on one machine (lab names, subnets, labels), any ref as the tree under test | Built |
| Result schema `fed-e2e-result/1` and verdict | Built |
| `env` entry point, units, scenario catalog, matrix page, watcher | Built |
| Scenarios in the catalog marked `not_built:` | Not built; each entry names why |
| Core router deployed by the `goat-zenoh` role | Built: `FE2E_CORE=role` (unit `pair-role`, scenarios `core-by-role`, `preflight-slot-list`); the default stays the hand-run router until the other units pass under it |
| Running the same scenarios on real hosts through `live-host.sh` | Designed, not built (§Real hosts) |

## Run one scenario on your own machine

You need Linux with Docker, the `wireguard` kernel module, a Rust toolchain and Python 3 with `pyyaml` (`jsonschema` validates results). The environment builds the tools itself.

```bash
ops/federation-e2e/env list                                  # scenarios, where each runs
ops/federation-e2e/env run --ref origin/main --avenue goatbuilder --scenario two-party-full
ops/federation-e2e/env run --ref my-branch --avenue both --scenario all --jobs 3 --out /tmp/fe2e
ops/federation-e2e/env up --ref my-branch                    # one federated pair, left running
ops/federation-e2e/env status                                # labs in use
ops/federation-e2e/env down --lab 3                          # remove one lab, nothing else
```

`run` prints the outcome per scenario and the run directory. The run directory holds each unit's step logs (`<avenue>/<unit>/NN-<step>.log`), `report.jsonl`, and `results-<avenue>.json` and `.md`. With `--out DIR` it also writes `DIR/<name>/<sha12>.md`, `DIR/<name>/<sha12>-<avenue>.json` and rewrites `DIR/latest.md`.

## What one sovereign is

Each sovereign is generated fresh for a run and shares nothing with another sovereign except the boundary network. That matches two organizations that never hold each other's keys.

| Piece | How it is made |
|---|---|
| Certificate authority, signing key, operator profile | `gen_sovereign` in `lib/engine.sh`, new per run, P-256 and Ed25519 |
| Its own repository | a clone of the ref under test, with `overlay.py` writing that sovereign's `goatbuilder.yaml`, inventory and group_vars |
| Core host | a privileged container running systemd and its own dockerd: the core router (`<id>-zenoh-router`, label `org.goat.component=zenoh-core`), `spire-server`, and from phase 12 the republisher |
| Edge host | a second privileged container: the edge router, the WireGuard interface, the driver |
| Release producers | samples written under `release/<set>/` on the core, as a producer would write them |
| Peering directory | saltlick behind a TLS front with a local CA, on the boundary network; never the public directory |

Party ids are the repository's synthetic ids (`xaa`, `xab`, `xac`, `xad`). run.sh's defaults keep the ids it has always used.

## Isolation between runs

Several runs share one Docker daemon, and other work on the machine uses Docker too. Everything the environment creates is therefore named, addressed and labeled by lab, and removal goes by label only.

| | Value for lab N (1 to 14) |
|---|---|
| Containers | `fe2e-lN-<id>-core`, `fe2e-lN-<id>-edge`, `fe2e-lN-directory[-tls]` |
| Networks | `fe2e-lN-<id>` (one per sovereign), `fe2e-lN-boundary` (internal) |
| Subnets | `10.<230+N>.0.0/16`: sovereign `/24`s at `.1`, `.2`, `.4`, `.5`, boundary at `.3` |
| Label | `org.goat.fe2e.lab=fe2e-lN` on every container and network |
| Images | `fe2e-host:<hash of host/>`, `fe2e-republisher:<sha12>` |
| State | `FE2E_HOME` (default `~/.cache/fe2e`): `src/<sha>`, `bin/<sha>`, `runs/`, `labs/` (one lock per lab) |

`env down --lab N` removes the containers and networks carrying that lab's label and nothing else. Containers inside a host (edge router, probes, republisher) live in that host's own dockerd and disappear with it.

The engine reads these names from `FE2E_*` variables (header of `lib/engine.sh`). Their defaults reproduce run.sh's names, so CI runs as before.

## Network profiles

| Profile | Built how | Status |
|---|---|---|
| Listening edge, direct path | the listener's boundary address is reachable on the tunnel port | Built (unit `pair`) |
| Edge behind NAT, no inbound; forwarder path | a NAT router holds the edge's boundary address (outbound masquerade, nothing inbound); the party's forwarder host on the boundary, applied by `goat-peering-forwarder` and `peering_wg_via_forwarder` | Built (unit `forwarder`); the drivers do not apply the forwarder, so the unit runs the operator's three plays |
| UDP 51820 already held on an edge | a process holding the port before phase 1; the peering on a free inner port | Built (unit `preflight`, scenarios `preflight-port-in-use`, `port-51820-held`) |
| WireGuard transport | `goat-peering-wg` | Built |
| IPsec transport | `goat-peering-edge` (strongSwan, XFRM) under a waiver | Built (unit `ipsec`): IKE and the child SA between two edge hosts are measured here; the federation path is BLOCKED on a ref whose init offers `ipsec-fips` only on a FIPS kernel and whose phase 11 cannot apply `goat-peering-edge` |
| Stock Ubuntu 26.04 AppArmor | `FE2E_WG_APPARMOR=host` keeps the machine's `wg` profile in force | Built (unit `apparmor`) |

The machine's AppArmor profile `wg` (shipped by Ubuntu 26.04 LTS in `/etc/apparmor.d/wg`) attaches by path to `/usr/bin/wg` in every container and permits key files only under `/etc/wireguard`. `goat-peering-wg` keeps its key at `/etc/goat-peering-wg/<interface>.key`, so on such a machine the tunnel unit fails with `fopen: Permission denied`. By default the engine copies `wg` to `/usr/local/sbin` inside each host, which runs unconfined, as on a host without that profile. Scenario `ubuntu-2604-apparmor` runs without that copy.

## Units and scenarios

A unit brings sovereigns up once in one lab and records steps; several scenarios read one unit's steps. This keeps the number of bring-ups small: the two-party unit alone feeds six scenarios.

| File | Holds |
|---|---|
| `units/<name>.sh` | `unit_main`, sourced by `lib/unit.sh` after the engine. Records steps with `rec <name> [--expect-exit N] [--expect-in TEXT] -- <command>` |
| `scenarios/catalog.yaml` | per scenario: `id`, `title`, `unit` (or one per avenue), `runs_where`, `pass_condition`, `checks` (each reads one step name), and `not_built: <reason>` until its unit exists |
| `lib/verdict.py` | turns steps into outcome words and writes `results-<avenue>.json` |
| `lib/matrix.py` | one table of ref by scenario from every result file |

The ref under test supplies the driver, roles and scripts. The harness is the checkout `env` sits in, copied into the run directory first, so editing it does not change a run in progress. A unit never changes the product to make a check pass; a missing capability ends as FAIL or BLOCKED with a reason that names it.

## Outcome words and the result schema

Every check and every scenario ends in exactly one of five words. A required check that is BLOCKED or SKIP never counts as passing, and `env run` exits 0 only when every required scenario is PASS or NOT APPLICABLE.

| Word | Means | How a check gets it |
|---|---|---|
| PASS | expected evidence observed | its step ran and met every expectation |
| FAIL | violation observed | its step ran and did not |
| BLOCKED | prerequisite or evidence unavailable | the unit stopped before the step, finished without it, or marked it BLOCKED with a reason |
| SKIP | not exercised | the scenario is not built, or its unit was not run |
| NOT APPLICABLE | excluded by the agreement | the unit marked it, naming the agreement clause |

A scenario takes the worst required outcome in the order FAIL, BLOCKED, SKIP; NOT APPLICABLE only when all its required checks are. The schema is `ops/federation-e2e/result.schema.json` (`fed-e2e-result/1`): the ref name and sha (and for an integration preview the branch heads merged), the avenue, the harness sha, per scenario the pass condition, where it runs, the outcome and its reason, and per check `required`, `outcome`, `reason`, `observer` (`local` here; `peer-reported` is reserved for bilateral evidence) and the step log that is its evidence. A reason is required for every outcome except PASS.

## Where each scenario runs

GitHub runners can run privileged containers with the `wireguard` module, but have four cores and a job time limit. The split:

- **CI** (`.github/workflows/federation-e2e.yml`): `ops/ci/check-federation-e2e-harness.sh` (the catalog and the verdict's outcome rules on fixtures, no Docker), run.sh's prepare-only, and run.sh's scenarios 1 to 3, which record the steps of `two-party-full` (`runs_where: ci`) on both avenues.
- **Local** (`runs_where: local`), on one Linux machine: every other scenario, including the pair unit's own checks (persistent reception, the republisher boundary, out-of-lane, identity, acceptance exit status, reruns), three or four sovereigns, NAT and the forwarder, held ports, killing the driver at each phase, released binaries, IPsec, node ports, and the integration preview. `lib/watch.sh` runs these on every new head. Running the pair unit itself in CI through `env` is designed, not built.

`env list` prints each scenario's place.

## Watching branches

`lib/watch.sh --out DIR [--notes FILE]` measures, each round: `origin/main` (the baseline), the integration branch, a preview that merges every `origin/ws/*` head onto the integration branch, and every remote branch matching its patterns, newest first. A head already measured with the same harness fingerprint (catalog, units, engine) is skipped. A scenario that passes on main and fails, blocks or skips on a branch is a regression: it is marked in `DIR/latest.md` and appended to the notes file. With `--reverts`, each merge on the integration branch is also measured reverted, which scenario `module-revert` compares with the integration head before that merge.

## Real hosts

The same scenarios are designed to run on real machines through `live-host.sh`; that avenue is designed, not built.

The engine reaches a host through four operations, and only these: run a command on a host (`docker exec` today), copy a file to and from a host (`docker cp`), start a host, and remove a host. `live-host.sh` already provides the first three over SSH for the two-host drill (`live.sh`), and its hosts are reset per scenario rather than created. The design:

- a host driver variable, `FE2E_HOSTS=container|ssh`, selecting the implementation of those four operations in `lib/engine.sh`; `ssh` maps a host name to an address and key from a hosts file the operator supplies, and "start" and "remove" become `live-host.sh reset`;
- units unchanged: they name hosts by role (`<id>-core`, `<id>-edge`) and never by address;
- network profiles become preconditions checked before a unit runs (inbound reachability, a held port), not things the harness creates; a profile the hosts cannot provide makes its scenarios BLOCKED with that reason;
- each operator runs only the units for the hosts they hold, and the two sides' result files are exchanged by file, with the partner's checks labeled `observer: peer-reported`.

## Files

| Path | Purpose |
|---|---|
| `ops/federation-e2e/env` | entry point |
| `ops/federation-e2e/run.sh` | CI's two- and three-party run, on the same engine |
| `ops/federation-e2e/lib/engine.sh` | hosts, sovereigns, phases, probes, snapshots |
| `ops/federation-e2e/lib/unit.sh` | runs one unit in one lab |
| `ops/federation-e2e/lib/verdict.py`, `lib/matrix.py`, `lib/watch.sh` | outcomes, the matrix page, the watcher |
| `ops/federation-e2e/units/` | the units |
| `ops/federation-e2e/scenarios/catalog.yaml` | scenarios and checks |
| `ops/federation-e2e/result.schema.json` | `fed-e2e-result/1` |
| `ops/federation-e2e/overlay.py` | one sovereign's declarations in its own clone |
