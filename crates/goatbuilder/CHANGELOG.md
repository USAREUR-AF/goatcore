# goatbuilder changelog

Each release is the tag `goatbuilder/v<semver>` on `main`, built and signed by
`.github/workflows/goatbuilder-release.yml`. Design of record:
`docs/design/goatbuilder.md`; conformance: `docs/design/goatbuilder-conformance.md`.

## Unreleased

<!-- BEGIN generated: changelog unreleased, from crates/goatbuilder/changelog.d; edit the entries, not this section -->
Changes since the last release are one file each in `crates/goatbuilder/changelog.d/`; `python3 ops/release/changelog.py show` prints them as the next release will.
<!-- END generated: changelog unreleased -->

## 0.8.0 — 2026-11-26

What changed since 0.7.3, the previous release: `goatbuilder deploy` verifies
the goatnet's signed intent, requires a digest on every pin and applies the
release ladder before any host, and its dry run exits 1 when a real run would
refuse; a pins component may ship as a signed release file or bring its play
from another repository at a pinned commit; machine types are read from files,
with two new types, the federation boundary and the network tap; every error
goatbuilder raises carries its catalog code. Some exits change from 2 to 1 by
the contract's meanings; each is listed below.

### For someone deploying

#### Added: phase 14's acceptance can fire its probes on one machine and read the edge's trace from another

For a peering driven from two machines, `acceptance.sh` takes
`FED_ACCEPTANCE_TRACE=deferred`: it fires every probe, reads no edge log,
records the five rows that need the edge's ACL trace as PENDING, and writes the
ids it fired and their window to `<work>/acceptance/ids.json`. A PENDING check
is required and never a pass, so this half exits 3 (INCOMPLETE). On the edge,
`ops/federation/standup/edge_trace_filter.py --ids … --window …` keeps only
the log lines that carry one of those ids, within the window and 5 minutes
either side. `FED_EDGE_TRACE_FILE=<that file>` then decides the pending rows
by id, fires nothing, and refuses a file for other ids or another window.
With neither set, phase 14 runs as before. (#961)

#### Changed: the access-rule cutover has a cut-off mode that only updates existing policies

`ops/ansible/netbird-acl-cutover.yml -e cut_off=true` is for a site cut off
from the others. It updates the access-rule policies the site already has, by
their ids, and never creates or deletes one: NetBird assigns a created
policy's id, so two cut-off sites that each created the same rule would leave
two policies after the heal. A run that would create a policy, reap one
(`reap_orphans=true`), delete the Default policy, or tear down refuses with
GB-295 before it writes anything, naming each policy it would have created and
each id it would have deleted. Disabling the Default policy is an update and
still runs. `netbird-acl-bootstrap.yml`, which standup runs before the
cutover, honours the flag too: a missing group is refused with GB-295, since a
created group's id is NetBird's as well. The flag is the operator's statement;
nothing detects a cut. After the heal, run both at the hub without `cut_off` to
make the creates and deletes. Without `cut_off` both run as before (#708).

#### Added: ops/goatbuilder/c3-measure.sh runs gate row C3's check on a standing goatnet as one command

`ops/goatbuilder/c3-measure.sh --env <goatnet> --out <directory>` runs
`doctor`, `status`, `diff` and `verify` for each component `status` names,
then `ops/goatbuilder/c3-agreement.py`. It changes nothing on a host, in a
ledger or in the goat context; a door the verbs open for it is closed at exit,
and one already open is left open. Each verb is stopped after
`C3_VERB_TIMEOUT` seconds (default 600). It times each verb and keeps the
reports and a `record.json` in the directory, mode 0700, or in a new
directory under `$TMPDIR` when `--out` is missing. Its last line is one
summary that names no host, and shows the goatnet only as `<env>`. It exits
0 when the three verbs agree on every machine, 3 on a disagreement, and 2
when the run is not a measurement, a machine not reached included. A value
shaped like a credential, and the value of an environment variable that
`ops/ci/check-no-live-secret.sh` treats as a live secret, is left out of
`record.json`, the summary line and the kept stderr.

#### Added: a component's play may be in another repository, at a pinned commit

A component of `ops/pins/fleet-pins.yaml` may declare `source:` with
`repository`, `commit` (a full commit) and `play`, in place of
`deploy_playbook`. `goatbuilder deploy` and `ops/scripts/deploy-component.sh`
fetch that commit with your own git access, hold the copy to it on every run, and
run its play; nothing of it is put in the checkout. A source that cannot be
reached, is not exactly the commit, has no such play, or carries a role goatcore
has is refused before any host (GB-385). The script needs goatbuilder on PATH, or
`GOATBUILDER_BIN`.

#### Fixed: the stack's watcher no longer starts a joiner's old Postgres during its re-initialisation

A joiner whose existing Postgres cluster has another encoding than its
founder's is refused by `site-bootstrap.yml` (the play `goatbuilder join-node`
and `standup` run at phase 13 and 6), which says to stop the stack, remove the
`pgdata-a` volume and re-run. While the stack was stopped,
`mgmt-host-compose-watch` started the old `postgres-a` again within 30 s. The
watcher now honours a pause file, `/run/mgmt-host-compose-watch.paused`: while
it exists it revives nothing, logs that it is paused, and reports
`compose_watch_paused` 1. The refusal says to pause it first with the reason
`postgres-reinit`, and the re-run clears that pause, and only that one, once the
cluster has the founder's encoding (#1128).

#### Fixed: the stack's watcher starts only after a deploy has brought the stack up, and is paused during it

`mgmt-host.yml`, which `site-bootstrap.yml` runs in `join-node`'s phase 13 and
`standup`'s phase 5, installed and started `mgmt-host-compose-watch` before it
wrote the new compose files and ran `up -d`: a play's roles run before its
tasks. On a joiner's re-initialisation the watcher then started the old
`postgres-a` from the old compose file on the emptied volume, so Postgres
initialised without the founder's encoding. The watcher now starts after the
stack is up, and the deploy pauses it from its first step to its last
(`/run/mgmt-host-compose-watch.paused`, reason `mgmt-host-deploy`, kept if
another pause is set). A deploy that fails in between keeps the pause and says
so, with the command to clear it, and the new alert `ComposeWatchPaused` fires
when a pause lasts over 30 minutes. A reboot clears it (#1172).

#### Fixed: deploy applies the release ladder (GB-373) as deploy-component.sh does

`goatbuilder deploy`, dry run and real, now judges every member group's
declared digest against the release ladder (ADR 1120) before the secrets, the
transport or any host, with `ops/release/ladder_gate.py`, the module
`deploy-component.sh` calls. It refuses in the script's words when the rung
below pins another digest, or when no rung below pins the component at all
(fail-closed), and passes the lowest rung, a goatnet that is not a live rung,
and an open hotfix record. When the rung below pins the digest, a real run reads
that rung's ledgers (`--verify-only`) and refuses unless it converged; a dry
run names that read. Before, a dry run said it would deploy where the script
refused, and a real run met only `ansible-preflight.sh`'s gate for direct
plays, which does not read the rung below's ledgers.

New flags `--skip-rung --reason "<why>"`, with `--ack "<operator>"`, take the
recorded exception: the gate refuses either half alone, and the reason and the
ack go into the deploy ledger row (`ladder_skip`). `--reason` without
`--skip-rung` is refused. A gate that cannot run refuses. `--verify-only` is not
judged, and `release --dry-run` is not judged either, as `rc.sh`'s dry run runs
no deploy (#1187).

#### Changed: `deploy --dry-run` runs every check that touches no host, and exits 1 when a real run would refuse

A deploy dry run, with `goatbuilder deploy` or `deploy-component.sh`, now runs
the fresh-source check and the secrets gate as a real run does. Each refusal is
a failed row, the whole plan is still printed, and the run exits 1 (refused).
A release-ladder refusal, which ended a dry run before its plan, is reported the
same way. Before, a dry run ended `ok`, exit 0, on a tree whose pin had been
edited and with a required secret unset. A dry run now fetches the trunk
(`--offline` judges against the cached one).

Deploy itself refuses a pin with no `@sha256:` digest, before any host, in a real
run and a dry run on both avenues (new code GB-382). The pin is the pins.yml
value as YAML reads it, so a digest in a trailing comment does not count, and
the digest must be exactly 64 lowercase hex characters. `verify` and
`--verify-only` are unchanged.

`goatbuilder deploy` no longer opens a goatnet's door before refusing a run for
an unset secret; `deploy-component.sh` already refused first.

#### Changed: deploy verifies the goatnet's signed intent before any host

`goatbuilder deploy` and `deploy-component.sh` verify the goatnet's signed
intent from the checkout, with no network, before any host is touched: the
intent is there, its `content_hash` is its body's, its signature verifies
against the committed public key of its CA, it is what the tree renders, and
each member group's `pins.yml` is what the pins table renders. These are the
checks `goatbuilder pins --check` makes, for this one goatnet. One that does not
hold is refused (new code GB-383, exit 1); a dry run reports it after the whole
plan. `--verify-only` is not judged.

**A goatnet whose intent was never signed, or whose pins moved without signing
again, can no longer be deployed with either tool** until its intent is signed
and committed. `deploy-component.sh` needs the Python module `jsonschema`
beside PyYAML, as `ops/pins/generate-pins.py` does. The public key is read from
the checkout, so this check proves the pins are what that key signed, and not
that the checkout's key is the right one.

`ops/pins/generate-pins.py --check --goatnet <name>` is the script's half.

#### Added: `doctor --target <type> --site <site>` checks a site's machines of one type against its file

`goatbuilder doctor --target <type> --site <site>` (and `--node <machine>` for
one) reads the type's file and the checkout's inventory: the platform family
each machine of the type declares, accepted or refused as `join-node` would
(GB-376), and whether the type's plays are in this tree. It reaches no host and
changes nothing. The machine's end state (OS version, encryption, chrony before
docker and the rest of the list in docs/design/goatbuilder-build-targets.md
section 3.5) is a row that says it is not checked yet.

#### Changed: `goatbuilder endstate` grades pgBackRest active from its acceptance rows, and the unused sidecar is removed

pgBackRest runs inside the database container, and nothing ran the
`pgbackrest-runner` sidecar. `goatbuilder endstate` used to grade pgBackRest
active when that sidecar container was running. A service with no container of
its own is now active only when its acceptance rows pass (pgBackRest's stanza
`info`). It is FAIL when one fails, and UNKNOWN when none ran. An empty
container list is never read as running.

To take it off a host, deploy the pgbackrest role; it removes the retired sidecar
there: the container, running or stopped, the overlay
`docker-compose.pgbackrest.yml`, and the `PGBACKREST_IMAGE` line in `.env`. The
bind-mounted repository is not touched. Before that, the role
stops and names any file in the repository, log or spool directory that the
database user does not own. A pgbackrest command run as root, as the sidecar did
when exec'd into, leaves such files, and backups and restore then fail. The
role changes no ownership itself.

#### Fixed: the errors the rest of goatbuilder raises carry their catalog code

Sixty-four more errors had no catalog code, so a log of a run that met one
(stderr not a terminal, no explanation page) showed only `goatbuilder: <text>`.
Each now prints its code. They use existing codes where one fits: a file
goatbuilder reads or writes GB-254 (a pins file, a run record, the survey page,
teardown.yaml), a program that could not be started GB-320 (bash, ansible, ssh,
git, go, a checkout's script), the mock converger's fixture GB-024, the config
GB-025, a plan file that is not JSON GB-052, a missing deploy playbook GB-005,
a component block fleet-pins.yaml lacks GB-257, the assembled tree's marker
GB-259, a release graduation that printed no record GB-020, a git command
`doctor` runs that failed GB-258, an internal error GB-322, and a mutation
without a terminal GB-008. One is new: GB-326, a file of the checkout that is
not YAML (fleet-pins.yaml, doors.yaml, a role's deploy-history.yml,
teardown.yaml).

Five checks inside `goatbuilder federate` now carry a code: phase 1's and the slot
recheck's exit disagreeing with its result GB-315, phase 1 writing no results and
an edge result naming the wrong side GB-321, and an imported edge result that
changed while it was read GB-277.

Eighty-four more errors were a file, program or JSON/YAML error passed up as it
came, with no code. Each now carries one: a file that cannot be read or written
GB-254 (it names the file), a value that cannot be written as JSON or YAML
GB-322, a program that could not be started GB-320, fleet-pins.yaml that is not
YAML GB-326, a mock fixture's pins GB-024, and the end-state grader's output
GB-066. By file, under `crates/goatbuilder/src/`: converger.rs 10,
doctor.rs 2, endstate.rs 1, fastlane.rs 1, federate/backout.rs 3,
federate/handoff.rs 3, federate/measure.rs 1, federate/peering_files.rs 3,
federate/plan.rs 1, federate/rotation.rs 3, federate/state.rs 1,
federate/status.rs 2, inspect.rs 8, pins.rs 2, promote.rs 2, push.rs 1,
release.rs 5, runs.rs 3, standup.rs 10, survey.rs 2, tutor.rs 13, verbs.rs 7. One that carries GB-322, GB-320 or GB-066 exits 1, where it exited 2 (on
the federation verbs, an internal error already exited 1); the others still
exit 2.

Exit codes, by the contract's meanings (1 refused: a precondition failed; 2
usage: the command line was wrong). Each of these errors exited 2 with no code.
Every place here is on goatbuilder's own error path, where an uncoded error exits
2; none is on the federation verbs' path, where it exits 1. One that now carries
GB-020 (`release`), GB-025 (`validate` and every verb that loads the config: it
does not match the typed model, or does not convert to JSON), GB-258 (`doctor`'s
git, whose error every caller reads with a fallback), GB-277, GB-315, GB-320,
GB-321 or GB-322 exits 1, since none is a usage error. The others (GB-005,
GB-008, GB-024, GB-052, GB-254, GB-257, GB-259, GB-326) still exit 2. Every other exit is unchanged.

#### Added: `goatbuilder explain teardown` and `explain federate-backout` answer for each of their steps

The phase catalog now describes `teardown` and `federate backout`.
`goatbuilder explain teardown` lists the standup phases teardown takes down,
in reverse (2, 5, 6, 7, 8, 9, 10, 12, 14), and `explain teardown 12` says
which of the phase's steps come down, which are kept, and which a teardown
refuses because reversing them destroys data or reaches another site.
`goatbuilder explain federate-backout` lists the back-out's seven steps
(survey, confirm, remove, recheck, core, directory, record) and answers for
each. Tests hold the entries to what each verb walks, so a new step cannot
ship without one. Nothing a run does changes.

#### Fixed: a failed step's row prints its own catalog code

When a step stopped a run, its `failed` row named the phase and the exit
code but no catalog code. A drill log that kept the terminal output then
held an error without a code whenever the interpretation under it named
none, or named only another tool's code (X-ACCEPT-001 at phase 14). Each
row now prints its code: GB-183 for a `federate` phase, and five new codes:
GB-331 for a `standup` phase, GB-332 for a `join-node` phase, GB-333 for
standup's image pull, GB-334 for an observability leg and GB-335 for a
`teardown` stop. No exit code changes.

#### Fixed: `fastlane` reads and pins an image under the owner its fleet ref names

A component whose image another organisation builds is pinned as an absolute
`ghcr.io/<owner>/<name>` ref. `goatbuilder fastlane` looked it up under this
checkout's own owner, and would have pinned it there. It now reads and pins it
under the owner the ref names, as `ops/scripts/fastlane.sh` does.

#### Changed: phase 6 composes the agreement only with the peering's declared partner

Phase 6 now tells `compose` which two parties the agreement is between: us
and the peering's declared partner (`FED_COUNTERPARTY_ID`, from
`goatbuilder.yaml`). A counterparty package of any other party is refused
(GB-191) and no agreement is written, even when the keyring binds that party's
key, as a keyring shared by several peerings can. Only the two parties' keyring
lines are passed to compose. `goat federate compose` takes the parties as
`--party <sovereign_id>`, once or twice; without it, compose behaves as before.

#### Added: a two-driver peering's phase 2 runs on the edge driver and the controller

A peering declared `drivers: two` used to stop at phase 2's hand-off with
nothing to run on the other side. Now the edge driver runs the hand-off's
command, `goatbuilder federate <peering> --side edge --from 2 --to 2
--expect-commit <commit>`. It makes the federation interface on the edge and
prints the way back with a confirm code. The result holds the interface's
public key, and no private key leaves the edge. On the controller,
`goatbuilder federate import <peering> 2 <file> --sha256 <code>` takes it in,
and `--from 2` runs the controller's half: the SPIRE bundle export on the
SPIRE server host, which never reaches the edge host. Phase 3's import then
refuses an edge half that measured another interface or another public key
than phase 2's result handed over (GB-114). When there is nothing to compare,
the import says in a row of its own that the key was not checked, and why. A phase 2 that also runs
a forwarder's steps is refused as not built (GB-267), as are the edge's own
leaf keys, which phase 8 still makes on the controller.

#### Added: the `federation-boundary` type

`join-node --class federation-boundary` builds the machine that peers with
another goatnet, on a small computer or a host-class machine. Its phase 13
installs nothing: `goatbuilder federate` makes each peering on it, from an
agreement both sides approve. `standup` builds none.

#### Fixed: a federation verb's fault prints a catalog code

The federation verbs (`goatbuilder federate init|lint|compose|diff|verify|apply`,
on their own or inside a peering run) printed a fault their library raised as
`goatbuilder: <text>` with no code. It now prints the new code GB-338, a
federation verb faulted, under the same message, which names what failed (a
file, a flag, a party). It still exits 1, as before and as `goat federate`
does.

#### Added: a peering's forwarder declares its machine's SSH user, interface and SSH sources

`forwarder:` on a peering in goatbuilder.yaml takes `ansible_user`,
`public_interface` and `ssh_sources`, the forwarder machine's own values.
`goatbuilder federate allocate` renders each declared one into the forwarder
group's `peering-forwarder.yml` (`ansible_user`,
`peering_forwarder_public_interface`, `peering_forwarder_ssh_sources`), so the
forwarder needs no hand-written host file. Peerings on one forwarder that
declare different values are refused (GB-339). A value left out stays where
it was: in your own group_vars file, or the role's default. (#934)

