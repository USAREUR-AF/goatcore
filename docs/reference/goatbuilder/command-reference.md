# goatbuilder — command reference

_Generated from the `goatbuilder` command tree — always in sync with what the binary does. Regenerate with `goatbuilder docs --markdown`._

## `goatbuilder`

Build and ship a goat environment: declare, converge, verify — under one deploy contract

**Options**

| Flag | Description | Default |
|---|---|---|
| `--output, -o <OUTPUT>` | Output format: a table for a person; JSON or YAML for a program | `table` |
| `--repo, -C <REPO>` | The repository root. Defaults to the checkout this binary is run in |  |
| `--yes` | Confirm a declarative verb without prompting (for automation) |  |
| `--frozen` | Refuse any run that would need to fast-forward the checkout (strict C3) |  |
| `--offline` | An islanded site: do not fetch the trunk; judge the checkout against the cached one, and record the run as offline (C3; ops/scripts/fresh-source.sh) |  |
| `--mock <MOCK>` | Run against a fixture directory instead of hosts: no registry, no transport, deterministic (goatbuilder-conformance.md §"The fixture") |  |
| `--lang <LANG>` | Language for help and error explanations (a tag such as fr, uk, de); output a pipeline reads stays English. Default: the OS locale |  |
| `--color <COLOR>` | Color the table's ok/FAIL glyph and verdict (auto\|always\|never). `auto` colors only a terminal; NO_COLOR and CLICOLOR_FORCE override. JSON and YAML are never colored | `auto` |
| `--verbose, -v` | Log what a run does — the external commands it shells out to, and the network phase of `update` — to stderr, so `-o json\|yaml` stays a clean document. Or set RUST_LOG=goatbuilder=debug |  |
| `--timeout <TIMEOUT>` | Seconds to bound the connect/reach phase before failing fast — the one phase that can block on an unreachable host (`update`'s fetch of the served version). Does NOT cap a play's own run or the binary download (stall-bounded). Also reads GOATBUILDER_TIMEOUT |  |

**Subcommands**

- [`goatbuilder validate`](#goatbuilder-validate) — Check a config against the versioned schema and the cross-field rules, offline
- [`goatbuilder doctor`](#goatbuilder-doctor) — Probe what a run needs before anything is cut: deps, checkout freshness, each environment's transport, and that every component's playbook resolves to a definite target mode
- [`goatbuilder status`](#goatbuilder-status) — Declared against recorded, per machine: converged, pending or unknown (C7, C8). Reads only. Exit 3 while any machine is not converged
- [`goatbuilder deploy`](#goatbuilder-deploy) — Put the pin main declares onto one environment and prove it: resolve the component and the environment's groups from the pins, the play and its target mode, every secret the play templates, the transport; run the play through ansible-preflight; read the deploy ledger back from every host and call it done only when the newest row names the declared digest with action=deploy (C7). On a fixture (`--mock`) the scripted converger stands in for the hosts
- [`goatbuilder preflight`](#goatbuilder-preflight) — Before a rollout, the readiness and reversibility readout (ADR 1211): three facts — State (stateless \| stateful), Rollback (pin-revert \| restore-required \| none) and Scope (single \| coupled) — declared per component and FAIL-CLOSED when undeclared, with the readiness half the other verbs prove (ref pullable, pins --check, the secrets gate, fresh source, the declared end state) and a validation of the rollback fact (rollback --dry-run for pin-revert, the retention window for restore-required). `deploy` runs it first; honesty-graded, never a false green. Reads only; exit 3 when not clean
- [`goatbuilder release`](#goatbuilder-release) — Take a release candidate from a tag on main through the dev goatnet's live validation to the preprod rung (ADR 1173); after the soak, with `--from prod`, graduate it, fold it into fleet:, deploy prod and re-verify every rung. Replaces ops/scripts/rc.sh step for step
- [`goatbuilder rollback`](#goatbuilder-rollback) — Roll a deployed component back to its N-th most recent ref, by artifact type, reading the on-host ledger as the source of truth (ADR 1060). Replaces ops/scripts/rollback.sh: exit 3 refused unsafe (data, cert, forward-only, reversible:false), 4 guided (config, acl)
- [`goatbuilder fastlane`](#goatbuilder-fastlane) — One command from a merged commit to every goatnet (ADR 1127): the deliberate exception to the ladder. Replaces ops/scripts/fastlane.sh step for step; refuses an unpinned digest or an open graduation set, and runs the deploys through this binary
- [`goatbuilder standup`](#goatbuilder-standup) — A site from nothing to verified: the standup's phases in the doctrine's order with a fail-closed gate at each end, the observability host as a parallel leg, the rescue classes and the cross-goatnet surfaces last (ADR 1183). Replaces ops/scripts/goat-standup.sh step for step. On a fixture (`--mock`) every component of every class is converged by the scripted converger and the declared → verified time is the one-hour goatnet's measure
- [`goatbuilder teardown`](#goatbuilder-teardown) — A site from verified to nothing — standup's mirror, records kept (a burn-down appends teardown rows, never erases). On a fixture (`--mock`) it rehearses the burn-down in reverse phase order; on real hosts it prints the order and refuses to execute until each phase declares its teardown play (docs/design/goatbuilder-speed.md)
- [`goatbuilder join-node`](#goatbuilder-join-node) — One more machine of a class at a live site, declared → converged → verified: the node-addition sequence of docs/tutorials/02-first-federated-node.md as numbered phases with its gates (reach, key, repo work first, baseline, kernel WireGuard before and after enrolment, the kernel ACL plane as the enrolment gate, the class's converge, acceptance)
- [`goatbuilder federate`](#goatbuilder-federate) — A sovereign peering declared, converged and verified (ADR 1157): our half measured, the packages exchanged through the peering directory, one agreement composed and verified, the leaves minted from it, the rules rendered by the one writer, the edge deployed, the transport the agreement chose opened, acceptance in all four tiers. Replaces ops/scripts/peering-standup.sh step for step
- [`goatbuilder pins`](#goatbuilder-pins) — ONE resolution pass with TWO renderings: every listed member's group_vars pins.yml and every listed goatnet's IntentV1 artifact, from ops/pins/fleet-pins.yaml. Replaces ops/pins/generate-pins.py byte for byte. `--check` validates, regenerates and refuses drift, an unsigned or tampered intent, and a governed pin defined outside pins.yml (exit 1)
- [`goatbuilder promote`](#goatbuilder-promote) — Guide and observe release graduation across the goatnet ladder (ADR 1120, 1125): status, open a candidate, graduate, graduate a set, the hotfix path, review, readiness, abandon, check. Replaces ops/release/graduate.py step for step. Nothing here graduates by itself: a mutation runs only when invoked, prints what it did, and asks for --yes without a terminal (C10, ADR 1177)
- [`goatbuilder explain`](#goatbuilder-explain) — Interpret an error: a code (`GB-007`, `X-SSH-001`, `E_OFF_MESH`), or any error text — this binary's own, or a tool's words it carried (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Prints what happened, why, what to do and where to read more; `-` reads the text from stdin
- [`goatbuilder tutor`](#goatbuilder-tutor) — Lessons, vimtutor-style: each names the exact command to run against a throwaway copy of a fixture (or, read-only, against this checkout), then checks what that command actually did — its exit and its rows — before the next unlocks. `tutor` shows the lesson you are on and marks it done when your runs satisfy it; `tutor <n>`, `--list`, `--reset`
- [`goatbuilder secrets`](#goatbuilder-secrets) — The declared secrets posture (ADR 1204): which store is in force (1password, openbao, sops, git-crypt, env-file, env), what is available, how to supply what is missing, and the store's own checks. `init --env <goatnet>` writes the zero-SaaS quickstart's env file: a strong random value for every variable the goatnet's plays template, 0600, gitignored, never rotating a value already there. Same module as ops/scripts/with-secrets
- [`goatbuilder verify`](#goatbuilder-verify) — Read every host's newest deploy-ledger row for one component and say whether it names the ref main declares (C7). Reads only; exit 3 while any host is pending or unknown. The same check `deploy --verify-only` runs
- [`goatbuilder diff`](#goatbuilder-diff) — What is different from main right now (C8): main's declared ref against each host's newest ledger row, for one component or, without one, every component of the environment, read in one pass per member group. Reads only; exit 3 on any drift or unreadable host
- [`goatbuilder runs`](#goatbuilder-runs) — The runs this machine recorded, newest first: one row per run, with its id, verb, exit code, number of commands and arguments. Each run of a verb that drives hosts leaves a record at <state>/runs/<id>/run.json, kept until you remove it. Reads only
- [`goatbuilder show`](#goatbuilder-show) — One recorded run, phase by phase and command by command, each with its exit code and log, then its inputs with their sources. `-o json` prints the record itself. Reads only
- [`goatbuilder history`](#goatbuilder-history) — The deploy ledger as a timeline, newest first, on every host of the environment (C8): one row per record, with the machine, the time, the action (deploy or rollback) and the version. Each record's prior version and the operator who ran it are in `-o json`. Reads only
- [`goatbuilder plan`](#goatbuilder-plan) — Write what a declarative verb would do to a plan file for review: the declared refs, the play and target, the ledger, main's commit, the transport (deploy); the artifact type and the target ref (rollback). Touches no host's state. Run it as reviewed with `apply`
- [`goatbuilder apply`](#goatbuilder-apply) — Run exactly a reviewed plan. Refuses (exit 1, naming what moved) when main's declaration, the pins commit, the play or the rollback target moved since the plan was written; a changed transport is reported, not refused
- [`goatbuilder adopt`](#goatbuilder-adopt) — Declare what an environment already runs: read every host's ledgers and propose a pins deviation for each component whose running ref differs from main's declaration. A report by default; with --yes the edits are written into a fresh worktree of main for review. Never deploys
- [`goatbuilder assay`](#goatbuilder-assay) — Check that a goatnet was built correctly, layer by layer: every acceptance check that applies to the site, including the checks an operator runs with the goat CLI. The standup's acceptance phase as its own command. Reads only; exit 3 while any check fails or cannot answer
- [`goatbuilder endstate`](#goatbuilder-endstate) — Check that a goatnet is in its declared end state: every service its own declarations say it runs is deployed, running, healthy and doing its job, with its zero-trust edges wired, and nothing runs that it does not declare. Reads only; exit 3 while anything is off
- [`goatbuilder survey`](#goatbuilder-survey) — How consistent the software is inside each goatnet and across the ladder's goatnets, read in one parallel pass. Reads only
- [`goatbuilder update`](#goatbuilder-update) — Replace this binary with the release its goatnet serves at `get.<site>.<zone>/goatbuilder/`: the download is checked against its `.sha256` and the signed `SHA256SUMS` before anything is touched, and the swap is atomic. Already current: one row, exit 0. `--check` reads only (exit 3 when behind). The surface: `GOATBUILDER_UPDATE_URL`, else `--site` from its group_vars, else the one the last update used
- [`goatbuilder completion`](#goatbuilder-completion) — Print a shell completion script to stdout (`goatbuilder completion zsh >> ~/.zshrc`). For live candidates a static script cannot know — the sites, components and goatnets this checkout declares — wire a hook to `goatbuilder _complete` (see the generated script's header)
- [`goatbuilder docs`](#goatbuilder-docs) — The command reference: `goatbuilder docs` points at it; `--markdown` emits the whole thing, walked from the live command tree so it can never drift from `--help`

## `goatbuilder validate`

Check a config against the versioned schema and the cross-field rules, offline

**Arguments**

| Argument | Description |
|---|---|
| `<CONFIG>` | The config file. Defaults to ops/goatbuilder/goatbuilder.yaml under the repo, or goatlite.yaml at its root when the checkout has no config of its own |

## `goatbuilder doctor`

Probe what a run needs before anything is cut: deps, checkout freshness, each environment's transport, and that every component's playbook resolves to a definite target mode

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | Limit the environment checks to one environment |  |
| `--site <SITE>` | Also scan every input a standup of this site will reach (role asserts, `\| mandatory`, the secrets its plays template) and list each one that is missing, all at once, before any host is touched |  |
| `--all` | Print every check, not only the problems. `-o json` always has them all |  |
| `--credentials` | Check the stored credentials as well: the AWS profile, and whether a 1Password session is open. Either can prompt you (a Keychain or biometric prompt, or a wait on a passphrase), so a plain run reads only what is declared on this machine and prompts for nothing |  |
| `--browser-tests` | Require the browser tests' toolchain (the console's Playwright suite the release's live rows run). Without this flag a missing toolchain is a note, not a failure |  |

## `goatbuilder status`

Declared against recorded, per machine: converged, pending or unknown (C7, C8). Reads only. Exit 3 while any machine is not converged

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | The environment to read |  |
| `--component <COMPONENT>` | One component; default every component declared for the environment |  |

## `goatbuilder deploy`

Put the pin main declares onto one environment and prove it: resolve the component and the environment's groups from the pins, the play and its target mode, every secret the play templates, the transport; run the play through ansible-preflight; read the deploy ledger back from every host and call it done only when the newest row names the declared digest with action=deploy (C7). On a fixture (`--mock`) the scripted converger stands in for the hosts

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | The environment (the pins' goatnet key) to converge |  |
| `--component <COMPONENT>` | One component. Required on real hosts; on a fixture, default every component declared for the environment |  |
| `--ack <ACK>` | Who is declaring (C4: every declaration and record carries the actor) | `` |
| `--group <GROUP>` | One member group of the environment instead of every member |  |
| `--dry-run` | Decide everything and print it; touch no host |  |
| `--verify-only` | Skip the play; only read the ledgers back |  |

## `goatbuilder preflight`

Before a rollout, the readiness and reversibility readout (ADR 1211): three facts — State (stateless | stateful), Rollback (pin-revert | restore-required | none) and Scope (single | coupled) — declared per component and FAIL-CLOSED when undeclared, with the readiness half the other verbs prove (ref pullable, pins --check, the secrets gate, fresh source, the declared end state) and a validation of the rollback fact (rollback --dry-run for pin-revert, the retention window for restore-required). `deploy` runs it first; honesty-graded, never a false green. Reads only; exit 3 when not clean

**Arguments**

| Argument | Description |
|---|---|
| `<COMPONENT>` | The component (a fleet-pins key) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | The environment (the pins' goatnet key) |  |
| `--site <SITE>` | On real hosts: the `site:` whose ledger the rollback validation reads (as `ops/ansible/group_vars` names it), so one goatnet's reversibility is checked and no other |  |

## `goatbuilder release`

Take a release candidate from a tag on main through the dev goatnet's live validation to the preprod rung (ADR 1173); after the soak, with `--from prod`, graduate it, fold it into fleet:, deploy prod and re-verify every rung. Replaces ops/scripts/rc.sh step for step

**Arguments**

| Argument | Description |
|---|---|
| `<COMPONENT>` | The component (a fleet-pins key) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--rc <RC>` | The rc number; default the next free one on origin |  |
| `--ack <ACK>` | Who nominates (required: a candidate reaches preprod and is attributed) | `` |
| `--from <FROM>` | Resume at a step: tag \| dev \| rows \| preprod \| preprod-rows \| prod (preprod-rows: preprod already runs the tag; redo only its deploy check, live rows and evidence, without reopening the record or re-pinning) | `tag` |
| `--tag <TAG>` | An existing tag to validate as-is instead of cutting one |  |
| `--dry-run` | Print every command; mutate nothing |  |

## `goatbuilder rollback`

Roll a deployed component back to its N-th most recent ref, by artifact type, reading the on-host ledger as the source of truth (ADR 1060). Replaces ops/scripts/rollback.sh: exit 3 refused unsafe (data, cert, forward-only, reversible:false), 4 guided (config, acl)

**Arguments**

| Argument | Description |
|---|---|
| `<COMPONENT>` | The component (an Ansible role with a deploy-history.yml) |
| `<N>` | Steps back (1..7) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--list` | Show the deploy history, newest first (host ledger if reachable, else in-repo) |  |
| `--status` | Show the running ref and the artifact type per host |  |
| `--dry-run` | Print the remote shell each host would run; touch nothing |  |
| `--env <ENV>` | The environment, on a fixture (`--mock`): the rollback declares the earlier ref there and converges it (C9). Real hosts read it from the ledger |  |
| `--site <SITE>` | On real hosts: only the `hosts:` entries of deploy-history.yml with this `site:`, so one goatnet is rolled back and no other |  |
| `--ack <ACK>` | Who is declaring, on a fixture (C4: the declaration and every record carry the actor) | `` |

## `goatbuilder fastlane`

One command from a merged commit to every goatnet (ADR 1127): the deliberate exception to the ladder. Replaces ops/scripts/fastlane.sh step for step; refuses an unpinned digest or an open graduation set, and runs the deploys through this binary

**Arguments**

| Argument | Description |
|---|---|
| `<COMPONENT>` | The component (a fleet-pins key) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--ref <REF>` | The sha-tag or full ref CI built from a merged commit | `` |
| `--reason <REASON>` | Why this cannot wait for the ladder (recorded) | `` |
| `--ack <ACK>` | Who ships it (required: a fastlane reaches production) | `` |
| `--dry-run` | Resolve and report; stop before any mutation |  |
| `--no-deploy` | Land the pin only; deploy per goatnet afterwards |  |

## `goatbuilder standup`

A site from nothing to verified: the standup's phases in the doctrine's order with a fail-closed gate at each end, the observability host as a parallel leg, the rescue classes and the cross-goatnet surfaces last (ADR 1183). Replaces ops/scripts/goat-standup.sh step for step. On a fixture (`--mock`) every component of every class is converged by the scripted converger and the declared → verified time is the one-hour goatnet's measure

**Arguments**

| Argument | Description |
|---|---|
| `<ENV>` | The environment (goatnet) the site belongs to |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--site <SITE>` | The site id (the `site_id` in the site's group_vars) |  |
| `--profile <PROFILE>` | Topology profile, lan\|internet; default read from the site's config |  |
| `--ack <ACK>` | Who is declaring (C4) | `` |
| `--from <FROM>` | Resume from this phase, 1..14 (every phase is idempotent) | `1` |
| `--to <TO>` | Stop after this phase, 1..14 | `14` |
| `--skip-obs` | Build no observability host even when the site declares one |  |
| `--obs-only` | Run only the observability legs, in the foreground |  |
| `--dry-run` | Print every phase and command; run nothing |  |
| `--parallel` | Run phase 12's independent steps together, wave by wave, as ops/goatbuilder/standup-dag.yaml declares (the script's --parallel) |  |
| `--check` | Run the acceptance phase (13) on a goatlite, where it is off by default; `goatbuilder assay <site>` runs it on its own. Every other goatnet runs it always |  |

## `goatbuilder teardown`

A site from verified to nothing — standup's mirror, records kept (a burn-down appends teardown rows, never erases). On a fixture (`--mock`) it rehearses the burn-down in reverse phase order; on real hosts it prints the order and refuses to execute until each phase declares its teardown play (docs/design/goatbuilder-speed.md)

**Arguments**

| Argument | Description |
|---|---|
| `<ENV>` | The environment (goatnet) the site belongs to |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--site <SITE>` | The site id |  |
| `--profile <PROFILE>` | Topology profile, lan\|internet; default read from the site's config |  |
| `--dry-run` | Print the burn-down order; run nothing |  |
| `--services` | Stop the services only (compose-down steps, volumes kept) and leave every step that holds data, or is not built, in place |  |
| `--from <FROM>` | First standup phase to take down, 1..14 | `1` |
| `--to <TO>` | Last standup phase to take down, 1..14 | `14` |

## `goatbuilder join-node`

One more machine of a class at a live site, declared → converged → verified: the node-addition sequence of docs/tutorials/02-first-federated-node.md as numbered phases with its gates (reach, key, repo work first, baseline, kernel WireGuard before and after enrolment, the kernel ACL plane as the enrolment gate, the class's converge, acceptance)

**Arguments**

| Argument | Description |
|---|---|
| `<ENV>` | The environment (goatnet) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--site <SITE>` | The site the node joins |  |
| `--node <NODE>` | The node's inventory name |  |
| `--class <CLASS>` | The machine class: core \| obs \| breakglass \| network-jump \| phone |  |
| `--address <ADDRESS>` | The node's LAN address (required to run; phase 1 says how to find it). Not taken with `--via usb` |  |
| `--pass-var <PASS_VAR>` | The env var holding the node's first-login password (default <SITE>_NODE_PASS) |  |
| `--via <VIA>` | How this machine reaches the goatnet's hosts: `mesh` (this machine is on its mesh) or `door` (through its env door; the plan opens it first). `usb`, with `--class phone`: the node over the Android debug bridge until it is enrolled, the goatnet's other hosts over the mesh | `mesh` |
| `--from <FROM>` | Resume from this phase | `1` |
| `--to <TO>` | Stop after this phase | `99` |
| `--dry-run` | Print every phase and command; run nothing |  |

## `goatbuilder federate`

A sovereign peering declared, converged and verified (ADR 1157): our half measured, the packages exchanged through the peering directory, one agreement composed and verified, the leaves minted from it, the rules rendered by the one writer, the edge deployed, the transport the agreement chose opened, acceptance in all four tiers. Replaces ops/scripts/peering-standup.sh step for step.

At a terminal, `goatbuilder federate <partner>` is the whole procedure: it asks for the partner's key and CA fingerprint and the agreement digest, makes the plan and asks to approve it, shows what our core will admit and asks again, then carries on through acceptance. A run that stops resumes where it stopped. Values come from the deployment's declarations; a FED_* variable set in the environment overrides one.

Each stage also runs alone: `plan <partner>`, `deploy <partner>`, `test <partner>`, `status [<partner>]`. `allocate [<peering>]` writes a peering's values and files for review; `backout <partner>` backs one peering out and leaves the others as they were. `goatbuilder federate init|lint|compose|diff|verify|apply` runs one of the six federation verbs in process, with the flags and exit codes of `goat federate <verb>`.

**Arguments**

| Argument | Description |
|---|---|
| `<WITH>` | The peering name (the counterparty, as declared), or a stage run alone: `status [<partner>]`, `deploy <partner>`, `test <partner>`, `allocate [<peering>]` |
| `<PARTNER>` | The partner, after a stage word; after `allocate`, the one peering to allocate and render, beside peerings that are still hand-written |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--context-dir <CONTEXT_DIR>` | The six verbs: the directory holding federation.toml, mtls.cert.pem and ca-roots.pem (default: $GOAT_CONTEXT_DIR, else the goat context directory, found the way goat finds it) |  |
| `--cert <CERT>` | The six verbs: our mTLS certificate (default: mtls.cert.pem in the context directory) |  |
| `--ca-roots <CA_ROOTS>` | The six verbs: the CA roots (default: ca-roots.pem in the context directory) |  |
| `--context <CONTEXT>` | The six verbs: the goat context to read, as `goat --context` (default: $GOAT_CONTEXT, else current_context in the profile's config.toml, else default) |  |
| `--profile-dir <PROFILE_DIR>` | The six verbs: the goat profile directory, as `goat --profile-dir` (default: $GOAT_PROFILE_DIR, else ~/.goat) |  |
| `--over-ssh <OVER_SSH>` | init: measure this peering's edge and core over SSH from this machine, reaching the hosts of inventory groups <PEERING>_edge and <PEERING>_core in ops/ansible/inventory/peering-edges.ini as the plays reach them. Only read-only commands run there; nothing is copied to either host |  |
| `--ssh-edge-group <SSH_EDGE_GROUP>` | init --over-ssh: the edge's inventory group (default <PEERING>_edge) |  |
| `--ssh-core-group <SSH_CORE_GROUP>` | init --over-ssh: the core's inventory group (default <PEERING>_core) |  |
| `--edge-host <EDGE_HOST>` | init --over-ssh: the edge host to measure, when its group has several |  |
| `--core-host <CORE_HOST>` | init --over-ssh: the core host to measure, when its group has several |  |
| `--router-container <ROUTER_CONTAINER>` | init --over-ssh: the core router container, by hand (default: the name the core host's variables give, else the one discovery finds there) |  |
| `--side <SIDE>` | init --over-ssh: measure one half of phase 3 on this machine, for a peering no one machine reaches both hosts of (docs/design/federation-router-discovery.md): `edge` measures only the edge and writes the result to --half-out; `core` measures only the core, joins the edge half read from --edge-half, and builds the package. Without it, both hosts are measured here, as always. On a peering run, `--side edge` is the edge driver's run of a two-driver peering (`drivers: two`): not built yet, and refused with GB-267 (docs/design/federation-two-drivers.md) |  |
| `--half-out <HALF_OUT>` | init --over-ssh --side edge: the file the edge half is written to |  |
| `--edge-half <EDGE_HALF>` | init --over-ssh --side core: the edge half the edge driver wrote |  |
| `--edge-half-sha256 <EDGE_HALF_SHA256>` | init --over-ssh --side core: the edge half's sha256 as the edge driver printed it, read from there and confirmed here (the full digest, or its first 32 hex digits; spaces and dashes are ignored) |  |
| `--expect-commit <EXPECT_COMMIT>` | With --side edge: the commit the edge driver's checkout must be at, as a hand-off object names it. The edge driver's run is not built yet: refused with GB-267 (docs/design/federation-two-drivers.md) |  |
| `--plan <PLAN>` | deploy: the plan to execute (default <work>/plan.json) |  |
| `--approve-plan <APPROVE_PLAN>` | deploy: approve exactly this plan hash, after reading the plan. --yes never approves a plan |  |
| `--confirm` | Phases 5, 6 and 10: confirm the keys, the agreement digest and the admission here, at this terminal, instead of stopping for them by hand. The default when both stdin and stderr are a terminal |  |
| `--no-confirm` | Stop at phases 5 and 10 for the steps by hand even at a terminal, as before confirming at the terminal was the default |  |
| `--partner-key <PARTNER_KEY>` | Phase 5: the partner's verifying key, as read aloud on the call |  |
| `--partner-ca-fingerprint <PARTNER_CA_FINGERPRINT>` | Phase 5: the partner's CA fingerprint, as read aloud on the call |  |
| `--agreement-digest <AGREEMENT_DIGEST>` | Phase 6: the agreement digest, as compared with the partner |  |
| `--place-pointer-package <PLACE_POINTER_PACKAGE>` | Phase 5: place the package the partner's single pointer named and the directory held back, after confirming on the call that it is the package for this peering (its digest, as the stop shows it) |  |
| `--approve-admission <APPROVE_ADMISSION>` | Phase 10: approve exactly this admission digest, after reading what our core will admit. --yes never approves it |  |
| `--review-in-pr` | Phase 10: write what our core would admit for review in a pull request, and stop |  |
| `--env <ENV>` | The environment the peering is declared on (checked against goatbuilder.yaml peerings:) |  |
| `--as <AS_PARTY>` | Which party we are (default: read from the declaration) |  |
| `--edge-group <EDGE_GROUP>` | The ansible group the edge runs in (default <peering>_edge) |  |
| `--work <WORK>` | The working directory (default $FED_WORK, else /var/lib/goat-federation/<peering>) |  |
| `--from <FROM>` | Resume from this phase | `1` |
| `--to <TO>` | Stop after this phase | `99` |
| `--dry-run` | Print every phase and command; run nothing |  |
| `--resume` | Skip the phases the run journal (<work>/state.json) shows passed, whose inputs and local outputs are unchanged now; checks and plays always run again. This is the default; the flag is kept so scripts that pass it keep working |  |
| `--no-resume` | Run every phase in range, even ones the run journal shows passed with unchanged inputs (the behavior before resuming was the default) |  |

**Subcommands**

- [`goatbuilder federate init`](#goatbuilder-federate-init) — Generate our half of a federation, read from the running system
- [`goatbuilder federate lint`](#goatbuilder-federate-lint) — Check one package: is it well-formed, and does it agree with itself?
- [`goatbuilder federate compose`](#goatbuilder-federate-compose) — Combine two packages into one agreement and print its digest
- [`goatbuilder federate diff`](#goatbuilder-federate-diff) — Two digests disagree? Run this. It names the field
- [`goatbuilder federate verify`](#goatbuilder-federate-verify) — Could we actually deliver this agreement? Dry-run it here
- [`goatbuilder federate apply`](#goatbuilder-federate-apply) — Render the access rules and configuration this agreement implies
- [`goatbuilder federate plan`](#goatbuilder-federate-plan) — The bounded plan of one peering before anything changes: versions, agreement digest, rendered-configuration hashes, owned resources, restarts, acceptance policy and phase 1's readiness, written to <work>/plan.json with a stable hash. Reads only; changes no host
- [`goatbuilder federate backout`](#goatbuilder-federate-backout) — Back out ONE peering: survey its hosts, confirm, remove its tunnel, SPIRE federation, edge router, republisher, forwarder mapping and core admission, and leave every other peering and the core as they were
- [`goatbuilder federate import`](#goatbuilder-federate-import) — A two-driver peering's import of an edge result, as a hand-off object names it: checked against the edge driver's confirm code, placed under the working directory and journaled. Built for phase 1; a later phase's import refuses with GB-267 (docs/design/federation-two-drivers.md)

## `goatbuilder federate init`

Generate our half of a federation, read from the running system.

Every capability in the package is MEASURED here, not typed: what transports are installed and configured, what image the router is actually running, what version the fabric is. A capability an operator types is a claim, and a wrong claim stays wrong until the link fails in front of a partner.

Where a value cannot be derived this fails and names it. It never prompts and never fills one in — a silent default is how a link ends up with no rate limit.

Exit 23 means something could not be derived; the list is the output.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--with-transport` | Also derive `transport_offer` — that we are willing to connect |  |
| `--with-data` | Also derive `data_offer` — that we are willing to exchange |  |
| `--signing-key <SIGNING_KEY>` | Our Ed25519 signing key: 64 hex characters (the RFC 8032 seed) or 44 base64 characters. Without it this prints the exact bytes to sign and stops — it will not invent a key |  |
| `--out <OUT>` | Write the package's canonical bytes here |  |
| `--json` | One JSON object instead of the glyph lines |  |
| `--declaration <DECLARATION>` | Read the declaration from this file instead of federation.toml in the context directory: a peering's own federation.toml, which goatbuilder generates from the environment's. Identity is still read from the context directory |  |

## `goatbuilder federate lint`

Check one package: is it well-formed, and does it agree with itself?

Runs the structural rules on a single file — required members, integers that are really integers, no duplicate member names, complete blocks, a signature of the right shape. It does NOT verify the signature: the verifying key is bound to a party over a channel authenticated independently of this link, so a lone package cannot be authenticated by looking at it. `compose` does that.

Reads one file and nothing else. No network, no credentials.

Exit 21 means the package is refused, and the finding names the rule.

**Arguments**

| Argument | Description |
|---|---|
| `<PACKAGE>` | The package to check |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | One JSON object instead of the glyph lines |  |

## `goatbuilder federate compose`

Combine two packages into one agreement and print its digest.

Both parties compose independently and compare the digest. Equal digests prove both sides hold the identical agreement — that is the whole point of the format, and the comparison may travel by any channel at all, including one nobody trusts.

A refusal is an outcome, not a failure: it is the mechanism working. Exit 20, with the rule that fired.

Reads two files and the keys you pass. No network, no credentials, no host access — this runs in an empty container, which is what lets a partner trust it with their package.

**Arguments**

| Argument | Description |
|---|---|
| `<FIRST>` | One party's package |
| `<SECOND>` | The other party's package. Order does not matter: the agreement has no "us" and "them", so both sides compute the same bytes whichever way round they pass the files |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--key <KEYS>` | A verifying key bound to a party, as `<sovereign_id>=<44 base64 characters>`. Repeat for each party |  |
| `--keyring <KEYRING>` | A JSON file of bound keys, `{"<sovereign_id>": "<base64>"}`. Convenient for several partners; it establishes nothing that --key does not |  |
| `--now <NOW>` | The evaluation date, `YYYY-MM-DD`. Defaults to today in UTC — never the local date, which would let two parties either side of local midnight reach different verdicts on the day a package falls due |  |
| `--out <OUT>` | Write the agreement's canonical bytes here — no trailing newline, no byte-order mark, because that is the digest input |  |
| `--print` | Print the agreement's canonical form on stdout |  |
| `--json` | One JSON object instead of the glyph lines |  |

## `goatbuilder federate diff`

Two digests disagree? Run this. It names the field.

This is the verb for the one loud failure this design has. Two parties compose the same pair, compare digests, and they differ — so each side prints its agreement and this diffs them, and the offending field names itself. Loud in the first hour instead of silent for three weeks.

It also answers the case that reads as impossible: the digests differ and the two agreements are identical. There is then no field to show, and the instinct is to re-run everything. The answer is one measurement — a trailing newline, a byte-order mark, or an encoding difference — and this prints which of the three it is.

Exit 22 when they differ, 0 when they match.

**Arguments**

| Argument | Description |
|---|---|
| `<OURS>` | The agreement we composed — the exact bytes we hashed |
| `<THEIRS>` | The agreement they composed |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | One JSON object instead of the glyph lines |  |

## `goatbuilder federate verify`

Could we actually deliver this agreement? Dry-run it here.

Agreeing is not the same as being able to deliver. This runs the agreement through our own configuration generator WITHOUT applying it and asserts the output would be valid on this system, then prints the conformance attestation to publish: one line over the digest, signed with the same key that signed our package.

Reads local state. Changes nothing.

Exit 24 means we could not deliver it, and the failing check says why.

**Arguments**

| Argument | Description |
|---|---|
| `<AGREEMENT>` | The composed agreement, as the bytes that were hashed |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--as <AS_PARTY>` | Which party we are. Read from the declaration file when omitted |  |
| `--signing-key <SIGNING_KEY>` | Sign the attestation with this Ed25519 key — the same key that signed our package (§10). Without it the line is printed unsigned and says so |  |
| `--json` | One JSON object instead of the glyph lines |  |

## `goatbuilder federate apply`

Render the access rules and configuration this agreement implies.

The only verb here that writes. The agreement is the sole input, so the release set we forward and the prefix the far side admits cannot drift apart by being typed twice.

Prints what it would write and stops. Pass --confirm to write.

**Arguments**

| Argument | Description |
|---|---|
| `<AGREEMENT>` | The composed agreement |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--as <AS_PARTY>` | Which party we are. Read from the declaration file when omitted |  |
| `--out <OUT>` | Where to write. Files land under this directory, mirroring the repo paths they belong at |  |
| `--edge-group <EDGE_GROUP>` | The ansible group the edge runs in. The generated group_vars land at `group_vars/<GROUP>/`, and `--limit <GROUP>` is what deploys them |  |
| `--partner-audience <PARTNER_AUDIENCE>` | Release-sets this partner holds an audience slot in, so the edge grants it a READ of `keys/<set>/**` and it can obtain a data key (ADR 1169 decision 3). Repeatable |  |
| `--confirm` | Write. Without this, the rendering is printed and nothing is touched |  |
| `--json` | One JSON object instead of the glyph lines |  |

## `goatbuilder federate plan`

The bounded plan of one peering before anything changes: versions, agreement digest, rendered-configuration hashes, owned resources, restarts, acceptance policy and phase 1's readiness, written to <work>/plan.json with a stable hash. Reads only; changes no host

**Arguments**

| Argument | Description |
|---|---|
| `<WITH>` | The peering name (the counterparty, as declared) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | The environment the peering is declared on |  |
| `--as <AS_PARTY>` | Which party we are |  |
| `--edge-group <EDGE_GROUP>` | The ansible group the edge runs in (default <peering>_edge) |  |
| `--work <WORK>` | The working directory (default $FED_WORK, else /var/lib/goat-federation/<peering>) |  |
| `--no-live` | Skip phase 1's live reads of the hosts; each is reported SKIP |  |
| `--recheck` | Recheck the saved plan instead of writing one: refused when the file was altered, an input changed, or a live prerequisite fails |  |

## `goatbuilder federate backout`

Back out ONE peering: survey its hosts, confirm, remove its tunnel, SPIRE federation, edge router, republisher, forwarder mapping and core admission, and leave every other peering and the core as they were

**Arguments**

| Argument | Description |
|---|---|
| `<PARTNER>` | The partner whose one peering is removed (its sovereign id, as declared) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | The environment the peering is declared on (checked against goatbuilder.yaml peerings:) |  |
| `--as <AS_PARTY>` | Which party we are (default: read from the agreement in the working directory) |  |
| `--edge-group <EDGE_GROUP>` | The ansible group the edge runs in (default <partner>_edge) |  |
| `--work <WORK>` | The working directory (default $FED_WORK, else /var/lib/goat-federation/<partner>) |  |
| `--dry-run` | Survey the peering's hosts read-only and print every resource the back-out would remove, per host; change nothing |  |

## `goatbuilder federate import`

A two-driver peering's import of an edge result, as a hand-off object names it: checked against the edge driver's confirm code, placed under the working directory and journaled. Built for phase 1; a later phase's import refuses with GB-267 (docs/design/federation-two-drivers.md)

**Arguments**

| Argument | Description |
|---|---|
| `<WITH>` | The peering name (the counterparty, as declared) |
| `<PHASE>` | The phase whose edge result this is |
| `<RESULT>` | The result file the edge driver wrote |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--sha256 <SHA256>` | The confirm code the edge driver printed (the result's sha256, or its first digits) |  |
| `--work <WORK>` | The working directory (default `$FED_WORK`, else /var/lib/goat-federation/<peering>) |  |

## `goatbuilder pins`

ONE resolution pass with TWO renderings: every listed member's group_vars pins.yml and every listed goatnet's IntentV1 artifact, from ops/pins/fleet-pins.yaml. Replaces ops/pins/generate-pins.py byte for byte. `--check` validates, regenerates and refuses drift, an unsigned or tampered intent, and a governed pin defined outside pins.yml (exit 1)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--check` | Validate + diff instead of writing; exit 1 on drift |  |

## `goatbuilder promote`

Guide and observe release graduation across the goatnet ladder (ADR 1120, 1125): status, open a candidate, graduate, graduate a set, the hotfix path, review, readiness, abandon, check. Replaces ops/release/graduate.py step for step. Nothing here graduates by itself: a mutation runs only when invoked, prints what it did, and asks for --yes without a terminal (C10, ADR 1177)

**Subcommands**

- [`goatbuilder promote status`](#goatbuilder-promote-status) — Cross-rung matrix: per component, the effective value on each live rung, graded
- [`goatbuilder promote open`](#goatbuilder-promote-open) — Start a candidate: write the record, print the preprod deviation edit
- [`goatbuilder promote graduate`](#goatbuilder-promote-graduate) — Verify the hop gates, mark the record graduated, print the fold-to-fleet edit
- [`goatbuilder promote graduate-set`](#goatbuilder-promote-graduate-set) — Graduate every member of a set in one act, or refuse and name every member that is not ready
- [`goatbuilder promote hotfix`](#goatbuilder-promote-hotfix) — Expedited prod path: record + prod deviation edit; owes a backfill
- [`goatbuilder promote review`](#goatbuilder-promote-review) — The start-of-day / end-of-day review: every open record sorted by what a person must DO. Read-only
- [`goatbuilder promote readiness`](#goatbuilder-promote-readiness) — The `release-readiness-v1` document. Read-only, derived, no verdict of its own
- [`goatbuilder promote abandon`](#goatbuilder-promote-abandon) — Mark a record abandoned WITH a machine-readable reason (C8)
- [`goatbuilder promote check`](#goatbuilder-promote-check) — CI invariants C1–C10 (fail-closed, current-tree). Exit 0/1

## `goatbuilder promote status`

Cross-rung matrix: per component, the effective value on each live rung, graded

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` |  |  |

## `goatbuilder promote open`

Start a candidate: write the record, print the preprod deviation edit

**Arguments**

| Argument | Description |
|---|---|
| `<COMPONENT>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--ref <REFERENCE>` |  |  |
| `--reason <REASON>` |  |  |

## `goatbuilder promote graduate`

Verify the hop gates, mark the record graduated, print the fold-to-fleet edit

**Arguments**

| Argument | Description |
|---|---|
| `<COMPONENT>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--ack <ACK>` |  |  |
| `--override-runtime-health <OVERRIDE_RUNTIME_HEALTH>` | Graduate although the runtime-health gate fails; REASON and the --ack name are written into the record |  |

## `goatbuilder promote graduate-set`

Graduate every member of a set in one act, or refuse and name every member that is not ready

**Arguments**

| Argument | Description |
|---|---|
| `<NAME>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--ack <ACK>` |  |  |

## `goatbuilder promote hotfix`

Expedited prod path: record + prod deviation edit; owes a backfill

**Arguments**

| Argument | Description |
|---|---|
| `<COMPONENT>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--ref <REFERENCE>` |  |  |
| `--reason <REASON>` |  |  |
| `--ack <ACK>` |  |  |
| `--previous-ref <PREVIOUS_REF>` | The ref being moved away from; required when fleet: has already been rewritten |  |

## `goatbuilder promote review`

The start-of-day / end-of-day review: every open record sorted by what a person must DO. Read-only

**Arguments**

| Argument | Description |
|---|---|
| `<COMPONENT>` | Limit to one component |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` |  |  |
| `--ack <ACK>` | Your name, filled into the printed graduate commands |  |
| `--detail` | The narrative per item: what it is, if left, if acted, command |  |
| `--no-header` | Omit the REVIEW line (a wrapper prints its own) |  |

## `goatbuilder promote readiness`

The `release-readiness-v1` document. Read-only, derived, no verdict of its own

**Options**

| Flag | Description | Default |
|---|---|---|
| `--out <OUT>` | Write the document here instead of stdout |  |

## `goatbuilder promote abandon`

Mark a record abandoned WITH a machine-readable reason (C8)

**Arguments**

| Argument | Description |
|---|---|
| `<RECORD>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--reason <REASON>` |  |  |
| `--superseded-by <SUPERSEDED_BY>` |  |  |

## `goatbuilder promote check`

CI invariants C1–C10 (fail-closed, current-tree). Exit 0/1

## `goatbuilder explain`

Interpret an error: a code (`GB-007`, `X-SSH-001`, `E_OFF_MESH`), or any error text — this binary's own, or a tool's words it carried (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Prints what happened, why, what to do and where to read more; `-` reads the text from stdin

**Arguments**

| Argument | Description |
|---|---|
| `<INPUT>` | A code, or words of the error. `-` reads stdin. A flag (`-o`, `--lang`) parses wherever it appears; to read text that itself starts with a hyphen, put `--` before it |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--report` | Also write the text and its interpretation to ~/.goat/error-reports/ (a file to share when the catalog does not know the error) |  |
| `--list` | List every code the catalog knows |  |

## `goatbuilder tutor`

Lessons, vimtutor-style: each names the exact command to run against a throwaway copy of a fixture (or, read-only, against this checkout), then checks what that command actually did — its exit and its rows — before the next unlocks. `tutor` shows the lesson you are on and marks it done when your runs satisfy it; `tutor <n>`, `--list`, `--reset`

**Arguments**

| Argument | Description |
|---|---|
| `<LESSON>` | The lesson to open (1..); default the first not yet done. A finished lesson opens to read again and changes nothing |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--lesson <LESSON_FLAG>` | The same as the number on its own: `tutor --lesson 2` |  |
| `--list` | Every lesson, with done / next / locked / needs-a-newer-build |  |
| `--reset` | Start over: a fresh copy of every fixture, no progress |  |
| `--workspace <WORKSPACE>` | Where the fixtures' copies, the progress and the run log live (default $GOATBUILDER_TUTOR, else ~/.goatbuilder/tutor) |  |

## `goatbuilder secrets`

The declared secrets posture (ADR 1204): which store is in force (1password, openbao, sops, git-crypt, env-file, env), what is available, how to supply what is missing, and the store's own checks. `init --env <goatnet>` writes the zero-SaaS quickstart's env file: a strong random value for every variable the goatnet's plays template, 0600, gitignored, never rotating a value already there. Same module as ops/scripts/with-secrets

**Arguments**

| Argument | Description |
|---|---|
| `<ARGS>` | store \| available \| missing NAME… \| hint NAME… \| doctor \| init --env GOATNET [--file PATH] [--force] \| exec -- CMD… |

## `goatbuilder verify`

Read every host's newest deploy-ledger row for one component and say whether it names the ref main declares (C7). Reads only; exit 3 while any host is pending or unknown. The same check `deploy --verify-only` runs

**Arguments**

| Argument | Description |
|---|---|
| `<COMPONENT>` | The component (a fleet-pins key) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | The environment (the pins' goatnet key) |  |
| `--group <GROUP>` | One member group of the environment instead of every member |  |
| `--dry-run` | Print what would be read and the digest required; read nothing |  |

## `goatbuilder diff`

What is different from main right now (C8): main's declared ref against each host's newest ledger row, for one component or, without one, every component of the environment, read in one pass per member group. Reads only; exit 3 on any drift or unreadable host

**Arguments**

| Argument | Description |
|---|---|
| `<COMPONENT>` | The component (a fleet-pins key); omit it for every component |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | The environment (the pins' goatnet key) |  |
| `--all` | Every component the environment runs (the default without a component) |  |

## `goatbuilder runs`

The runs this machine recorded, newest first: one row per run, with its id, verb, exit code, number of commands and arguments. Each run of a verb that drives hosts leaves a record at <state>/runs/<id>/run.json, kept until you remove it. Reads only

**Options**

| Flag | Description | Default |
|---|---|---|
| `--verb <VERB>` | Only the runs of this verb (standup, join-node, deploy, federate, ...) |  |
| `--last <LAST>` | How many runs to list | `20` |

**Subcommands**

- [`goatbuilder runs diff`](#goatbuilder-runs-diff) — Compare two recorded runs: their arguments, their inputs (value and source), their commands in order, and each phase's exit code. Reads only

## `goatbuilder runs diff`

Compare two recorded runs: their arguments, their inputs (value and source), their commands in order, and each phase's exit code. Reads only

**Arguments**

| Argument | Description |
|---|---|
| `<A>` | The first run: an id, a unique prefix of one, or `last` |
| `<B>` | The second run |

## `goatbuilder show`

One recorded run, phase by phase and command by command, each with its exit code and log, then its inputs with their sources. `-o json` prints the record itself. Reads only

**Arguments**

| Argument | Description |
|---|---|
| `<RUN>` | The run: its id, a unique prefix of it, or `last` |

## `goatbuilder history`

The deploy ledger as a timeline, newest first, on every host of the environment (C8): one row per record, with the machine, the time, the action (deploy or rollback) and the version. Each record's prior version and the operator who ran it are in `-o json`. Reads only

**Arguments**

| Argument | Description |
|---|---|
| `<COMPONENT>` | The component (a fleet-pins key or a role with a ledger) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--component <COMPONENT_FLAG>` | The component, as a flag (the same as the argument) |  |
| `--env <ENV>` | The environment (the pins' goatnet key) |  |

## `goatbuilder plan`

Write what a declarative verb would do to a plan file for review: the declared refs, the play and target, the ledger, main's commit, the transport (deploy); the artifact type and the target ref (rollback). Touches no host's state. Run it as reviewed with `apply`

**Arguments**

| Argument | Description |
|---|---|
| `<VERB>` | deploy \| rollback |
| `<COMPONENT>` | The component |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | The environment (deploy) |  |
| `--group <GROUP>` | One member group (deploy) |  |
| `--n <N>` | Steps back (rollback) | `1` |
| `--site <SITE>` | One goatnet to roll back (rollback), its `site:` in the ledger (its `site_id`). Required for a component that spans more than one goatnet; carried in the plan so apply stays narrowed to it |  |
| `--out <OUT>` | Where to write the plan |  |

## `goatbuilder apply`

Run exactly a reviewed plan. Refuses (exit 1, naming what moved) when main's declaration, the pins commit, the play or the rollback target moved since the plan was written; a changed transport is reported, not refused

**Arguments**

| Argument | Description |
|---|---|
| `<PLAN>` | The plan file `goatbuilder plan` wrote |

## `goatbuilder adopt`

Declare what an environment already runs: read every host's ledgers and propose a pins deviation for each component whose running ref differs from main's declaration. A report by default; with --yes the edits are written into a fresh worktree of main for review. Never deploys

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | The environment (the pins' goatnet key) |  |

## `goatbuilder assay`

Check that a goatnet was built correctly, layer by layer: every acceptance check that applies to the site, including the checks an operator runs with the goat CLI. The standup's acceptance phase as its own command. Reads only; exit 3 while any check fails or cannot answer

**Arguments**

| Argument | Description |
|---|---|
| `<SITE>` | The goatnet site (its `ops/ansible/group_vars/<site>_hosts` name) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--plan` | List the checks that apply to this site, by layer; probe nothing |  |
| `--only <ONLY>` | Run only these checks, by the names `--plan` prints (comma-separated) |  |
| `--var <VARS>` | Override one value for this run (`k=v`), e.g. `goat_via=user@host` for a machine that holds the goat context |  |
| `--rows-out <ROWS_OUT>` | Also write each row's verdict to this file as JSON lines |  |
| `--values <VALUES>` | Use this value set instead of deriving one from the site's group_vars (a fixture's, for a rehearsal with --plan) |  |

## `goatbuilder endstate`

Check that a goatnet is in its declared end state: every service its own declarations say it runs is deployed, running, healthy and doing its job, with its zero-trust edges wired, and nothing runs that it does not declare. Reads only; exit 3 while anything is off

**Arguments**

| Argument | Description |
|---|---|
| `<SITE>` | The goatnet site (its `ops/ansible/group_vars/<site>_hosts` name) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--facts <FACTS>` | Grade facts read earlier (`endstate.py observe`) instead of reading the hosts |  |
| `--assay-rows <ASSAY_ROWS>` | The assay's rows (`goatbuilder assay --rows-out`) for the functional grade, instead of running the assay first |  |
| `--no-assay` | Skip the assay: the functional grade then reads unknown |  |
| `--strict` | A grade nothing can observe (no health signal, no acceptance row) fails too |  |
| `--root <ROOT>` | Read the declarations from this tree instead of the repository (a fixture) |  |

## `goatbuilder survey`

How consistent the software is inside each goatnet and across the ladder's goatnets, read in one parallel pass. Reads only.

Inside a goatnet: main's declared ref against each host's newest ledger row and the containers it runs from that image (a running one; a stopped one only on a warm standby with none running), hosts of one goatnet that disagree, the goat CLI on each host against the one its goatnet serves, and what goat-get serves against main. Across goatnets: one row per component, one column per goatnet, with the ladder's grade for every difference, as promote status grades it. Exit 3 on any drift or anything unread; a difference the ladder declares is reported, not failed.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENVS>` | One goatnet (repeatable); default the ladder's rungs (dev, preprod, prod) |  |
| `--all` | Every goatnet fleet-pins.yaml declares, not only the ladder's |  |
| `--intra` | Only the findings inside each goatnet |  |
| `--inter` | Only the findings across goatnets, and the matrix |  |
| `--html <HTML>` | Also write a self-contained HTML report to this file (or `survey.html` in this directory) |  |

## `goatbuilder update`

Replace this binary with the release its goatnet serves at `get.<site>.<zone>/goatbuilder/`: the download is checked against its `.sha256` and the signed `SHA256SUMS` before anything is touched, and the swap is atomic. Already current: one row, exit 0. `--check` reads only (exit 3 when behind). The surface: `GOATBUILDER_UPDATE_URL`, else `--site` from its group_vars, else the one the last update used

**Options**

| Flag | Description | Default |
|---|---|---|
| `--site <SITE>` | The goatnet that serves goatbuilder (its group_vars site) |  |
| `--check` | Report running against served; change nothing |  |
| `--force` | Reinstall even when already current (re-verifies the checksum) |  |

**Subcommands**

- [`goatbuilder update verify-release`](#goatbuilder-update-verify-release) — Verify a downloaded goatbuilder release without installing it: the keyless signature over `<DIR>/SHA256SUMS` (`SHA256SUMS.cosign-bundle`), with the checks `update` makes before it installs, for the release workflow of `--signer` at `goatbuilder/v<version>`; then every other file in `<DIR>` against that `SHA256SUMS`. Reads only. Exit 0 verified; 1 refused (GB-261 the signature, GB-074 a file)

## `goatbuilder update verify-release`

Verify a downloaded goatbuilder release without installing it: the keyless signature over `<DIR>/SHA256SUMS` (`SHA256SUMS.cosign-bundle`), with the checks `update` makes before it installs, for the release workflow of `--signer` at `goatbuilder/v<version>`; then every other file in `<DIR>` against that `SHA256SUMS`. Reads only. Exit 0 verified; 1 refused (GB-261 the signature, GB-074 a file)

**Arguments**

| Argument | Description |
|---|---|
| `<DIR>` | The downloaded release: its files, SHA256SUMS and SHA256SUMS.cosign-bundle |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--signer <SIGNER>` | `<owner>/<repo>` the release was built and signed in |  |
| `--trusted-root <TRUSTED_ROOT>` | A Sigstore trusted_root.json trusted beside the embedded public one |  |

## `goatbuilder completion`

Print a shell completion script to stdout (`goatbuilder completion zsh >> ~/.zshrc`). For live candidates a static script cannot know — the sites, components and goatnets this checkout declares — wire a hook to `goatbuilder _complete` (see the generated script's header)

**Arguments**

| Argument | Description |
|---|---|
| `<SHELL>` | The shell to generate for |

## `goatbuilder docs`

The command reference: `goatbuilder docs` points at it; `--markdown` emits the whole thing, walked from the live command tree so it can never drift from `--help`

**Options**

| Flag | Description | Default |
|---|---|---|
| `--markdown` | Emit the full command reference as Markdown (for the committed reference or offline reading) |  |

