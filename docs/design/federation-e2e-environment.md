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
| `goatbuilder-only-both-sides` and `nothing-hand-written`, the one command on a terminal (`units/onecmd.sh`, `units/nohand.sh`) | Built and measured (§The one command, and nothing hand-written) |

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

## What one sovereign is

Each sovereign is generated fresh for a run and shares nothing with another sovereign except the boundary network. That matches two organizations that never hold each other's keys.

| Piece | How it is made |
|---|---|
| Certificate authority, signing key, operator profile | `gen_sovereign` in `lib/engine.sh`, new per run, P-256 and Ed25519 |
| Its own repository | a clone of the ref under test, with `overlay.py` writing that sovereign's `goatbuilder.yaml`, inventory and group_vars |
| Edge host firewall | the edges' parent group `peering_edges` switches the goat-host-firewall baseline on from the first commit (`group_vars/peering_edges/host-firewall.yml`, written by `edge_fw_declare` in `lib/hostfw.sh`, called by `gen_sovereign`), so a tree with no peering yet has no peering's group directory, which `goatbuilder federate allocate` would read as a peering already running (GB-230), with the management sources the lab's sessions come from, including the play's own SSH session at 127.0.0.1, and one reasoned override for the probe sessions' REST ports. A tree without the role is left as it is. `units/nodesec.sh` turns it back to the role's default to measure that default's refusal (scenario `node-ports-declared`). Built |
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
