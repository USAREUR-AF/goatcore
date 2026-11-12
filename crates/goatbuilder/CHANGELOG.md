# goatbuilder changelog

Each release is the tag `goatbuilder/v<semver>` on `main`, built and signed by
`.github/workflows/goatbuilder-release.yml`. Design of record:
`docs/design/goatbuilder.md`; conformance: `docs/design/goatbuilder-conformance.md`.

## Unreleased

### Changed: `update` verifies the release signature itself, and always

`goatbuilder update` now verifies the keyless signature over the release's
`SHA256SUMS` in the binary, with no `cosign` needed and nothing fetched beyond
the release's own signature bundle. Before, the signature was checked only
when `cosign` was on PATH; without it, the update installed with the
signature "not checked". Now an install without cosign enforces it too. A
release whose signature does not verify, or that serves none, is refused
(GB-254), and nothing is installed. Every goatbuilder release published
with a signature (0.4.1, 0.4.2, 0.4.3, 0.5.1) verifies.

The signature must be the release workflow's (`goatbuilder-release.yml`) in
the repository `goatbuilder.yaml` names, at the served version's tag. The
Sigstore trusted root is built in. After a Sigstore key rotation, set
`GOATBUILDER_UPDATE_TRUSTED_ROOT` to a `trusted_root.json` that holds the new
key.

## 0.5.3 — 2026-11-12

0.5.2 was tagged but never published: its release run stopped at startup,
because the shared release workflow's docs job asks for `packages: read` and
neither release workflow granted it. 0.5.3 is the first published release of
everything in 0.5.2 below, plus that fix and the fixes merged since.

### Fixed: the release workflows grant what their shared workflow asks for

`goatbuilder-release.yml` and `goat-release.yml` now grant `packages: read`,
which the docs job of `_release-binary.yml` needs for the project's package
cache.

### Fixed: declared values reach the phases as one word each

The goatlite's administrator address reaches the bootstrap phases through the
environment as data, never through the shell, and a bootstrap command run
without that variable is refused, naming it. `join-node`'s founder site, SSH
user, AWS profile and enrolment name are passed the same way, including the
line run on the new node.

### Fixed: `verify` reads a failed ledger read as unknown

A host whose ledger could not be read is reported as unknown, not as not
converged.

## 0.5.2 — 2026-11-12

The federation documents corrected against the code, declared values passed
to standup's phases as one word each, `status` read from the hosts themselves,
an edge ingress check that now runs when the core is on its own host, and
several smaller fixes.

### Added: `status` reads real hosts

`goatbuilder status <env> <component>` now reads each member group's hosts the
way `diff` reads them, and grades each host by `verify`'s rule: the declared
digest and a converged action. A host that cannot be read is reported as
unknown, never as converged or not provisioned.

### Fixed: declared values reach standup's phases as one word

A declared state directory, AWS profile or other path is passed to each phase's
shell as one word, exactly as declared. A value containing a space, a quote or
`$(…)` is no longer split or run by the shell. A plain value is written exactly
as before, so existing plans are unchanged.

### Fixed: an edge's ingress check reads the core on its own host

When the edge and the core run on separate hosts, the edge role's ingress check
used to find no core, print a skip, and let phase 12 pass with a core whose only
store was `from/**`. It now reads the core on the core host, at the
configuration path the router was started with, and a check that cannot run
fails phase 12 instead of skipping. **Behaviour change:** a deployment whose core
keeps only a broad `from/**` store, or whose core cannot be read, now stops at
phase 12.

### Fixed: `validate` names GB-025 on every failing row

Each failing row now starts with `GB-025:`, so `goatbuilder explain GB-025`
applies, in the text output and in `-o json`.

### Fixed: `release` and `rc.sh` print commits at a fixed length

Both print a commit as 12 characters, so the two avenues print the same line
however large the repository grows, and the release parity check compares them
equal.

### Changed: a recorded key rotation makes phase 3 run again

When a key rotation leaves its record, `<work>/rotation.json`, phase 3 measures
and signs our package again instead of being skipped on resume. Without that
file, every phase's recorded inputs are unchanged, so an upgrade reruns
nothing. No command writes the record yet.

