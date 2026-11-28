# Federation e2e environment

Status: built; not measured as a whole: the full matrix runs from `ops/federation-e2e/env` against `main` (run `8cd5b71721aa-20261114T043329-1599826`, the federation test environment's full matrix on `main` at 8cd5b717: 28 passed, 7 failed, 3 blocked, 2 skipped), and the fast subset runs in `.github/workflows/federation-e2e.yml`, but not every scenario is built (`back-out-one-peering`'s two-peering form and `module-revert` are skipped as not built) — gate: the full scenario matrix runs from `ops/federation-e2e/env` against `main` with every scenario built, and the fast subset runs in `.github/workflows/federation-e2e.yml`.

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
| `goatbuilder-only-both-sides` and `nothing-hand-written`, the one command on a terminal (`units/onecmd.sh`, `units/nohand.sh`) | Built and measured (§The one command, and nothing hand-written) |
| `bilateral-acceptance`, each side's acceptance holding the partner's signed result (`units/bilateral.sh`, gate row B2) | Measured: passed on `main` at 959ebead, run `959ebeadac57-20261119T230055-2154120` (§The partner's half of acceptance) |
| `lane-pacing-burst`, a burst of samples about 1 ms apart on one lane, read at the receiving edge (`units/burst.sh`, `lib/burst.py`): every sample crosses in put order, no closer than 1/hz, and is stored, none is dropped by the edge's downsampling, the sender counts no `queue_full`; optionally, with the sender's queue lowered, what does not fit is counted and warned (ADR 1244) | Built and measured: run `c5658cf1e57c-20261115T135415-1778226` passed on the pacing republisher, paced and overflow (50 crossed in order and stored, none dropped; at queue depth 8, 34 counted as `queue_full` and 16 crossed). The log parsing and the verdict are tested on recorded-shape logs (`burst_checks` in `tests/test_harness.py`) |
| `back-out-one-peering`, one of two peerings on one edge and core backed out while the other carries traffic (`units/backout-one.sh`, gate row B7) | Built and measured (§Backing out one of two peerings) |
| `forwarder-by-goatbuilder`, the forwarder brought up by `goatbuilder federate` from the peering's `forwarder:` declaration (`units/forwarder-gb.sh`) | Measured: run `a522944a1dcc-20261120T064349-3125772`, 20 of 20 checks (§The forwarder applied by the command) |
| `forwarder-two-peerings-by-goatbuilder`, two peerings of one side relayed by one forwarder, both applied by `goatbuilder federate` (`units/forwarder-gb-two.sh`) | Measured: run `a522944a1dcc-20261120T064349-3125772`, 25 of 25 checks (§The forwarder applied by the command) |

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

`--directory-ref REF` (built) runs the peering directory from a saltlick commit instead of the pinned image: env fetches `origin` in the saltlick clone (`FE2E_SALTLICK_REPO`, default the `saltlick` clone beside this goatcore clone), resolves REF, clones that commit into `FE2E_HOME/saltlick/<sha>` and builds `fe2e-saltlick:<sha12>`, which units receive as `FE2E_SALTLICK_IMAGE`. Each unit's `tools.txt` records `FE2E_DIRECTORY_REF` and `FE2E_DIRECTORY_SHA`; the result carries `directory: {ref, sha}`; the files under `--out` are named `<sha12>-saltlick-<dsha12>`; and the ref and commit enter the harness fingerprint, so `lib/matrix.py` never compares such a row with `main` on the pinned image. Without the option the fingerprint is unchanged.

`--goatbuilder-version VER` (built) makes every unit run the published goatbuilder VER instead of the one built from the ref: env downloads `goatbuilder-linux-amd64` from the release `goatbuilder/vVER` into `FE2E_HOME/release/goatbuilder-VER` and uses it only when it matches the release's `.sha256` (`lib/gbrelease.sh`, `release_tool`). A local build of the release's commit is not the same binary, byte for byte, as the one operators install, so a rehearsal of a release runs the asset. The rest of the tree under test still comes from the ref, so the ref must be the release's commit: when its `crates/goatbuilder` version is not VER, the run is refused and every scenario is BLOCKED, naming both versions. Each unit's `tools.txt` records `GOATBUILDER_RELEASE` and `GOATBUILDER_SHA256`; the files under `--out` are named `<sha12>-goatbuilder-<VER>`; and `goatbuilder=release:VER` enters the harness fingerprint, so `lib/matrix.py` never compares such a row with one on a local build. Without the option the fingerprint is unchanged.

## What one sovereign is

Each sovereign is generated fresh for a run and shares nothing with another sovereign except the boundary network. That matches two organizations that never hold each other's keys.

| Piece | How it is made |
|---|---|
| Certificate authority, signing key, operator profile | `gen_sovereign` in `lib/engine.sh`, new per run, P-256 and Ed25519 |
| Its own repository | a clone of the ref under test, with `overlay.py` writing that sovereign's `goatbuilder.yaml`, inventory and group_vars. An edge on the driver mounts phase 8's files from `<work>/pki`; an edge on a machine of its own (`--edge-addr`, the operator-workspace driver) has `goat_zenoh_edge_install_certs` on, so the role installs them, as `goatbuilder federate allocate` renders it |
| Edge host firewall | the edges' parent group `peering_edges` switches the goat-host-firewall baseline on from the first commit (`group_vars/peering_edges/host-firewall.yml`, written by `edge_fw_declare` in `lib/hostfw.sh`, called by `gen_sovereign`), so a tree with no peering yet has no peering's group directory, which `goatbuilder federate allocate` would read as a peering already running (GB-230), with the management sources the lab's sessions come from, including the play's own SSH session at 127.0.0.1, and one reasoned override for the probe sessions' REST ports, those of the drill's own probes and those of the probes a partner's phase 14 starts through `lib/probe-launcher.sh` (`PL_PORT`, the one value the launcher's units read too). A partner's driver host joins this network for phase 14's drill reach at a fixed address, one per partner (`drill_reach_ip`), which the edge admits for SSH and those ports. The ops host is admitted to those ports too: with `FE2E_DRIVER=workspace` it is the driver, whose own probe launcher in a bilateral run starts its sessions on its edge (a driver on the edge reaches them over loopback; `hostfw_checks` in `tests/test_harness.py`). A tree without the role is left as it is. `units/nodesec.sh` turns it back to the role's default to measure that default's refusal (scenario `node-ports-declared`). Built |
| Core host | a privileged container running systemd and its own dockerd: the core router (`<id>-zenoh-router`, label `org.goat.component=zenoh-core`), `spire-server`, and from phase 12 the republisher |
| Edge host | a second privileged container: the edge router, the WireGuard interface, the driver |
| Release producers | samples written under `release/<set>/` on the core, as a producer would write them |
| Peering directory | saltlick behind a TLS front with a local CA, on the boundary network; never the public directory. The pinned saltlick image by default; with `--directory-ref REF` on `env run` or `env up`, the image built from that saltlick commit with its own Dockerfile (built) |

