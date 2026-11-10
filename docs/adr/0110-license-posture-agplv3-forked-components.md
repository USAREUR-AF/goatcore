# ADR 0110 — License posture for AGPLv3-relicensed control-plane components (netbird-mgmt and friends)

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-27 |
| Repo scope | Multi |
| Supersedes | — |
| Superseded by | — |
| Related | [`0000-why-adrs-and-procurement-resilience.md`](0000-why-adrs-and-procurement-resilience.md), [`0104-no-prime-capture.md`](0104-no-prime-capture.md), [`0106-sovereign-tooling-preference.md`](0106-sovereign-tooling-preference.md), [`0111-vendor-friendly-license-posture.md`](0111-vendor-friendly-license-posture.md), [`0301-patched-netbird-management-image.md`](0301-patched-netbird-management-image.md), [`0706-custom-netbird-image-build-ci.md`](0706-custom-netbird-image-build-ci.md), Finding `F-010` |

## TL;DR (for a layperson)

The upstream NetBird project changed the license on its management
server in August 2025 to a stricter open-source license called AGPLv3.
We patch and run that management server. AGPLv3 has one extra rule
beyond ordinary open-source licenses: if you change the program and
let other people use it over a network, you must offer those users
the source code under the same license. We accept that rule, because
we already publish all our patches and we are not trying to hide our
modifications from anyone — partners, allies, or the public. This ADR
writes down the discipline that keeps the rule cheap to comply with,
and names the escape valves we keep available in case a specific
mission partner is policy-blocked from running AGPLv3 software.

## Context

In August 2025, `netbirdio/netbird` relicensed four directories —
`management/`, `signal/`, `relay/`, `combined/` — from BSD-3-Clause to
AGPLv3. The remaining directories (client, dashboard, library code)
stay BSD-3-Clause. See finding `F-010`
for the verification trail. Three contemporaneous facts about our
deployment:

1. **Our fork base post-dates the relicense.** [`ops/netbird/Dockerfile`](../../ops/netbird/Dockerfile)
   pins `NETBIRD_REF=53b04e512af0d94f619e870467115c43d200830b`
   (2026-04-16). The image we run is built from AGPLv3 sources.
2. **We modify it.** `git apply patches/skip-migrations.patch` runs
   against `./management` before `go build`. That makes the resulting
   binary a "modified work" under AGPL terms, which is the predicate
   for §13 (the Affero clause).
3. **We serve it over a network.** Mesh peers reach the management
   API; Phase 4+ mission partners may reach a federated mgmt plane.
   That makes those peers / partners "users interacting with the
   program over a network," the second §13 predicate.

§13 says: when both predicates are true, we must offer those network
users the corresponding source under AGPL terms. There is no path
that runs modified `management/` over a network and avoids §13; the
question is only how we comply.

The strategic question this ADR answers: **do we accept AGPL
downstream and codify the discipline, or do we route around the
relicense?** Three paths exist:

- **(a) Accept AGPL.** Publish patches, run the code under AGPL,
  treat §13 as a normal compliance obligation.
- **(b) Rebase to pre-AGPL `v0.52.2` and clean-room CVE backports.**
  This is what `openzro/openzro` is doing. Heavy ongoing engineering
  cost — every CVE in `management/` after Aug 2025 needs a clean-
  room reimplementation by contributors who haven't read the AGPL'd
  diff.
- **(c) Ride `openzro` as our LTS upstream.** Lets someone else carry
  the clean-room cost. Trades upstream dependency on netbirdio for
  upstream dependency on a one-day-old single-maintainer project.
  Premature.

## Decision

**Accept AGPLv3 downstream.** Path (a). Codify the discipline below;
keep paths (b) and (c) named as escape valves with explicit trigger
conditions.

> **Reconciliation with ADR 0111 (vendor-friendly license posture).**
> Accepting AGPL for one component does not contradict the broader
> vendor-friendly stance in ADR 0111. AGPL §13 forbids vendors from
> keeping proprietary modifications of the AGPL component itself
> closed — but it does *not* forbid vendor profit on AGPL software.
> Vendors profit via service, support, hosted offerings, integrations,
> and proprietary extensions on the *API side* of the process boundary
> (D4). Red Hat, Confluent, MongoDB-pre-SSPL, and Elastic-pre-relicense
> all built large businesses on this exact model. The process-boundary
> discipline below is what protects vendor proprietary work from being
> implicated by the AGPL component running next to it.

