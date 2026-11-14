# Federation core router discovery

## Status

**Built:** `goat federate init` selects a running core router once and reads its
image and version from that container. The core role publishes a stable discovery
label, and both federation e2e harnesses use the role's naming pattern and label.
**Built:** the measurements read through a `Probe` (`crates/goat-federation/src/derive.rs`),
so the same derivations measure this machine or a host reached over SSH; goatbuilder's SSH
probe and the router precedence below (`crates/goatbuilder/src/federate/measure.rs`), with
offline tests; the library's `run_init_measured` (`crates/goat-federation/src/verbs/init.rs`),
which takes init's measured half from its caller, and goatbuilder's `measure::over_ssh`,
which supplies it. **Measured** once against two throwaway hosts built from the harness host
image (sshd, NOPASSWD sudo, a WireGuard interface, a labeled router container): every value
read over SSH, the declared, discovered, wrong-name and unreachable cases. **Built:**
`goatbuilder federate init --over-ssh <peering>` (with `--ssh-edge-group`, `--ssh-core-group`,
`--edge-host`, `--core-host`, `--router-container`), and the driver's phase 3 using it when
`FED_MEASURE` unset or `ssh`, on both avenues (`peering-standup.sh` takes the same switch);
`FED_MEASURE=here` measures the machine the run is on, as before. **Built:** phase 3 in two
halves for a peering no one machine reaches both hosts of (§"Phase 3 in two halves"), with
offline tests; unmeasured on real hosts, and not yet used by the one command's phases.
**Not built:** automatic selection among several cores. Live deployment and
bilateral federation acceptance are separate release gates.

> The verb moves to `goatbuilder federate init` ([ADR 1225](../adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md), Proposed), which runs
> where the deployment's declaration is. This document describes the selection as built in
> `goat federate init` today.

## Selection contract

The standard role names its service and container `{{ zenoh_stack }}-router`, with
`zenoh_stack` defaulting to `{{ site_id }}-zenoh`. The CLI must not substitute a
context name for either variable: contexts can be aliases and stacks can be
customized. Issue #238 exposed a fixed-name probe that worked in the harness but
could not find the standard role's router.

The CLI lists running containers using the runtime's formatted ID, name and
`org.goat.component` label. Candidates are containers labeled `zenoh-core`, existing
standard `*-zenoh-router` containers, or the legacy `goat-zenoh` container. An edge
router's name alone does not qualify it. Image names alone do not select a core.
The new role label supports custom stack names. Existing standard and legacy
installations work without redeploying to add the label; an older custom layout
uses an explicit selector until its next normal role rollout. Applying the added
Compose label can recreate a container during that rollout.

`FED_ROUTER_CONTAINER` explicitly selects a container name or ID. It selects the
observation target, never the capability values. The selector is passed as an
argument, rejects options and expressions, and never falls back to discovery if
that container is absent or stopped. Both standup drivers inherit the exported
variable into phase 3.

The sovereign edge role (`goat-zenoh-edge`) reads the core's storage
configuration for two advisory checks and finds the core with the same label and
names, through Docker on the host the play runs against. Its
`goat_zenoh_edge_core_container`, when set, wins; empty, the default, discovers.
No match or several matches make those checks report "could not run" with the
reason, never a pass (`roles/goat-zenoh-edge/tasks/discover-core.yml`).

Without `DOCKER_HOST`, Docker and Podman are supported; unavailable runtimes are
reported in failure diagnostics. An explicit `DOCKER_HOST` restricts observation
to Docker on that daemon, so an unreachable remote core cannot be replaced by a
local Podman core. Podman's own connection configuration is inherited normally.
Several different eligible containers cause refusal with guidance to set
`FED_ROUTER_CONTAINER`. Runtime aliases reporting the same full container ID are
deduplicated.

## Phase 3 from the operator's machine

Status: WIP — gate: the federation end-to-end test runs phase 3 for both sides from outside
the edge and core hosts, with no `goat` binary, no signing key and no `FED_ROUTER_DOCKER_HOST`
on either host, and the packages pass lint and compose as before.

