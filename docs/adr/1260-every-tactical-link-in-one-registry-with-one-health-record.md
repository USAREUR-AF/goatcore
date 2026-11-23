# ADR 1260 — Every tactical link is in one registry, reports one health record, and is seen and controlled in Fieldkit Feeds

| Field | Value |
|---|---|
| Status | Proposed — designed, not built; build order in [`fabric-feeds.md`](../design/fabric-feeds.md) §10 |
| Date | 2026-11-23 |
| Repo scope | goat core (design), goat-transformers (`goat-xf-pipe-zenoh` health), goatstuff (`goat-feeds`), private overlays (declared-link manifests) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1259](1259-fabric-feeds-self-service-approved-delivery-of-fabric-topics-to-mesh-systems.md) (fabric feeds), [ADR 1036](1036-environment-health-surface.md) (environment health surface), [ADR 1034](1034-fabric-health-surface.md) (fabric health), [ADR 1123](1123-client-demand-must-not-amplify.md) (client demand must not amplify), [`fieldkit-and-bleating-edge.md`](../design/fieldkit-and-bleating-edge.md) (the Fieldkit rule), [`fabric-feeds.md`](../design/fabric-feeds.md), [`environment-health-surface.md`](../design/environment-health-surface.md) |

## TL;DR (for a layperson)

A *tactical link* is a process that carries data between the goat fabric and a system
that is not on the fabric: a radar's tracks sent to a weapon system's console, or a
drone's position received from that console. A site soon has dozens of them, created
in different places: in the Fieldkit Feeds form, in deployment files, and by hand
during an exercise. This decision puts every link in one list per site, has every link
report the same short health record onto the fabric, and shows both in Fieldkit Feeds.
Anyone can then see what links exist, whether each is moving data, what data it
carries, and who owns it. A link that is running but listed nowhere, or listed but
silent, is shown as a problem rather than forgotten.

## Context

Fieldkit Feeds (ADR 1259) shows and controls the feeds people request in its form. On a
live site the same kind of link also comes from three other places, and Fieldkit Feeds
cannot see those:

| Where the link comes from | Example | Seen in Fieldkit Feeds today |
|---|---|---|
| The Fieldkit Feeds form (`goat-feeds` registry) | a radar feed to an operator console; a drone position received from it | Yes |
| A deployment declaration (processors pipeline encodes and raw relays, boundary-node feeds) | the fused air picture to an operator console; raw radar tracks to a partner's boundary node | No |
| A hand-started unit or container during an exercise | a one-off relay to a new destination | No |
| Bridges inside other services | a sensor node's fabric bridge | No |

The links in the second, third and fourth rows run as the same program
(`goat-xf-pipe-zenoh`) or a close relative, so they behave alike, but each place keeps
its own record and none reports health anywhere a person looks. An operator console can
go without data because its link was never declared, or was declared for a sibling
console, and nothing shows it.

## Decision

1. **One registry per site holds every tactical link.** The `goat-feeds` registry gains
   a second kind of row. *Requested* links are made in the form and controlled there, as
   today. *Declared* links come from deployment files: each role that starts links
   (processors pipeline, boundary node, any other) renders a manifest of the links it
   owns, and `goat-feeds` imports it read-only. A declared row carries its owner and the
   file and switch that control it.
2. **Every link publishes one health record onto the fabric.** Each link process
   publishes, every 10 seconds, a small record on `ops/links/health/<site>/<link id>`:
   state, messages in, converted, failed, sent per destination, time of the last
   message, the last error, and a summary of what passed (messages per second, distinct
   track identities, ASTERIX SAC/SIC or other source identifiers seen, data class). The
   record follows the environment health envelope (`EnvHealthV1`, ADR 1036): honesty
   graded, volatile, published by the process itself, with no central poller.
3. **Drift is a visible state, not a silent one.** The registry joins its rows with the
   health records it hears and shows three conditions: *undeclared* (a health record with
   no row: a hand-started link), *silent* (a row with no health record for 60 seconds:
   declared but not running), and *unreachable* (the destination check fails).
4. **Fieldkit Feeds is the one place to see and act.** A Links view lists every row at
   the site with its owner, kind, health and data summary. Requested links keep their
   actions (pause, resume, renew, remove). Declared links show the switch that controls
   them and where it lives; a change goes through that file and its deploy. The CLI
   equivalent is `goat feeds ls` and `goat feeds show <id>` (the Fieldkit rule, [`fieldkit-and-bleating-edge.md`](../design/fieldkit-and-bleating-edge.md)).
5. **The site roll-up joins the environment health surface.** One record per site,
   `ops/links/health/<site>`, counts links by state. `goat health`, the panoscope
   environment band and stethoscope render it like the fabric and time domains.
6. **Surfaces never read link data to describe it.** The data summary comes from the
   link's own health record. No surface subscribes to a link's payload on a viewer's
   request (ADR 1123).

## Consequences

- An operator can answer "does this console get every live sensor, and is it flowing?"
  from one page or one command, without reading deployment files.
- A link started by hand stays visible as *undeclared* until its declaration merges, so
  exercise changes are not lost after the exercise.
- Every role that starts links owes a manifest; a role without one leaves its links
  *undeclared* on the page, which is the intended pressure.
- `goat-xf-pipe-zenoh` gains a health publisher; its counters already exist in its
  status line.
- Declared and requested links remain two kinds until declared links move into the
  registry as requested rows owned by a service identity (designed in ADR 1259).

## Alternatives considered

- **A separate monitoring dashboard scraping each host's units.** Rejected: it would
  duplicate the registry, need a poller per host, and still not know which links were
  intended.
- **Only metrics in the observability stack.** Kept as an addition (the `/metrics`
  endpoint stays), rejected as the answer: metrics do not say who owns a link or what
  controls it, and they are not on the fabric for island-mode sites.
