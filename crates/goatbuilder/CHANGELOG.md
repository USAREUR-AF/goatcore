# goatbuilder changelog

Each release is the tag `goatbuilder/v<semver>` on `main`, built and signed by
`.github/workflows/goatbuilder-release.yml`. Design of record:
`docs/design/goatbuilder.md`; conformance: `docs/design/goatbuilder-conformance.md`.

## Unreleased

<!-- BEGIN generated: changelog unreleased, from crates/goatbuilder/changelog.d; edit the entries, not this section -->

### For someone deploying

#### Changed: every site's end state declares every service, and a joining site's missing ones are named gaps

`goatbuilder endstate` and `ops/goatbuilder/endstate/endstate.py` now declare
every service a site cut off from the others needs at every site that
replicates: the replication core (Traefik, NetBird management, signal, Ory,
the Kratos self-service UI), the NetBird dashboard, the portal, the id console,
the onboarding engine and mesh DNS. No catalog row is founder-only. What a
joining site does not run yet is graded `GAP`, "declared, not yet built at
this site", with the gate row that closes it: I1 and I4 for the admin
surfaces, I3 for mesh DNS. A `GAP` is counted and printed and does not by
itself make the site red. Acceptance run with `--endstate` reports a row such a
service owns as `SKIPPED` with the same text instead of N/A. `goatbuilder
join-node` lists the gaps under "What <site> does not run yet", from the
catalog, before the services the site turns off (ADR 1233, #671).

#### "Fixed: a host-allocation SKIP says which case it is and what clears it"

A plan whose host-allocation check was SKIP cannot be deployed: deploy does
not run that check again, so it refuses the plan (GB-188), and a new plan
records the same SKIP. The refusal used to say only "host-allocation was SKIP
when the plan was made". Now the plan view, the refusal and the run record
carry the check's detail, which gives the case and the route:
- the inventory names no host for this peering: run `goatbuilder federate
  allocate <partner>`, or name the hosts in `peering-edges.ini`;
- another peering on a shared machine has a value that does not resolve: each
  value is named with its key and the group whose vars set it;
- its agreement was not found: both paths that were looked at are named.

GB-188's catalog entry gains this cause (#742).

<!-- END generated: changelog unreleased -->

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