Run as `goat federate init`, phase 3 measures the machine it runs on. The operator copies
`goat` and a signing key to the edge, gives the edge Docker access to the core, and copies the
package back. goatbuilder instead measures both hosts from the operator's machine:

- **How it is asked for.** `goatbuilder federate init --over-ssh <peering>` with init's own
  flags (`--with-transport`, `--with-data`, `--signing-key`, `--out`). The groups default to
  `<peering>_edge` and `<peering>_core`; `--ssh-edge-group` and `--ssh-core-group` name others.
  The checkout whose inventory names the hosts is `-C <repository>`, else the git toplevel of
  the current directory. A host, group or router flag without `--over-ssh` is refused, as is
  `--over-ssh` on any verb but init. Without `--over-ssh`, init measures this machine exactly
  as `goat federate init` does. In a peering run, `FED_MEASURE=ssh` (the default; `here`
  measures the machine the run is on) makes phase 3 `goatbuilder federate init ... --over-ssh <peering> --ssh-edge-group <edge group>
  [--router-container $FED_ROUTER_CONTAINER] --repo <repository>`, run in process whoever runs
  the other verbs (`FED_VERBS`), with no `DOCKER_HOST`; `FED_ROUTER_DOCKER_HOST` is not used.
  A GB-113 refusal over SSH (no router found, or several) names both ways to select one:
  `--router-container`, and `FED_ROUTER_CONTAINER`, which the one command passes as that flag.
  `peering-standup.sh` prints and runs the same words through `$GOATBUILDER_BIN`, and the
  parity gate compares both avenues with `FED_MEASURE` unset and set. Phase 1's local
  `docker compose` check still reads the machine the run is on (set
  `FED_SKIP_LOCAL_DOCKER_CHECK=1` when that is not the edge).
  With `FED_MEASURE=here` and the verbs in process, phase 3's `DOCKER_HOST` (from
  `FED_ROUTER_DOCKER_HOST`) and `FED_ROUTER_CONTAINER` reach the library's measurement as
  values: `goatbuilder federate init` passes them to `measure_here_with`
  (`crates/goat-federation/src/verbs/init.rs`), which gives `DOCKER_HOST` to the container
  runtime's commands as their own environment. goatbuilder never writes either into its
  process environment, which every thread shares (built;
  `crates/goatbuilder/tests/no_process_env_in_unit_tests.rs` refuses any write under
  `src/federate/`). `goat federate init` still reads both from its environment.

- **Which hosts.** The single host of the edge group and of the core group (`<partner>_core`)
  in `ops/ansible/inventory/peering-edges.ini`, the inventory the peering plays use, read with
  `ansible-inventory --playbook-dir .` so group_vars and host_vars apply as in a play. A group
  with several hosts needs `--edge-host` or `--core-host`; an empty group, an unreadable
  inventory or a connection other than `ssh` or `local` refuses with GB-110.
- **How they are reached.** `ansible_host`, `ansible_user`, `ansible_port`,
  `ansible_ssh_private_key_file`, and `ansible_ssh_common_args` and `ansible_ssh_extra_args`
  (a jump host is `-o ProxyJump=…` there), with `BatchMode=yes`, `ConnectTimeout=10` and
  `StrictHostKeyChecking=accept-new`. The plays become root (`ops/ansible/ansible.cfg`), so
  each read runs under `sudo -n` unless the host sets `ansible_become: false`;
  `ansible_become_user` is honored. `ansible_connection=local` runs the reads on this machine.
  Before measuring, each host runs `true` the same way: a host that does not answer refuses
  with GB-111 and the ssh command, and a sudo that wants a password refuses with GB-112.
  Two differences from a play: the plays' `ansible.cfg` turns host key checking off and adds
  its own `ssh_args`, and phase 3 uses neither, so a host whose key is not yet known is
  learned once and a changed key refuses (X-SSH-003). The inventory's SSH options are passed
  to `ssh` as given, as Ansible passes them, so a `ProxyCommand` there runs on the operator's
  machine; `BatchMode=yes` comes first and cannot be turned off by them.
