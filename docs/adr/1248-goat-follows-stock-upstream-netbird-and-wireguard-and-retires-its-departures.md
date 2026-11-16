# ADR 1248 — goat follows stock upstream NetBird and WireGuard, and retires each of its own departures when upstream offers the same

| Field | Value |
|---|---|
| Status | Proposed. The watch list it governs is written ([`upstream-netbird-wireguard-watch.md`](../design/upstream-netbird-wireguard-watch.md)); merging this record changes no build and no goatnet. |
| Date | 2026-11-15 |
| Repo scope | goat core (`ops/netbird/`, `goat-relay/`, `goat-relay-protocol/`, `docs/design/upstream-netbird-wireguard-watch.md`) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 0300](0300-netbird-as-mesh-control-plane.md) (NetBird as mesh control plane) · [ADR 0301](0301-patched-netbird-management-image.md) (patched management image) · [ADR 0302](0302-custom-federated-signal-server.md) (federated signal server) · [ADR 0303](0303-wireguard-data-plane.md) (WireGuard data plane) · [ADR 0306](0306-p2p-with-relay-fallback.md) (relay fallback) · [ADR 0602](0602-rosenpass-hybrid-rekey.md) (Rosenpass) · [ADR 0607](0607-wireguard-fips-waiver.md) (WireGuard FIPS waiver) · [ADR 0709](0709-block-33-relay-rewrite-architecture.md) (`goat-relay`) · [ADR 1053](1053-relay-auth-skew-grace-secret-rotation.md) · [ADR 1057](1057-relay-liveness-credits-data-traffic.md) (relay liveness) · [ADR 1118](1118-kernel-wireguard-jetson.md) (kernel WireGuard on Jetson) · [ADR 1134](1134-rosenpass-key-distribution-by-digest.md) (Rosenpass key by digest) · [ADR 0858](0858-zenoh-image-upstream-pivot.md) (the Zenoh return to the stock image) |

## TL;DR (for a layperson)

goat's private network is built on two open-source projects, NetBird and
WireGuard. goat uses them as their own developers ship them wherever it can.
Where goat has changed them or replaced a part, that change is recorded as a
deliberate departure, and it is removed once the upstream project offers the
same thing. goat also keeps a written list of upstream work that could make the
network faster or more capable, says for each item what would make goat take it
up and what it would cost goat's own parts, and names who checks the list.

## Context

goat already carries departures from stock NetBird and WireGuard, each decided
in its own record:

- Seven patches applied to the pinned NetBird source in `ops/netbird/patches/`
  ([ADR 0301](0301-patched-netbird-management-image.md),
  [ADR 1081](1081-control-plane-storm-resilience.md); list in
  [`ops/netbird/README.md`](../../ops/netbird/README.md)). The pin is
  `NETBIRD_REF` in [`ops/netbird/Dockerfile`](../../ops/netbird/Dockerfile),
  upstream v0.71.4.
- `goat-relay`, a Rust relay written from a protocol description, which must stay
  byte-for-byte compatible with unmodified NetBird clients
  ([ADR 0709](0709-block-33-relay-rewrite-architecture.md) D1, D3;
  [`netbird-relay-wire-protocol.md`](../design/netbird-relay-wire-protocol.md)).
  Its liveness rule differs from the Go relay on purpose
  ([ADR 1057](1057-relay-liveness-credits-data-traffic.md)).
- A federated signal server ([ADR 0302](0302-custom-federated-signal-server.md)).
- Out-of-tree WireGuard kernel modules for Jetson kernels that compile WireGuard
  out ([ADR 1118](1118-kernel-wireguard-jetson.md)).

Several records already say that a departure ends when upstream catches up:
ADR 0301 drops its patch set when its behaviors land upstream, ADR 1134 D6
states that fork deltas are patches offered upstream, and ADR 0858 moved Zenoh
back to the stock upstream image once the reason for a custom build was gone.
No record states this as one rule across NetBird and WireGuard, and no record
lists the upstream work that goat is waiting on or could adopt. Upstream also
changes things goat depends on: the NetBird relay wire protocol, which
`goat-relay` must follow, and Rosenpass, which NetBird marked deprecated in
netbirdio/netbird#8372.