### Fixed: the federation documents match what the code does

The operator runbook, the two-sovereigns and boundary-node tutorials, the
design document, the reference and the roadmap were read against the release
code and corrected. A peering names its `tunnel`, never a `transport`.
`goatbuilder federate plan` writes the plan, and phases 1 to 9 run under
`goatbuilder federate <partner>`. The core router's dry run comes after the
admission is approved. `status` reads the run journal and the evidence, not
the live peering. The run asks at phases 5, 6 and 10. The runbook now lists
the admission settings for a core that the goat-zenoh role does not deploy,
and gives the acceptance session's commands.

### Fixed: GB-129 says when a narrower offer is refused

A peering's `offer:` may narrow what it offers whenever goatbuilder generates
its federation.toml. GB-129 now says that the narrower list is refused only
for a hand-written federation.toml (`federation_toml: hand`), which phase 3
offers whole.

### Fixed: an interrupted phase 8 no longer leaves private keys behind

Phase 8 mints new leaves in a private staging directory, `<out>/.mint-*`, and
renames them into place. A run killed while minting left that directory, and
the private keys in it, behind for good. The next run now removes every
`.mint-*` entry once it holds the output directory's lock and has found no
interrupted rotation to recover. Installed leaves, the CA certificate, the lock
file and a rotation's retained previous set are never touched.

### Changed: CI no longer depends on the Ubuntu archive

When a runner cannot reach Ubuntu's mirror, the jobs that install packages
or build images fail over to the project's own package cache, and say so in
one line. A forced-failover self-test proves that path on a real runner.
This changes how goatbuilder is built and checked, not the binary.

## 0.5.1 — 2026-11-11

The same goatbuilder as 0.5.0. Version 0.5.0 was tagged and its binaries were
built, but its release was not published: the release's documentation job
installs gettext from the runner's Ubuntu mirror, and that mirror did not
answer. 0.5.1 is the first published release that carries everything listed
under 0.5.0 below.

### Fixed: a release no longer waits on a distribution mirror

The release's documentation job uses gettext when the runner already has it,
tries the Ubuntu mirror for three minutes, and otherwise installs it with
Homebrew from the runner image.

## 0.5.0 — 2026-11-11

Federation moves from `goat` into goatbuilder. An operator now federates
with a partner from goatbuilder alone: one decision (the partner, and what
it receives) and two confirmations, typed at the terminal.

### Added: `goatbuilder federate <partner>` runs the whole procedure

```bash
goatbuilder federate xab
```

At a terminal, one run does every phase. It asks for the partner's
verifying key and CA fingerprint (read aloud and typed back) and for the
agreement digest. It then makes the plan, shows it, and asks you to approve
the plan's hash. Next it shows what our core will admit and asks you to
approve that. Our core router restarts only after that approval. The run
then opens the transport, deploys the edge, federates SPIRE, and runs
acceptance last. A run that stops resumes where it stopped (`--no-resume`
starts over), and every phase is safe to run again. Without a terminal, or
in a dry run, it stops at phase 10 and names `goatbuilder federate plan`,
`deploy` and `test` instead. `--no-confirm` keeps the two confirmations by
hand as before. The verified agreement decides what may flow:
`federation.toml` owns the offered release sets, and a `peerings:` entry
(`offer:`, or `--offer`) may only narrow them.

### Added: `plan`, `deploy`, `test`, `status` and `backout`, each on its own

```bash
goatbuilder federate plan xab          # the bounded plan, written to <work>/plan.json with a stable hash; changes no host
goatbuilder federate deploy xab        # carry out the approved plan (--approve-plan <hash>), phases 10 to 13
goatbuilder federate test xab          # acceptance alone; evidence in <work>/evidence/acceptance/
goatbuilder federate status [xab]      # what each peering's journal and evidence say
goatbuilder federate backout xab --dry-run   # survey what backing one peering out removes, per host
goatbuilder federate backout xab       # remove that one peering and leave every other one as it was
```

