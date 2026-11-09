# Federation core router discovery

## Status

**Built:** `goat federate init` selects a running core router once and reads its
image and version from that container. The core role publishes a stable discovery
label, and both federation e2e harnesses use the role's naming pattern and label.
**Not built:** remote host provisioning or automatic selection among several cores.
Live deployment and bilateral federation acceptance are separate release gates.

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

Without `DOCKER_HOST`, Docker and Podman are supported; unavailable runtimes are
reported in failure diagnostics. An explicit `DOCKER_HOST` restricts observation
to Docker on that daemon, so an unreachable remote core cannot be replaced by a
local Podman core. Podman's own connection configuration is inherited normally.
Several different eligible containers cause refusal with guidance to set
`FED_ROUTER_CONTAINER`. Runtime aliases reporting the same full container ID are
deduplicated.

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