- **What runs there.** Only the commands derive issues, listed by `is_read_only_probe`: `wg
  show`, `swanctl --version`, `ipsec --version`, `systemctl is-active` and `list-unit-files`,
  `docker`/`podman` `ps` and `inspect`, `exec <id> zenohd --version` in exactly that shape, and
  `cat` under `/proc/sys/crypto/` and `/sys/class/net/`, each in exactly the shape derive sends
  (so neither `wg show all private-key` nor a bare `docker inspect` passes). The probe refuses
  anything else before ssh starts. The host's command line is one quoted string after `--`
  and the destination. A read that fails for a reason other than an absent file (the
  connection dropped, sudo would not run `cat`) is an unknown, never a "no": the FIPS flag
  read that way withholds `ipsec-fips` rather than reporting the host as not in FIPS mode.
  An interface name that is not a Linux interface name is refused before any read, for
  `goat federate init` too.
- **Where each value comes from.** Transports, the PQC unit, the interface public key and the
  interface MTU are read on the edge; the router's image and version on the core. Identity
  (trust domain, CA fingerprint) and the declaration come from the operator's own profile, and
  the package is signed on the operator's machine with a key resolved there. Each value's
  `source` names the inventory host and destination it was read on.
- **Which router.** First `--router-container <name>`, the operator's selector. Then the
  deployment's variables for the core host: the goat-zenoh role names its container
  `{{ zenoh_stack }}-router` with `zenoh_stack` defaulting to `{{ site_id }}-zenoh`, so
  `zenoh_stack` (with `{{ site_id }}` resolved), else `site_id` with the role default, gives the
  name. Role defaults are not inventory variables, which is why `site_id` alone is read this
  way; a test fails if the role's default or container name changes. Last, the discovery above,
  run on the core host over SSH. A named router, by flag or by variable, is never replaced by a
  discovered one. A router that cannot be found refuses with GB-113 and these three fixes; a
  router that is found but whose version cannot be read stays an underived value with init's
  exit 23.

## Phase 3 in two halves

Status: WIP — gate: goatbuilder 1.0 gate row B10, a scenario with two drivers per sovereign
in which the edge driver has no route to the core (the contract is
docs/design/federation-two-drivers.md, in review).

Some sites have no machine that reaches both the edge host and the core host. Phase 3's
measurement then runs in two halves, one on each machine:

- **The edge half**, on a machine that reaches the edge:
  `goatbuilder federate init --over-ssh <peering> --side edge --half-out <file>` with init's own
  flags. It resolves and reaches only the edge (the core group need not be in its
  inventory), measures the transports, the PQC unit, and the declared federation interface's
  public key and MTU, and writes them to `<file>` (schema `goatbuilder.phase3-edge-half/v1`,
  with the peering, the edge host, the interface it was measured for and `measured_at`). It
  prints the file's sha256 and a confirm code (its first 32 hex digits, in groups of four), and
  builds and signs nothing.
- **The core half**, on a machine that reaches the core:
  `goatbuilder federate init --over-ssh <peering> --side core --edge-half <file>
  --edge-half-sha256 <code>` with the same init flags. The code is required: the operator reads
  it from the edge driver, the way phase 5's keys are read. Before anything in the file is
  used, the file's sha256 must begin with the code. The half is then refused (GB-114) when its
  schema, peering or fields are not an edge half's; when this checkout's inventory names a
  different host as the edge (when it names none, that is said, and the host is not checked);
  when it was measured more than 24 hours ago, or more than five minutes in the future; or when
  it was measured for another interface than the package asks for. It resolves and reaches only
  the core, measures the router and the SPIRE trust domain exactly as the one-machine run does
  (the same router choice, GB-113 when there is none), and joins the two. Each edge value's
  `source` ends with the file and its confirmed sha256, so the package's provenance names them,
  and init builds and signs the package there.