### D1 — Scope of AGPL components

The AGPLv3 obligation attaches to the `management/`, `signal/`,
`relay/`, `combined/` directories of upstream netbird and to any
binaries built from them. Today that is:

- `netbird-mgmt` (built in [`ops/netbird/Dockerfile`](../../ops/netbird/Dockerfile)) — modified, AGPLv3.
- `netbird-signal` and `netbird-relay` — currently run unmodified
  upstream binaries; AGPLv3 applies but §13 obligations on us are
  trivial because we are not modifying.
- Any future component we fork from those directories.

The rest of upstream netbird (client agent, dashboard, libraries)
is still BSD-3-Clause. Our own services (`mgmt-host-bootstrap`,
`signal-server`, `db-policy-engine`, `gatewatch`, `airdefenders`,
`ntp-health-agent`, `admit-audit`, etc.) are not derived works of
AGPL code; they remain under whatever license we choose for them.

### D2 — Patches stay in-tree and publicly reviewable

All patches against AGPL upstream live at
[`ops/netbird/patches/`](../../ops/netbird/patches/) (and the
analogous path for any future signal/relay/combined fork). Patches
are applied by `git apply` in the build, never via `replace`
directives that hide the modification surface. New patches are
added in normal PRs against this repository. This is the simplest
shape that makes §13 source-availability automatic for any network
user who has access to the repo URL.

### D3 — Repository visibility

The project repository is public (or, if a future
operational decision requires private visibility for a window, the
narrower set comprising `ops/netbird/`, `ops/netbird/patches/`, the
fork-base `NETBIRD_REF` pin, and a bill of any other AGPL forks
must remain publicly reachable). This is an explicit condition of
remaining on path (a); a fully-private repo while running modified
AGPL on a network would put us in §13 violation. The
network-user-source-availability mechanism is documented in D5.

> **Spike — verify repo visibility before Block 22 cutover (Phase 2D).**
> Confirm the project repository is public. If not, choose between
> (i) flipping to public, (ii) carving out a public sub-repo
> containing the AGPL forks + patches, or (iii) shifting to path
> (b)/(c). Spike artifact: a 1-page note in `docs/project/` recording
> the chosen path and the reason. Spike trigger: before Block 22E
> (public DNS retirement) lands, since after that the mgmt API moves
> behind silent transport and the §13 source-availability link must
> be advertised through a different channel (D5).

### D4 — Process-boundary discipline

AGPL components run as **separate processes** (Docker containers,
systemd-managed binaries, etc.), reached by our own services over
local sockets, network calls, or gRPC. We never:

- Link AGPL code as a library into our own Go services.
- Embed AGPL source files into a non-AGPL codebase.
- Use Go `replace` directives to pull AGPL packages into our own
  modules.

Process-boundary aggregation is the standard, FSF-blessed pattern
that keeps the AGPL obligation cleanly attached to the AGPL binary
and out of services that merely call into its API. This rule
applies to every existing AGPL component (today: netbird-mgmt) and
any future one (e.g., if we ever fork netbird `signal/` or
`relay/`).

### D5 — Network-user source-availability mechanism

Mesh peers and federated partners count as "users interacting over
a network." We satisfy §13 by exposing the source-pointer in two
places:

- **Repository URL** in the `LICENSE`-pointer file at the root of
  this repository (`README.md` and a new `LICENSE-NOTICE.md`
  pointing at `ops/netbird/Dockerfile` for the pinned AGPL ref).
  This covers users who have repository access.
- **Management-API status response** advertises the running
  binary's commit pin + a public source URL. Already partly
  surfaced via the admit-audit sidecar; extend the response in the
  Block 22 cutover work to include `agpl_source_url` +
  `agpl_source_ref` fields.

Effort: under one day. Tracked as a small companion item in
`../project/implementation-plan.md`
under Block 22 (D5 lands alongside Block 22E).

### D6 — Phase 4+ mission-partner federation review

Before any Phase 4+ federation conversation that would put a
mission partner's mgmt plane in our trust path (or vice versa), an
explicit AGPL-posture confirmation step is required. Most
DoD-adjacent organizations are familiar with AGPL and accept it
for non-classified open-source components; some have policies that
disqualify AGPL software from specific boundaries. The review
takes <1 hour per partner and is recorded in the partner's
federation runbook.

If a specific partner is policy-blocked from running AGPL software
in their boundary, the escape valves are paths (b) or (c). Choose
per the trigger conditions below; do not make the choice
speculatively now.