Party ids are the repository's synthetic ids (`xaa`, `xab`, `xac`, `xad`). run.sh's defaults keep the ids it has always used.

## Isolation between runs

Several runs share one Docker daemon, and other work on the machine uses Docker too. Everything the environment creates is therefore named, addressed and labeled by lab, and removal goes by label only.

| | Value for lab N (1 to 14) |
|---|---|
| Containers | `fe2e-lN-<id>-core`, `fe2e-lN-<id>-edge`, `fe2e-lN-directory[-tls]` |
| Networks | `fe2e-lN-<id>` (one per sovereign), `fe2e-lN-boundary` (internal) |
| Subnets | `10.<230+N>.0.0/16`: sovereign `/24`s at `.1`, `.2`, `.4`, `.5`, boundary at `.3` |
| Label | `org.goat.fe2e.lab=fe2e-lN` on every container and network |
| Images | `fe2e-host:<hash of host/>`, `fe2e-republisher:<sha12>`, and `fe2e-saltlick:<sha12>` with `--directory-ref` |
| State | `FE2E_HOME` (default `~/.cache/fe2e`): `src/<sha>`, `bin/<sha>`, `saltlick/<sha>`, `runs/`, `labs/` (one lock per lab). `src/<sha>` and `saltlick/<sha>` are plain clones that hold every object of the commit themselves (`lib/srctree.sh`): a commit no ref holds is fetched into the clone by sha, so a shallow source repository, where git ignores `--shared`, still gives a whole tree. The clones the watcher's preview and reverts merge and revert in, and the one the upgrade unit checks a release tag out in, are plain clones at a ref that own their objects too (`src_clone_ref`), so none loses a commit when the source prunes; a shallow source is refused for them, since its history is cut. Built (`tests/test_harness.py`, check 5) |