`deploy` takes the admission by flag too: `--approve-admission <digest>`,
or `--review-in-pr` to write the summary for a pull request and stop.
`goatbuilder federate init|lint|compose|diff|verify|apply` are the package
verbs `goat federate` had, with the same flags and exit codes. They now run
in goatbuilder, and `goat federate <verb>` prints one line naming the
goatbuilder command.

### Added: `goatbuilder federate allocate [<peering>]` writes the per-peering files

```bash
goatbuilder federate allocate xab --dry-run   # show every file it would write
goatbuilder federate allocate xab             # one peering, beside peerings still written by hand
goatbuilder federate allocate                 # every declared peering
```

`allocate` gives each peering its own values: the tunnel interface, the
listen ports, the link subnet, the edge router port and the republisher
metrics port. No two peerings on one machine collide, and a re-run never
moves a value. A peering that already runs by hand keeps its values unless
you pin them or move it on purpose (`FED_ALLOCATE_MOVE`). For a new
peering, `allocate` renders the files a person used to write, for review
and commit:

- its inventory, `ops/ansible/inventory/peering-<edge group>.ini`, beside a
  hand-written `peering-edges.ini`;
- both deployment halves, the edge's and the core's;
- its `federation.toml`.

A peering that already runs by hand keeps its hand-written files.

### Added: values come from the declarations the deployment already has

The run derives what used to be exported by hand. A variable set in the
environment still wins, as an override.

- **Signing key and CA:** `secrets.federation` in `goatbuilder.yaml` names
  the signing key, our CA certificate and our CA key (`signing_key`,
  `ca_cert`, `ca_key`). A relative path is read inside the goat context.
- **Peering directory:** `environments[].peering_directory` names its `url`
  and, when it is not a public one, its `ca_cert`.
- **Machines:** a peering without `hosts:` takes its edge and core from the
  site inventory.
- **Site values and core certificate:** the site values the rendered halves
  need, and the core certificate's names, come from their owners.
- **Phase 14's surfaces:** our core's REST endpoint, the edge log and the
  SPIRE server come from the core and edge declarations.

A declaration that is ambiguous stops with a catalog error that names
what was looked for and where.

### Added: admission on a core deployed by the goat-zenoh role

On a core deployed by the goat-zenoh role, phase 10 needs no restart
command. Admission finds the core router's inventory group, its `site_id`
and the role's play from the role's own declarations. It reads and places
the org fragment at the role's `goat_zenoh_acl_org_path` on the core host,
and places our router leaf where the role reads it. It writes the partner
into the role's `zenoh_sovereign_peerings`
(`group_vars/<core router group>/peerings.yml`) as part of the reviewed
change. It runs the role's play with `--check --diff` as the dry run, then
for the restart. The slot check is not applicable when the core keeps no
committed slot list. `FED_CORE_PLAY`, `FED_CORE_RESTART` and the other
`FED_CORE_*` settings stay overrides.

### Deprecated: `goat federate <verb>`

`goat federate <verb>` still runs in 0.5.0 and prints one line naming the
`goatbuilder federate` command. It is removed from `goat` 30 days after the
0.5.0 release, and not before one peering between two standing environments
has been completed with `goatbuilder federate`. After that, `goat federate`
answers only with an error that names `goatbuilder federate`. The script
driver `ops/scripts/peering-standup.sh` stays as it is: documented, not
supported (ADR 1201), running the same fourteen phases.

## 0.4.3 — 2026-11-08

### Changed: `update` takes its release signer from the checkout

`goatbuilder update` verifies a new binary against the repository named by
`repo.release_source` in `ops/goatbuilder/goatbuilder.yaml`, else `repo.source`;
no repository name is compiled in. The config accepts `repo.release_source`.

### Added: `join-node` turns on signal federation for a joiner's core node

After the joiner's Spock wiring, phase 13 runs the signal-federation steps
(docs/design/signal-federation-pair.md): a gate,
`ops/scripts/signal-federation-check.py <site> --peer <founder> --resolve
--require-env`, that refuses a half-configured pair from the repository;
`signal-federation-transport.yml` on the joiner, then the founder;
`traefik-acme.yml` on both; the joiner's relay tunnel; the founder's relay pin
(`relay-ingress-add-site.yml`); and `signal-federation-verify.yml`, which
proves an offer forwarded and delivered. A joiner declaring
`signal_federation_absent_reason` gets the gate and a SKIP line.

