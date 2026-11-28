# ADR 1266 — A tactical data chain is declared source to receiver, judged hop by hop, and pages when data stops between two hops

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-26 |
| Deciders | operator |
| Repo scope | goatstuff (`goat-feeds`: the chain judge, its API, the Chains view and the alert rules), goat core (this decision, the design, boundary-node tunnel state), goat-transformers (`goat-xf-pipe-zenoh` health, per ADR 1260), private overlays (chain declarations, receiver contracts) |
| Amends | [ADR 1260](1260-every-tactical-link-in-one-registry-with-one-health-record.md) (adds chains, a verdict per chain, and paging) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1036](1036-environment-health-surface.md) (health envelope), [ADR 1123](1123-client-demand-must-not-amplify.md) (surfaces never read payload on request), [ADR 1168](1168-self-hosted-paging-and-automatic-oncall-enrolment.md) (pager), [ADR 1253](1253-simple-boundary-node-pseudo-boundary-node-and-one-boundary-monitor.md) and [ADR 1263](1263-simple-boundary-node-partner-ipsec-gateway-fails-closed-and-real-data-needs-a-validated-module.md) (boundary nodes), design [`tactical-chain-observability.md`](../design/tactical-chain-observability.md) |

## TL;DR (for a layperson)

A radar's tracks reach a partner's air-defense display through several steps: the radar
publishes onto the fabric, a converter picks them up, a boundary node sends them through a
tunnel, and the partner's system receives them. Today each step has its own status page,
and finding out why the partner sees nothing means visiting all of them by hand. This
decision writes each such path down once, end to end, has one small service per site check
every step of every path continuously, and shows one answer per path: flowing, source
quiet, or broken at a named step. When data reaches one step and does not reach the next,
it pages a person, with the step and the evidence in the page.

## Context

ADR 1260 gives every tactical link one health record and one registry. That answers "is
this link moving data?". It does not answer the question operators actually ask about a
partner feed: "the radar is up, is the partner getting it, and if not, where does it stop?".
That question spans links of different kinds, owned by different roles, on different hosts:

| Step | Where it runs | What shows it today |
|---|---|---|
| The sensor publishes | a sensor node | the fabric, if someone subscribes |
| A converter or relay | a pipeline host | the unit's journal line |
| The boundary node receives and sends | the boundary host | its monitor (ADR 1253) |
| The tunnel to the receiver | the boundary host | a root-only command |
| The receiver gets it | the partner's system | nothing, unless the partner says so |

Each step's view describes only that step, and a step's input and output can differ (a
boundary node rewrites identity and format on the way out), so reading one step's figure as
another's gives a wrong answer. A link's health in isolation also cannot say which step broke,
because "no data" at a step is normal when the step before it is quiet.

## Decision

1. **A chain is declared, source to receiver.** A chain names one source and one receiver
   and the ordered hops between them. Hops are references to things already declared: the
   source's fabric key (estate), the pipeline pipes (processors pipeline), the boundary-node
   feed, the tunnel, and the receiver. Chains are generated from those declarations, not
   written by hand, so a chain cannot drift from the deployment. Each chain also carries the
   **receiver contract**: the categories, SAC/SIC and port the receiver expects.

2. **Every hop reports the same short record.** Pipes publish the ADR 1260 link health
   record. The boundary node already reports per feed what arrived and what it sent
   (`/api/v1/status`, `/api/v1/modifications`); it adds the tunnel's state (handshake age or
   SA state, bytes), written by a small root timer to a file the unprivileged monitor reads.
   The source hop uses the first pipe's input count, and the source's own heartbeat where it
   has one (a radar's CAT034 status), so "radar up, no targets" is told apart from "radar off".

3. **One judge per site, inside Fieldkit Feeds, named Daisychain.** The chain judge, **Daisychain**, is part of
   `goat-feeds` in goatstuff, which already holds the site's link registry (ADR 1260); it is
   not a new service. It runs on each site's host (never on a sensor or boundary node), reads the hop records every 15 seconds,
   and gives each chain one verdict:

   | Verdict | Meaning | Paged |
   |---|---|---|
   | `flowing` | every hop moved data in the last window and the output matches the receiver contract | no |
   | `source-quiet` | the source sent nothing, and every hop after it is idle too | warning only |
   | `broken at <hop>` | the hop before moved data and this hop did not, for 2 minutes | **yes** |
   | `wrong-shape` | data reaches the receiver but its SAC/SIC or category differs from the contract | **yes** |
   | `receipt-unconfirmed` | everything up to the tunnel is flowing; the receiver gives no acknowledgement | no; shown |

   It reads records only, never payloads (ADR 1123).

4. **Pages go through the existing pager.** Daisychain exposes its verdicts as metrics
   and alert rules for the site's Alertmanager, which pages through ntfy on the relay tier
   (ADR 1168). A page names the chain, the hop, and the two counts that prove it ("pipe
   `relay-x`: 1,200 in over 2 min; boundary feed `x`: 0 received"). Recovery sends a
   resolve. `source-quiet` goes to the warning tier, because a quiet radar is often a radar
   with nothing to see.

5. **One place to look.** Fieldkit Feeds gains a Daisychain view: every chain at the site, needs
   attention first, each opening to its hops left to right with the count at each. The CLI
   is `goat chains ls` and `goat chains show <id>`; the API is Daisychain's
   `/api/v1/chains`. Each hop links to its own detail (a pipe's record, the boundary node's
   API). The receiver's own view stays the boundary node's tunnel-side API (ADR 1253).

6. **The last hop is stated honestly.** A UDP receiver that sends nothing back cannot be
   seen to receive. The verdict then stops at `receipt-unconfirmed`, and each receiver is
   asked to send `goat.bn-feedback/v1` acknowledgements, which turn the last hop into a
   measured one.

## Consequences

- "Is the partner getting the radar, and where does it stop?" is one page, one command, or a
  page on the phone, instead of a manual investigation across hosts.
- Every role that starts links already owes a manifest (ADR 1260); chains reuse those
  manifests, so no second declaration is maintained.
- The boundary host gains a root timer that writes tunnel state; the monitor stays
  unprivileged.
- A source with no heartbeat and bursty data can only be judged `source-quiet`, never
  "down"; the verdict says which evidence it had.
- Until a receiver sends acknowledgements, a chain can be `flowing` up to the tunnel while
  the receiver's own display is misconfigured; the receiver's tunnel-side API is the
  receiver's check for that.

## Alternatives considered

| Option | Why not |
|---|---|
| Per-link health only (ADR 1260 as written) | Answers "is this link moving" but not "where does the path stop"; quiet is normal when the step before is quiet |
| Panoscope's fabric picture | Shows the fabric, not the hops off it: the pipes, the boundary node, the tunnel and the receiver are outside it |
| Synthetic probes injected at the source | Adds traffic the receiver would display; real data already exercises every hop |
| A central poller across sites | Breaks in island mode and adds a dependency per site; each site judges its own chains |
| Alerting on each link separately | One break would page once per downstream link; the chain verdict pages once, at the hop that broke |