`env down --lab N` removes the containers and networks carrying that lab's label and nothing else. Containers inside a host (edge router, probes, republisher) live in that host's own dockerd and disappear with it.

The engine reads these names from `FE2E_*` variables (header of `lib/engine.sh`). Their defaults reproduce run.sh's names, so CI runs as before.

## Network profiles

| Profile | Built how | Status |
|---|---|---|
| Listening edge, direct path | the listener's boundary address is reachable on the tunnel port | Built (unit `pair`) |
| Edge behind NAT, no inbound; forwarder path | a NAT router holds the edge's boundary address (outbound masquerade, nothing inbound); the party's forwarder host on the boundary, applied by `goat-peering-forwarder` and `peering_wg_via_forwarder` | Built (unit `forwarder`). The unit brings the forwarder up by running the operator's three plays by hand, on either avenue. `goatbuilder federate` applies a forwarder from the peering's `forwarder:` declaration in phase 2 (`crates/goatbuilder/src/federate/forwarder.rs`, `splice`; built, unit tests only, [peering-forwarder.md](peering-forwarder.md) §7), but the unit does not use that path yet; `peering-standup.sh` does not apply one |
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

A scenario takes the worst required outcome in the order FAIL, BLOCKED, SKIP; NOT APPLICABLE only when all its required checks are. The schema is `ops/federation-e2e/result.schema.json` (`fed-e2e-result/1`): the ref name and sha (and for an integration preview the branch heads merged), the avenue, the harness sha, per scenario the pass condition, where it runs, the outcome and its reason, and per check `required`, `outcome`, `reason`, `observer` (`local`, or `peer-reported` for a check whose evidence the partner reported by file, as the solo unit's are; a catalog check sets it with `observer:`) and the step log that is its evidence. A reason is required for every outcome except PASS.

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

## The one command, and nothing hand-written

Two units drive `goatbuilder federate <partner>` on a pseudo-terminal and answer its prompts from the partner's side of the lab (`lib/answer.py`): the partner's signing key and CA fingerprint, the agreement and package digests. Every switch stays in its released position.

`goatbuilder-only-both-sides` (`units/onecmd.sh`) federates both sovereigns by the one command alone and lists every step the harness had to take outside it.

`nothing-hand-written` (`units/nohand.sh`) starts one sovereign with no peering at all. Its operator's input is the peering entry in `goatbuilder.yaml` and the offer in `ops/federation/<us>/federation.toml`; every other file and value must come from `goatbuilder federate allocate` and `goatbuilder federate <partner>`. Both cores are deployed by the `goat-zenoh` role and declare only `site_id`. Admission's values (`FED_CORE_*`, `FED_SLOT_CHECK`) are withheld on both sides: the sovereign admits through the plain run, the partner through `federate plan` and `federate deploy`. Once both have passed phase 13, phase 14 runs inside the command on each side.

The unit is strict by default (`FE2E_NH_MODE=strict`): nothing is given back, nothing is admitted by hand, and the first refusal is the result. `FE2E_NH_MODE=loop` keeps the lab up and waits at each refusal for a fixer's build, then resumes that side from the failed phase (`units/nohand-loop.sh`). `survey` gives each missing value back and lists it, to show every gap in one pass.

What a person still had to write is the list `RECORD/hand-written.txt`. It may hold only three reviewed lab-only lines (`nh_lab_only`), and any other line fails the scenario:

| Lab-only line | Why it belongs to the lab |
|---|---|
| `goat_fed_republisher_image` | the lab builds the republisher image locally; a deployment pins its own |
| `peering_wg_fips_waived_reason` | the lab hosts are stock Ubuntu without a FIPS module; a deployment records its own waiver |
| phase 14's counterparty surfaces (`FED_THEIRS_REST`, `FED_SPIRE_THEIRS`, the probe launcher, a network reach into the partner) | only a drill owns both sides; a production operator holds only its own, so phase 14 exits 3 until the partner's half is proven (`federation-acceptance.md`) |

## The partner's half of acceptance

**Measured** (unit `bilateral`, scenario `bilateral-acceptance`, `runs_where: local`): it passed on `main` at 959ebead in run `959ebeadac57-20261119T230055-2154120`, all 7 required checks; its two optional checks, a combined PASS on each side, did not pass, as expected (see below). It is the scenario gate row B2 asks for: each side's acceptance report includes the other side's signed result, with no step outside the command.

The unit runs `nothing-hand-written` first. Then both operators run `goatbuilder federate test <partner> --bilateral` at the same time, each on its own driver, with only the values that run was given in `nothing-hand-written`: no counterparty surface (`FED_THEIRS_REST`, `FED_SSH_THEIRS`, `FED_SPIRE_THEIRS`, `FED_PROBE_LAUNCHER`) and no reach into the partner's network. Each side is given its own probe launcher, `FED_PROBE_LAUNCHER_OURS` (`lib/probe-launcher.sh`): its sessions run on that side's own edge host (locally when the driver is the edge, `FE2E_PL_LOCAL`, else over ssh) with that side's agreed leaf, a forged name and a leaf naming the partner's core from its own CA, and trust the partner's edge by the partner's public CA certificate (`federation-acceptance.md` §6.4). Each side's phase 14 derives the session from the agreement, publishes its signed result on its own core, and verifies the partner's under the key bound at phase 5 (`federation-acceptance.md` §6.3).

The checks read each side's evidence directory afterwards (`/var/lib/goat-federation/<partner>/evidence/acceptance` on the driver):

| Check | Passes when |
|---|---|
| `run` | the run is not refused (exit 2) and says it published its signed result |
| `own-surfaces` | the run's `result.json` records no counterparty surface in `inputs` (`theirs_rest`, `ssh_theirs`, `probe_launcher`, `spire_theirs`) and records the side's own launcher (`probe_launcher_ours`) |
| `holds-signed-result` | the newest combined result under `bilateral/` is `authenticated: true`, its `peer_key` is the key the run's keyring (`keys.txt`) holds for the partner, and the partner's result is kept under `peer/<partner>/` |
| `verdicts-agree` | the two sides' combined verdicts are equal |
| `verdict-pass` | required: the combined verdict is PASS |

A combined PASS is required: each side judges the partner's own probes on its own edge's trace and the partner's staged workload certificate against its own `spire-server` (`federation-acceptance.md` §6.4), so with own surfaces only every row can pass. A tree without `federate test --bilateral` marks every check BLOCKED, and so does a run in which phase 13 did not pass on both sides.

## Backing out one of two peerings

`back-out-one-peering` (`units/backout-one.sh`) peers {A} with {B} and with {C} on one edge host and one core host deployed by the `goat-zenoh` role (`FE2E_CORE=role`), then runs `goatbuilder federate backout {C}` on {A} while a flow runs from {A} to {B}. {A} declares its peering with {C} only when that peering starts (`FE2E_LATER_<sovereign>` and `declare_later` in `lib/engine.sh`), as a deployment adds a second peering: {A}-{B} comes up whole first. Declared from the start, {A}-{C} would have no agreement while {A}-{B} is admitted, and admission's host-allocation check reads SKIP, which deploy refuses; or its tunnel would already be on the shared edge when {A}-{B}'s phase 11 loads the host firewall baseline, which refuses to drop a tunnel it has not recorded (GB-210). {A}'s core admits each peering through goatbuilder's own phase 10, after phase 6 has compared the agreement digest (`--agreement-digest`, read from the partner's copy); the lab's own route to a role core is withheld from the back-out, as in `nothing-hand-written`. The goat-zenoh play pins the core's REST surface to 127.0.0.1, so after goatbuilder's role runs the unit reads {A}'s REST surface through a relay on the core host; it is lab-only and changes nothing in the deployment.

Measured twice: run `8c6c054c2841-20261115T014642-2624675`, goatbuilder at main 8c6c054c with the unit before its merge, and run `23b8b4bef548-20261115T022840-576158`, on 23b8b4be, the head of #812, whose federation-e2e, federate and goat-zenoh role files are identical on main dd549fb5: 88 of 89 checks pass in each. Every resource of {A}-{C} is removed, {A}-{B} is unchanged and passes strict acceptance from both sides before and after. The flow across the back-out lost 2 of 132 values (longest gap 1.0 s) in the first run and 6 of 165 (3.1 s) in the second, each loss at one of the two restarts of the shared core router; that check is recorded, not required (`known_failure` in the catalog, #811).

## The forwarder applied by the command

`forwarder-by-goatbuilder` (`units/forwarder-gb.sh`) is `nothing-hand-written` with {B} behind a NAT router that forwards nothing inbound, listening through a peering forwarder it operates, on the network of `units/forwarder.sh`. {B}'s operator input is `nothing-hand-written`'s plus one part: `forwarder:` on its peering entry, with its `endpoint` and `ansible_host` (`NH_FORWARDER_<side>` in `units/nohand.sh`) and the forwarder machine's own values, `ansible_user`, `public_interface` and `ssh_sources`: the user it is reached as, its public interface, and the address it admits SSH from (`nh_operator_more`). No group_vars file is written by hand: allocate renders those values into the forwarder group's `peering-forwarder.yml`, and the scenario checks that file holds them and is the group's only file. Its offer's MTU is 1300 (`NH_MTU_<side>`), the carrier's. The unit runs no play. `goatbuilder federate allocate` renders the forwarder's inventory group, its group_vars and the offer of its public port, and phase 2 of `goatbuilder federate {A}` on {B} runs the forwarder plays ([peering-forwarder.md](peering-forwarder.md) §7). The checks are `forwarder-path`'s, read the same way, plus what allocate rendered and the phase 2 transcript. Measured with the machine values in a hand-written group_vars file (`forwarder-host.yml`): run `ca6744e801b1-20261115T184540-472531`, 19 of 19 checks passed. Measured with the values declared on the peering entry: run `a522944a1dcc-20261120T064349-3125772`, 20 of 20 checks passed.

It relays one peering. `forwarder-two-peerings-by-goatbuilder` (`units/forwarder-gb-two.sh`) is §7's full gate: a third sovereign, and {B}'s second peering, with {C}, added beside the first (`NH_ADD_PEERING=1` in `nh_side_from_nothing`: the entry is appended and the environment's offer releases to the new partner too; `NH_STEP` and `OC_PREFIX` give the second pair's steps the prefix `nh2`). Each pair is driven as `nothing-hand-written` drives one. After both are up, the checks read both: allocate lists both peerings for the forwarder, the second peering's phase 2 runs the forwarder play, each carrier handshakes, each partner's endpoint is its own public port on the forwarder, each tunnel carries both ways and a value crosses it each way, the forwarder's configuration is exactly the two carriers' and holds no party's key, and both acceptances pass. Because adding the second peering makes allocate re-render the first's files, the first peering's agreement digest on both sides is compared with the one from before, and its acceptance runs again. Measured: run `a522944a1dcc-20261120T064349-3125772`, 25 of 25 checks passed.

## Solo party (an outside partner)

**Built** (unit `solo`, scenario `solo-external-partner`, `runs_where: local`). One lab sovereign federates with a partner that is not in the lab, so that an operator's laptop can drive the partner's side against an environment the lab never touches. `FE2E_PARTNER=external` (`lib/engine.sh`) makes `setup_hosts` create only the lab sovereign's core and edge, with no peering directory: phase 5 runs by hand (`FED_NO_DIRECTORY=1`). Nothing in the lab logs in to the partner's hosts. Only the WireGuard tunnel crosses, with SPIRE's bundle forward and the samples inside it. Everything else crosses as files through two folders that the operator carries.

How to run it:

```bash
FE2E_PARTNER=external FE2E_PARTNER_FACTS=$PWD/facts.txt \
FE2E_EXCHANGE_OUT=$PWD/out FE2E_EXCHANGE_IN=$PWD/in FE2E_SOLO_PUBLIC_ADDR=203.0.113.7 \
  ops/federation-e2e/env run --ref <sha> --avenue goatbuilder --scenario solo-external-partner \
    --jobs 1 --out <dir> --goat-version xae=0.5.19 --parties "xae xab"
```

`--parties "<lab id> <partner id>"` names the lab sovereign (A) and the partner (B). The core is deployed by the goat-zenoh role. The lab edge's public side:

| Partner edge | Lab edge | Set |
|---|---|---|
| listens (`listens=true`) | dials, out through the docker bridge's NAT; listener=false | `FE2E_SOLO_PUBLIC_ADDR`: the address the partner's edge sees our packets from. Its tunnel firewall drops every other source |
| cannot listen (`listens=false`) | listens on a UDP port the machine publishes (`docker -p` on the edge host) | `FE2E_SOLO_PUBLISH=<host port>` and `FE2E_SOLO_PUBLIC_ADDR` (the address the partner dials). Built, not yet run lab to lab |

**The facts file** (`FE2E_PARTNER_FACTS`) holds `key=value` lines about the partner. `#` starts a comment.

```
sovereign_id=xab
edge_address=198.51.100.20      # the partner edge's public address
edge_port=51823                 # its inner WireGuard UDP port
listens=true
release=release/all/** release/xae/**
```

**What the laptop does.** Each pause writes `out/<step>.waiting`, which says what the pause waits for. The pause then waits for a file in `in/`, for `FE2E_SOLO_STEP_TIMEOUT` seconds (default 6 hours). After that the unit stops, and every later check reads BLOCKED, naming the step. The unit refuses to start if `in/` already holds answers from an earlier run.

| Step | The lab writes to `out/` | The laptop puts in `in/` |
|---|---|---|
| phase 5 (`exchange`) | `xae.canonical.json`, `xae-ca.pem`, `xae.spiffe-bundle`, and `xae-voice-call.txt` (`signing_key=`, `ca_fingerprint=`) | the partner's `xab.canonical.json`, `xab-ca.pem`, `xab.spiffe-bundle`. The package must match the facts file |
| voice call (`voice-call`) | none; read `xae-voice-call.txt` aloud | `voice-call.ok`: `signing_key=<44 base64>` and `ca_fingerprint=sha256:<hex>`, both the partner's and as read aloud. The fingerprint must match `xab-ca.pem` |
| after compose (`digest`) | `xae-agreement-digest.txt` | `digest.ok`, holding the partner's digest. If it differs from ours, the unit stops and reports the mismatch |
| phase 10 (`admit`) | none | `admit.ok` (may be empty): go ahead and admit the partner's edge on our core |
| phase 11 (`tunnel`) | none | `tunnel.ok`, once the partner's phase 10 is done |
| phase 14 (`accept`) | `acceptance-xae.json` (our result) | `accept.ok`, once the partner's phase 13 is done |
| persistent reception (`persist`) | none | `persist.ok`, once both sides have run acceptance. Optionally `acceptance-xab.json`, which is recorded as peer-reported |
| samples, round 1 | `xae-sample-1.txt` (`key=from/xae/all/<nonce>`, `value=<nonce>`), then `xae-received-1.txt` (what our store read of the partner's sample) | `xab-sample-1.txt`: a value the partner staged under `release/all/` on its core, named by its wire key (`key=from/xab/all/<k>`, `value=<v>`). Then `xab-received-1.txt` (`key=`, `read=`, `result=stored`): what the partner's store read of our sample |
| core restart | `xae-core-restarted.txt`, after our core restarts (our edge does not) | `xab-core-restarted.txt`, after the partner restarts its receiving core |
| samples, round 2 | `xae-sample-2.txt`, `xae-received-2.txt` | `xab-sample-2.txt`, after reading `xae-core-restarted.txt`; then `xab-received-2.txt` |

The checks on the lab side are local observations. What the partner reports reading of our samples, and the partner's acceptance, are recorded with `observer: peer-reported`, never as local observations.

**Strict acceptance from one side.** Phase 14 runs locally, with only the lab's own surfaces: its core's REST, its edge's ACL trace and its SPIRE server. At the ref measured (28492160), `acceptance.sh` cannot complete from one side. Each delivery, persistence and refusal check, and the SPIRE check, needs a surface that only the partner holds, and the exchange of the partner's half is not built (`docs/design/federation-acceptance.md` §3). So the unit records the counts. When every check that did not run is waiting on a partner-held surface, it marks `acceptance-strict-local` BLOCKED and names that reason. When a check failed, it is FAIL. The file-exchanged samples above are the lab's evidence that delivery works in both directions.

**Validated lab to lab** with `ops/federation-e2e/tests/solo-pair.sh --ref <sha> --dir <scratch>`. Two solo units, `xae` and `xaf`, each run in its own lab and each is the other's outside partner. Their folders are crossed, so one side's `out/` is the other's `in/`. The script answers each pause the way the laptop would. Both edges' public sides sit on one docker network that the script creates and removes (198.51.100.0/24), standing in for the internet: `xae` dials and `xaf` listens.

**Measured** on 2026-11-10 at goatbuilder 28492160 with released goat 0.5.19. Each side passed 24 of 26 checks: every phase, the facts, the voice call, an equal agreement digest, the role admit, the handshake, phase 13, the tunnel carrying traffic both ways, and persistent reception in both directions, including after each receiving core restarted. The partner's half of the persistent-reception checks is peer-reported. Each scenario comes out BLOCKED on `acceptance-strict-local`. Both sides printed `acceptance-counts: pass=2 fail=0 skip=1 blocked=14`, because each blocked check needs a partner-held surface. `partner-acceptance` (optional, peer-reported) is BLOCKED for the same reason. Strict acceptance with `fail=0 skip=0` from each side alone needs the bilateral exchange of acceptance halves, which the product does not have.

## A joining site's plan against its declared set (gate row I3)

**Built** (unit `join-site`, scenario `join-site-plan`, `runs_where: local`). The unit starts no host and no container. It copies the tree under test's fixture `ops/goatbuilder/fixtures/join-node-third-site`, with the tree's real end-state model, the gates `join-node`'s dry run calls and the plays whose presence it reads. It models the single-host joining site `gamma` with `endstate.py model`, then prints `goatbuilder join-node --dry-run` for its core node `gamma-core-1` with stand-in secret values in the environment, which the plan must not print. `lib/joinsite.py` then judges the plan against the model. Every service the model marks present on the node must be brought up by a command of the plan that runs its play on that node, or named under the plan's "does not run yet" heading, never both and never neither. Which play brings each service up is the harness's own table (`PLAYS`), kept apart from `join-node`'s, so the plan is judged against a second account of the same facts. Tested without Docker in `tests/test_harness.py` (`joinsite_checks`). On `main` at 7fa42e94 the judgement fails: Prometheus, Alertmanager, Grafana and the obs exporters are declared on `gamma-core-1`, not built and not named. With #953's naming it passes. A join of two sites on hosts (a founder stood up, a joiner added with `join-node`, both sites graded) is designed, not built.

## A founder and a joining site on two hosts (gate row I3)

**Built for the dry runs; the runs on hosts are designed, not built** (unit `join-site-hosts`, scenario `join-site-two-hosts`, `runs_where: local`). `lib/joinsite_lab.py layout` turns the tree's fixture `join-node-third-site` into a goatnet of two sites on one site network (`$NB.6.0/24`; the federation units use `$NB.1` to `$NB.5`): the founder `alpha` and the single-host joiner `gamma`. beta and the phone site delta are taken out. alpha declares signal federation off with its reason, as a founder with no second federating site does. alpha's node is named `mgmt-alpha-01`, because `standup` limits several of the founder's plays to `mgmt-<site>-01*`; under the fixture's name `alpha-core-1` those plays would match no host. The unit prints `standup`'s plan for alpha and `join-node`'s for `gamma-core-1` with `--dry-run` against that inventory, with stand-in secret values that neither plan may print. `lib/joinsite_lab.py reach` then fails a plan any of whose commands names the fixture's documentation addresses, or an ssh target or `-l` limit that is not one of the lab's names or addresses. The joiner's plan is judged against its declared set as in `join-site-plan` (`lib/joinsite.py`). Tested without Docker in `tests/test_harness.py` (`joinsite_lab_checks`).

The checks for the runs on hosts are marked BLOCKED: both hosts started from the fed-e2e host image, `standup` phases 1 to 14 at the founder and `join-node` phases 1 to 15 at the joiner with no step outside the commands, the end state graded at both sites, and the joiner's declared set equal to its deployed one but for the catalog's named gaps.

## Files

| Path | Purpose |
|---|---|
| `ops/federation-e2e/env` | entry point |
| `ops/federation-e2e/run.sh` | CI's two- and three-party run, on the same engine |
| `ops/federation-e2e/lib/engine.sh` | hosts, sovereigns, phases, probes, snapshots |
| `ops/federation-e2e/lib/hostfw.sh` | the goat-host-firewall baseline's values for a sovereign's edge and core |
| `ops/federation-e2e/lib/unit.sh` | runs one unit in one lab |
| `ops/federation-e2e/lib/verdict.py`, `lib/matrix.py`, `lib/watch.sh` | outcomes, the matrix page, the watcher |
| `ops/federation-e2e/units/` | the units |
| `ops/federation-e2e/tests/solo-pair.sh` | the solo unit validated lab to lab: two solo units with crossed exchange folders |
| `ops/federation-e2e/scenarios/catalog.yaml` | scenarios and checks |
| `ops/federation-e2e/result.schema.json` | `fed-e2e-result/1` |
| `ops/federation-e2e/overlay.py` | one sovereign's declarations in its own clone |