## Decision

1. **Stock upstream is the default.** goat runs NetBird and WireGuard as upstream
   ships them, at a pinned version, unless a record decides a departure.
2. **Every departure is recorded and has a retirement condition.** A departure is
   a patch in `ops/netbird/patches/`, a goat component that replaces an upstream
   one (`goat-relay`, the federated signal server), or a module goat builds in
   place of a vendor's. Each one names, in its record or in the watch list, the
   upstream change that would retire it. When that change is in an upstream
   release goat can pin, the departure is removed in its own pull request.
3. **Departures go upstream where they help every NetBird user.** goat offers a
   change upstream when it is not specific to goat's deployment, with tests,
   signed commits and the contributor agreement upstream asks for.
4. **Upstream performance and functionality work is watched in one list,**
   [`docs/design/upstream-netbird-wireguard-watch.md`](../design/upstream-netbird-wireguard-watch.md).
   Each row names the upstream item and its link, what it changes, what it would
   require of goat's components (wire compatibility for `goat-relay` and
   `goat-relay-protocol` in particular), its effect on goat's TLS and FIPS
   posture, the trigger for adoption, its state (`watching`, `proposed upstream
   by us`, `adopted`, `dropped`) and the goatcore issue that carries the work.
5. **Adoption is a later, separate decision.** A row reaching its trigger opens
   that decision in the issue the row names; this record adopts nothing.
6. **`goat-relay` follows the upstream wire protocol.** A change to the protocol
   in an upstream release is a change `goat-relay`, `goat-relay-protocol` and
   `netbird-relay-wire-protocol.md` must make before goat pins a client that
   uses it. Where an upstream change lowers goat's transport protection (for
   example a relay transport without TLS), goat carries it as an option and keeps
   its current transports as the default.
7. **The list is kept by the maintainers.** The `goat-owners` team reviews it.
   It is read and updated in the pull request that moves `NETBIRD_REF`, in any
   pull request that changes `goat-relay-protocol`, and when an upstream pull
   request or discussion that a row links changes state.

## Alternatives considered

- **Keep each departure's retirement only in its own ADR.** The records exist,
  but a reader moving the NetBird pin has no single place that says which
  departures the new version could retire. Rejected.
- **Put the watch list in this ADR.** The list changes as upstream changes, and
  an accepted ADR is not edited. Rejected; the ADR holds the rule and the design
  doc holds the list.
- **Maintain a long-lived fork of NetBird.** ADR 0301 already rejected this for
  the management server; the cost of rebasing grows with every upstream release.
  Rejected.
- **Adopt upstream changes automatically when the pin moves.** A wire-protocol
  change can break `goat-relay` for every client that relays through it, and a
  change can lower transport protection. Rejected; each adoption is decided.

## Consequences

**Positive.** A reader can see in one place what goat changed, why, and what
upstream change would remove it. Moving the NetBird pin includes a check of what
it retires and what it breaks. Upstream protocol changes that affect
`goat-relay` are recorded before they reach a release.

**Negative.** The list must be kept current by hand at each pin move, which adds
review work to those pull requests. A watched item can sit at `watching` for a
long time; the state column shows that.

## Assessor view (SCA-R)

The rule reduces the amount of modified third-party code goat carries, which
supports SA-22 (unsupported system components) and SI-2 (flaw remediation:
upstream fixes reach goat with fewer conflicts). Recording every departure with
its retirement condition supports CM-3 (configuration change control) and SR-3
(supply chain controls), since an assessor can see each modification to a
supplied component and its reason. Decision 6 keeps an upstream transport
change that removes TLS from becoming goat's default without its own decision,
consistent with SC-8 (transmission confidentiality). WireGuard's FIPS position
is unchanged and stays governed by [ADR 0607](0607-wireguard-fips-waiver.md).

## Trigger conditions for revisit

- NetBird or WireGuard upstream changes license or governance in a way that
  makes stock upstream unacceptable (see ADR 0300 and ADR 0110 triggers).
- goat stops using NetBird or WireGuard.
- The number of departures grows to where a patch set no longer applies to an
  upstream release without rework at each pin move; then the choice between
  patches and a maintained fork is reopened.
