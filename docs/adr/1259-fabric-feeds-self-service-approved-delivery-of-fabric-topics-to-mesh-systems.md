# ADR 1259 — Fabric feeds: self-service, approved delivery of fabric topics to mesh systems

| Field | Value |
|---|---|
| Status | Accepted — WIP: first build is `goat-feeds` (goatstuff `ops/serve/goat-feeds/`), on trial; the stages in [`fabric-feeds.md`](../design/fabric-feeds.md) §9 are designed, not built |
| Date | 2026-11-22 |
| Repo scope | goat core (design), goatstuff (service) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1075](1075-producer-data-labels-and-access-request-loop.md), ADRs 1074 and 1076 (data-access grants), [ADR 1109](1109-fleet-management-dimension-and-pull-convergence.md), [ADR 1123](1123-client-demand-must-not-amplify.md), [ADR 1129](1129-warrant-scoped-expiring-network-authorizations.md), [ADR 1165](1165-island-mode-multi-writer-every-site-admits.md), [ADR 1253](1253-simple-boundary-node-pseudo-boundary-node-and-one-boundary-monitor.md), [`fabric-feeds.md`](../design/fabric-feeds.md) |

## TL;DR (for a layperson)

Systems on the mesh, such as a weapon system's operator console, need data from the
fabric in their own format at their own address. Until now each one was wired by hand
by an engineer. This decision gives people a form: pick a topic, pick the receiving
system, the port and the format, and submit. A tactical admin for that site can switch
the feed on at once; anyone else's request waits for a tactical admin or a platform
admin. One delivery service per site sends every approved feed, and anyone can see
whether a feed is flowing and why not.

## Context

On 2026-11-22 a fused air picture was sent to an operator console on the mesh as ASTERIX
CAT062 over UDP. It took one hand-edited env file for one `goat-xf-pipe` systemd unit, one
private-overlay PR and one Ansible deploy. The
operator asked for the same thing to work "scalably and durably for potentially
thousands of systems like this".

What already exists and is reused: the portal's discover, request and approve flow
(ADR 1075) and its data-access grants (ADRs 1074–1076); the goat-transformers engine
and its packs on every core node, which convert between thousands of message formats
and send over UDP or TCP; warrant-generated mesh rules (ADR 1129); and the fleet deploy
path. The simple boundary node (ADR 1253) sends across a goatnet boundary to a partner.
It is a different job and is not replaced.

## Decision

1. **A feed is a registry row, not a file or a unit.** It names a topic, a destination
   peer (by identity, not mesh IP, which changes on re-enrolment), a port, a transport,
   an output format and its framing, identifiers such as an ASTERIX SAC/SIC, a purpose,
   an owner and an end date.
2. **Approval.** A request is pending until decided. A tactical admin, scoped to a
   site, may switch on feeds to systems at that site directly and approve others'
   requests there. A platform admin may do both anywhere. Real data always needs one of
   the two; synthetic topics may be self-service. Every request, decision and change is
   audited with who and why.
3. **One delivery service per site.** It pulls the active feeds for its site and
   converges (ADR 1109). It holds one fabric subscription per distinct topic and output
   form, and fans each converted message out to every feed that shares it, so fabric
   load grows with distinct topics, not with feeds (ADR 1123). It runs at every site
   (ADR 1165).
4. **Operable by design.** Each feed shows whether it is running, messages in, sent and
   failed, the last message time, the last error, and the delivery command it runs. The
   service exposes `/healthz` and Prometheus metrics. UDP feeds report "sending", never
   "received", because UDP gives no answer back.
5. **Plain code.** The service is Go (a control-plane HTTP service, ADR 0112), with its
   registry in the site's Postgres (`postgres-a`); the data path stays the Rust
   `goat-xf-pipe-zenoh`. The page is plain JavaScript with no framework, no package manager
   and no build step, so it stays easy to change while people use it.

## Consequences

- New feeds stop needing an engineer, an overlay PR and a deploy.
- A new role, tactical admin, exists per site. It is a sign-in group, granted the way
  other groups are.
- The first build covers the form, approval, audit, delivery and troubleshooting.
  Data-access enforcement against grants, warrant-generated mesh rules per feed,
  expiry reminders, automatic switch-off and the scale test are later stages
  ([`fabric-feeds.md`](../design/fabric-feeds.md) §9).
- Feeds declared in git today, such as processors-pipeline outputs sent to a console, can move into the registry later, so there is one sender to watch.