#### Fixed: under goatlite, the doctor and the browser step name the right phase and command

`goatbuilder doctor` says a goatlite's missing secrets file is created by
phase 1 of the standup, the phase whose "create the local secrets store" step
writes it; it said phase 3. The browser step that pauses a LAN standup at
phase 11 names `goatbuilder secrets hint <SITE>_NB_API_TOKEN` as the command
that says where to store the token, instead of the posture script behind it.

#### Fixed: a goatlite standup no longer points at 1Password or goatbuilder.yaml

Phase 6 of a goatlite standup told the learner to copy the bootstrap setup key
to 1Password. It now says to keep the key only until it enrols the first peer,
and only in the goatnet's local secrets store. Phase 1 told a goatlite to
declare `secrets: {store: env-file}` in `ops/goatbuilder/goatbuilder.yaml`, a
file a goatlite does not have; under goatlite it no longer gives that advice.
The full preset's text is unchanged in both places.

#### Changed: an IPsec rotation proves the leaf charon signs with, and checks the strongSwan version

`goatbuilder federate rotate <partner> ipsec-leaf` and `ipsec-ca` now check
more on the edge (`goat-peering-edge`). Before anything moves, the rotation
reads the running charon's version (`swanctl --version --daemon --raw`) and
refuses a strongSwan series it was not tested with, or a version it cannot
read, with GB-341. The tested series are 5.9 and 6.0, in
`peering_edge_rotation_strongswan_tested`; add a series there only after
running the rotation on it. It also refuses a staged leaf on the live leaf's
key (GB-290), and refuses, naming the peering, when another peering on the
edge lacks its certificate, key or CA files (GB-343). Once the new IKE SA is
up, the rotation reloads the credentials from the files alone
(`swanctl --load-creds --clear`), which drops the old certificate that a
plain `--load-creds` keeps listed. The confirm then needs charon to list
exactly one certificate of our id: the live leaf's serial, holding its
private key, on a key other than the old leaf's. Otherwise the old pair is
put back (GB-342), and the message names the serials. `swanctl --list-sas`
names only the identities, which an old and a new leaf share, so the SA alone
never showed which leaf it ran on. The confirm record
`.previous-<id>/confirmed` now also names the serial, both keys' digests and
the strongSwan version. Measured on a real charon, strongSwan 5.9.8 and
6.0.4, with one and with two peerings on the edge.

#### Added: join-node checks a joining site's own copy of the seed play before phase 13 (GB-362)

A site that keeps its own `ops/ansible/<site>-seed-from-peer.yml` has it
checked against the generic `site-seed-from-peer.yml` it was derived from, as
`goatbuilder standup` already did: `goatbuilder join-node` runs
`ops/scripts/check-site-play-current.py` as phase 13's first step at a joiner's
core node. A copy with no `# derived-from:` line, or one made from an older
generic play, stops the run there with GB-362, before any phase-13 play
changes the node. A site with no copy runs as before.

#### Fixed: join-node reads goatnet_founder as Ansible does

`join-node` decided whether a site joins a goatnet by matching the line
`goatnet_founder: false` in the site group's vars.yml. So `False`, `"false"`,
`no` and `off` read as a founder, and a `goatnet_founder` nested under another
key read as the site's own. It now reads the top-level key with Ansible's
booleans, as site-bootstrap does (`goatnet_founder | default(true) | bool`).

#### Fixed: a joiner's join-node needs no OWNER_EMAIL

A joiner's phase 13 passed `-e bootstrap_owner_email="$OWNER_EMAIL"` to its
bootstrap passes. A joiner's account arrives from the founder, so the value
was never used, but a run with no `OWNER_EMAIL` set stopped at phase 13
(unbound variable). A joiner's phase 13 no longer passes it. A founder site's
join-node still does.

#### Fixed: join-node reaches the node at its mesh address from phase 12 on

Phase 14 of `goatbuilder join-node` unmasks the apt timers over ssh. It used
`--address`, the node's LAN address, while every play from phase 12 on already
reached the node at the mesh address phase 10 records. A node whose LAN address
no longer answered after enrolment stopped the run at phase 14. Phase 14 now
takes the node's address from the mesh inventory; `--address` is used only by
phases 1 to 11. Through an env door (`--via door`), which does not reach the
mesh, phase 14 still uses `--address`.

#### Changed: join-node's phase 1 checks the node answers and goes on, so one run goes from phase 1 to 15

Phase 1 of `goatbuilder join-node` used to be a person's step, "reach the box
on the site LAN". Every run stopped there and went on only with `--from 2`.
It is now a check: the node answers over SSH at `--address`, or a phone answers
over USB (`adb get-state`). The run goes on to phase 15 with no operator step.

A box that refuses the login still counts as answering. Before phase 2 seeds
your key, a box that takes only its first-login password refuses every batch
login, and phase 2 proves the login.

A box that does not answer stops the run at phase 1 with GB-314 and the step
for a person. Resume with `--from 1`. So does a Jetson's USB bridge address,
192.168.55.0/24: its sshd answers, but it is reachable only over a direct
USB-C cable. Pass the box's real LAN address.

#### Fixed: join-node's phase 15 runs the acceptance rows against the site's end state

Phase 15 ran `run-acceptance.py` with no `--endstate`. A row for a service the
site declares absent read FAIL and stopped the run. Phase 15 now runs the same
acceptance line as standup: it writes the site's end-state model to the state
directory and passes it with `--endstate`, so such a row reads N/A.

#### Changed: join-node's phase 1 says which case it hit when ssh fails and prints nothing