## Alternatives considered

- **Path (b) — rebase to pre-AGPL `v0.52.2` and clean-room CVE
  backports.** Loses access to all post-Aug-2025 upstream feature
  + security work. Adds clean-room engineering discipline (no
  contributor reading AGPL diffs) as a permanent process tax.
  openzro is already paying this cost; we would duplicate it.
  Rejected as the default; kept as an escape valve.
- **Path (c) — ride `openzro` as upstream.** Trades a multi-
  contributor upstream (`netbirdio`) for a single-maintainer
  one-day-old upstream. Bus-factor risk too high. Reject for now;
  revisit if openzro stabilizes (multi-maintainer + 6+ months of
  consistent release cadence + a corporate or governmental
  backer).
- **Use NetBird's commercial / closed-source license.** Trades
  AGPL §13 for vendor lock-in and dollars. Conflicts with
  [ADR 0104](0104-no-prime-capture.md) (no prime capture) and
  [ADR 0106](0106-sovereign-tooling-preference.md) (open-source
  first). Rejected.
- **Replace netbird-mgmt with a different mesh control plane
  (Tailscale/Headscale, Defined.net, custom).** Each has its own
  license posture and operational shape. Tailscale/Headscale
  bring different (BSD/MIT) licenses but are not architectural
  drop-ins for our multi-site Spock-replicated topology. Out of
  scope for this ADR; tracked as an exit ramp under §"Trigger
  conditions for revisit."

## Consequences

**Positive.**

- Our patches are visible by default, which is consistent with
  [ADR 0000](0000-why-adrs-and-procurement-resilience.md) and
  [ADR 0104](0104-no-prime-capture.md). Anyone — partner,
  assessor, allied government — can audit what we changed.
- AGPL is a compatibility signal for the open-source ecosystem
  that this project participates in (most DoD-adjacent
  open-source communities accept AGPL).
- We avoid the recurring engineering tax of clean-room CVE
  backports.

**Negative.**

- A future mission-partner federation conversation may require
  the escape valve. The cost of the spike when that fires is
  bounded by the discipline above (process boundary, surgical
  patches, in-tree publication) — paths (b) or (c) take days to
  weeks of engineering, not months.
- Some closed-source vendor partners may decline to integrate
  via an AGPL-component-bearing path. We accept this; the
  mission model deliberately disfavors such partners
  (ADR 0104).
- Process-boundary discipline (D4) is a real engineering rule
  that future contributors must understand. Mitigated by this
  ADR + a two-line note in CONTRIBUTING.

## Assessor view (SCA-R)

Open-source licensing posture is consistent with NIST SP 800-160
vol. 1 (engineering trustworthy systems) — the source-availability
property the AGPL §13 mechanism enforces is the same property
SCA-Rs already need from us for source review. AGPL itself is on
the OSI-approved list. NIST SP 800-53 SR-3 and SR-4 (supply
chain provenance) are served better by the in-tree-patches
discipline (D2) than by binary-only third-party dependencies.

For partner-federation conversations: the AGPL-posture
confirmation step (D6) becomes a stock checklist item in the
partner runbook. If a partner cannot accept it, that is a
substantive negotiation, not a quiet failure.

## Trigger conditions for revisit

- **A specific mission partner is policy-blocked from running
  AGPL software in their boundary.** Action: open a decision
  review, pick (b) or (c), document the trade.
- **`openzro` matures into a stable LTS upstream** (multi-
  maintainer, 6+ months cadence, named institutional backer,
  corporate or governmental contributor base). Action: evaluate
  path (c) in a 1-week spike against the in-flight Block 22 +
  Block 22H work. Don't switch lazily.
- **Upstream `netbirdio` relicense moves further** (e.g.,
  AGPL-with-additional-restrictions or commercial-only). Action:
  forced rebase to pre-AGPL with clean-room (path b) or to
  openzro (path c). The discipline above (process boundary,
  in-tree patches) makes either rebase tractable rather than
  catastrophic.
- **Repo visibility changes** (a future operational reason
  drives the repo private). Action: per D3 spike protocol, carve
  out a public AGPL-fork sub-repo OR shift paths.
- **A new AGPL fork enters our stack** (e.g., we fork
  `signal/` or `relay/`). Action: re-affirm D2 + D4 + D5 apply
  to it; no new ADR needed unless the deployment shape differs
  materially.

---

*See `GLOSSARY.md` for terms and acronyms.*