### Added: `join-node --via door`

`join-node` gave every play `-i inventory/mgmt-hosts.ini -i
inventory/mgmt-hosts-mesh.ini`, so it ran only from a machine on that
goatnet's mesh. `--via <mesh|door>` (default `mesh`) picks the transport;
`door` gives every play `-i inventory/mgmt-hosts.ini -i
inventory/doors-groupvars.ini` and never the mesh inventory too
(docs/design/env-doors.md). The plan's first line is the door step,
`ops/scripts/door-open.sh <env>`, and a run opens the door before phase 1,
refusing if it does not open.

### Changed: `join-node` phases 2, 6 and 13 at a joiner obs site

- Phase 2 first probes `ssh -o BatchMode=yes <user>@<address> sudo -n true`
  and skips `seed-ssh-key.sh` with a note when it passes (a box installed
  unattended with the operator's key and NOPASSWD sudo). The dry-run prints
  the probe.
- Phase 6 no longer masks every failure of the first `wireguard-kmod.yml`
  pass. It passes only when the run succeeds, or when its sole failure is the
  role's `verify wt0 is kernelspace wireguard` check reporting that `wt0` does
  not exist (the node is not enrolled yet). Any other failure stops the run,
  with the play's output kept in a temporary file the message names.
- Phase 13 for class `obs` at a joiner site (the core group's
  `goatnet_founder: false`) skips `utility-host-bootstrap.yml` (Forgejo, the
  pub/sub state-of-record primary): the state of record stays with the
  founder. The plan line says so, as a `SKIP` phase.

### Fixed: a joiner's bootstrap wires its peers, and Ory replicates both ways

`join-node` (phase 13) and `standup` ran a joiner's bootstrap as
`site-bootstrap.yml -e site_id=<site>`. An extra variable sets site_id on every
inventory host, so the Spock roles' walk over the other sites found none and
wired zero peers. A joiner's passes now run `site-bootstrap-for.yml -e
bootstrap_site=<site>`, which sets site_id as a play variable, or the site's
own `<site>-bootstrap.yml` when it keeps one.

- Ory (Phase 7D) is staged across both sides: pass 3 builds the joiner's
  ory_narrow node and repset only (`-e ory_spock_wire_subscriptions=false`);
  the founder-side mirror now runs Phase 7D as well as 7B
  (`spock-mesh-wire.yml`, `ory_spock_reconcile_history: false`); then a new
  step re-runs `spock-mesh-wire.yml` on the joiner, which subscribes to the
  founder and reconciles revocation history in both directions.

### Fixed: `join-node` runs every node against its own class's group

An obs, breakglass or network-jump node was baselined, kmod-converged and
gated against the site's CORE group: `-e target=<site>_hosts -l <obs node>`
matches no host, so the play passed having done nothing. Phases 3-12 now use
the class's group, and an obs node's config preflight runs as
`standup-config-preflight.py <site>_hosts --obs <site>_obs_hosts`.

- Phase 12 refuses any `TBD_` left in the node's group_vars. It only tripped
  on a line carrying the node's name, which a `mesh_ip_a: "TBD_…"` never does.
- Phase 7 refuses the unnamed default CA (`<mode>-goat-ca-<date>`). It refused
  any `ca_id` starting `dev-`, which is the key-mode prefix every goatnet's own
  CA id carries.
- Phase 8 names the dashboard of the goatnet a joiner enrols into (its founder).
- Phase 1 says the Jetson USB-bridge address applies to a Jetson only.
- Phase 9 passes the goatnet's enrolment apex (`site_enrollment_dns`) and the
  setup key over stdin; it checked a placeholder URL with no key, and its
  command carried an unclosed quote.
- Phase 13 reads secrets inside the play (`lookup('env', …)`) instead of
  `"$(printenv …)"` on the command line, which the calling shell expanded to the
  unresolved `op://` reference before `with-secrets` ran, and put values on argv.

### Fixed: `wireguard-kmod` on a kernel with WireGuard in-tree

A stock distribution kernel ships WireGuard; the role now ends for such a host
instead of refusing for want of prebuilt artifacts.

## 0.4.1 — 2026-11-02

goatbuilder names no deployment of its own: what a person reads uses
placeholders, and `deploy` and `release` honour the checkout's values.

**Upgrading from 0.4.0:** nothing changes for goat, whose values are declared in
`ops/goatbuilder/goatbuilder.yaml`. Another project should declare `deploy:`
(`aws_profile`, `ssh_user`, `state_dir`); with no `deploy.ssh_user`, a real
`join-node` refuses. `fastlane` refuses a checkout with no `repo.source`.

### Changed: goatbuilder names no deployment of its own

Help text, error messages, the error catalog, the reference pages and the tutor
use placeholders instead of goat's goatnets, profile and organisation; lessons
that run against the checkout take their goatnet, site and component from it.
`deploy`, `release`, `doctor`, `federate` and `standup` read the AWS profile
and image organisation from the checkout, so `GOAT_AWS_PROFILE` and `GHCR_ORG`
work on every verb. `deploy.state_dir` declares the local state directory
(undeclared: `~/.goat/state`). See docs/design/goatbuilder-portability.md.

## 0.4.0 — 2026-11-02

Plain pages a person can act on, a `doctor` that prompts for nothing, an
`env-file` default store, placeholder acks refused everywhere, and fastlane
working again with identity and control-plane components refused.

**Upgrading from 0.3.0:** `doctor` checks stored credentials only with
`--credentials`; `fastlane --set` is gone; `--ack "<your name>"` is refused;
a checkout that declares no secrets store now gets `env-file` (goat declares
1Password in `ops/goatbuilder/posture.yml`, so goat is unchanged).

### Added: a project declares how it is deployed

An optional `deploy:` block in `goatbuilder.yaml` holds `aws_profile` and
`ssh_user`. `join-node` reads both and refuses a real run with no SSH user
declared; `adopt` takes the image organisation from `repo.image_namespace`, and
`--env` completion offers the stage names of the checkout. See
docs/design/goatbuilder-portability.md.

### Changed — the human surface (#2341)

A first operator walked `goatbuilder tutor` end to end; everything a person
sees was rewritten against docs/design/goatbuilder-output.md. `-o json`,
`-o yaml` keep their rows, with one addition: a check that did not run carries
`"skipped": true`. Piped output is unchanged except that `doctor`, mock `status`
and `federate` print their pages on a pipe too.

- `doctor`: a summary page — how many checks passed, then one entry per problem
  with its fix; `--all` prints every row. Secrets are judged by what this
  machine holds, not what one command loaded, and no fix suggests
  `aws-vault remove`.
- `status` and `diff`: one table (COMPONENT, MACHINE, STATE, RUNNING,
  DECLARED). Pending, drift and unknown are states, never `FAIL`.
- `history`, `promote status`, `survey`, `endstate`, `assay --plan`,
  `explain --list`: titled columns on a terminal.
- `deploy`, `verify`, `rollback`, `release`, `fastlane`: titled checks in plain
  English (stage, wait before prod, test version). A dry run counts its
  commands, lists any secret that would stop a real run, and ends with what
  actually happened. The page keeps one line per machine, and a failure names
  the machine to fix.
- `standup`, `teardown`, `join-node`, `federate`: a plan is a table plus the
  steps you do yourself, laid out like tutor steps, and says up front that
  nothing has run.
- `tutor`: 18 lessons (the old lesson 11 split into three), `--lesson N`
  reopens a finished lesson, each ticked step prints the next command, the
  first page welcomes the learner (the ASCII banner and the progress line's
  goat are gone), and all 29 locales moved with their lessons.
- `completion` survives a closed pipe (`goatbuilder completion zsh | head`).

### Fixed — fastlane works again (#2341)

`fastlane` refused every component: it read the retired `goat-maintainer` GHCR org
first (ADR 1195) and only the newest 40 image versions. It now reads every
page of `ghcr.io/usareur-af` only, honours `deploy_playbook:` through the same
resolver as `push`, refuses a component whose declared rollback is not a pin
revert (ADR 1127), and drops `--set`, which claimed to ship a whole graduation
set while moving one component.

### Fixed — pages that claimed what they had not seen (#2341)

An independent read of every verb's output, and of the diff, found pages that
said things that were not so. Each is now what it observed.

- `doctor` reported "All N checks passed" on a machine whose 1Password session
  was closed and whose credential store was locked, and exited 0 where it used
  to exit 1: not reading a credential (so that nothing prompts for a password)
  had been recorded as a pass. Not checked is now its own state, counted
  separately, named on the page, and shown as `skip` in `--all`. Its component
  count counted rows, so one component on three goatnets read as three; a
  variable templated twice was named twice inside its own count; and a play
  that takes no argument printed a bare `:;`.
- `update --check` answered an HTTP connect failure with an SSH diagnosis,
  because the X-SSH-002 catalog entry matched the bare words "Connection
  refused". Its patterns now carry the port ssh names.
- A machines table counted only the machines it held, so a deploy where two of
  five failed read "3 of 3 converged". A run with a failure in it reads as its
  steps.
- `federate --dry-run` printed "Nothing has run" and created the peering's work
  directory.
- An error document's `Why` listed every cause the error can have, which reads
  as a claim about this one. It is `Causes`.
- `promote review` cut the NEXT column mid-word, which is the only place a row
  says what to do, and then explained a token the cut had removed.
- `survey` ended without a newline, so a shell prompt landed on its last
  command.

### Changed — flags, rows and the tutor's log (#2341)

- `--mock <dir>` is refused by the verbs that do not read a fixture
  (`validate`, `doctor`, `release`, `fastlane`, `pins`, `promote`, `secrets`,
  `endstate`, `assay`, `join-node`, `update`), which used to accept it and read
  the real checkout. The refusal names the fourteen verbs that take one.
- `promote readiness` prints a page on a terminal (how many components are at
  the same version everywhere, how many are mid-release, what to do) instead of
  the 400-line document. A pipe, a redirect and `-o json` still get the
  document, unchanged.
- The tutor's log is written 0600 in a 0700 directory and capped at a megabyte,
  and `secrets exec -- <command>` is no longer logged at all: it carries
  someone else's command line.
- `assay --plan` calls its rows checks, as every other verb does, and prints
  AREA, CHECK and EXPECT; how each check is made stays in `-o json`.

## 0.3.0 — 2026-10-31

Readiness and reversibility before a rollout, a boundary-node worked example,
and `goat`-CLI parity on top of the 0.2.0 verb set.

### Added — `preflight` (ADR 1211)

`goatbuilder preflight <component> --env <env>` prints three facts before a
rollout — State (stateless | stateful), Rollback (pin-revert |
restore-required | none, with the restore pointer and retention window) and
Scope (single | coupled) — and `deploy` runs it as its first (prep) phase. The
facts are declared per component beside `artifact_type` in fleet-pins
(`owns_state`, `rollback`, `restore_from`, `coupled_with`); coupling also
derives from an open graduation set (ADR 1125). An undeclared component fails
closed to stateful / rollback none and grades not-clean (GB-076) — a gap made
visible, never guessed green. A declared rollback is trusted only where it
agrees with the recorded artifact type (ADR 1127's safe-class rule); a
`none`/`restore-required` rollout proceeds on `deploy` only with `--ack`,
written into the deploy ledger row. Both avenues stay coherent (ADR 1201):
`deploy-component.sh --preflight` and `ops/scripts/rollout-preflight.py` print
the same facts.

### Added — `federate --mock`; tutorial 3; in-tree fixture renamed

`federate` now honours `--mock <dir>`, loading the fixture's flat
`goatbuilder.yaml` the way `standup` does, so a worked-example peering is
planned self-contained and an undeclared peering is refused (the plan's
commands still name the repository's own federation scripts). Adds the
boundary-node worked-example step and tutorial 3. The in-tree worked-example
fixture directory is renamed `ops/goatbuilder/fixtures/hello-galaxy/` ->
`ops/goatbuilder/fixtures/goatnet-from-scratch/` (and
`sync-hello-galaxy.sh` -> `sync-goatnet-from-scratch.sh`): in-tree this is only
the tutor's fixture, named for what it is; the standalone `hello-galaxy-*` demo
repos and the tutorial cross-references are unchanged.

### Added — CLI parity with `goat` (`completion`, `docs`, `--color`, `-v`, `--timeout`)

`goatbuilder completion <shell>` prints a shell completion script; `goatbuilder
_complete <site|component|env> <partial>` (hidden) feeds live candidates — the
sites, components and goatnets this checkout declares — to a shell hook.
`goatbuilder docs --markdown` emits the full command reference, walked from the
live clap tree, so it can never drift from `--help`; the committed copy is
`docs/reference/goatbuilder/command-reference.md` and a hermetic test
regenerates it and refuses a diff. New globals: `--color auto|always|never`
(honouring `NO_COLOR`/`CLICOLOR_FORCE`) colours the table's glyph and verdict
only; `--verbose`/`-v` logs the external commands a run shells out to and
`update`'s network phase to stderr (`RUST_LOG=goatbuilder=debug`); `--timeout`
(`GOATBUILDER_TIMEOUT`) bounds the connect/reach phase so an unreachable surface
fails fast, without capping a play or the stall-bounded binary download.

### Added — `update`

`goatbuilder update [--site <site>] [--check] [--force]` replaces the binary
with the release `get.<site>.<zone>/goatbuilder/` serves, as `goat update`
does: `.sha256`, `SHA256SUMS` and (with cosign on PATH) the signature checked
before anything is touched, atomic swap, a 30 s stall bound on the download.
`--check` exits 3 when behind. Other verbs print one behind-version line on a
terminal at most once a day (`GOATBUILDER_NO_VERSION_CHECK=1` silences it);
`doctor` adds a `goatbuilder version` row. New errors GB-072..GB-075. The
platform, version-compare, checksum and replace code is shared with `goat` in
`crates/goat-selfupdate`.

### Changed — phase numbers (breaking for `--from`/`--to`)

Every numbered sequence an operator sees is now whole numbers from 1 with no
gaps and no letters, in both avenues (`goatbuilder` and the scripts).

`standup` and `teardown` (and `goat-standup.sh`, `goat-teardown.sh`):

| Old | New | What |
|---|---|---|
| 0 (gates, incl. 0b host preflight) | 1 | gates: config, inputs, ACL, door, hosts |
| 0 (host baseline) | 2 | host baseline + LUKS |
| 4 | 3 | per-instance secrets |
| 5, 5b | 4 | leaf certs + name snapshot |
| 6, 6b | 5 | mgmt stack, relay-connect |
| 7, 7a, 7b, 7c | 6 | identity stack, re-run site-bootstrap, joiner seed + mirror |
| 9, 9b, 9c | 7 | public TLS + relay tier |
| 10 | 8 | mesh enrolment (MANUAL) |
| 11, 11b | 9 | router + SPIRE |
| 12 | 10 | dashboard + OIDC clients / LAN services |
| 12m | 11 | first login + bootstrap NetBird token (MANUAL) |
| 13 | 12 | declared ACL, service identities, services (the `--parallel` waves) |
| 14 | 13 | acceptance gate (`assay`) |
| 15 | 14 | rescue nodes + cross-goatnet surfaces |

`--from`/`--to` take the new numbers only. A lettered id (`6b`, `7b`, `12m`, …)
and `0` are refused (exit 2) with the phase they became; an old bare number is
read as the new one, so check the table before resuming a run started on an
older binary. The observability legs are named `host` and `stack` (were
`O1O2`, `O3`); their logs are `standup-<site>-obs-host.log` and
`-obs-stack.log`.

`federate` (and `peering-standup.sh`): phases are 1..11 (were 0..10; every
number moved up by one).

### Added

- `endstate <site>`: is the goatnet in its declared end state (ADR 1206)? The
  end state is derived from the site's own declarations and the service catalog
  (`ops/goatbuilder/endstate/catalog.yaml`), so a site without Grafana or the
  sandbox, or on a single machine, says so in its group_vars. Every declared
  service is graded deployed, active, healthy and functional (the assay's rows),
  with its edges: served name and certificate, OIDC client, SPIRE entry; anything
  running that nothing declares is an orphan. Exit 3 while anything is off
  (GB-068), 1 when it cannot run (GB-066); `--facts`, `--assay-rows`,
  `--no-assay`, `--strict`, `--root`. The script avenue is
  `ops/scripts/goatnet-endstate.sh`; `parity-endstate.sh` holds the two alike.
- `assay` runs its Python under the interpreter that has PyYAML (python3 when it
  does, else Ansible's own), as the deploy path does since #2120.

- `assay <site>`: prove a built goatnet layer by layer — every applicable row
  of the acceptance manifest, including the goat CLI rows, graded per layer;
  exit 3 while any row fails or cannot answer (GB-048), 1 when it cannot run
  (GB-049). `--plan` lists the applicable rows; `--values` takes a fixture's
  value set. Standup phase 13 (both avenues) prints the same per-layer table.

### Changed

- The long-running verbs (deploy, rollback, release, fastlane, standup,
  join-node, federate, teardown, apply, assay) hold the Mac awake for their own
  lifetime with `caffeinate -i -w <pid>`, as the shell drivers now do; one
  assertion per run, `GOAT_NO_CAFFEINATE=1` opts out (#2122). `doctor` shows
  whether `caffeinate` is on PATH on macOS.

## 0.2.0 — 2026-10-29

The release that runs the tutorials: every verb the design names is in it.

### Added

- **Sites:** `standup` (founder and joiner sites, `goat-standup.sh` phase for
  phase, `--dry-run`, `--from`/`--to`, `--parallel` phase-13 waves, the
  required-input scan as phase 0's gate), `join-node` (a machine joining a live
  goatnet, node-addition.md as fifteen phases), `federate` (`peering-standup.sh`
  phase for phase), `teardown` (fixture, and `--services` on real hosts).
  `standup --mock` rehearses a site on a fixture.
- **Release ladder as typed verbs:** `pins` (`generate-pins.py`'s resolution,
  `--check`, the typed deviation edits) and `promote` (`graduate.py`
  subcommand for subcommand). `release` and `fastlane` use them in process.
- **Reading state:** `verify`, `diff`, `history`; `plan deploy|rollback` and
  `apply <plan>`, which refuses when a planned fact moved; `adopt`, which
  proposes pins for what hosts already run.
- **Secrets posture (ADR 1204):** `secrets` — the declared store (1Password,
  OpenBao, sops, git-crypt, env-file, env, custom) and `secrets init`, the
  zero-SaaS quickstart.
- **Errors:** `explain <code|text>` over the shared catalog
  (`ops/errors/catalog.yaml`), including another tool's output embedded in an
  error.
- **Learning:** `tutor`, eleven lessons run with the real verbs on fixtures.
- **Languages:** every string in 29 languages; `--lang` / `GOAT_LANG`.
- **Speed:** SSH multiplexing for direct hosts (6.4 s → 1.5 s per Ansible
  call, M-120) and per-phase timing.
- **Tests:** contract clauses C1–C10 as tests; parity gates against the
  scripts for deploy, verify, history, release, rollback, fastlane, pins,
  promote, standup and teardown; the three-tier e2e harness (ADR 1200).

### Changed

- `deploy`, `rollback` and `standup` on a fixture (`--mock`) need
  `--ack "<name>"`; without it they exit 1 with GB-047 before anything is
  published. Every record names its actor (C4).
- Published declarations on a fixture are sealed and ordered (C2, GB-045); a
  fixture's local copy that is ahead of or diverged from its source is refused
  (C3, GB-046).

### Fixed

- `rollback --list` prints one row per ledger entry under either `yq`
  (#2066); a rollback whose target is another image is refused with exit 3.
- `release --from dev` reuses preprod's open record for the same ref instead
  of opening a second one (#2086); a re-declared deviation stays in place, so
  a re-run lands nothing.

## 0.1.0 — 2026-10-28

First signed release: `validate`, `doctor`, `status`, `deploy`, `release`,
`rollback`, `fastlane`.