When `goatbuilder join-node`'s phase 1 reach check gets an ssh failure with
no words, GB-314 now says which case it was: ssh exited with its code and
printed nothing on stderr, or the file its stderr went to was gone before the
check read it. Before, both read only `ssh exited <code>`. The check also reads
ssh's stderr before it stops its own timeout, and the timeout can no longer
remove the run's files. A node that answers, or that fails with ssh's own
words, is reported as before (#1048).

#### Added: join-node refuses a health publisher with no leaf in the site's manifest before any phase runs (GB-364)

A joining site's core node runs each health publisher the end-state model marks
present, with the leaf certificate phase 13 mints from
`ops/enrollment/manifests/<site>-certs.yaml`. A present publisher whose leaf the
manifest did not carry was found only when the staging play or the publisher's
role refused it, partway through phase 13. `goatbuilder join-node` now refuses
the plan before any phase runs, with GB-364, naming the publisher and the
manifest. A publisher the site declares absent needs no leaf.

#### Added: `goatbuilder join-node ... --reseed` seeds a node again after its founder lost the node's slot

A replicating node (class `core` or `phone`) away longer than its founder's
`max_slot_wal_keep_size` allows loses the slot the founder keeps for it, and
from then on receives nothing the founder writes. `goatbuilder join-node <env>
--site <site> --node <node> --class <class> --reseed` copies it again from the
founder in six steps, each one call of `ops/scripts/spock-reseed.py` over the
mesh: the slot is lost (GB-345), the founder has taken the node's own writes
(GB-346), the copy is well under the node's own slot cap (GB-347), the
subscription is dropped and created with no copy and each table copied again
after a local truncate, every table is copied with no slot lost (GB-348), and
every copied table is the same on both by count and digest (GB-349). The
container it reaches is `spock_mesh_postgres_container` (GB-350 when none is
set). Each step is journaled, a rerun starts at the step that stopped, and
`--dry-run` prints the six commands and runs nothing. The verb reads the
founder only; another site that lost its slot for the node is still re-seeded
by hand (docs/design/goatbuilder-hardware-measurement.md, "The re-seed in
goatbuilder").

#### Added: join-node refuses to leave a joiner subscribed over a seed the founder changed since (GB-367)

A joining site's seed carries the founder's data, and its Spock subscriptions
carry only writes made after they exist, so a founder write made between the
seed and the Spock wiring never reached the joiner, and nothing said so. Phase
13 now compares every seed table on the founder and the joiner (row counts and
checksums, read-only) before it wires Spock, and again after, once the
subscriptions from the founder have caught up. A table that differs stops the
run with GB-367, naming the table and its two row counts. Before Spock is
wired, re-seed and resume with `--from 13`.

#### Added: `goatbuilder join-node` runs a single-host joining site's observability stack

At a joining site with no obs host of its own (no `has_obs_host: true`), the
core run's phase 13 now ends with `ops/ansible/obs.yml` on the node: it
deploys Prometheus, Alertmanager and the obs exporters. Grafana is left off
(`grafana_enabled=false`), because its OAuth2 client would be written to the
site's Hydra, a write to the identity store at a second site that gate rows I1
and I4 hold back; the plan names it under "does not run yet" with gate row I3.

The step needs `obs_colocated_site_zones` in the site's group_vars (its site_id
mapped to its DNS zone) and `<SITE>_POSTGRES_EXPORTER_PASS` in the environment.
Without the declaration the step is a SKIP that names it, and the four
services are named under "does not run yet". `obs.yml` itself no longer asks
for Grafana's two secrets when the site sets `grafana_enabled: false`.

#### Added: `goatbuilder join-node` gives a joining site its secrets in phase 3

A joining site's core node used to need its secrets written by hand before
phase 13. Phase 3 now runs `ops/scripts/join-site-secrets.py`. The values
every site of a goatnet shares (NetBird's datastore key, the replicator, the
relay secret, Ory's network id, the Kratos cipher and the Hydra system secret,
the lines of `newsite-secrets.sh` marked `cluster`) are taken from the
founder's: as a reference to the founder's entry under a store of references,
or as the founder's value under a store of values. `newsite-secrets.sh` then
mints the site's own values, and each shared value is compared with the
founder's by digest, printing names only. Nothing is overwritten: a joiner
value that differs is refused and kept (GB-353), a founder value that is
missing or does not resolve is refused (GB-352), and under the `env` store a
joiner name that is not exported is refused (GB-354). `newsite-secrets.sh
--cluster-json` lists the shared names.

#### Fixed: join-node refuses a joining site whose data router and health publishers would name its Zenoh stack differently (GB-361)

`zenoh_stack` names a site's data router directories, its container and its
network. The router's role defaults it to `<site_id>-zenoh`, while each health
publisher's role defaults it to `goat-zenoh` (fabric-crawler requires it), and
Ansible gives each role only its own defaults. So a joining site whose
group_vars set no `zenoh_stack` got a router under one name and publishers that
looked for another. `goatbuilder join-node` now refuses such a site before any
phase runs, with GB-361, naming each publisher and the value it would read, and
the line to add: `zenoh_stack: <site_id>-zenoh` in `group_vars/<site>_hosts/`.
A site that sets it runs as before.

#### Fixed: a joining site provisions no bootstrap administrator of its own

`join-node` phase 13 runs `ory-deploy.yml` at a joining site, and the ory role
created a Kratos identity, with a generated password, for each `site_admins`
entry that site's Kratos did not yet hold. A joiner's identities arrive from its
founder by replication, so that gave each administrator a second identity at the
joiner. The role now runs its bootstrap-admin tasks only at a founder site:
`goatnet_founder` unset or true, read as site-bootstrap reads it. A founder's
run is unchanged.

#### Fixed: standup's and tutor's last uncoded errors, and pins' schema refusals, carry a code

Eight places in `goatbuilder standup` and `goatbuilder tutor` passed a system
error up with no catalog code, so a log of a run that met one showed no
code. A phase log that could not be opened for both of a phase's streams now
prints GB-254 and names the log. A tutor workspace whose fixture directory
could not be listed prints GB-254 and names the directory. Both still exit 2.
A failed wait for a phase, image pull or observability leg that standup
started now prints GB-322 and exits 1, as every GB-322 does, where it exited 2.

`goatbuilder pins`, and the verbs that run its check (`release`, the
preflight, the fast lane), refused a pins file that broke the pins schema with
no catalog code. Each of those refusals now carries the new code GB-365 and
names the file and the rule. It still exits 2. An unmigrated-pins list that
cannot be read now prints GB-254 and names the file, still exit 2.

#### Fixed: the last goatbuilder errors with no catalog code carry one

The last eight places in goatbuilder's source that raised an error with no
catalog code now raise one, so a log of a run that met one shows the code. The
federation verbs' context flags (`--context-dir`, `--cert`, `--ca-roots`,
`--context`, `--profile-dir`) and `--over-ssh` given to `goatbuilder federate
plan` or to a peering run print GB-239. `goatbuilder federate allocate
<peering>` naming a peering the config does not declare prints a new code,
GB-359. Both still exit 2. A key rotation's working files that cannot be
written print GB-254, still exit 2. A grown config that cannot be written as
YAML prints GB-322 and now exits 1, as every GB-322 does, where it exited 2:
a script that checks `goatbuilder grow`'s exit code sees the change. The check that holds the source to its list of uncoded errors now has
an empty list (#1093).

#### Changed: a checkout file that is not well formed is refused with GB-329, exit 1

A YAML file goatbuilder reads from the checkout that does not parse (a pin
table, the ladder, a record, a workflow), and an intent schema
(`ops/consistency/schema/intent-v1.schema.json`) that is not JSON or does not
compile, now stop with GB-329, "a file goatbuilder reads from the checkout is
not well formed", and exit 1: refused, a precondition failed. The message names
the file and the parser's reason.

#### Compatibility for scripts

- A YAML file that does not parse, and a schema that does not compile, were
  GB-254, "cannot be read or written", with exit 2. They are now GB-329 with
  exit 1.
- An intent schema that is not JSON had no code and exited 2. It is now
  GB-329 with exit 1.

A script that tests for exit 2, or matches GB-254, after one of these sees
exit 1 and GB-329. A file that is missing or cannot be read stays GB-254 (#929).

#### Added: the `network-tap` type; a tap states its mode, and inline is shown fail-closed

`ops/goatbuilder/targets/network-tap.yaml` is the network tap's type. A machine
of it declares `tap:` in its host_vars: `mode: listen` (fail-open, with its
`source`) or `mode: inline` (fail-closed: the link is down whenever the machine
is), its `ports` and its `management` interface. There is no default, and
`join-node` and `doctor --target` refuse a machine with none or a wrong one
(GB-380), as they refuse `tap:` set in more than one host_vars file. Both print
`inline, fail-closed` on an inline tap. The type requires an encrypted root,
which `doctor --target` reads from the checkout only, so it reports it UNKNOWN,
not as a pass.

`join-node` phase 13 installs goatrope, the tap's installer, as a release
component where `ops/pins/fleet-pins.yaml` pins it for the tap's group; phase 3
runs `goatbuilder deploy --dry-run` for it before anything on the machine
changes, so every check deploy makes before a host applies, and what deploy
would refuse is not installed. Where there is
no pin, phase 13 is a declared skip that says why. goatrope has no release yet,
so a machine joined as `network-tap` today is not a working tap.

#### Added: `peerings[].package` names a peering's federation package version; v1 runs, v2 is refused for now

A peering in `goatbuilder.yaml` may now declare `package: v1` or `package: v2`
(ADR 1242). Absent, it is v1, as every peering has run until now, and any other
value is refused by `goatbuilder validate`. v2 adds the announcement of a next
CA, so a CA can rotate without a gap; both parties select it in reviewed
configuration before either emits it. This release does not run v2 yet, so
`goatbuilder validate` refuses `package: v2` with GB-025 ("package v2 is not yet
supported by this goatbuilder") rather than quietly run the peering as v1.

#### Fixed: join-node refuses before phase 1 when a phase would read an unset variable (GB-369)

A phase whose command expands a variable that is not set, such as a founder's
`OWNER_EMAIL` in phase 13, stopped on bash's own `unbound variable` partway
through the node, and goatbuilder said only "No known error in its output".
join-node now checks every phase it will run before the first one and
refuses with GB-369, naming the phase and each variable. `goatbuilder explain`
and a failed phase's `interpreted` row name bash's line GB-369 too.

#### Changed: a member pin deviation names the goatnet value it overrides, or `goatbuilder pins` refuses (GB-357, GB-358)

A host group's own deviation in `ops/pins/fleet-pins.yaml` used to keep winning
after its goatnet moved, for example to a hotfix, and that group silently kept
the old image (ADR 1256). A member deviation with a `value:` now carries
`over:`, the value its goatnet resolves the component to without it: a ref,
`absent` or `none`.

**What refuses after you move your goatcore pin.** `goatbuilder pins`,
`ops/pins/generate-pins.py` and `ops/pins/report.py` stop with:

- **GB-358**, for each member value deviation with no `over:`. The message
  prints the line to paste, for example:
  `over: "ghcr.io/example/signal-server@sha256:<the goatnet's value>"`.
  Paste it under that member deviation to keep the member's pin, or delete the
  deviation so the member takes the goatnet's value.
- **GB-357**, when `over:` no longer matches because the goatnet's value changed.
  Delete the deviation to take the new value, or set the printed `over:` line to
  keep the member's pin deliberately.

An `absent: true` member and a goatnet-scope deviation need no `over:`, and
`over:` on either is refused. A member deviation that pins its goatnet's own
value prints a warning. Nothing stamps `over:` for you: deciding each one is
the point. `graduate.py hotfix` now names the members a hotfix would miss.

#### Fixed: the preflight's refusal of an unresolved secret reference has its own code, GB-368

When `ops/scripts/ansible-preflight.sh` finds a variable whose value is still
an `op://`, `bao://` or `bw://` reference, it refuses with GB-368 and names the
variable. `goatbuilder explain` and a failed phase's `interpreted` row used to
read that refusal as X-ANSIBLE-005, the stale-checkout refusal, and suggest a
`git pull`; they now say to run the command through `ops/scripts/with-secrets`,
or to unset a variable the play does not need.

#### Added: a pins component may ship as a release file (`artifact_type: release`)

`ops/pins/fleet-pins.yaml` takes a component that ships as a file, not an image
(gate J23). It declares `artifact_type: release` and a `release:` mapping: the
release's checksums file, the file for each architecture (`amd64`, `arm64`) and
the signer its signature or attestation must name. Its pin is
`<owner>/<repository>@<tag>@sha256:<digest of the checksums file>`. `goatbuilder
pins` checks the declaration and every pin (GB-365). Nothing changes for an
image component.

#### Added: `deploy`, `verify` and `rollback` work on a release component

A component that ships as a file (`artifact_type: release`) is fetched and
verified on the operator's machine before any host is touched: the checksums
file's digest, its signature by the declared workflow at the pin's tag, and each
architecture's file (GB-381). `goatbuilder deploy` and
`ops/scripts/deploy-component.sh` then install the host's own architecture's
file with the `release-file` role, and refuse in the same words. The script
needs goatbuilder on PATH, or `GOATBUILDER_BIN`.

`deploy --dry-run` fetches and verifies too, prints the whole plan and exits 1
when a real run would refuse. `verify` also reads the file each host runs and
requires the pin's. `rollback` switches every host of one goatnet back to the
file it ran before, with no fetch, one step (GB-379 for more). Verified files
are kept under the state directory; `GOATBUILDER_RELEASE_MIRROR` names a
directory to read instead of GitHub.

A deploy that follows a rollback to the same ref now writes its own ledger row,
for an image component too. Before, the host kept reading as rolled back.
`status`, `diff` and `survey` now list release components.

#### Added: `promote` and `fastlane` take a release component

`promote open`, `promote hotfix` and `fastlane` take a release component
(`artifact_type: release`) at its whole pin,
`<owner>/<repository>@<tag>@sha256:<digest>`, and refuse any other ref; a release
pin is refused for an image component. `promote check` refuses an open record or
a graduation-set member of the wrong kind. `fastlane` fetches and verifies the
release before it moves anything (GB-381).

A file has no container, so a `runtime-health` gate cannot be captured for one:
`promote graduate` says so, and passes it only with `--override-runtime-health
"<reason>" --ack "<name>"`, recorded on the record. `graduate.py` and
`fastlane.sh` do the same, in the same words.

#### Changed: a release component's `release.signer` is a workflow with no ref, and its tag may hold a slash

A release component's `release.signer` is the workflow that signs its
releases, `https://github.com/<owner>/<repository>/.github/workflows/<file>.yml`,
with no ref: the pin's tag is its ref, so one declaration serves every pin.
`goatbuilder pins` and `goat_consistency.resolve` refuse any other shape
(GB-365). A pin's tag may hold a slash (`tool/v1.2.0`). A release pin's
repository must be its signer's repository, compared without regard to case,
and each file name in `release:` is one segment of `[A-Za-z0-9._+-]`, as the
role on the host requires.

#### Fixed: a run record's probe log names only the variables the run reads

join-node, standup, deploy and doctor ask the secrets posture which of their
variables are available. That probe printed the name of every variable with a
value on the machine (never a value), and a run keeps each probe's output, so
every run record's `logs/NNN-probe.log` listed every environment variable's
name. The probe now asks for the variables the verb reads and answers for
those only. `goatbuilder secrets available NAME…` takes names the same way;
with none it still lists every name, for your own terminal.

#### Changed: a site's own seed play must say which generic play it was made from

A deployment may keep its own `ops/ansible/<site>-seed-from-peer.yml`, which
`goatbuilder standup` runs at phase 6 in place of the generic
`site-seed-from-peer.yml`. A copy made from an older generic play missed every
fix made to it since, and nothing said so. Such a copy now starts with
`# derived-from: site-seed-from-peer.yml sha256:<digest>`, the digest of the
generic play it was made from. Right before running it, standup checks that
digest against the current generic play (`ops/scripts/check-site-play-current.py`)
and refuses a copy with no such line, or made from an older version, with
GB-362. The message prints the digest to set once the copy is brought up to
date. A site with no copy, or whose file imports the generic play, is not
affected. An existing copy needs the line before its next standup run (#1129).

#### Fixed: a standup play limited to the founder is refused when its limit selects no host, instead of passing

Six standup plays run on the site's first management host alone, limited
with `-l 'mgmt-<site>-01*'`: SPIRE, the LAN service surface, the LAN TLS
routes, the dashboard, the portal and grant-keyd. When the site's founder was
named otherwise and another inventory host matched the pattern, ansible
skipped the play and exited 0, so the phase passed having done nothing. Each
of those commands now checks first that a host of `<site>_hosts` in the two
management inventories matches the limit, and refuses with GB-355, naming the
pattern, the group and the inventories, when none does. `goatbuilder standup`
and `ops/scripts/goat-standup.sh` run the same check. The naming rule is
unchanged.

#### Changed: standup passes secrets through the environment, not the command line

A standup phase that needs the Postgres superuser password, the NetBird
datastore key, the pgBackRest cipher pass, the relay secret or the Cloudflare
token now gives the play `-e 'name={{ lookup("env", "<SITE>_NAME") }}'`, as
`join-node` already did. Before, the shell expanded `$(printenv <SITE>_NAME)`
into the `ansible-playbook` command, and the phase-7 relay line wrote its
secrets to a temporary vars file. The play now reads each value from its own
environment, so the same variables must be set as before. A phase whose
`lookup("env", ...)` names an empty variable is refused before it runs, naming
the variable, as an empty `$(printenv ...)` was. `goat-standup.sh` prints the
same lines. A dry run shows the new form, and the recorded fixture plans are
regenerated.

#### Fixed: a standup phase that would read an unset variable refuses before it runs (GB-369)

A standup phase line with no bootstrap administrator declared carries
`"$OWNER_EMAIL"`. With it unset, the phase stopped on bash's own
`unbound variable`. `ops/scripts/standup-phase-run`, which runs every standup
phase for both drivers, now checks each variable the line expands, after it
loads the secrets store, and refuses with GB-369, naming each one, before the
phase runs (#1176).

#### Added: status and verify say each machine's state as data in -o json

Every `status` and `verify` document in `-o json` now carries `machines`: one
entry per graded machine row, with its `environment`, `host`, `component`,
`state` (`converged`, `pending` or `unknown`), a `reason` from a closed set
(`declared`, `unfinished`, `other-ref`, `no-record`, `other-image`,
`served-off`, `unreachable`, `no-answer`, `unreadable`) and `check`, the row it
grades. The list is empty when nothing was graded, and no other verb's document
carries it. A script no longer has to read the first word of a row's `detail`
to tell a machine that has not converged from one that could not be read. The
`checks` rows, the text pages and the exit codes are unchanged.

#### Added: `doctor --target network-tap --live` checks a tap's end state on the machine; `status` prints its mode

`goatbuilder doctor --target network-tap --site <site> --live` reads each tap
over Ansible, changes nothing, and holds it to its declared mode: no address on
a tap port or the bridge, nothing that sends, management and the default route
on the third interface, the bridge (inline) or the dropped egress (listen),
receive offloads and Energy-Efficient Ethernet off, the recorder on a port, no
port that boot waits for, a hardware watchdog, and an encrypted root. A fact that
cannot be read is unknown, which fails. Without `--live` no machine is reached.

`goatbuilder status` prints each tap's mode, `inline, fail-closed` or `listen,
fail-open`, from the checkout.

#### Added: `join-node` and `standup` read each machine type from a file, and a private overlay can add its own types

What a machine of each type (`core`, `obs`, `breakglass`, `network-jump`,
`phone`) converges as is now a data file in `ops/goatbuilder/targets/`, and the
kinds of hardware a type accepts are files in `ops/goatbuilder/families/`
(ADR 1257). Every plan is unchanged. A checkout without those directories uses
the files in goatbuilder; one that has them reads only its own. A private overlay adds types and families in
`ops/goatbuilder/overlay/`, with no public change. The format is in
`ops/goatbuilder/targets/README.md`.

New refusals, each before any phase: a type or family file that is not valid,
an unknown field included (GB-374); a platform family with no file (GB-375); a
machine whose declared `platform_family` its type does not accept (GB-376); and,
for a run, a play of the type that is not in the tree, with the repository it
comes from (GB-377). In an assembled tree, a file in `targets/` or `families/`
that is not the tree's goatcore's own is refused (GB-378): an overlay only adds.
A machine's `platform_family` is read as Ansible resolves it; one that declares
none is not checked. The list of known classes in GB-247's message is now in alphabetical
order.

#### Fixed: `teardown` says what each host had, stops the LAN surfaces, and names a refused stop's cause

Each stop of `goatbuilder teardown` and `ops/scripts/goat-teardown.sh` now
asks the host whether the component is there at all: a compose project's
file or containers, a service's containers, a systemd unit. A component that
was never deployed on the host, such as a phase-12 service after a standup
paused at the browser step (phase 11), is reported as "nothing to stop" and
writes no `teardown.jsonl` row; before, each such stop wrote a row for a
service that had never run. The run ends with how many stops took a
component down and how many found nothing on the host.

`teardown --services` now also stops the LAN name server (`lan-dns`), the
LAN Traefik and the SPIRE agent, which it left running on a goatlite. STUN
(the coturn unit) is still left running, and says so.

A stop that fails names the catalogued error its output carries, in its
failure row, an `interpreted` row and the cause column of
`teardown-<site>-timings.tsv`. The playbook preflight's refusal of a stale
checkout is X-ANSIBLE-005, with the `git pull --ff-only` that fixes it; it
was recorded as "uncatalogued". GB-021, fastlane's refusal, no longer
matches every line containing "REFUSE —", so other tools' refusals are not
read as a fastlane refusal.

#### Added: a target type lists its services, and `deploy --node` deploys to one machine of a group

A type file may list `services:`, components of `ops/pins/fleet-pins.yaml`.
`join-node` phase 13 deploys each to the machine it joins, and
`doctor --target --live` reads each back. One the pins cannot supply is refused
before any phase (GB-384). No shipped type lists one yet.

`goatbuilder deploy --group <group> --node <machine>` and
`deploy-component.sh --group <group> --node <machine>` play on, and read back,
that machine alone, so a neighbour that is down does not stop it. The name must
be one host of the group in `inventory/mgmt-hosts.ini`; a group's name or a
host of another group is refused before any door opens or play runs.

The rollout preflight now grades a release component's declared `pin-revert` as
one; it read as not revertible, so every deploy of one asked for `--ack`.

#### Fixed: goatbuilder update trusts the release signers it declares, so a repository move no longer strands an install

`goatbuilder update` used to trust only the repository that built the running
binary, or the one a checkout's `repo.release_source` named instead. A binary
built before the repository move therefore refused every later release.
From this release, goatbuilder carries a list of the release signers it
trusts, and checks a release against each of them. A checkout's
`repo.release_source` (else `repo.source`) is trusted beside that list,
never instead of it, and
an update run outside a checkout no longer stops with GB-078. A signature
must also name the signer's reusable release workflow at the release's tag
as its certificate's subject, and, when the certificate carries them, its
GitHub workflow repository, ref and name: the fields a cosign reinstall line
pins. With `repo.release_source` omitted, `repo.source` is the addition, as
before. When no signer
matches, GB-261 names each signer it checked and why each one refused. When
the signer changes in future, one release first trusts both the old signer and
the new one (ADR 1252). An install from before the move still needs one
reinstall: see GB-074. (#1036)

### For someone contributing

#### Designed: platform families and build target types for every machine kind, before 1.0

ADR 1257 (Proposed) and `docs/design/goatbuilder-build-targets.md` declare two
things as data:

- platform families, which hold hardware and its preparation, and where the
  golden path lives; a model joins a family;
- target types, a machine's role with its plays and acceptance, read by
  `standup`, `join-node` and the image tooling. The types are core, obs,
  break-glass, the network-gear jump box, octo-pod, the Billie node, the
  vendor gateway, first-bite, the network tap, the federation and simple boundaries,
  goat-processors and the phone.

A Billie node runs the sensor kit's relays for the sensors declared on it. A
site's network gear is not built by goatbuilder; it is reached only through the
network-gear jump box. The design also covers `goatbuilder doctor --target
<type>`, which checks a host's end state however it was installed,
`goatbuilder image <model>` as an optional golden path, and standup phase 11
without a manual step.

The network tap declares its mode: listen, which fails open, or inline, where
the link is down whenever the machine is. Both are supported.
Services are not types: `goatbuilder deploy` places them, and a new `release`
artifact type covers the ones that ship as files. A private overlay can add
types and services.

The 1.0 gate gains group J: one foundation, then five lanes in parallel (host
site, boundary, processing, small computer, services), with a CI-tier row for every type
and hardware-tier rows where there is equipment. Nothing is built yet, and no
command changes in this release.

#### Changed: a changelog entry is committed alone; the Unreleased section is one pointer line

A pull request that adds an entry in `crates/goatbuilder/changelog.d/` no
longer commits its render into `CHANGELOG.md`. The Unreleased section there is
one fixed line, and `changelog.py check` refuses anything else, so merging one
pull request no longer makes every other open one with an entry conflict.
`python3 ops/release/changelog.py show` prints the entries as the next
release will, and the `docs-guards` run puts that in its summary. A pull
request that only adds entry files merges at any base; one that changes
`CHANGELOG.md` itself (a release, or release notes) still has to be on a base
that holds every release (#1033).

#### Changed: the compatibility check is held to goatbuilder/v0.7.3

`ops/ci/check-goatbuilder-compat.sh` now compares each build with
`goatbuilder/v0.7.3`, the last published release: its command surface and its
kept `-o json` documents. The upgrade tests' list pins the four published
binaries of v0.7.2 and v0.7.3 (`crates/goatbuilder/tests/upgrade/releases.sha256`),
and the configuration files of both are kept for gate row A2 with the outcome
each release's published binary gives them.

#### Added: a report of every deploy play no goatbuilder verb reaches (gate J28)

`ops/ci/check-deploy-plays-reached.py` reads every `ops/ansible/*-deploy.yml` and
says how a goatbuilder verb reaches it: a pins component `goatbuilder deploy`
finds, a play a type file names, a play goatbuilder's own source runs, or one
of those plays' imports. A play none of them reaches is a gap, unless
`ops/goatbuilder/deploy-plays.yaml` lists it with its reason. It runs as a
report today and names 22 gaps; it becomes blocking with `--strict` once each
gap has an answer.

#### Changed: the standup and teardown parity gates say at which column two lines differ

`ops/goatbuilder/parity-standup.sh` and `parity-teardown.sh` compare the
script's plan with goatbuilder's. On a difference they printed a `diff` cut at
220 characters, so two long lines that differed further along printed the same.
They now print, for each differing line, its line number, the column of the
first different character, both lengths and the text around that column on
both sides (`ops/goatbuilder/parity-line-diff.py`) (#1181).

### To be removed

Nothing.

## 0.7.3 — 2026-11-17

What changed since 0.7.2, the previous release. `goatbuilder federate directory
ls|show|check` reads the peering directory and says, for a file a peering
cannot fetch, whether the party has not uploaded it or the directory's release
lacks the route. Every error `goatbuilder federate` raises carries its catalog
code, and `explain release` and `explain fastlane` answer for each step.

### For someone deploying

#### Changed: a joining site is declared to run the data router, admit-audit and SPIRE

The end-state catalog declares the data router (zenoh-router) and admit-audit
at every site that replicates, a founder and a joiner alike, since
`goatbuilder join-node` builds both at a joining site's core node. A pin's
`absent: true` or the site's own `admit_audit_enabled: false` still decides.
SPIRE (spire, spire-agent) is declared at every site too. A joining site's own
SPIRE server and trust domain (ADR 1164) is designed, not built, so at a joiner
`goatbuilder endstate` grades it `GAP` and the `join-node` plan lists it under
"does not run yet", each ending "(ADR 1164)" where the other gaps name their
gate row.

#### Added: `goatbuilder explain release` and `explain fastlane` answer for each of their steps

The phase catalog now describes `release`'s eight numbered steps (preflight,
tag, dev pin, dev deploy, dev rows, preprod, soak, prod) and `fastlane`'s
eight (component, graduation set, digest, dry run, move fleet, record the
debt, land, deploy). `goatbuilder explain release` lists them, and
`goatbuilder explain release 6` (or `explain release preprod`) says what the
step does, why, what it changes, how to tell it worked, the command by hand
and what to do when it failed, as `explain` already did for `standup`,
`join-node`, `deploy` and `federate`. A test holds each verb's entries to the
step lines it prints, so a new step cannot ship without one. Nothing a run
does changes.

#### Fixed: acceptance on a derived session learns a partner's challenge from its run stamp

Phase 14's acceptance script, run on a derived session
(`ops/federation/standup/acceptance.sh` with `FED_ACCEPTANCE_ATTEMPT`),
staged only its start time beside its samples, so a partner's rerun soon
after a first run could be judged by the first run's challenge, giving a
false FAIL. Each run now stages a run stamp beside its samples, holding its
challenge and the time it started in one value. A reader learns the
partner's challenge from the stamp and waits for the sample carrying it. It
ignores a stamp that started more than twice `FED_SESSION_WAIT` before its
own start, says so, and keeps waiting for this run's; a stamp it cannot read
is waited for, never taken as missing. The challenge is the one of the stamp
whose sample it found, so a rerun whose first run's sample never arrived is
judged by the rerun's; two runs' stamps on two sets end BLOCKED, never FAIL.
With only an earlier run's sample there, the delivery rows are BLOCKED, say
that an earlier run's sample is what is there, and the run ends open
(exit 3). A partner whose runner stages no stamp is read once a whole
`FED_SESSION_WAIT` has passed with no stamp of its own, since this run's
stamp may still be in flight; once a partner's stamp has been read (a
garbled one included), a sample without one is never learned; an unstamped
sample's row says its run identity is not proven, and a second unstamped
challenge is BLOCKED as two runs.

Limits: a partner's current run is kept when both sides start phase 14
within `FED_SESSION_WAIT` of each other and their clocks agree within
`FED_SESSION_WAIT`, which `goatbuilder federate test <partner> --bilateral`
checks first (GB-268). An earlier run that started less than twice
`FED_SESSION_WAIT` before ours is not told apart, nor is one from a runner
that stages no stamp; its result is still refused (GB-273), so the verdicts
can differ until a rerun, but a stale result never passes. Tested on the
in-process fabric only, not on hosts (#792).

#### Added: `goatbuilder federate directory ls|show|check` reads the peering directory

`goatbuilder federate directory` reads the peering directory (saltlick) and
changes nothing. `ls` lists the directory's release and every party with the
files it hosts; `show <party>` gives each hosted file's size and sha256;
`check <party> --for <counterparty>` checks each file a peering reads and says,
for a missing one, whether the party has not uploaded it (`NOT HOSTED`: the
party runs phase 5) or the directory's release does not serve the route
(`ROUTE MISSING`: its operator moves the directory to a newer release). The
URL is `--url` or `FED_DIRECTORY_URL`. New codes GB-370, GB-371 and GB-372 (#1000).

#### Fixed: every error `goatbuilder federate` raises carries its catalog code

Fifty errors the federate stages and verbs raised had no catalog code, so a log
of a run that met one (stderr not a terminal, no explanation page) showed only
`goatbuilder: <text>`, with nothing to look up. Each now prints its code and
`goatbuilder explain <code>` answers it. They use existing codes where one fits
(an undeclared environment GB-241, the working folder GB-184, a declaration
GB-144, a flag for another verb GB-239, a peering on several environments
GB-252, a host's back-out survey answering for another peering GB-253, a verb
line the driver cannot parse GB-242), and eight new ones: GB-315 a program a
step ran gave output or an exit that cannot be used, GB-316 a confirmation could
not be read from the terminal, GB-320 a program the run starts could not be
started, GB-321 a file a step wrote is missing or not what it wrote, GB-322 an
internal error, GB-323 a file outside the working folder cannot be written,
GB-324 a setting in the environment has a value goatbuilder does not take, and
GB-325 flags given to a stage or verb that do not go together, or one without
the flag it needs (a recheck given `--edge-group` or `--as` other than the saved
plan's among them).

Exit codes, by the contract's meanings (1 refused: a precondition failed; 2
usage: the command line was wrong). Each change is from the last release:

- `goatbuilder federate <partner>` and its stages (plan, deploy, backout, rotate,
  status, import, the edge run): an error with no code exited 2. One that now
  carries GB-184, GB-315, GB-316, GB-320, GB-321, GB-322 or GB-323 exits 1,
  since none is a usage error. One that carries GB-241, GB-252, GB-253, GB-324
  or GB-325 still exits 2.
- The federation verbs (`goatbuilder federate init|lint|compose|diff|verify|apply`,
  and the phases that run them in process): an error with no code exited 1. A
  command-line error now exits 2, the contract's usage error: `--over-ssh` given
  to a verb other than init (GB-239); a host flag or `--side` on init without
  `--over-ssh`, `--side` without the files it reads or writes, and
  `init --over-ssh` without `-C` (GB-325). The others still exit 1 (GB-144,
  GB-322, GB-323), and a verb line the driver cannot parse still exits 2
  (GB-242).

`federate allocate <peering>` naming an undeclared peering is not coded yet and
still exits 2. Every other exit is unchanged.

#### Changed: `goatbuilder join-node` names each declared service its run does not build

At a joining site, the dry-run plan and the run's summary already listed the
end-state model's gaps and the services the site turns off. They now also
list each service the model marks present on the node that no step of the run
builds. At a site with its own obs host, a service the node's other class
builds names that run, for example `join-node --class obs`. At a single-host
site, the observability stack the model places on the core node is not built
by any verb yet, and reads as the gap it is. A service
no step builds reads "no join-node step builds it yet (gate row I3)". The
admin surfaces keep their gap line (gate rows I1 and I4). Nothing a run does
changes (#953).

### For someone contributing

Nothing.

### To be removed

Nothing.

## 0.7.2 — 2026-11-15

What changed since 0.7.1, the previous release. `goatbuilder federate rotate
<partner> ipsec-leaf` and `ipsec-ca` rotate an `ipsec-fips` peering's IPsec
leaf: the edge role switches to the staged leaf and puts the old pair back if
the link does not come up. `goatbuilder grow` hands a goatlite over to the full
preset in the checkout, never on a host.

One behaviour change: the edge role sets `make_before_break = yes` on every
IPsec peering edge, so the first apply after this release restarts strongSwan
once on each ipsec-fips edge and drops that peering's link briefly. Apply both
sides of a peering in one window.

### For someone deploying

#### Fixed: a failed phase prints its fix once, and an incomplete acceptance names the command that resumes it

A failed phase's `interpreted` row printed the error catalog's fix twice:
inside the row's text, then again on its own `Fix:` line. It now prints
it once, on the `Fix:` line, and `-o json` keeps it in the row's `fix`
field. This applies to `standup`, `join-node`, `deploy` and `federate`.
When phase 14 ends INCOMPLETE (X-ACCEPT-001), the fix now names
`goatbuilder federate <peering> --from 14` and no longer names the scripts
avenue's `ops/scripts/peering-standup.sh` (#911).

#### Added: `goatbuilder federate rotate <partner> ipsec-leaf` and `ipsec-ca` rotate an ipsec-fips peering's leaf

On a peering whose agreement chose ipsec-fips, `goatbuilder federate rotate
<partner> ipsec-leaf` runs phases 11 to 14 and passes the rotation to
goat-peering-edge, which checks the leaf an operator staged, sets the live
pair aside once, switches and rolls back if the link does not come up.
`rotate <partner> ipsec-ca` also needs `--partner-holds-ipsec-ca <sha256>`,
the partner's attestation that it placed our new CA: the fingerprint it read
back (GB-303).
The rotation's record, `<work>/.rotation-ipsec`, lets a stopped run resume
with the same id; a run of another kind is refused (GB-298). `--retire <id>`
removes the set-aside key, only once phase 14 passed here after the switch
(GB-309; the partner's phase 14 is theirs to confirm on the call), and never while another rotation is under way. A
new rotation waits for the last one's retire (GB-298). A WireGuard
peering's keys are rotated with `leaves`, `carrier` or `transport`.

#### Added: goatbuilder grow hands a goatlite over to the full preset

`goatbuilder grow` rewrites `goatlite.yaml` as a full configuration with the
same values. Before it changes anything, it shows:
- the difference, leaf by leaf;
- the configuration file it writes;
- each file whose generated header it takes off;
- the removal of `goatlite.yaml`;
- the renderer command it runs, exactly as it runs it.

`--dry-run` stops there. At a terminal it then asks, and goes on only when
you type yes. It asks only when stdin and stderr are both a terminal; otherwise,
including `goatbuilder grow 2>file`, it needs `--yes`. With `--yes` it does not ask. A run writes
`ops/goatbuilder/goatbuilder.yaml`, takes the generated header off every file
the goatlite's render wrote, so each is yours, and removes `goatlite.yaml`.
It changes files in the checkout only, never a host. Before you commit, git
undoes it: restore `goatlite.yaml` and the files it handed over, and delete
`ops/goatbuilder/goatbuilder.yaml`.
It refuses, and changes nothing, when `goatlite.yaml` is not a goatlite
(GB-304), when `ops/goatbuilder/goatbuilder.yaml` already holds another
configuration (GB-305, which says so when the file is grow's own from an earlier run, since edited), or when the grown configuration is incomplete
(GB-306: a four-fact file names no `repo` or `components`). A renderer that
fails or refuses stops it with the renderer's own words (GB-258). Run again
after it finished, it says the goatlite was already handed over. It is step 3 of
the growth design; the other steps are not built, and it has not run on
hardware.

#### Added: goat-peering-edge rotates an IPsec peering's leaf from staged material, and puts the old one back if the link does not come up

An `ipsec-fips` peering's edge role, `goat-peering-edge`, rotates its IPsec
leaf when `peering_edge_rotation` names a rotation id. The role still writes
no secret: the offline ceremony issues the new leaf, and an operator stages it
beside the live files as `.next` files. The role checks the staged pair
(GB-289, GB-290), sets the live pair aside once in `.previous-<id>/`, switches,
re-authenticates the IKE SA, and puts the old pair back if the SA does not
come up (GB-302); a rotation rolled back stays reported as rolled back on
every later run of its id. An `ipsec-ca` rotation also needs the partner's
read-back of our new CA's fingerprint (GB-303) before our new CA replaces the
current one. `peering_edge_rotation_retire` removes the set-aside key only once
the confirm recorded the new leaf live (`.previous-<id>/confirmed`, the IKE SA
the re-authentication made), and never moves the live pair, the staged pair or
our CA (GB-309 for an id with nothing to retire, or a rotation not confirmed
or with an unreadable record; a re-run with the same id confirms it). The
confirm is recorded only once the SA from before the re-authentication is gone,
and a roll-back removes the record. A run killed during the set-aside or between
the switch's two moves is finished by the next run, or refused with nothing
moved (GB-311); the live pair is never left mixed. The confirm counts only an IKE SA the
re-authentication made, not the old one still up beside it, and the role now
sets `make_before_break = yes` in its charon drop-in on every IPsec peering
edge, instead of leaving it to the packaged strongSwan's default
(`peering_edge_make_before_break`). The first apply after this change restarts
strongSwan once on each ipsec-fips edge, which drops that peering's link
briefly: apply both sides of a peering in one window. The rotation always
rekeys the IKE SA before the re-authentication, so no rekey of either side, which
keeps the old authentication, can pass the confirm on the old leaf, and it refuses
an IKE rekey interval too short to hold the confirm. `peering_edge_remote_ca_next_path` accepts a
partner's new CA beside the current one during their rotation. It is opt-in,
and without a rotation id nothing changes. `goatbuilder federate rotate
<partner> ipsec-leaf` and `ipsec-ca` drive it.

#### Changed: `federate rotate <partner> ca` says the CA rotation is decided, not built

`goatbuilder federate rotate <partner> ca` is still refused with GB-282, and
nothing runs. Its message used to say that the CA rotation needed a decision
first. That decision is now made: ADR 1242 announces our next CA in package v2,
so the partner trusts both CAs for an overlap. The message now says the
rotation is decided and not yet built in this goatbuilder. Rotate the leaves
under the same CA with `rotate <partner> leaves` meanwhile.

### For someone contributing

#### Changed: the compatibility check is held to goatbuilder/v0.7.1

`ops/ci/check-goatbuilder-compat.sh` now compares each build with
`goatbuilder/v0.7.1`, the last published release: its command surface and its
kept `-o json` documents. The upgrade tests' list pins v0.7.1's four
published binaries (`crates/goatbuilder/tests/upgrade/releases.sha256`), and
v0.7.1's configuration files are kept for gate row A2 with the outcome its
published binary gives each.

### To be removed

Nothing.

## 0.7.1 — 2026-11-15

What changed since 0.7.0, the previous release. `join-node` builds a node on a
phone with `--class phone --via usb`, on a fixture and not yet measured on a
phone, runs a joining site's own mesh DNS, and refuses a node whose clock is
off. `standup` for a joiner turns signal federation on, and standup,
join-node and teardown keep timings files whose rows name their cause. Every
site's end state declares every service a cut-off site needs, and what a
joining site does not run yet is named as a gap. `federate rotate <partner>
leaves` replaces a running peering's leaves; `federate test
--bilateral` completes the partner's half of acceptance, opt-in; on a
two-driver peering the edge driver runs phases 1 and 3. A back-out keeps the
core leaf another peering still uses and removes the peering's host-firewall
entry. After a signing-key rotation, phase 5 hands the peering directory over
to the new key, and a host-allocation SKIP says what clears it or is accepted
into the plan. `goatbuilder.yaml` takes three optional `deploy` fields,
`cloud`, `owner` and `cross_goatnet`; undeclared, nothing changes.

Three behaviour changes: an environment that declares `converger: pull-agent` is
refused by `doctor` and by every verb that pushes to real hosts, with GB-307;
`join-node` refuses a node whose clock is unsynchronized or more than 250 ms
off, with GB-297; and a back-out with no other peering set up, and no record
of what the core leaf's paths held before, stops with GB-293 where it stopped
with GB-221.

### For someone deploying

#### Fixed: phase 14's round trip sends a fresh probe until one is acknowledged, so one dropped probe does not fail it

In a bilateral acceptance session both sides run the round trip at once.
Our probe could reach the partner's edge just behind their responder's
acknowledgement of their own probe, and the edge's downsampling dropped it
with nothing logged, so the round trip on that set failed after its
timeout although the peering worked. `fedcheck.sh` now sends a probe with
a fresh nonce until one is acknowledged: the first resend one gap after the
first probe (phase 14's sample gap), each later one twice as long after
the last, up to 4 seconds. Any acknowledged probe passes, and the check
ends within its timeout. A first probe our core refuses fails the check at
once with its HTTP status; a later refusal is counted and the check keeps
waiting. `responder.py` no
longer stops when one acknowledgement cannot be written; it tries it again (#808).

#### Fixed: phase 14 in a bilateral session no longer fails each side's own staging row

In a bilateral acceptance session (`FED_ACCEPTANCE_SESSION` or
`FED_ACCEPTANCE_ATTEMPT`), each side checked that its own samples were staged
by reading them back from `release/<set>/**` on its own core. A core keeps no
store on that staging name, so the read found nothing and both verdicts
failed on `publish/outbound/<set>`, although the partner saw the samples
arrive. The row now passes when our core accepts both samples (HTTP 2xx),
fails when it refuses one and is BLOCKED when it does not answer; the
partner's `delivery/inbound/<set>` still observes the arrival. (#789)

#### Fixed: backing a peering out removes its entry from the edge's host firewall

`goatbuilder federate backout` left the backed-out peering's entry in the edge's
host firewall baseline (`/etc/goat-host-firewall/peerings/`), so its ports stayed
open. The back-out now removes that entry and no other. When another peering
stays on the edge, the ruleset is loaded again without it, through the role's
lockout check and timed rollback. When none stays, the baseline is unloaded and
removed, and the edge is as it was before its first peering. If the reload or
the unload fails, the entry is put back so the back-out can run again. A pending
rollback of an earlier run refuses (GB-214), and `--dry-run` only reports.

#### Fixed: the C3 comparison says when a verify report read no machine

`ops/goatbuilder/c3-agreement.py` compares the `-o json` reports of `status`,
`diff` and `verify` for gate row C3. When a `verify` run read no machine,
because its transport or inventory read failed, the script used to say that
`verify` had not been run for that component, and exit 3 as if the three verbs
disagreed. It now names the report that read no machine and exits 2: the run is
not a C3 measurement. Fix the reach and run the sequence again. A component
with no ledger coverage (`ledger_component: null`) used to be reported as a
disagreement on every machine; it is now left out of the comparison and named
(#809).

#### Changed: an environment that declares converger: pull-agent is refused until pull is built

No pull converger is built, yet `deploy` and the other verbs that reach real
hosts used to push to an environment declared `converger: pull-agent`, and
`doctor` passed it. Now `doctor` fails it in an `env:<name>:converger` row.
`deploy` (its dry run too), `release`'s deploy rungs, `fastlane`'s deploy and
`rollback` refuse it with GB-307 before any step. The refusal names each place
the value is declared: the environment, a site or a class. `deploy --verify-only`
still reads, `validate` still accepts the value, and `--mock` still converges
it on a fixture. Declare `converger: push-ansible` for those machines until
the pull converger is built (#803).

#### Added: goatbuilder.yaml takes deploy.cloud, deploy.owner and deploy.cross_goatnet

Three optional `deploy` fields say what a goatlite decided from its preset,
so a full configuration grown from one behaves the same: `cloud: none`
means no play asks for a cloud account; `owner` is the bootstrap owner,
used in place of `$OWNER_EMAIL`; `cross_goatnet: false` means standup
phase 14 does not reach the cross-goatnet boxes. Undeclared, nothing
changes. The `grow` command that writes them is not built yet (#765).

#### Changed: every site's end state declares every service, and a joining site's missing ones are named gaps

`goatbuilder endstate` and `ops/goatbuilder/endstate/endstate.py` now declare
every service a site cut off from the others needs at every site that
replicates: the replication core (Traefik, NetBird management, signal, Ory,
the Kratos self-service UI), the NetBird dashboard, the portal, the id console,
the onboarding engine and mesh DNS. No catalog row is founder-only. What a
joining site does not run yet is graded `GAP`, "declared, not yet built at
this site", with the gate row that closes it: I1 and I4 for the admin
surfaces. Mesh DNS carries no gap, because `join-node` runs it at a joining
site (ADR 1235). A `GAP` is counted and printed and does not by
itself make the site red. Acceptance run with `--endstate` reports a row such a
service owns as `SKIPPED` with the same text instead of N/A. `goatbuilder
join-node` lists the gaps under "What <site> does not run yet", from the
catalog, before the services the site turns off (ADR 1233, #671).

#### Fixed: acceptance on a derived session ignores a partner sample left by an earlier run

Phase 14's acceptance script, run on a derived session
(`ops/federation/standup/acceptance.sh` with `FED_ACCEPTANCE_ATTEMPT`),
learns the partner's challenge from the partner's sample on our core.
Samples stay on the cores between runs, so a sample left by the partner's
earlier run taught that run's challenge: our signed result echoed a stale
one, the partner refused it (GB-273), and the two sides could end with
different verdicts until both ran again. Each run now stages the time it
started beside its samples, and a reader ignores a partner sample whose
start time is more than `FED_SESSION_WAIT` before its own, says so, and
keeps waiting for this run's. With only an earlier run's samples there, those
rows are BLOCKED and the run ends open (exit 3). A partner whose runner
stages no start time is read as before.

Limits: this rests on both sides' clocks agreeing within `FED_SESSION_WAIT`.
`goatbuilder federate test <partner> --bilateral`, which runs phase 14 on a
derived session (#720), runs phase 1's clock check (GB-268) alone first and
stops on a clock that is unsynchronized or over the budget. A partner's
earlier run that started within `FED_SESSION_WAIT` of ours is not told
apart; its result is still refused (GB-273), so the verdicts can differ
until a rerun, but a stale result never passes. Tested on the in-process
fabric only, not on hosts (#786).

#### Fixed: backing out one of two peerings keeps the core leaf the other still uses

Our core router serves one leaf to every peering. `goatbuilder federate
backout <partner>` no longer touches it while another peering is still set
up from this machine (its working folder beside this one holds an
`agreement.json`): the `core leaf` row reads `keep` and names that folder,
and the back-out removes everything else of the peering. When no other
peering is set up and nothing records what the leaf paths held before, the
back-out stops with GB-293, naming each path and the choice to make, instead
of stopping as GB-221 or showing the leaf as `absent` (#716).

#### Fixed: after a signing-key rotation, phase 5 hands the directory over to the new key

The peering directory binds our identity to the first key that wrote it and
takes a new key only on a request signed by the old one. Phase 5's directory
step now reads the key the directory binds. When it is not ours, it signs
the pointer's write with the previous seed, `FED_PREVIOUS_SIGNING_KEY` (or
`--previous-key`), and prints both keys. Before, every write after a
rotation was refused. If the previous seed is missing, unreadable or of a
third key, it prints GB-285 and publishes nothing; the step stays advisory
and phase 5 goes on. The `rotate … signing` stage is not built (#683).

#### Added: a two-driver peering's edge driver runs phase 1's edge checks and prints the way back

On a peering declared `drivers: two`, the edge driver runs the command the
controller's phase 1 hand-off names: `goatbuilder federate <peering> --side
edge --from 1 --to 1 --expect-commit <commit>`. It refuses to run unless its
checkout is that commit with nothing changed, and each file given with
`--expect <file>=<sha256>` has that digest (GB-278). It then runs preflight's
edge checks into `<work>/handoff/1/edge.json` and prints the way back: the
confirm code, and the `import` and `then` commands for the controller. The
edge driver's run of phase 3 is built too; phase 2's and those of 11 and
later are not yet (gate row B10).

#### Added: a two-driver peering hands phase 3 to the edge driver and joins its edge half

On a peering declared `drivers: two`, the controller now stops at phase 3
with a hand-off object whose result is the edge half
(`handoff/3/edge-half.json`). On the edge driver, `goatbuilder federate
<peering> --side edge --from 3 --to 3 --expect-commit <commit>` measures the
edge half over SSH, reaching only the edge host, and prints the way back with
its confirm code. On the controller, `goatbuilder federate import <peering> 3
<file> --sha256 <code>` checks the file as the core half would (GB-114): the
code, the peering, the 24-hour bound, and the edge host the inventory names
(`--edge-group` for a group other than `<peering>_edge`). Then
`goatbuilder federate <peering> --from 3` hashes the file again and measures
the core half against it, reaching only the core host. A two-driver run from
phase 3 with `FED_MEASURE=here` is refused before it acts (GB-267). Phase 2
and phases 11 to 14 are not built yet (gate row B10).

#### Added: `goatbuilder federate rotate <partner> leaves` and `carrier` replace a peering's keys, and an interrupted rotation finishes

`goatbuilder federate rotate <partner> leaves` replaces every peering leaf of
one running peering and runs the command again from phase 8 to 14. Phase 10
stops, as in every run, for the review of the new leaves, and phase 12 installs
them. Once phase 8 has passed, the command that carries on is the plain resume.

Phase 8's rotation is now resumable. The old pairs are copied aside at mode
0600 under the peering's own `pki` folder, and removed once the new pairs are
installed and validated. A run killed during the rotation is finished by the
next run from the pairs it had already minted, or, if they do not validate,
rolled back to the old pairs. No run mints a third pair.

`goatbuilder federate rotate <partner> carrier` replaces the forwarder carrier
keys of one running peering and runs the command again from phase 2 to 14.
Phase 2 sets each carrier key aside once and makes a new one. Until phase 2
passes, running the same `rotate <partner> carrier` again is the resume: it
reuses the rotation that is under way and sets nothing aside twice.

The other keys are refused before anything runs, each saying why: `transport`
and `signing` with GB-189, `transport` while the partner cannot yet receive
our new package; `ca` with GB-282, because rotating our CA needs an ADR first.

#### Added: `federate test --bilateral` completes the partner's half of acceptance, opt-in

`goatbuilder federate test <partner> --bilateral` runs phase 14 on a session
both sides work out from the agreement, with no manifest or result file
passed between the operators. Each side signs its result with its federation
signing key and publishes it on its own core, reads the partner's, and
verifies it under the key confirmed at phase 5; the combined result in
`<evidence>/bilateral/` is then marked authenticated. Both sides start phase
14 within `FED_SESSION_WAIT` (300 seconds by default) of each other, normally
on the same call; phase 1's clock check (GB-268) runs first, alone, and stops
the run on a clock that is unsynchronized or over the budget. It is opt-in:
without `--bilateral`, phase 14 runs as before and its exit is unchanged. A partner result that does not verify is GB-272,
one from another session or run GB-273, and an unset or unresolved signing key
GB-142. (#720)

#### Added: join-node refuses a node whose clock is off, before changing it

`goatbuilder join-node` phase 3 now reads the joining node's clock from its
own time service (`chronyc -c tracking`, else `timedatectl`), read-only, before
anything on the node changes. A node that is not synchronized to an outside
time source, or is more than 250 ms from UTC (`JOIN_NODE_CLOCK_BUDGET_MS`), is
refused with GB-297, naming the node and what it read. A Zenoh router refuses
a timestamp more than 500 ms ahead of its own clock, and nothing tells the
machine's owner. A clock that could not be read is reported as not verified,
never as passed. The rule is the federation preflight's clock check
(GB-268) (#766).

#### Added: join-node runs mesh DNS at a joining site

A site that joins a goatnet now runs its own mesh DNS, so it still resolves
the goatnet's service names when it is cut off from its founder. Phase 13
of `goatbuilder join-node`, for a joining site's core node, runs a replica
of the goatnet's zone on the node and registers the site's own nameserver
group, `<site>-mesh-dns`, which lists the site's replica first and the
founder's second. The group is written through the founder's management
API, with the founder's token (`<FOUNDER>_NB_API_TOKEN`), resolved through
the secrets injector; an unset or unresolved token is refused before the
step (GB-281). A site with no zone file to serve is refused when the plan
is made (GB-280). See ADR 1235 (#678).

#### Added: join-node builds a node on a phone with --class phone --via usb

`goatbuilder join-node <environment> --site <site> --node <node> --class phone
--via usb` joins the Linux VM on an Android phone as a `core` joiner
(docs/design/phone-node.md). Built on a fixture; not measured on a phone.

- `--via usb` reaches the node over the Android debug bridge until it is
  enrolled. The run writes the SSH configuration and the Ansible inventory
  line it uses into the state directory (`ops/scripts/phone-node-ssh.sh`), so
  none is written by hand, and takes no `--address`. From phase 12 the node is
  reached at its mesh address. A phone that does not answer is refused
  (GB-299).
- For `--class phone`, phase 2 prepares the phone's VM (Debian's kernel with
  its boot settings, Docker Engine, the deploy user) and then runs the
  integrity checks in it (`ops/scripts/phone-node-provision.sh`). A check that
  fails stops the run (GB-300). A VM already prepared goes straight to the
  checks. The script also runs by itself.
- The machine class `phone` declares once what a phone node differs in
  (`ops/ansible/group_vars/class_phone.yml`): no LAN address, no public
  ingress, no disk encryption inside the VM, memory limits for 4 GB, no
  observability host. The site lists its node under `[class_phone]` in
  `inventory/mgmt-hosts.ini` and writes none of those values; the config
  preflight takes `--class phone` and refuses a site that contradicts them
  (GB-301).

Every other class and transport plans and runs as before (#747).

#### Added: join-node and teardown keep a timings file, as standup does

`goatbuilder join-node` now writes a row for each step it runs to
`<state_dir>/join-node-<site>-<node>-timings.tsv`, and `goatbuilder teardown`
writes a row for each component it stops to
`<state_dir>/teardown-<site>-timings.tsv`. The columns are standup's: time
written, phase, step label, seconds, exit code. Both runs now end with
standup's timing line (the total and the five slowest steps), and join-node
prints "phase <n> took <s>s" for each step. A dry run writes nothing, and a
failed step is the last row, with its exit code (#802).

#### Added: a report's -o json document lists the deprecated items a run used

A run that uses a deprecated command, flag or value now lists each one in a
`deprecations` field of a report's `-o json` or `-o yaml` document and of a
refused run's error document, so a pipeline sees it without reading stderr.
The documents a verb prints in a shape of its own (`survey`, `show --json`,
`federate status`, `explain`, and `promote status --json`, `review --json` and
`readiness`) do not carry it.
Each entry names the command, the flag or value, the replacement, the release
that first warned and the warning line printed on stderr. A run that used none
has no such field, so every document is unchanged until the list in
`ops/goatbuilder/compat/deprecations.yaml` names something (#818).

#### Added: goatbuilder federate plan --accept host-allocation records a judged SKIP

A deployment that declares its other peerings by hand may not be able to make
every value the host-allocation check compares resolve. That check is not run
again by deploy, so its SKIP used to stop deploy for good. `goatbuilder
federate plan <partner> --accept host-allocation` now records that SKIP with
its exact detail inside the plan and its hash. The plan view shows it at
approval, and deploy goes ahead while the check reads exactly the same. When
it reads differently, deploy refuses (GB-188) and names what it reads now and
what was accepted. Only a check deploy does not run again can be accepted,
and only when it is SKIP (#779).

#### Fixed: a host-allocation SKIP says which case it is and what clears it

A plan whose host-allocation check was SKIP cannot be deployed as it stands:
deploy does not run that check again, so it refuses the plan (GB-188), and a
new plan records the same SKIP, unless the plan accepts it (`--accept
host-allocation`, below). The refusal used to say only "host-allocation was SKIP
when the plan was made". Now the plan view, the refusal and the run record
carry the check's detail, which gives the case and the route:
- the inventory names no host for this peering: run `goatbuilder federate
  allocate <partner>`, or name the hosts in `peering-edges.ini`;
- another peering on a shared machine has a value that does not resolve: each
  value is named with its key and the group whose vars set it;
- its agreement was not found: both paths that were looked at are named.

GB-188's catalog entry gains this cause (#753, fixes #742).

#### Changed: standup for a joiner site turns signal federation on

`goatbuilder standup <env> --site <joiner>` now runs the joiner's signal
federation with its founder in phase 9, after the mesh enrolment and before
the data router. It uses the same steps `join-node` runs in its phase 13: a
gate on both sites' declarations, the token and proxy on the joiner and then
the founder, each node's federation router, the joiner's relay tunnel and
pin, and the delivery proof. A joiner that declares
`signal_federation_absent_reason` gets the gate and a SKIP line. Before this,
standup left signal federation off, and the operator ran those plays by hand.
`ops/scripts/goat-standup.sh` plans the same steps. A founder's plan is
unchanged. `goatbuilder teardown` refuses those steps by name, since they
leave state at the founder. (#824)

#### Added: each timings row names its cause, and standup times phase 11 and sums the run

Every row of the timings files that `goatbuilder standup`, `join-node` and
`teardown` write now has a sixth column, its cause: `ok` for a step that
passed, the error code a failed step's output matched, or `uncatalogued` when
no code covers it. When standup stops at phase 11 for the operator step it
writes an open row for it, and the run resumed with `--from 12` closes that
row with the time the step took. The resumed run's timing line then adds the
total since phase 1 across every resume, phase 11's time, the total without
it, and the run's stops. `ops/scripts/goat-standup.sh` writes the sixth column
too, as `ok` or `failed` (#805).

#### Added: goat-standup.sh times phase 11, and standup's timing line names the slowest phases

`ops/scripts/goat-standup.sh` now writes an open row to the timings file when
it stops at phase 11 for the operator step, and the run resumed with
`--from 12` closes it with the time the step took, as `goatbuilder standup`
does. A run driven by the script, or by both drivers in turn, now has phase
11's time in its record. `goatbuilder standup`'s timing line now ends with the
three slowest phases across every resume, phase 11 left out, each counted by
wall time, so steps run side by side count once (#814).

### For someone contributing

#### Fixed: a changelog entry title written in quotes renders without them

`ops/release/changelog.py` read an entry's `title:` and `heading:` verbatim,
so a title written YAML-style, `title: "Added: …"`, rendered as a heading
with its quotes, and a quoted heading was refused as not one of the three.
A value wrapped in one matching pair of `"` or `'` is now read without that
pair; quotes inside a title are kept. The entries directory's README says a
title needs no quotes for its colon (#764).

#### Added: from a 1.0 baseline, a kept exit code no longer moves with a reason

`ops/goatbuilder/compat/compat.py` lets a pull request change a kept
invocation's exit code with its reason in `exit_changed:` only while the
last release is 0.x. Once the baseline is `goatbuilder/v1.0.0` or later, it
refuses `exit_changed:` unless the build's version in
`crates/goatbuilder/Cargo.toml` is a later major release, since a kept exit
code is part of the 1.0 promise (ADR 1228). Today's 0.7.0 baseline is
unaffected. (#813)

#### Changed: the compatibility check is held to goatbuilder/v0.7.0

`ops/ci/check-goatbuilder-compat.sh` now compares each build with
`goatbuilder/v0.7.0`, the last published release: its command surface and its
kept `-o json` documents. The upgrade tests' list pins v0.7.0's four
published binaries (`crates/goatbuilder/tests/upgrade/releases.sha256`), and
v0.7.0's configuration files are kept for gate row A2 with the outcome its
published binary gives each (#776).

#### Added: a deprecated command, flag or value warns on stderr when a run uses it

`goatbuilder` now reads the deprecation list it ships
(`ops/goatbuilder/compat/deprecations.yaml`) on every run. A run that uses a
command, a flag, or one value of a flag or positional argument that the list
names still works. It prints one line on stderr naming what it used, the
release that first warned, and what to use instead. A flag renamed with its
old name kept as an alias warns only when the old name is typed. An entry can
now deprecate one value of a positional argument (`flag: <COMPONENT>` with
`value:`), as the compatibility check already allowed. The list is empty
today, so no run prints a warning yet. The run's `-o json` document does not
carry the warnings yet (#798).

#### Added: CI holds the deprecation period once the last release is 1.0

The compatibility check (`ops/ci/check-goatbuilder-compat.sh`, in
goatbuilder-pr) now enforces ADR 1228's deprecation period. Once the last
release is 1.0 or later, a deprecation entry excuses a removed command, flag
or value only when all of these hold: the build is a later major release,
and this is the pull request that sets that version (its base branch is
still on the earlier major); the release the entry names shipped a
deprecation list that names the item, so its binary warned; two minor
releases, of any major, have been tagged since that release and before the
build's version; and 90 days have passed since that release was tagged. A refusal names the date it compared
against (today, UTC) and the date the 90 days end. Before 1.0 an entry
excuses a removal at once, as before. goatbuilder-pr fetches the release tags
for this check, and reads each named release's list from its tag (#800, #826).

#### Added: join-node's peer-groups query is measured against NetBird's own schema

`ops/netbird/tests/peer-groups-query.sh` starts the pinned NetBird management
on a disposable Postgres, enrols two real clients with keys from
`mgmt-host-bootstrap`, and runs the query `mesh-enroll-host.yml` uses at
join-node's phase 10 to read the groups the minting host's peer holds.
`tests/test_mesh_enroll_peer_groups.py` holds the query's shape always, and
its result when the script runs it. The query was designed and is now
measured. (#754)

#### Added: the release check starts at the cut-off, and the tag's tree check decides a rerun

The rules that hold while a release is cut (`docs/reference/goatbuilder/02-install.md`)
gain a fourth: the release check in the federation test environment starts on the version
commit at the cut-off, before the version pull request is opened. The tag goes on that pull
request's merge commit. Before the tag, a diff between the checked commit and the merge
commit must name nothing under `crates/`, `ops/ansible/` or `ops/federation-e2e/`, or each
stage those paths feed runs again on the merge commit (#772).

### To be removed

Nothing.

## 0.7.0 — 2026-11-14

What changed since 0.6.1, the previous release. Every refusal now names its
error code, and a failed phase of any verb names the error in its output with
its fix; `explain` answers for the phases of every verb.
A run records every value it uses and where each came from. `goatbuilder
status` reads real hosts, and `goatbuilder update verify-release` checks a
downloaded release without installing it. `join-node` names what a joining site
does not run and why, adds a node at a third site from phase 1 to 15, has no
manual phase after reaching the node, and deploys a joining site's data router,
its leaves, the health publishers it declares and, with signal federation off,
its front door. Phase 10 places and restarts nothing when its admission is
already in place. A back-out stops on a probe that
cannot tell, takes back what admission placed, and drops a role core partner's
store whole.

Two behaviour changes: an undeclared peering exits 1, refused, where it exited
2; and a back-out that used to pass over a probe that could not tell now stops
on it. Nothing that is half built ships switched on.

### For someone deploying

#### Changed: phase 10 leaves our core router running when the admission it would make is already in place

`goatbuilder federate deploy <partner>` with the admission done by the command
no longer places, dry-runs and restarts our core router again when nothing it
would place has changed. After a restart passes, the run journal records the
admission as placed. A later phase 10 is the same admission when it has the same
digest of what is placed (the approval digest without the plan hash) and every
file on the core is byte for byte what it would place. That phase 10 then
places nothing, restarts nothing and asks for no approval. This happens when
phase 10 runs again because a rotated leaf the core does not hold changed its
inputs. A file on the core changed by hand, or an admission whose restart
failed, is admitted again as before.

#### Changed: a back-out drops the partner's store whole on a goat-zenoh role core

`goatbuilder federate backout <partner>` now takes the partner's entry out of
`zenoh_sovereign_peerings` in `group_vars/<core router group>/peerings.yml`,
keeping every other line, runs the role's play, which restarts the router
without the partner's store, and then runs `ops/ansible/zenoh-drop-partner-store.yml`,
which deletes `<storage path>/sovereign_from_<partner>` once the router runs
without it (ADR 1234). Commit the edited `peerings.yml` through review. A store
the role kept because the router does not yet run without it is reported as
GB-270. The role no longer renders the shared `sovereign_from` store on
`from/**`: on a core that had it, its directory stays on disk with no retention
and is listed in `.undeclared-stores` until you dispose of it. The federation
partner bundle changed with the specification; partners holding the previous
bundle need the new one (#629).

#### Fixed: a back-out stops on a probe that cannot tell, and takes back what admission placed

`goatbuilder federate backout` reads a thing as absent only on the exact
"not found" line its tool prints for that name (docker, podman, `wg`, `ip`,
`nft` and SPIRE). A probe that failed any other way, such as a folder it
could not list or a system bus it could not reach, reads `unknown`, and an
`unknown` row stops the back-out instead of counting as absent.
**Behaviour change:** a back-out that used to pass over such a probe now
stops on it.

The back-out also takes back what admission placed, by the route admission
took. A core fragment the driver placed (`FED_CORE_*`) is removed the same
way. On a core deployed by the goat-zenoh role, the core router's
certificate and key are deleted only where they are byte for byte this
peering's leaf, and our core router's leaf is restored from the earliest
admission that touched it. An IPsec link's MSS clamp rules are removed by
their own handles (#554).

#### Fixed: the docker-engine role checks the root the running containerd uses

The `docker-engine` role reads the root directory the running containerd uses
before it writes `/etc/containerd/config.toml`. When the two differ and
containers are running, the play fails and writes nothing; with no container
running, the role writes the file and restarts containerd in the same play.
A final check compares the running daemon's root with
`docker_engine_containerd_root`. Before, a restart the handler missed could
leave containerd on an empty root after its next restart, so `docker exec` and
every healthcheck answered `NotFound` while the services kept running. The
data-root migration runbook gives the check by hand (#664).

#### Added: the end state grades federation-proxy where a site turns it on

`goatbuilder endstate` now has a row for `federation-proxy`, the node-local
proxy signal federation dials its partner through. A site that sets
`signal_federation_proxy_enabled` and does not run it, or runs it without
the flag, is now reported. The flag defaults to false, so a site that does
not set it is graded as not running the proxy. A site without the
`signal-federation-transport` ledger grades its deployment as unknown, never
as failed (#637).

#### Changed: every refusal names its catalog code, and an undeclared peering exits 1

The refusals that printed no catalog code now print one, on the line after
the message (`(GB-NNN — goatbuilder explain GB-NNN)`): a command line goatbuilder
cannot parse (GB-242); a mutation without `--yes` and without a terminal for
`apply`, `standup`, `teardown`, `join-node` and `federate` (GB-008); a
federation verb given site flags before it (GB-238) or a flag it does not take
(GB-239); an unknown `FED_VERBS` (GB-240); `--mock` on a verb that reads the
checkout (GB-276, new); and a peering that is not declared (GB-149). The
undeclared peering now exits 1, refused, as GB-149 says and as `federate
allocate`'s same refusal already did; it exited 2 (gate row C5) (#656).

#### Added: a two-driver peering's controller imports phase 1's edge result and runs its core half

On a peering declared `drivers: two`, `goatbuilder federate import <peering> 1
<file> --sha256 <code>` takes in the edge driver's phase 1 result: it checks
the file against the confirm code the edge driver printed, places it under
`<work>/handoff/1/` and records its sha256 in the run journal. `goatbuilder
federate <peering> --from 1` then hashes the file again, runs phase 1's core
checks (preflight `--side core`), joins the two sides' results, and goes on to
phase 2's hand-off. A result that cannot be imported is GB-277. The edge
driver's own run, and the import of later phases, are not built yet (gate row
B10).

#### Changed: what federate says about phase 10 names the command that runs it first

Phase 10 admits the partner's edge on our core, and `goatbuilder federate
deploy` runs it once you approve the plan and what our core admits. A dry run's
plan page now says "At phase 10, goatbuilder runs this for you:" and names
`goatbuilder federate plan`, `deploy` and `test` before the steps by hand. When a
run stops at phase 10 without a terminal, it names `plan`, then `deploy
--approve-plan <plan hash> --approve-admission <digest>`, then `test`, and gives
admitting by hand, then `--from 11`, as the fallback, which still works.
`goatbuilder explain federate 10` leads with the same commands. Phase 10 is
still listed as a manual phase.

#### Fixed: federate status resumes from the first phase it cannot show passed

`goatbuilder federate status` named the next command from the phase the last
run ended on, plus one. On a journal a later goatbuilder wrote, with phases
recorded as an outcome this build does not know (for example `deferred`), it
advised a `--from` past those phases, although such an outcome never counts as
passed. It now advises `--from` the first phase the journal does not show
passed. After a failure or an operator step, the advice is unchanged (#632).

#### Added: explain answers for join-node's phases and deploy's steps, and their failures name the error

`goatbuilder explain join-node [<phase>]` and `goatbuilder explain deploy
[<step>]` answer from the phase catalog, as they already do for `standup` and
`federate`: what the phase does, why, what it changes, how to tell it worked,
what it runs and the command by hand. With no phase given, they list the
verb's phases.

When a join-node phase or a group's deploy play fails, its output is matched
against the error catalog. The failure then carries an `interpreted` row: the
error's code, its likely cause and fix, the log, and the command that runs
that step again by hand. With no known error, the row gives the phase's own
guidance.

The standup runbook's phase table and the federate table in the sites chapter
of the reference are now rendered from the phase catalog. The runbook's table
has one row a phase, with what it does and how to tell it worked; `explain
standup <phase>` gives the rest.

#### Added: join-node deploys the data router at a joining site

`goatbuilder join-node` now runs the site's own Zenoh router on a joining
site's core node, in phase 13 after signal federation: `zenoh-deploy.yml`
for the site's core group, limited to the node. It was a hand step after
the command. The play still needs the node's router leaf certificate,
which join-node does not stage yet.

#### Added: join-node deploys the health publishers a joining site declares

`goatbuilder join-node` now deploys a joining site's health publishers on
its core node, in phase 13 after the data router. It reads the site's end
state (`endstate.py model <site>`) and runs the deploy play of each
publisher the model marks present (fabric-health, mesh-health,
presence-health, time-health, consistency-health, fabric-crawler), for the
site's core group limited to the node. They were hand steps after the
command. A publisher the model marks present whose play is not in the
checkout is refused when the plan is made (GB-288). When the model cannot be
read, the plan says so on a SKIP line and deploys none.

#### Added: join-node sets up a joining site's front door when signal federation is off

`goatbuilder join-node` now sets up the front door of a joining site that
declares signal federation off, in phase 13 after the data router, for the
site's core group limited to the node. A site that declares
`dashboard_cert_delivery_mode: acme` gets its public certificate and routes
(`traefik-acme.yml`); any other site gets its LAN routes
(`lan-traefik-routes.yml`, from the internal-CA leaf the phase mints before
the router). Before, only signal federation set up the front door, so such a
site had none. With federation on, nothing changes.

#### Added: join-node mints and stages a joining site's leaves

`goatbuilder join-node` now mints a joining site's router and service leaves
and stages them, in phase 13 before the data router. It runs
`mint-site-certs.sh <site>` from the site's manifest
(`ops/enrollment/manifests/<site>-certs.yaml`), which keeps any leaf already
minted, then `stage-service-leaves.yml` for the service leaves the manifest
carries, limited to the node. Both were hand steps after the command. A
site with no manifest is refused when the plan is made (GB-286), and so is a
manifest with no router leaf (GB-287).

#### Added: `join-node` names what the site does not run, and why

The dry-run plan and the summary of a real `join-node` run end with a section
headed "What <site> does not run". It lists every catalog service the
end-state model (`endstate.py model <site>`) marks absent at the site, each
with the model's reason. A service the site turns off reads its flag, for
example "`isitup_enabled` is false in group_vars". At a joining site, the
NetBird dashboard, the portal, the id console and the onboarding engine read
"single-homed at the founder (ADR 1084 §D3); every site once ADR 1165 lifts
it for this class", followed by the declaration that keeps each one off.
`goatbuilder endstate` gives an absent service the same reason, so the plan
and the grade cannot disagree. When the model cannot be read, the section
says so in one line and lists nothing; the run is not stopped. (#612)

#### Changed: join-node has no manual phase after reaching the node

`goatbuilder join-node` stopped at phase 8, to mint a setup key, and at
phase 10, to enrol the node by hand. Now phase 10 enrols the node itself. It
mints a single-use, one-hour key where the NetBird database is (for a joiner
site, the founder's first host), hands it to `netbird up` through a 0600 file
on the node, and checks that the node is on the kernel ACL plane. The key
never reaches your machine and is not stored. Phase 10 then writes the node's
mesh address into your checkout; the run's summary and its run record name
each file it wrote, for you to review and commit. Phase 8 now checks that the
site declares `mesh_enrollment_url`, and phase 9 sends no setup key. Phase 2
resolves the first-login password through the secrets injector and refuses
one that still holds a reference (GB-269); the password reaches `sudo` on
standard input. Only phase 1, reaching the node, stops the run (#661).

#### Fixed: join-node adds a node at a third site, phases 1 to 15

Three refusals stopped `goatbuilder join-node` for a node not yet enrolled at
a goatnet that already has a joiner. Phases 3 and 7 refused the mesh-address
placeholder that phase 12 fills: they now run the config preflight with
`--before-enrolment`, which warns on that one placeholder and still fails on
any other. Pass 1 of phase 13 waited for a management service that needs
the Ory deployed by the next step: it now runs the stack plays only, as
`standup` does. A third site that declares signal federation off, with its
reason, was refused because its founder federates with another site: it now
passes, unless the founder's federation names it (#613).

#### Changed: container memory limits and Postgres memory are settable, and cron is installed

Every container memory limit and reservation in the roles and compose files
is now a variable whose default is the value it had, so a small host can
lower them in its group_vars. Postgres `shared_buffers` and
`max_connections` keep the image's values unless
`mgmt_postgres_shared_buffers` or `mgmt_postgres_max_connections` is set;
`mgmt-host.yml` refuses an `mgmt_postgres_max_connections` that differs
from `postgres_max_connections`. The roles that schedule jobs with cron
(`apt-cacher-ng`, `docker-registry-mirror`, `oci-substrate-t2-sync`,
`isitup`, `operator-idp-postgres` and the observability play) now install
`cron` first, so a minimal Debian host runs those jobs (#615).

#### Changed: the forwarder's carrier read-back reads each peer's public key and allowed address

`goat-peering-forwarder` reads each carrier back with `wg show <carrier>
allowed-ips` and `listen-port`, and a back-out checks that a carrier exists with
`listen-port`. When a carrier does not hold what the declaration says, the failure
message lists each peer's public key and allowed address, then the port the
carrier listens on (#663).

#### Changed: the WireGuard read-back of phase 11 reads each peer's public key and allowed address

`goat-peering-wg` reads the interface back after phase 11 with `wg show
<interface> allowed-ips`, and the carrier's with `allowed-ips` and
`latest-handshakes`. When the interface does not hold what the agreement says,
the failure message lists each peer's public key and allowed address, then the
interface's addresses. GB-271's explanation describes the message as it is now.

#### Fixed: a phase 11 link that did not come up is named as GB-271

When phase 11 of `goatbuilder federate` fails because the peering's
WireGuard interface does not hold the agreed peer and address, the failure's
`interpreted` row now names GB-271 instead of X-ANSIBLE-003, which pointed
at an empty secret and `deploy --dry-run`. The likely causes, most likely
first: the link unit is still running as it was before phase 11 and was not
restarted; the unit was restarted but its link script stopped partway; or
something else changed the interface afterwards. The next step: restart
`goat-peering-wg@<interface>.service` on the edge and check the interface
again, read the unit's journal if it still does not match, then resume with
`goatbuilder federate <peering> --from 11`. `goatbuilder explain GB-271`
gives the same. Every other failed assertion still reads X-ANSIBLE-003
(#646).

#### Fixed: the command reference lists federate's own flags

`goatbuilder docs --markdown` and `docs/reference/goatbuilder/command-reference.md`
documented a global flag only on the root command. The flags `federate`
declares for all its verbs (`--over-ssh`, `--side`, `--half-out`,
`--edge-half`, `--edge-half-sha`, the host, group and router-container flags,
and the goat context flags) appeared on no command. They are now documented
once, on `goatbuilder federate`, so the compatibility check also holds them
against the last release (#633).

#### Changed: a refusal names its error code

A refusal from `standup`, `join-node`, `release`, `rollback`, `pins`,
`doctor`, `promote`, `fastlane`, the fresh-source check, the converger,
`inspect` and the configuration loader now prints its `GB-nnn` code, so
`goatbuilder explain <code>` gives its causes and fixes. The error catalog gains
GB-238 to GB-260, GB-263 and GB-264 for refusals that had no code. A
refused `release` step ends its row with the code instead of dropping it,
and GB-008 names every verb that asks for `--yes`. A few refusals of the
command line itself and two of `federate`'s still print no code; they get
theirs in a later change (#560).

#### Fixed: a release re-run on a later day opens no dev-pin PR

`goatbuilder release` declares a candidate on the dev rung with a reason that
carries the day. Re-running a dev step that had already landed, on a later
day, rewrote that reason and landed a pin PR that changed nothing but the
date. A rung that already declares the candidate now keeps its deviation as
it stands, reason and date included (#622). The script avenue,
`ops/scripts/rc.sh`, does the same (#621).

#### Fixed: rollback reaches a goatnet through its door, as deploy does

`goatbuilder rollback --site <goatnet>`, on a site whose map is one host,
tries the host directly. When it does not answer, rollback goes through the
env door that `deploy` uses for that site (the netbird profile, the mesh
host on tcp/22, then the door registry). The connection still goes to the
ledger's own host and checks that host's key: the door only carries it,
so a door that lands on another host can never stand in for it. A door that
does not open falls back to direct ssh. Without `--site`, or for a site of
several hosts, nothing goes through a door. `ops/scripts/rollback.sh` now
reaches the ledger's host the same way, where it used to reach whatever
host the door landed on, and the live canary passes `--site` to both
rollback avenues and to `rollback --list` (#644).

#### Added: a run lists every value it uses and where each came from

`goatbuilder standup`, `join-node`, `deploy` and `federate` now record every
value their commands use in the run record's `inputs` (`goatbuilder show <run>`,
`goatbuilder runs diff`), each with its source: a file and line, the
declaration and rule it was derived from, an environment variable, a default,
an allocation or an argument. A dry run prints the same list, as `input` rows
under `-o json` and as a section of the plan page, and names the secrets
posture probes it ran to read them. A secret is shown by its source only,
never its value (#649).

#### Added: site bootstrap installs the hub-directive applier at Spock sites that name a hub

`site-bootstrap.yml` now applies the `spock-hub-directives` role (ADR 1205)
at each site that replicates with Spock and sets `goatnet_hub_site`. A Spock
site that names no hub gets a message and no applier. The play carries the
tag `hub-directives` beside its phase tag, so it can be run alone. (#596)

#### Fixed: deploy no longer refuses a plan with an edge host firewall as changed (GB-188)

With the host firewall baseline on for an edge, `goatbuilder federate` phase 10's
deploy refused the approved plan on every attempt with GB-188 ("host_firewall
changed since the plan; host-firewall BLOCKED") and printed the renderer's JSON
into the transcript. Inside a run, a command started in the background lost
the output its caller had piped to the runner's own streaming, so deploy's
recheck read no ruleset from the renderer. The runner now leaves a stream the
caller set to the caller, and deploy's recheck reads the same ruleset and hash
the plan did.

#### Fixed: phase 13 no longer replaces the partner's refreshed SPIFFE bundle with the older exchanged one

`goat-peering-spire` imported the partner's bundle from the file exchanged in
phase 5 on every phase 13, with `spire-server bundle set`. That overwrites
the bundle our server had already refreshed from the partner's bundle
endpoint. After the partner's SPIRE rotated its keys, a rerun of phase 13
put the old keys back: their new identities stopped verifying until the next
refresh, and that refresh authenticates the endpoint with the bundle held.

Phase 13 now imports the file only in two cases: when our server holds no
bundle for the partner, or when the file's `spiffe_sequence` is newer than the
one held. Otherwise it keeps the held bundle and says so. For a partner whose
SPIRE server was rebuilt with new keys, confirmed on the call, set
`peering_spire_bundle_replace: true` to import the file over any bundle held.

#### Fixed: the audit-subscriber leaf is staged with the other service leaves

`stage-service-leaves.yml` now stages the audit-subscriber leaf and its key
into `/opt/audit-subscriber/certs`, where the audit-subscriber role reads
them, and `goatbuilder join-node` stages it in phase 13 when the site's
manifest carries it. Before, a site that declared its own audit-subscriber
copied the leaf by hand before `audit-subscriber-deploy.yml` could pass. The
role reads the CA root from the router's TLS directory, which the data
router's play fills, so the CA root is not staged.

#### Added: a failed standup phase names the error in its output

When a standup phase fails, its output (the log the run kept of it) is matched
against the error catalog, as `goatbuilder federate` already does. The failure
then carries an `interpreted` row: the error's code, its likely cause and fix,
the log, and the command that runs that phase again by hand. With no known
error in the output, the row gives the phase's own guidance from the phase
catalog. This covers the numbered phases, the image pull, the observability
legs and the phase-12 steps that `--parallel` runs in waves, each read from its
own log (#652, #669).

#### Added: goatbuilder status reads real hosts

`goatbuilder status --env <env>` without `--mock` refused with GB-026. It now
reads each machine's deploy ledger the way `goatbuilder diff` reads it and grades
it the way `goatbuilder verify` grades it, so the three agree about every
machine. It reads only: no play runs and nothing is written. It exits 3 while
any machine is pending or unknown. GB-026 is retired: no release after this one
prints it (#648).

#### Fixed: a back-out that stops after its plan says why, with its code

`goatbuilder federate backout <partner>` printed its plan and then stopped
without saying why when the survey refused (GB-221) or the slot recheck was
blocked (GB-223). The plan was all that reached the terminal. The refusal, its code and
its fix are now printed after the plan, as are a later stop's rows: an
unconfirmed back-out, a removal play that failed (GB-225), and a back-out
that did not converge. A `federate` dry run whose values could not be listed
now says so after its plan, too.

A run record (`<state>/runs/<id>/run.json`) of a run that stopped without an
error now keeps the rows that stopped it, under `stop`, and
`goatbuilder show <run>` prints them (#668).

#### Added: `goatbuilder update verify-release` verifies a downloaded goatbuilder release without installing it

`goatbuilder update verify-release <dir> --signer <owner>/<repo> --version
<x.y.z> [--trusted-root <file>]` verifies a downloaded goatbuilder release
without installing it, with the checks `goatbuilder update` makes before it
installs: the keyless signature over `SHA256SUMS` (`SHA256SUMS.cosign-bundle`)
for the release workflow of `<owner>/<repo>` at `goatbuilder/v<version>`, and
every other file in `<dir>` against that `SHA256SUMS`. It reads only. A
refusal names the failed check: GB-261 for the signature, GB-074 for a file
(#655).

#### Fixed: an edge deployed from a workspace gets the certificates it mounts, or phase 12 says why

`goatbuilder federate` run from a machine that is not the edge host (an operator's
workspace) used to deploy an edge whose leaves and CAs lived only on the
workspace, in `<work>/pki`: the edge started, mounted nothing and never reached
the core. Phase 12 now refuses first (GB-275) when the edge group's vars point
the edge at files in `<work>/pki`, the inventory names the edge as another
machine, and `goat_zenoh_edge_install_certs` is off; the fix is to turn that
switch on. With it on, the `goat-zenoh-edge` role now also installs our CA's
certificate as `<cert_dir>/our-ca.pem`, from `<work>/pki/our-ca.pem` (never the
CA's key: a source holding a private key is refused), and `goatbuilder federate
allocate` renders our CA's path and the core endpoint's override at that path
(#658).

### For someone contributing

#### Added: a library that signs a side's acceptance result and verifies the partner's

`crates/goat-federation/src/acceptance_sig.rs` signs a phase 14 result with
the side's federation signing key and verifies a partner's under the key
bound to the partner at phase 5. The signature covers a fixed-format
statement whose first line is `goat-acceptance-result/v1` and which names
the agreement, session, attempt, both parties, the echoed challenge and the
sha256 of the exact result bytes. Nothing else the key signs begins that
way, and the package and directory signers refuse it. A result that does
not verify is GB-272, and one from another session is GB-273. No command
reaches the library yet; it is the first part of gate row B2 (#654).

#### Fixed: CI runs on main to the end, and several checks say why they fail

- A run on `main` is no longer cancelled by the next push to `main`, so the
  commit a release is tagged on keeps its own complete run (#618).
- The playbook syntax step prints each failing playbook's error, and an
  Ansible collection that a timed-out install left half written is
  installed again (#625).
- `.envrc.example` is regenerated, and a check holds it to the environment
  variables the repository reads (#627).
- The filter plugin tests moved out of `ops/ansible/filter_plugins`, so
  Ansible no longer imports them on every playbook run (#626).
- The hermetic deploy job's release re-run and soak cases pass on a run
  that crosses midnight UTC (#621).

#### Changed: the compatibility check holds more of the last release, now goatbuilder/v0.6.1

`ops/ci/check-goatbuilder-compat.sh` compares each build with the last
published release, now `goatbuilder/v0.6.1`. It keeps `-o json` documents
for the verbs that change state (`deploy`, `rollback`, `plan`, `standup`,
`teardown`) and for run journals, and holds a field that only some entries
of a list carry from a reviewed list (`conditional.yaml`). A kept command's
exit code changes only with a reason in `exit_changed`, and a value a flag
accepted in the last release is still accepted. The published binaries'
checksums are one list, `crates/goatbuilder/tests/upgrade/releases.sha256`,
which the upgrade tests own and the check only reads, and it holds
v0.6.0's and v0.6.1's binaries. The configuration files of both are kept
for gate row A2 beside those of the earlier releases (#566, #617, #562,
#563, #631, #634, #616, #674, #675, #676).

#### Added: contract clause C11, legibility, is written beside C1 to C10

Clause C11 is now in the deploy contract, in `docs/design/goatbuilder.md`
and the contract chapter (`docs/reference/goatbuilder/06-contract.md`):
for every action the tool takes, its command, its inputs with their
sources, and its reason can be read before, during and after the run. Both
name its conformance test, `crates/goatbuilder/tests/legibility.rs`, and
say what each of its four checks requires. C11 does not hold yet: the
checks that fail today are listed in
`crates/goatbuilder/tests/legibility-known-failing.txt` (#647).

#### Changed: a documents-only pull request runs only the documents' configuration examples

`goatbuilder-pr` decides from the pull request's own diff whether it changes
documents only (`ops/ci/goatbuilder-pr-scope.sh`). A documents-only pull
request then runs the check that every whole configuration example in the
documents validates, and not the whole build, test and macOS jobs; a change
to the tutorials or to anything else still runs everything (#685).

#### Fixed: the federation test environment's other clones own their objects

The watcher's preview and reverts and the upgrade unit made `--shared`
clones, which read every object from the source and lose a commit once the
source prunes it. They now use `src_clone_ref` in
`ops/federation-e2e/lib/srctree.sh`: a plain local clone at a branch, tag
or sha, which keeps its objects and refuses a shallow source or an empty
argument before anything is removed. No harness script makes a `--shared`
clone any more (#643).

#### Fixed: the federation test environment's source clone holds every object of the commit under test

The federation test environment builds each commit under test from its own
clone. From a shallow source repository, that clone could miss a commit
that no ref held, so the checkout failed with `fatal: unable to read tree`
and the build found no `crates/` directory. The clone
(`ops/federation-e2e/lib/srctree.sh`, used for goatcore and for the
saltlick directory) is now a plain clone, and fetches the commit by sha
when it cannot read its tree, so it holds the commit's objects itself. A
clone from an earlier run is reused only when it is checked out at that
commit and can read its tree. A failure stops with `no tree for <sha>`
instead of going on to build nothing, and a sha that is not 40 lower-case
hex characters is refused before anything is removed: an empty one would
have named the directory of every cached tree (#638).

#### Fixed: the goat client's doctor tests write their fake netbird from a child process

Two tests of `goat doctor` failed at random: each wrote a fake `netbird`
script and ran it at once, and when another test thread forked while the
script was still open for writing, running it failed with ETXTBSY. A test
helper now writes the script and sets its mode from a short-lived child
process, so the test process never holds it open. Only test code changes
(#314).

#### Fixed: an in-process federation verb no longer writes the process environment

When goatbuilder runs a federation verb in process, phase 3's `init` used
to read `DOCKER_HOST` and `FED_ROUTER_CONTAINER` from the process
environment, so goatbuilder wrote both into it with `std::env::set_var` for
the verb's duration, which every thread shares. The values are now passed
to the measurement (`derive::RouterEnv` in goat-federation), and
`DOCKER_HOST` reaches the container runtime's commands as their own
environment. `goat federate init` reads its environment as before. A test
refuses any `set_var` or `remove_var` under `crates/goatbuilder/src/federate/`
(#641).

#### Changed: the federation test environment's kill-resume aims phase 11's kill

The kill-resume scenario kills phase 11 once the WireGuard role has written
the link's files and before its handler restarts the link, instead of a
fixed delay after the phase starts. The gap that 0.6.1 fixed (#597) is now
found on every run, not by chance (#620).

#### Changed: a changelog entry is a file of its own, and a new ADR's index row is rendered

A pull request adds its goatbuilder changelog entry as a file in
`crates/goatbuilder/changelog.d/` instead of editing `CHANGELOG.md`;
`ops/release/changelog.py` renders the Unreleased section and folds the
entries into a release. The index row of an ADR above 1234 is
rendered from the ADR's own header by `ops/release/adr_index.py`, so adding
an ADR edits no shared file. `docs-guards` checks both (#623, #636).

#### Changed: the pull request template drops its co-author trailer line

The pull request template no longer carries a checklist line about
co-author trailers added by tooling. The rule is unchanged: ADR 0109 still
states it, and `pr-readiness` still refuses such a trailer in a commit
message (#315).

#### Added: the rules that hold while a release is cut

The release procedure (`docs/reference/goatbuilder/02-install.md`) now says
when the tag goes on, and three rules from the version branch's cut-off to
the tag: merges freeze except for the version pull request and the changes it
names; a run on `main` cancelled by a later merge is run again once and is not
a failure; and an engineering question that comes with a recommended option
does not hold the release once 30 minutes pass without an answer, while a
question of policy, cost or people, or about an act that cannot be undone,
waits (#673).

#### Added: rotating the WireGuard peering key, as a library and in the role, not yet on the command line

`rotation::transport_run` rotates a peering's WireGuard key (`wg-fed0`). It
writes `<work>/rotation.json` naming the rotation once, keeps the rotation's
id in `<work>/.rotation-transport` until phase 2 passes, and runs from phase
2. `goat-peering-wg`'s `tasks/transport-key.yml` then sets the key aside once,
in a `.previous-<id>/` directory, and makes a new one, and
`tasks/transport-live.yml` restarts the link while the interface holds a key
other than the one on disk. Without a rotation id, phase 2 never rotates the
key. The `rotate` verb does not call it yet, and the partner taking the new
package is not built (#680).

#### Fixed: standup passes the trunk and the mirror token to its phases as run state

`goatbuilder standup` no longer writes `GOAT_FRESH_SOURCE_TRUNK` and the
pub/sub-state mirror bot's token into its own process environment in the
middle of a run. Both are run state, given to each phase's process as its
own environment. The trunk goes to every phase, as before. The token goes
only to the two phases that read it, the mint step and the mirror play, and
is removed from every other phase's environment, even when the operator's
own environment holds it: least privilege for child processes. A test
allows a process-environment write only in `main`, before any thread
starts (#642).

#### Added: standup's phases are in the phase catalog, and explain answers for them

`ops/phases/catalog.yaml` now describes standup's 14 phases and its named
steps (the image pull and the observability legs, with their joins), each
with what it does and why, what it changes, how to tell it worked, what it
runs, the command by hand and what to do when it failed. A test builds
standup's plan for six site shapes and holds the catalog to it, so the two
cannot drift. `goatbuilder explain standup [<phase>]` answers from the
catalog, by number or name, as it does for federate (gate row F4) (#651).

#### Added: a test that backing out a role-admitted core gives its org fragment back

A unit test holds the back-out to the federation test environment's real
fixtures: admission's `merge_org` turns the core's org fragment from before
the peering into the admitted one, and `backout_core.py plan` turns the
admitted one back into the fragment from before, byte for byte
(`crates/goatbuilder/tests/fixtures/backout-role-core/`) (#547).

#### Added: every whole configuration example in the docs is validated

`crates/goatbuilder/tests/doc_configs.rs` runs every `yaml` block under
`docs/` that holds `schema: goatbuilder/v1` through this build's
`goatbuilder validate`. An example meant to be refused says so, with its
code, on its first line. The two examples that their own release refused,
in the configuration reference and tutorial 00, are fixed, and the test
runs on every pull request that touches goatbuilder or a document (#568).

### To be removed

Nothing.

## 0.6.1 — 2026-11-13

The fix for 0.6.0's known issue: a `goatbuilder federate` run killed in phase
11 now resumes and brings the WireGuard link up. It also carries the first
piece of the two-driver run (gate row B10), which changes `--side edge` on a
peering run and reserves the peering name `import`; see below.

### For someone deploying

#### Fixed: a federate run killed in phase 11 resumes and brings the link up

A run killed after phase 11 wrote the WireGuard link's files, and before it
restarted the link, left the interface with no peer. Resuming with `--from 11`
changed no file, so the link was never restarted and phase 11 refused, then
phase 12. The `goat-peering-wg` role now reads the interface and restarts the
link whenever the interface does not hold the agreed peer and address (#597).

#### Added: a two-driver peering stops at the first phase that needs the edge

A peering may declare `drivers: two` on its own `peerings:` entry (schema
`peering.drivers`, `one` or `two`, default `one`), for a site where no one
machine reaches both the edge host and the core host. Such a run stops before
acting at phase 1 or 2 and hands the edge driver a printed object naming every
command it runs (GB-265, exit 0; with `-o json`, the report's `handoff`
member). A two-driver run that would reach phase 3 or 11 to 14, and a
two-driver `federate deploy`, are refused before acting (GB-267). A controller
whose checkout is not clean gives no object (GB-266). Nothing after the stop
is built yet: the edge driver's run and `import` are refused with GB-267
(#601).

Behaviour changes for every peering:

- `goatbuilder federate <partner> --side edge`, with no other measure flag, is
  refused with GB-267 (exit 1) instead of the uncoded measure-flag message
  (exit 2).
- `import` is a reserved peering name, and a declared peering named `import`
  is refused by the configuration check.
- `goatbuilder federate` gains the subcommand `import` (refused, GB-267) and
  the flag `--expect-commit`.

A peering without `drivers: two` runs as before.

#### Images and roles built from this repository

- `ory-deploy.yml` reads the Postgres superuser password from the site's
  `<SITE>_PG_PASS` in the deploy environment when no `-e` gives it, and checks
  it is set before anything reaches the host. The sign-in provider's secret is
  checked before its file is written, and the migrations and the start carry
  `--no-deps`, so an Ory deploy never recreates `postgres-a` (#593).
- The audit-subscriber role's lifecycle steps run `mc` with a writable `HOME`,
  so they no longer stop at the first step on a host with a new MinIO
  container (#595).

### For someone contributing

- The federation test environment's two-sovereign job runs as a build job, a
  job per scenario and the required check over them, under its unchanged
  name (#602). A script gives each stage a key from its whole input
  directories (#603); reusing a stage's result by that key is not built.
- CONTRIBUTING: a fix shows its failure on main before its pull request
  opens (#588).
- Tests: deploy's success path is held to what it prints and records (#606);
  the federation test environment records a link that does not re-form
  (#600), declares `drivers: two` in its two-driver scenario (#608), and its
  hermetic job bounds and retries its package installs (#548).
- Designs: one path for faults on peers we do not own, ADR 1232 (#594);
  `join-node` builds the whole declared site, designed (#604); island mode's
  pre-minted certificate batch (#605); ADR 1233, a joiner runs the
  replication core, proposed (#607).

### To be removed

Nothing new. `goat federate <verb>` remains announced for removal as in 0.6.0.

## 0.6.0 — 2026-11-13

What changed since 0.5.3, the previous release. Every run that drives hosts
keeps a record, read with `goatbuilder runs`, `show` and `runs diff`, and
every process it starts is named before it prints. `goatbuilder update`
verifies the release signature itself, without `cosign`. Admission keeps
everything else in the core's peerings file and refuses one it cannot read as
literal entries. A back-out reads the core the way admission wrote it and
stops on anything it cannot read. Several peerings each keep their own
directory package by default, which is a behaviour change.

Nothing that is half built ships switched on. Work that was not proven by
this release's cutoff is listed under "Not in 0.6.0" below; no command in
0.6.0 reaches any of it.

### For someone deploying

#### Changed: `update` verifies the release signature itself, and always

`goatbuilder update` now verifies the keyless signature over the release's
`SHA256SUMS` in the binary, with no `cosign` needed and nothing fetched beyond
the release's own signature bundle. Before, the signature was checked only
when `cosign` was on PATH; without it, the update installed with the
signature "not checked". Now an install without cosign enforces it too. A
release whose signature does not verify, or that serves none, is refused
(GB-261), and nothing is installed. Every goatbuilder release published
with a signature (0.4.1, 0.4.2, 0.4.3, 0.5.1 and 0.5.3) verifies.

The signature must be the release workflow's (`goatbuilder-release.yml`) in
the repository `goatbuilder.yaml` names, at the served version's tag. The
Sigstore trusted root is built in. After a Sigstore key rotation, set
`GOATBUILDER_UPDATE_TRUSTED_ROOT` to a `trusted_root.json` that holds the new
key.

#### Changed: a run journal a later goatbuilder wrote is read, not refused

`<work>/state.json` written by a later goatbuilder, with an outcome word or a
field this one does not know, is now read. The word is shown as written
(`14 deferred (written by a later goatbuilder)`), never counts as passed, so a
resumed run runs that phase again, and a rewrite keeps what it does not know.
Only a later schema is refused (GB-182), naming the goatbuilder that wrote it.
A plan a later goatbuilder made is refused by name (GB-201): make a new plan.

#### Added: a record of every run, read with `runs`, `show` and `runs diff`

Every run of a verb that drives hosts writes a record,
`<state>/runs/<id>/run.json` (`goatbuilder.run/v1`, mode 0600), where
`<state>` is `deploy.state_dir`, else `~/.goat/state`. It holds the run's id,
the goatbuilder version and the tree's commit, the verb and its arguments,
the start and end times, the exit code and the error that ended the run, each
phase, and each command with its host, log and exit code. A run that stops
inside a phase writes its record too; a refusal of the arguments before any
phase, and a dry run, write none. The record has a place for the run's inputs
and their sources, but `standup`, `join-node`, `deploy` and `federate` leave
it empty in 0.6.0. A secret's value is never written.

- `goatbuilder runs [--verb V] [--last N]` lists the records, newest first.
- `goatbuilder show <run>` prints one record phase by phase and command by
  command; `-o json` prints the record itself.
- `goatbuilder runs diff <a> <b>` compares two records: arguments, inputs,
  commands in order, and each phase's exit code.

A run is named by its id, a unique prefix of it, or `last`.

#### Added: every process goatbuilder starts is named before it prints

Every child process `goatbuilder` starts goes through one runner. Inside a
run, before the process can print anything, a frame on stderr names the
phase, its purpose, the exact command, the host and its log, under
`<state>/runs/<id>/logs/`, and the process's own output follows live.
`standup`, `join-node`, `deploy`, `federate` and the other verbs that drive
hosts all start their processes this way, and `-v` prints each command a run
starts. Under `-o json` or `-o yaml`, a child's output inside a run goes to
stderr, so stdout carries only the document.

#### Added: a dry run prints the commands the real run executes

`--dry-run` now prints, on a terminal as well as in `-o json`, the command
each step will run, exactly as the real run runs it: every phase of
`federate`, `standup`, `deploy` and `join-node`, verbatim and on one line, so
it can be pasted. What the dry run had to run itself to make the plan, such
as a read-only probe, is listed under "This dry run ran", and what only a
real run will run is listed under "A real run also runs", so nothing the real
run does is left out. `release`'s dry run still describes each step in the
plan's words, and labels them so.

#### Added: a catalog of federate's fourteen phases, and `explain federate <phase>`

Each phase of `goatbuilder federate` is described in one catalog,
`ops/phases/catalog.yaml`: what it does, why, what it changes, how to tell it
worked, and the command by hand. The runbook's phase table is rendered from
it. The other verbs' phases are not in a catalog yet.

`goatbuilder explain federate <phase>`, by number or name, answers from the
catalog, as `explain` answers for an error code, and `goatbuilder explain
federate` lists the phases.

#### Added: a failed federate phase is matched against the error catalog

When a phase of `goatbuilder federate` fails, the end of its logged output
(the last 64 KiB) is matched against the error catalog. The failure then
carries an `interpreted` row: the matching code with its title, likely cause
and fix, the log, the phase's command by hand from the phase catalog, and the
`goatbuilder explain <code>` that tells more. When no known error matches,
the row says so and gives the phase's own guidance. The failure line, the
resume hint and the exit code are as before. Standup, join-node and deploy
do not interpret a failed phase yet (#585).

#### Changed: several peerings each keep their own directory package by default

The peering directory (the `saltlick` directory service, proven against its
v0.5.0 release) keeps one package per counterparty by default, so one
environment can hold several peerings, each with its own package.
`FED_DIRECTORY_PER_COUNTERPARTY` is now on unless set off. **Behaviour
change:** a deployment that relied on one package for every partner sets
`FED_DIRECTORY_PER_COUNTERPARTY=0`.

**With a directory older than saltlick v0.5.0**, which has no package per
counterparty, phase 5 says so (`WARN the directory does not host a package per
counterparty`), publishes to the single package slot, and then holds the
partner's package behind `theirs.pointer.json` until it is confirmed. Nothing
unconfirmed is placed. A party with more than one peering keeps the single
package its first peering published, so it is never replaced (0.5.x replaced
it, which gave the first partner the second partner's package). A later
partner then reads that first package, and its agreement does not match: the
digest compared at phase 6 shows the mismatch, and no data crosses on it.
Before adding a second peering, move the directory to saltlick v0.5.0 or
later. Where 0.5.x
placed the partner's package directly, 0.6.0 stops for a confirmation: confirm
the package on the call and re-run with
`FED_DIRECTORY_PLACE_POINTER_PACKAGE=1`, or keep the 0.5.x behaviour, its
replaced package included, with `FED_DIRECTORY_PER_COUNTERPARTY=0`.

#### Added: phase 3 and phase 1 can run one side at a time

For a site where no single machine reaches both the edge host and the core
host, phase 3's measurement runs in two halves, each on the machine that
reaches its host: `goatbuilder federate init --over-ssh <peering> --side
edge --half-out <file>` measures the edge, and the core side reads that file.
Phase 1's preflight takes `--side edge` or `--side core` in the same way.
The full run does not yet hand over between the two machines.

#### Fixed: admission keeps the other variables of the core's peerings file

On a core deployed by the goat-zenoh role, phase 10's admission writes the
partner's entry into the core router group's `peerings.yml`. It used to
rewrite that file holding only `zenoh_sovereign_peerings`, dropping every
other variable and every comment. It now replaces only that list's block and
keeps every other line byte for byte, so a value such as `0644` is never
written back with another meaning.

#### Fixed: admission refuses a peerings file it cannot read as literal entries

An entry whose `sovereign` was written as a template never matched, so
admission appended a second entry for the same partner. A
`zenoh_sovereign_peerings` written as a template string, a mapping or a
scalar was read as empty, so admission wrote back a file holding this
partner alone. Admission now refuses both forms with GB-237 before anything
is written, and names the entry by its position.

#### Fixed: a back-out reads the core the way admission wrote it

`goatbuilder federate backout <partner>` now passes the same inventories as
the admission (`peering-edges.ini`, then every `peering-*.ini`), so a site
whose hosts are listed only per peering is surveyed. On a core admitted
through the goat-zenoh role, it reads the seed, org fragment and live
configuration at the paths admission resolved, instead of guessing them; a
role core whose paths do not resolve is refused with GB-221. A survey that no
host answers, when this peering's `agreement.json` exists, is refused with
GB-222. A survey row that cannot be read stops the back-out (GB-221) instead
of being counted as absent.

The back-out also takes out the two marker lines admission puts around a
partner's lines in the core's org fragment (`// BEGIN peering <partner>` and
`// END peering <partner>`), so a fragment admitted and then backed out is
byte for byte what it was before. A block that holds anything other than
that partner's lines, begins twice, or never ends is refused, and nothing is
written. Another partner's blocks stay.

#### Fixed: a killed federate run no longer leaves its journal's temporary file

The run that takes a peering's lock removes every `.state.json.<pid>.tmp`
that a killed run left in the peering's working folder, before it reads the
journal, and reports each one as a `journal` row.

#### Fixed: values reach the phases as one word each

Values standup reads from a site's group_vars and inventory, among them
`join_founder_site`, `join_founder_group`, the ACL site and
`public_mgmt_fqdn`, are passed to its planned commands as one shell word
each, as declared values already were. A value containing `$(…)`, a backtick
or `;` is no longer run by the shell. A plain value is written exactly as
before, so existing plans are unchanged. `deploy` passes a group's target to
`ansible-playbook` as two words, the flag and its value, instead of one.

#### Fixed: `update` orders release candidates correctly

A release candidate is now older than its release and older than the next
candidate, so `goatbuilder update` on `1.0.0-rc.1` offers `1.0.0` and
`1.0.0-rc.2` instead of answering that it is up to date.

#### Fixed: every error's "read more" path ships with goatcore

Twenty-five error-catalog entries pointed `explain` at documents that are not
in this repository. Each now names a document that is, on the same subject.
GB-120 now lists `backout` and `rotate` among the reserved peering names.

#### Fixed: the documented signature check matches the release's certificate identity

The by-hand `cosign verify-blob` in the install reference
(`docs/reference/goatbuilder/02-install.md`), the first-run guide and
tutorial 1 gave `--certificate-identity-regexp
'^https://github.com/usareur-af/goatcore/'`. cosign matches that expression
case-sensitively against the certificate's subject, which names the
repository in upper case, so the documented check always failed. The
documents now give the release's own identity:
`'^https://github\.com/USAREUR-AF/goatcore/\.github/workflows/_release-binary\.yml@refs/tags/goatbuilder/v'`.

#### Images and roles built from this repository

- **The republisher and keyd images built from this release on use
  quinn-proto 0.11.19 and quinn-udp 0.5.16**, for RUSTSEC-2026-0523 (a
  MAX_STREAM_DATA frame can open streams beyond the stream limit),
  RUSTSEC-2026-0524 (a long Retry token can panic or exhaust memory in a QUIC
  client) and RUSTSEC-2026-0538 (a receive loop can spin after a truncated
  datagram). A host runs them once its image pin moves to a digest built
  after this change. goatbuilder does not use them (#590).
- **The edge reads only the configuration path the core router names.** The
  edge role's ingress check reads the core's configuration at the path the
  router's own arguments name (`-c` or `--config` in any of their forms), and
  `/etc/zenoh/config.json5` only when they name none. An attached `-c<path>`
  is read as a path only when it starts with `/` or `.`, never as another
  option's value. Arguments it cannot read, a configuration argument with no
  usable path, two different paths, or an override that contradicts the
  router are now refused with the reason. **Behaviour change:** a core whose
  arguments the check cannot read now stops phase 12.
- **The edge refuses a key storage wider than the key-bundle grant.** The
  comparison of the core router's `keys/` storages with
  `goat_zenoh_edge_key_bundle_key_exprs` reads the core where it runs and
  fails closed when it cannot. It refuses every storage under `keys/` that
  the grant does not include, not only a literal `keys/**`. **Behaviour
  change:** a core with such a storage, or one the check cannot read, now
  fails the edge role.
- **The Docker apt repository follows the host's distribution.** The
  docker-engine role took Docker's Ubuntu repository for every host, so a
  Debian 13 host's baseline stopped at the apt refresh. It now takes the
  repository from the host's distribution (`docker_engine_apt_distro`).
  Ubuntu hosts render the same source line as before.
- **The status page's list can be refreshed on its own.** A list-only play,
  `ops/ansible/isitup-rows.yml`, renders the isitup status page's probe list
  and recreates only its container when the list changed, reading no
  sign-in secret. `data/meta.json` now records a stamp of the list it was
  rendered from, and `ops/scripts/isitup-list.py check` (or the play with
  `-e isitup_rows_verify_only=true`) fails when a host's list is behind its
  declaration, writing nothing.
- **The pgbackrest role installs `cron`** before writing its backup schedule,
  so a minimal Debian 13 host no longer stops at the schedule task.

#### Known issue: a federate run killed in phase 11 does not bring the link up on resume

If a `goatbuilder federate` run is killed in phase 11 after it wrote the
WireGuard link's files and before it restarted the link, the interface is
left with no peer. Resuming with `--from 11` changes no file, so the link is
not restarted: phase 11 refuses with "The interface does not hold what the
agreement says", then phase 12 refuses. To recover, restart the peering
WireGuard unit on the edge host (`systemctl restart` on the unit the role
names in `peering_wg_service`), then resume with `--from 11`. A run that is
not killed is not affected. 0.5.3 has the same gap. Fixed in the next
release (#597).

#### Fixed: the pre-deploy source check accepts a fix carried in two repositories

The check that runs before every deploy (`ops/scripts/lib/source_of_record.py`,
run by `fresh-source.sh` and by goatbuilder) refused a valid assembled tree
when a pre-disclosure fix carried one patch in goatcore and one in goatstuff
and both touched a file the two repositories share. Each patch is now judged
against its own repository (#368). This shipped in 0.6.0 and was missing
from its notes.

#### Not in 0.6.0

None of these ships switched on, and no command in 0.6.0 reaches them.

- **`goatbuilder inputs` and `goatbuilder why`.** The libraries that list and
  trace every value's source are built; no command lists or traces them yet,
  and no verb fills the run record's inputs.
- **`goatbuilder federate rotate`.** Rotating a peering's leaves is built as
  a library; no command reaches it.
- **The two-driver hand-over in the command.**
- **`--step`.**
- **`status` measured against a standing goatnet.**
- **`goatbuilder grow`, a goatlite becoming a full environment.**
- **Phase catalogs for standup, join-node and deploy.**
- **The back-out measured on running hosts.**

### For someone contributing

- **Compatibility with the last release is checked on every pull request.**
  Configuration files from each published release since 0.4.1 are kept and
  validated (gate row A2); the commands, flags, documents and error codes are
  compared with a baseline recorded from the published 0.5.1 binary (A3,
  A4); and an error code a release printed cannot be dropped or retitled
  (A5). The upgrade tests list the
  published 0.5.3 binaries, and `update` is tested moving a 0.5.1 install to
  this build and back (E3).
- **Deprecation.** ADR 1228 is accepted: a deprecated command, flag or
  configuration field warns for at least two minor releases and 90 days, and
  only a major release removes it. The library that prints the warning is
  built; no command is deprecated with it yet.
- **A stable line that follows releases (ADR 1231).** All work lands on
  `main`. A whole-tree release is the signed tag `goatcore/vX.Y.Z` at the
  commit of the goatbuilder release with the same version, and the branch
  `stable` follows it, moved only by `.github/workflows/stable.yml`. Nothing
  runs until the first such tag is cut.
- **Release notes** from this release on have three parts: for someone
  deploying, for someone contributing, and to be removed.
- **Outside pull requests** carry the label `community`, and
  `CONTRIBUTING.md` says what a contributor can expect.
- **CI.** A workflow that calls another whose permissions it does not grant
  fails a check. The documented `cosign` identities are checked against the
  release signers (`ops/ci/check-doc-signer-identity.py`), and the error
  catalog's full check runs whenever a document changes.
- **The federation test environment** proves one directory package per
  partner against saltlick v0.5.0, backing a peering out to the hosts'
  state before it, each key rotated in turn on a running peering, a peering
  finished by a released goatbuilder, and a flow monitor that measures
  whether data keeps arriving while a step runs.
- **Tests.** A test that writes a stand-in script and then runs it writes it
  from a child process, so another test thread can no longer make running it
  fail with ETXTBSY. goatbuilder's unit tests pass the environment they need
  instead of setting it; the standup bats test has a home of its own; the sshd and
  Ansible tests fail instead of skipping when their tools are missing; and
  the back-out tests remove their working directories.
- **Dependencies.** The quinn-udp dependency of the `goat` client and
  panoscope-core (through zenoh) and of goat-relay moves to 0.5.16, for
  RUSTSEC-2026-0538 (a receive loop can spin after a truncated datagram), and
  goat-relay's quinn-proto moves to 0.11.19, for RUSTSEC-2026-0523 and
  RUSTSEC-2026-0524. goatbuilder does not use them (#590).
- **Libraries.** An error-catalog entry printed as a document carries its
  version (`goat-errors.entry/v1`); `explain` does not print it yet.
- **Design documents.** The 1.0 gate gains group H (a node on a phone) and
  group I (a joined site that runs cut off), and rows B1, B3 and E4 are
  stated. ADR 1229 is proposed. The island-mode documents say that the
  administration surfaces run at one site per goatnet until ADR 1165 lifts
  each write class.

### To be removed

- **`goat federate <verb>`**, in the `goat` client, as announced in 0.5.0:
  it still runs and names the `goatbuilder federate` command to use. It is
  removed from `goat` no earlier than 30 days after 0.5.0, and not before one
  peering between two standing environments has been completed with
  `goatbuilder federate`.
- Nothing in goatbuilder.

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

### Changed: `rotate` is a reserved peering name

A `peerings:` entry named `rotate` is refused with GB-120, as the other
words of `goatbuilder federate` already were, because `goatbuilder federate
rotate` is the stage that rotates a peering's keys (#493). This shipped in
0.5.3 and was missing from its notes.

### Fixed: a federate run keeps its derived values out of the process environment

The values `goatbuilder federate` derives (phase 1's inputs,
`GOAT_PROFILE_DIR` and `FED_EDGE_GROUP`) are kept in the run's own
environment and passed to every process the run starts, instead of being
written into goatbuilder's process environment (#498). Within one run, the
same values reach the same commands as before. This shipped in 0.5.3 and was
missing from its notes.

### Fixed: the republisher warns when a release stops taking new keys

When an agreement's `envelope.distinct_keys` ceiling is reached, the outbound
republisher refuses each new wire key, as before, and now logs that refusal
at WARN, at most once a minute, instead of at debug level only. The warning
names the release, never the key. Keys already admitted keep flowing (#500).
This shipped in 0.5.3 and was missing from its notes.

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
