### goatbuilder — English (en), the arbiter
### Generated from the doc comments on Cli/Cmd (crates/goatbuilder/src/main.rs) by
### GOATBUILDER_I18N_WRITE=1 cargo test -p goatbuilder i18n:: — do not edit by hand.

i18n-machine-translation-warning = note: { $lang } output is machine-translated and not yet native-reviewed — the English text is authoritative. Report translation errors to { $channels }. Suppress with GOAT_I18N_NO_WARN=1.

help-about = Build and ship a goat environment: declare, converge, verify — under one deploy contract
help-flag-output = Output format: a table for a person; JSON or YAML for a program
help-flag-repo = The repository root. Defaults to the checkout this binary is run in
help-flag-yes = Confirm a declarative verb without prompting (for automation)
help-flag-frozen = Refuse any run that would need to fast-forward the checkout (strict C3)
help-flag-offline = An islanded site: do not fetch the trunk; judge the checkout against the cached one, and record the run as offline (C3; ops/scripts/fresh-source.sh)
help-flag-mock = Run against a fixture directory instead of hosts: no registry, no transport, deterministic (goatbuilder-conformance.md §"The fixture")
help-flag-lang = Language for help and error explanations (a tag such as fr, uk, de); output a pipeline reads stays English. Default: the OS locale
help-flag-color = Color the table's ok/FAIL glyph and verdict (auto|always|never). `auto` colors only a terminal; NO_COLOR and CLICOLOR_FORCE override. JSON and YAML are never colored
help-flag-verbose = Log what a run does — the external commands it shells out to, and the network phase of `update` — to stderr, so `-o json|yaml` stays a clean document. Or set RUST_LOG=goatbuilder=debug
help-flag-timeout = Seconds to bound the connect/reach phase before failing fast — the one phase that can block on an unreachable host (`update`'s fetch of the served version). Does NOT cap a play's own run or the binary download (stall-bounded). Also reads GOATBUILDER_TIMEOUT
help-cmd-validate = Check a config against the versioned schema and the cross-field rules, offline
help-cmd-doctor = Probe what a run needs before anything is cut: deps, checkout freshness, each environment's transport, and that every component's playbook resolves to a definite target mode
help-cmd-status = Declared against recorded, per machine: converged, pending or unknown (C7, C8). Reads only. Exit 3 while any machine is not converged
help-cmd-deploy = Put the pin main declares onto one environment and prove it: resolve the component and the environment's groups from the pins, the play and its target mode, every secret the play templates, the transport; run the play through ansible-preflight; read the deploy ledger back from every host and call it done only when the newest row names the declared digest with action=deploy (C7). On a fixture (`--mock`) the scripted converger stands in for the hosts
help-cmd-preflight = Before a rollout, the readiness and reversibility readout (ADR 1211): three facts — State (stateless | stateful), Rollback (pin-revert | restore-required | none) and Scope (single | coupled) — declared per component and FAIL-CLOSED when undeclared, with the readiness half the other verbs prove (ref pullable, pins --check, the secrets gate, fresh source, the declared end state) and a validation of the rollback fact (rollback --dry-run for pin-revert, the retention window for restore-required). `deploy` runs it first; honesty-graded, never a false green. Reads only; exit 3 when not clean
help-cmd-release = Take a release candidate from a tag on main through the dev goatnet's live validation to the preprod rung (ADR 1173); after the soak, with `--from prod`, graduate it, fold it into fleet:, deploy prod and re-verify every rung. Replaces ops/scripts/rc.sh step for step
help-cmd-rollback = Roll a deployed component back to its N-th most recent ref, by artifact type, reading the on-host ledger as the source of truth (ADR 1060). Replaces ops/scripts/rollback.sh: exit 3 refused unsafe (data, cert, forward-only, reversible:false), 4 guided (config, acl)
help-cmd-fastlane = One command from a merged commit to every goatnet (ADR 1127): the deliberate exception to the ladder. Replaces ops/scripts/fastlane.sh step for step; refuses an unpinned digest or an open graduation set, and runs the deploys through this binary
help-cmd-standup = A site from nothing to verified: the standup's phases in the doctrine's order with a fail-closed gate at each end, the observability host as a parallel leg, the rescue classes and the cross-goatnet surfaces last (ADR 1183). Replaces ops/scripts/goat-standup.sh step for step. On a fixture (`--mock`) every component of every class is converged by the scripted converger and the declared → verified time is the one-hour goatnet's measure
help-cmd-teardown = A site from verified to nothing — standup's mirror, records kept (a burn-down appends teardown rows, never erases). On a fixture (`--mock`) it rehearses the burn-down in reverse phase order; on real hosts it prints the order and refuses to execute until each phase declares its teardown play (docs/design/goatbuilder-speed.md)
help-cmd-join-node = One more machine of a class at a live site, declared → converged → verified: the node-addition sequence of docs/tutorials/02-first-federated-node.md as numbered phases with its gates (reach, key, repo work first, baseline, kernel WireGuard before and after enrolment, the kernel ACL plane as the enrolment gate, the class's converge, acceptance)
help-cmd-federate = A sovereign peering declared, converged and verified (ADR 1157): our half measured, the packages exchanged through the peering directory, one agreement composed and verified, the leaves minted from it, the rules rendered by the one writer, the edge deployed, the transport the agreement chose opened, acceptance in all four tiers. Replaces ops/scripts/peering-standup.sh step for step
help-cmd-federate-long =
    A sovereign peering declared, converged and verified (ADR 1157): our half measured, the packages exchanged through the peering directory, one agreement composed and verified, the leaves minted from it, the rules rendered by the one writer, the edge deployed, the transport the agreement chose opened, acceptance in all four tiers. Replaces ops/scripts/peering-standup.sh step for step.

    At a terminal, `goatbuilder federate <partner>` is the whole procedure: it asks for the partner's key and CA fingerprint and the agreement digest, makes the plan and asks to approve it, shows what our core will admit and asks again, then carries on through acceptance. A run that stops resumes where it stopped. Values come from the deployment's declarations; a FED_* variable set in the environment overrides one.

    Each stage also runs alone: `plan <partner>`, `deploy <partner>`, `test <partner>`, `status [<partner>]`. `allocate [<peering>]` writes a peering's values and files for review; `backout <partner>` backs one peering out and leaves the others as they were. `goatbuilder federate init|lint|compose|diff|verify|apply` runs one of the six federation verbs in process, with the flags and exit codes of `goat federate <verb>`.
help-cmd-pins = ONE resolution pass with TWO renderings: every listed member's group_vars pins.yml and every listed goatnet's IntentV1 artifact, from ops/pins/fleet-pins.yaml. Replaces ops/pins/generate-pins.py byte for byte. `--check` validates, regenerates and refuses drift, an unsigned or tampered intent, and a governed pin defined outside pins.yml (exit 1)
help-cmd-promote = Guide and observe release graduation across the goatnet ladder (ADR 1120, 1125): status, open a candidate, graduate, graduate a set, the hotfix path, review, readiness, abandon, check. Replaces ops/release/graduate.py step for step. Nothing here graduates by itself: a mutation runs only when invoked, prints what it did, and asks for --yes without a terminal (C10, ADR 1177)
help-cmd-explain = Interpret an error: a code (`GB-007`, `X-SSH-001`, `E_OFF_MESH`), or any error text — this binary's own, or a tool's words it carried (ansible, ssh, op, aws, git, gh, cosign, docker, netbird). Prints what happened, why, what to do and where to read more; `-` reads the text from stdin
help-cmd-tutor = Lessons, vimtutor-style: each names the exact command to run against a throwaway copy of a fixture (or, read-only, against this checkout), then checks what that command actually did — its exit and its rows — before the next unlocks. `tutor` shows the lesson you are on and marks it done when your runs satisfy it; `tutor <n>`, `--list`, `--reset`
help-cmd-make = Hidden (src/fun.rs)
help-cmd-argan = Hidden (src/fun.rs)
help-cmd-secrets = The declared secrets posture (ADR 1204): which store is in force (1password, openbao, sops, git-crypt, env-file, env), what is available, how to supply what is missing, and the store's own checks. `init --env <goatnet>` writes the zero-SaaS quickstart's env file: a strong random value for every variable the goatnet's plays template, 0600, gitignored, never rotating a value already there. Same module as ops/scripts/with-secrets
help-cmd-verify = Read every host's newest deploy-ledger row for one component and say whether it names the ref main declares (C7). Reads only; exit 3 while any host is pending or unknown. The same check `deploy --verify-only` runs
help-cmd-diff = What is different from main right now (C8): main's declared ref against each host's newest ledger row, for one component or, without one, every component of the environment, read in one pass per member group. Reads only; exit 3 on any drift or unreadable host
help-cmd-runs = The runs this machine recorded, newest first: one row per run, with its id, verb, exit code, number of commands and arguments. Each run of a verb that drives hosts leaves a record at <state>/runs/<id>/run.json, kept until you remove it. Reads only
help-cmd-show = One recorded run, phase by phase and command by command, each with its exit code and log, then its inputs with their sources. `-o json` prints the record itself. Reads only
help-cmd-history = The deploy ledger as a timeline, newest first, on every host of the environment (C8): one row per record, with the machine, the time, the action (deploy or rollback) and the version. Each record's prior version and the operator who ran it are in `-o json`. Reads only
help-cmd-plan = Write what a declarative verb would do to a plan file for review: the declared refs, the play and target, the ledger, main's commit, the transport (deploy); the artifact type and the target ref (rollback). Touches no host's state. Run it as reviewed with `apply`
help-cmd-apply = Run exactly a reviewed plan. Refuses (exit 1, naming what moved) when main's declaration, the pins commit, the play or the rollback target moved since the plan was written; a changed transport is reported, not refused
help-cmd-adopt = Declare what an environment already runs: read every host's ledgers and propose a pins deviation for each component whose running ref differs from main's declaration. A report by default; with --yes the edits are written into a fresh worktree of main for review. Never deploys
help-cmd-assay = Check that a goatnet was built correctly, layer by layer: every acceptance check that applies to the site, including the checks an operator runs with the goat CLI. The standup's acceptance phase as its own command. Reads only; exit 3 while any check fails or cannot answer
help-cmd-endstate = Check that a goatnet is in its declared end state: every service its own declarations say it runs is deployed, running, healthy and doing its job, with its zero-trust edges wired, and nothing runs that it does not declare. Reads only; exit 3 while anything is off
help-cmd-survey = How consistent the software is inside each goatnet and across the ladder's goatnets, read in one parallel pass. Reads only
help-cmd-survey-long =
    How consistent the software is inside each goatnet and across the ladder's goatnets, read in one parallel pass. Reads only.

    Inside a goatnet: main's declared ref against each host's newest ledger row and the containers it runs from that image (a running one; a stopped one only on a warm standby with none running), hosts of one goatnet that disagree, the goat CLI on each host against the one its goatnet serves, and what goat-get serves against main. Across goatnets: one row per component, one column per goatnet, with the ladder's grade for every difference, as promote status grades it. Exit 3 on any drift or anything unread; a difference the ladder declares is reported, not failed.
help-cmd-update = Replace this binary with the release its goatnet serves at `get.<site>.<zone>/goatbuilder/`: the download is checked against its `.sha256` and the signed `SHA256SUMS` before anything is touched, and the swap is atomic. Already current: one row, exit 0. `--check` reads only (exit 3 when behind). The surface: `GOATBUILDER_UPDATE_URL`, else `--site` from its group_vars, else the one the last update used
help-cmd-completion = Print a shell completion script to stdout (`goatbuilder completion zsh >> ~/.zshrc`). For live candidates a static script cannot know — the sites, components and goatnets this checkout declares — wire a hook to `goatbuilder _complete` (see the generated script's header)
help-cmd-docs = The command reference: `goatbuilder docs` points at it; `--markdown` emits the whole thing, walked from the live command tree so it can never drift from `--help`
help-cmd-_complete = Hidden: emit live completion candidates for shell hooks, one per line (the sites, components and goatnets this checkout declares). Reads only, never fails loudly — a completion hook must not corrupt the prompt
help-cmd-bound = Hidden (crates/goat-fun)
tutor-l1-title = Check your setup
tutor-l1-text =
    The goatbuilder CLI builds, deploys and releases GOAT software. Before you use it, check your setup with two commands:
    - The goatbuilder `validate` command checks the config file, `ops/goatbuilder/goatbuilder.yaml`. It reads only this checkout and needs no network.
    - The goatbuilder `doctor` command checks what a real deploy needs: your tools, your sign-ins, whether this checkout matches main, and how goatbuilder reaches each goatnet. A goatnet is one GOAT environment.

    The goatbuilder `doctor` command prints how many checks passed, then each problem with its fix. In this lesson a problem is information, not a mistake. Read the fixes, but you do not have to apply them now.

    Most lessons need no sign-in. Lessons 2 to 5, 10, 11 and 16 use a practice copy. Lessons 8, 9, 12, 13, 15, 17 and 18 only read this checkout. Lesson 6 needs `gh` and the mesh, lesson 7 needs `gh`, and lesson 14 needs the mesh. No lesson needs 1Password.
tutor-l1-s1 = Check the config file
tutor-l1-s2 = Ask the doctor about one goatnet
tutor-l2-title = Read state: converged, pending, unknown
tutor-l2-text =
    The goatbuilder `status` command compares two things for each machine: the version main declares for it, and the version its own record says it runs. It changes nothing.

    Each machine is in one of three states:
    - converged: its record names the declared version. This is done.
    - pending: its record names an older version, or nothing yet.
    - unknown: goatbuilder cannot read its record.

    Only converged counts as done. If any machine is pending or unknown, `status` exits with code 3.

    From this lesson on, `--mock` points goatbuilder at a practice copy in your tutor folder instead of real machines. Nothing you do there reaches a real machine.

    Nothing has been deployed to the practice copy's `dev` yet. So expect four machines pending and one unknown. You will see converged machines in lesson 3.
tutor-l2-s1 = Read the state of dev
tutor-l3-title = Deploy: plan it, run it, prove it
tutor-l3-text =
    Deploying puts the declared version on every machine, then reads each machine's record to prove it arrived. A deploy is done only when every machine's record names that version. A deploy that ran without errors is not enough.

    You will deploy `portal` to the practice copy's `preprod`. `--dry-run` shows what would happen and changes nothing. `--ack` records who did it; the command fills in your name from git. After the deploy, `status` should show both machines as `ok` and converged.
tutor-l3-s1 = Plan the deploy
tutor-l3-s2 = Deploy portal to preprod
tutor-l3-s3 = Check that both machines converged
tutor-l4-title = Drift and history
tutor-l4-text =
    Two commands help you understand a goatnet before and after a change. Both only read.
    - The goatbuilder `diff` command lists every machine whose record differs from the declared version. A difference is called drift.
    - The goatbuilder `history` command lists every version a component had on each machine, newest first.

    On `dev`, every machine is behind. The history of `portal` on `preprod` now includes your deploy from lesson 3.
tutor-l4-s1 = List the drift on dev
tutor-l4-s2 = Read the history of portal on preprod
tutor-l5-title = Roll back, and see an unsafe rollback refused
tutor-l5-text =
    A rollback declares an earlier version and deploys it like any other version. The goatbuilder CLI then reads each machine's record to prove it arrived.

    Some changes cannot be undone this way. Deploying an older version cannot undo data a component has written, a certificate, or a change that only moves forward. The goatbuilder CLI refuses those with exit code 3 and says what to do instead.

    The practice copy's `db-schema` component holds data, so the goatbuilder CLI refuses to roll it back. That refusal is the expected result.
tutor-l5-s1 = Plan the rollback
tutor-l5-s2 = Roll portal back one version
tutor-l5-s3 = Try to roll back db-schema, which goatbuilder refuses
tutor-l6-title = Release a new version: dev, then preprod, then prod
tutor-l6-text =
    A release moves a new version through three goatnets in order: dev, then preprod, then prod.

    1. Someone tags the version on main, and CI builds it.
    2. dev: goatbuilder deploys it and runs the acceptance tests.
    3. preprod: goatbuilder deploys it again, runs the tests again and records the results.
    4. The version then waits on preprod for a set time. Nothing moves to prod by itself.
    5. prod: after that wait, a person runs the release with `--from prod`. The goatbuilder CLI deploys to prod and checks all three goatnets.

    The goatbuilder `promote status` command shows where every component is. A candidate is a version being tested. The general form of a release is `goatbuilder release <component> --tag <tag> --from dev`. The commands below use one component of this checkout and an example tag. Each one has `--dry-run`, so nothing is tagged or deployed.

    The dry runs need `gh` and the mesh. If you are not an admin of the repository, put `GOAT_RC_ROLE_CHECK=0` before each `release` command.
tutor-l6-s1 = See where every component is
tutor-l6-s2 = Dry-run the release from dev
tutor-l6-s3 = Dry-run the prod step
tutor-l7-title = Fastlane, and when not to use it
tutor-l7-text =
    Fastlane puts one version on every goatnet at once. It skips the wait on preprod and the release records.

    Use it only for an urgent fix that you can undo by going back to the previous version. Never use it for a database change, an identity or access change, or anything you cannot undo that way.

    Fastlane refuses a version that CI has not built, and a component that is part of a release still in progress.

    The general form is `goatbuilder fastlane <component> --ref <version> --reason <why>`. Run the example below. It has `--dry-run`, so nothing changes. It needs `gh`.
tutor-l7-s1 = Dry-run a fastlane
tutor-l8-title = Pins: find drift and fix it the right way
tutor-l8-text =
    One file sets which version each goatnet runs: `ops/pins/fleet-pins.yaml`. The goatbuilder CLI generates the other version files from it: the `pins.yml` files and the signed intent files.

    The goatbuilder `pins --check` command compares the generated files with `fleet-pins.yaml` and refuses any file that differs. The goatbuilder `pins` command regenerates them.

    Your tutor folder has a copy of the repository in which someone edited one `pins.yml` file by hand. Never fix a hand edit with another hand edit. Change `fleet-pins.yaml`, then regenerate.
tutor-l8-s1 = Check this checkout
tutor-l8-s2 = Check the copy and find the hand edit
tutor-l8-s3 = Regenerate the copy's files
tutor-l8-s4 = Check the copy again
tutor-l9-title = Explain an error
tutor-l9-text =
    When goatbuilder refuses, it prints a code. Codes that start with `GB-` come from goatbuilder, `E_` from the goat CLI, and `X-` from another tool that goatbuilder ran.

    The goatbuilder `explain` command takes a code, or the text of an error line, and tells you what happened, why, how to fix it, and where to read more.

    Asking for a goatnet this checkout does not declare makes a real error to explain.
tutor-l9-s1 = Make a real error with a goatnet that does not exist
tutor-l9-s2 = Explain the error by its code
tutor-l9-s3 = Explain the error by its text
tutor-l10-title = Tutorial 1: build a new goatnet's first site
tutor-l10-text =
    This lesson builds a whole goatnet in a practice copy. The example goatnet is `zagros`. Its first site, `alpha`, has two core machines, one monitoring machine and one rescue machine.

    The goatbuilder `standup` command sets up every machine in a fixed order and checks every record. Its last line says the site is done. On real machines, that line also shows how long it took.

    The full tutorial is `docs/tutorials/01-new-goatnet.md`.
tutor-l10-s1 = Build the first site, alpha
tutor-l10-s2 = Check the result
tutor-l11-title = Tutorial 2: add a second site
tutor-l11-text =
    This lesson starts where tutorial 1 ended. It adds a second site, `beta`, in a new practice copy, `zagros-2`.

    A second site copies its user accounts from the first site. Its standup also changes the first site once, so that copying works in both directions. A step named `alpha/core` does this.

    The full tutorial is `docs/tutorials/02-first-federated-node.md`.
tutor-l11-s1 = Build the second site, beta
tutor-l11-s2 = Check both sites
tutor-l12-title = Add a machine to a running site
tutor-l12-text =
    The goatbuilder `join-node` command adds one more machine to a site that is already running. It prints a plan first, with one row per phase. Each phase has a type:
    - tool: goatbuilder runs a script.
    - ansible: goatbuilder runs a playbook.
    - gate: a check that must pass before the next phase.
    - manual: you do this yourself, and the run pauses.

    The command below has `--dry-run`. It reads this checkout and changes nothing. Under the table, read what you do by hand.
tutor-l12-s1 = Plan adding one machine
tutor-l13-title = Connect to another organization's goatnet
tutor-l13-text =
    The goatbuilder `federate` command connects your goatnet to a goatnet that another organization runs. Each side builds its half of an agreement. You swap halves, and each side combines them into the same agreement.

    Confirm the other side's key in a separate way, such as a call you trust. A key that arrives inside their package proves nothing.

    The command below has `--dry-run`, so it changes nothing. A real run stops at phase 4, because you do that phase by hand.
tutor-l13-s1 = Plan a federation
tutor-l14-title = Keep goatbuilder up to date
tutor-l14-text =
    The goatbuilder CLI can update itself from the copy that your goatnet serves at `get.<site>.<zone>/goatbuilder/`.

    The goatbuilder `update --check` command only reads. It compares your version with the served version and changes nothing. Exit code 0 means you are up to date. Exit code 3 means a newer version exists.

    A real update, without `--check`, verifies the download's checksum and signature before it replaces anything. It then swaps the file in one step.

    The goatbuilder CLI needs to know which goatnet to ask: pass `--site` with that goatnet, or set `GOATBUILDER_UPDATE_URL`. You must be on that goatnet's mesh. If you are not, `--check` says that it cannot reach the server and names the address it tried. That is a correct result for this lesson.
tutor-l14-s1 = Ask whether a newer goatbuilder exists
tutor-l15-title = Assay: check how a goatnet was built
tutor-l15-text =
    The goatbuilder `assay` command checks that a goatnet was built correctly, one layer at a time, using the acceptance tests. Each test gets one of three results:
    - PASS: the test passed.
    - FAIL: the goatnet has a problem.
    - UNKNOWN: nothing could answer the test.

    Only PASS counts. Any FAIL or UNKNOWN makes the exit code 3.

    `--plan` lists the tests that apply to a goatnet, by layer, and touches no machine. Use it to see what a real assay would cover. The general form is `goatbuilder assay <goatnet> --plan`.
tutor-l15-s1 = List the tests that apply to one goatnet
tutor-l16-title = Survey: are the versions consistent?
tutor-l16-text =
    The goatbuilder `survey` command checks in one pass whether the same versions run where they should:
    - Inside each goatnet: does every machine run the declared version, and do machines that should match actually match?
    - Across dev, preprod and prod: one row per component, one column per goatnet.

    A difference that the release process expects is reported but is not an error. An example is a new version that is on dev but not on prod yet. A difference that nothing declares reads as `difference not declared`. A machine goatbuilder cannot read is unknown, never consistent.

    Run it on the practice copy. Its `dev` is behind on purpose, so expect findings and exit code 3.
tutor-l16-s1 = Survey the practice copy
tutor-l17-title = Endstate: does a goatnet run exactly what it declares?
tutor-l17-text =
    The goatbuilder `endstate` command checks two things about a goatnet:
    1. Every service it declares is deployed, running, healthy and correctly connected.
    2. Nothing runs that it does not declare. An undeclared service is called an orphan.

    The goatbuilder CLI works out what a site should run from the site's own settings, not from a separate list.

    A check that has nothing to read is marked UNOBSERVED. That is an honest answer, not a failure. A FAIL or an orphan makes the exit code 3.

    The general form is `goatbuilder endstate <goatnet>`. The command below checks an example site from saved data, so it touches no machine.
tutor-l17-s1 = Check the example site
tutor-l18-title = Bonus: be curious
tutor-l18-text =
    This lesson is optional. Run the command below and the tutor is finished.

    The goatbuilder CLI has a few small hidden features. They are not in the help or in these lessons. Some answer to words that long-time Unix users often type into a new program, just to see what happens. Most things you try will do nothing.

    This lesson does not say what they are. Finding them is the point.

    The command below lists every error that goatbuilder can explain. It is a good place to start exploring.
tutor-l18-s1 = List every error goatbuilder can explain
tutor-heading = Lesson { $n } of { $total } · { $title }
tutor-setup = First, run this once in this terminal:
tutor-not-recording = This terminal is not recording your runs, so the tutor cannot tick your steps. Run this once, then run the steps:
tutor-type = Do these steps in your terminal ({ $met } of { $total } done):
tutor-prereq = Before you start: { $prereq }
tutor-not-yet = Not yet: { $why }
tutor-rerun = After each step, run `goatbuilder tutor` again. It checks what you did and ticks the step.
tutor-review = You finished this lesson, so this page is only to read. Run `goatbuilder tutor` to go back to where you are.
tutor-where = That command is not part of lesson { $n }. Run `goatbuilder tutor` to see where you are.
tutor-other-lessons = To read a lesson again: `goatbuilder tutor --lesson 1`. To see them all: `goatbuilder tutor --list`.
tutor-welcome = Welcome to GOATBUILDER Tutor
tutor-bar-progress = Progress
tutor-bar-done = { $done } of { $total } lessons done
tutor-after-step = Step { $met } of { $total } done. Next:
tutor-after-lesson = Lesson { $n } is complete. Run `goatbuilder tutor` to start lesson { $next }. To read it again: `goatbuilder tutor --lesson { $n }`.
tutor-after-final = Lesson { $n } is complete, and so is the tutor. Run `goatbuilder tutor` to see it marked done. To read it again: `goatbuilder tutor --lesson { $n }`.
tutor-lesson-done = Lesson { $n } done: { $title }.
tutor-all-done-reset = Every lesson is done. `goatbuilder tutor --reset` starts over; the tutorials go further: docs/tutorials/.
tutor-all-done = Every lesson is done. The tutorials go further: docs/tutorials/.
tutor-workspace = Workspace: { $path }
