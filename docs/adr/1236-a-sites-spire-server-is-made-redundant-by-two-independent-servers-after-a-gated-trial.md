# ADR 1236 — A site's SPIRE server is made redundant by two independent servers, adopted only after a gated trial

| Field | Value |
|---|---|
| Status | **Proposed.** Nothing in a running goatnet changes on merge. The decision is the direction and the gates; the build follows a trial that passes them |
| Date | 2026-11-14 |
| Deciders | operator |
| Repo scope | goat core (the `spire` role, `spire-trust-sync`, the peering SPIRE role) |
| Amends | [ADR 1164](1164-per-site-spire-trust-domains.md) D5 (a second server at one site) |
| Supersedes | — |
| Superseded by | — |
| Related | ADR 0412 (SPIFFE/SPIRE workload identity) · [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) · [`spire-failover-promotion.md`](../operations/spire-failover-promotion.md) · [`island-mode-multi-writer.md`](../design/island-mode-multi-writer.md) · [`federated-trust-model.md`](../design/federated-trust-model.md) |

## TL;DR (for a layperson)

Every running program in goat proves who it is with a certificate that lasts one hour, issued by one SPIRE server on one machine at its site. If that machine is lost, a person promotes a standby by hand. If that takes longer than an hour, the programs at that site can no longer prove who they are.

This decision sets the direction for removing that dependency on one machine: run two SPIRE servers at a site that share nothing, and put a small agent in front of them that asks whichever one is answering. It is adopted only if a trial in a lab passes the gates listed below.

## Context

A site has three distinct failures of workload identity, each with its own answer:

| Failure | Answer | Status |
|---|---|---|
| The node holding the SPIRE server is lost | A person promotes a passive node with the staged CA key ([`spire-failover-promotion.md`](../operations/spire-failover-promotion.md)) | play and runbook written |
| A site is cut off from the issuer | A SPIRE server and trust domain per site ([ADR 1164](1164-per-site-spire-trust-domains.md)) | designed, not built |
| The server is lost and nobody is there to promote within the one-hour SVID lifetime | This ADR | not built |

SPIRE's own high availability is several servers on one shared datastore. That does not fit a goat site:

- The `spire` database is one instance on one node, deliberately in no Spock replication set ([`island-mode-multi-writer.md`](../design/island-mode-multi-writer.md)). Two servers sharing it still depend on that node.
- A copy replicated by Spock is not a shared datastore. Replication is asynchronous with last-write-wins, and SPIRE expects every server to see the same records at once. ADR 1164 rejected this for that reason.
- A clustered database also needs its own certificates, issued by an authority other than SPIRE, so SPIRE would depend on something beneath it.

## Decision

### D1 — Two independent servers, one trust domain, nothing shared

A site that needs its SPIRE server to survive the loss of a node runs two SPIRE servers on two nodes. Each has its own local datastore and its own signing keys. Both serve the site's one trust domain, and every relying party trusts the union of the two bundles. Registration entries reach both servers from the same declared configuration, which is how the `spire` role already creates them.

Each workload node runs one SPIRE agent per server, with [spire-ha-agent](https://github.com/spiffe/spire-ha-agent) in front. It gives workloads one Workload API socket, serves the united bundle and sends each request to a server that is answering. The design is described in [the SPIFFE project's account of it](https://spiffe.io/blog/2026-07-19-bottom-turtle-ha-architecture/).

### D2 — Adoption waits for a trial that passes every gate

The trial runs on two nodes of one site in a lab environment. Each gate is shown by a command and its output, recorded as a measurement.

| Gate | Passes when |
|---|---|
| G1 Loss of either server | With either server stopped, every SPIFFE-identified goat service renews its SVID past the one-hour lifetime with no person acting |
| G2 Start with one side down | A node that reboots while one server is unreachable brings its workloads up with valid SVIDs |
| G3 Federation | A federated trust domain validates SVIDs issued by either server, through the bundle endpoint arrangement the peering SPIRE role sets up, and keeps doing so across a CA rotation on one side |
| G4 Router trust | The Zenoh router's trust file carries both servers' CAs and `spire-trust-sync` keeps it current as each rotates. The router matches certificate names only (F-424), so the trial states which names each server may issue and shows they are the same set |
| G5 Rebuild | A destroyed server is rebuilt from declared configuration, gets new keys, and rejoins without re-issuing identities on the surviving side |
| G6 Standing of the component | spire-ha-agent's own documentation calls it early in development. Adoption needs either that statement to change upstream or the operator's recorded acceptance of it for this use |

A trial that fails a gate ends with the failure written down. The manual promotion runbook stays the answer until then.

### D3 — What this does not change

- A cut-off site is still answered by ADR 1164. Two servers at one site do nothing for a site that cannot reach either.
- The trust domain name, the federation mechanism and the rule that recognition is not authorization are unchanged.
- The manual promotion runbook and the staged CA key stay in place until a build under this ADR is live at a site.

## Alternatives

- **SPIRE's shared-datastore high availability on the site's Postgres.** Rejected for the three reasons in Context.
- **Keep manual promotion only.** It covers node loss when a person acts within the hour. It is the fallback, not the end state, because a site must keep running when nobody is there.
- **Lengthen the SVID lifetime.** It widens the window and leaves the single server in place. A longer-lived credential is also a weaker one.

## Consequences

- Two CA keys per site to hold in custody, and two rotating CAs for every relying party to follow.
- Each workload node runs two SPIRE agents and spire-ha-agent in place of one agent.
- ADR 1164 D5 no longer points at shared-datastore high availability; it points here.
- A component outside goat's control enters the identity path. G6 exists for that reason.