- **Why the binding.** The edge half carries the edge's WireGuard public key, which the core side
  signs into our package, and the partner dials it. A half changed on its way from the edge
  driver would put another key into a package signed by us. The confirmed digest makes the
  operator, who saw the edge driver's output, the check.
  The confirm code must be read from the edge driver's own screen, or the output of its run,
  never copied from anything that travelled with the file: a code carried beside a changed
  file would confirm the change. That step is what the binding rests on.
- **Why 24 hours.** The two halves of one phase 3 are measured and joined in one working
  session. Past a day, the edge may have been rebuilt or re-keyed (a new WireGuard key, a
  changed MTU), and a package signed with the old values would carry them.
- **Why not a signature.** No key the core side already trusts is on the edge driver: the edge
  driver holds no CA key, and the package signing key is used where the package is built, on
  the controller. Signing the half needs a new key for the edge driver, with its public half
  pinned on the controller (for example in goatbuilder.yaml). That is not built; until it is,
  the confirmed digest is the binding.
- **The one-machine run is unchanged.** `measure` is the two halves joined, from the same
  reads in the same order as `derive::measure` (a test compares the two). `--side` without its
  files, a file without its side, or either without `--over-ssh` is refused.

Not built yet: the digest bound in the controller's journal (there is no journal around
`init --over-ssh` on its own; the one command's hand-off records it), the one command's phase 3
handing off between the drivers, the edge driver's signing key, and measurement on two real
hosts. Follow-ups: goat_federation::derive exporting its field names as constants (the core side
now takes them by asking derive's reads through a probe that answers nothing), and
goat_federation::verbs exposing the rule for which interface init measures, so the edge side
does not repeat it.

## Measurements and limits

The CLI inspects the selected container's running state, full ID and configured
image. Version probes use the same runtime and immutable ID, trying `zenohd` and
`/zenohd`. A failed version probe never changes runtime or container. Missing,
stopped, unreadable or ambiguous targets leave capabilities underived; `init`
retains its existing exit-23 refusal and does not emit a completed package.

The image-to-FIPS classification and version parser retain their existing rules.
This change adds no new cryptographic attestation and does not accept a declared
image, version or FIPS boolean as measured evidence. The reads span multiple
runtime commands; they are not an atomic snapshot, and a container that stops
during observation causes the version probe to fail.

The metadata probes use the documented [Docker listing format](https://docs.docker.com/reference/cli/docker/container/ls/)
and [Podman listing format](https://docs.podman.io/en/stable/markdown/podman-ps.1.html).
The inspection adapter accounts for Docker's `Id` and Podman's `ID` template fields.

## Verification

Phase 3 over SSH: derive is tested against recorded host outputs (each command it issues is
checked against the read-only list, and edge reads never reach the core or the reverse);
goatbuilder's tests cover the ssh argument vector from an inventory (address, user, port, key,
jump options, `sudo -n`, quoting), refusal of non-read-only commands before ssh starts, the
router precedence, the GB-110, GB-111 and GB-113 refusals, and a remote read error that is
ssh's own not being taken for the file's permission; `run_init_measured` is tested to ask
its measurer once, for the declared interface only when a transport is offered, and to
return a measurer's error as is. `goatbuilder federate init --over-ssh` is tested end to end
short of a real host (an inventory naming this machine with `ansible_connection=local`: the
groups, the router from `zenoh_stack`, the GB-113 and GB-110 refusals), the flag refusals, and
the driver's phase 3 words with every other phase unchanged. Not yet run inside the
end-to-end harness (scenario `phase3-from-outside`, requested).

Rust regression cases cover standard and legacy names, custom labels, explicit
selection, invalid selectors, missing and stopped containers, multiple cores,
Podman, runtime aliases, permission errors, malformed metadata and version
failures without target switching. An Ansible fixture renders the actual core
Compose template with the default and a custom stack, then checks the resulting
service name, container name and discovery label. The existing Python CI discovers
that fixture; the CLI CI runs the Rust tests.

The federation e2e harnesses use `<side>-zenoh-router` and the role label for core
creation, restart and log collection. Their existing fresh-delivery and refusal
checks remain the live integration proof; offline tests alone do not establish it.
