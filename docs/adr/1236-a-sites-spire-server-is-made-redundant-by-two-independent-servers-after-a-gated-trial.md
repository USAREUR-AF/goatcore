# ADR 1236 — A site's SPIRE server is made redundant by two independent servers, adopted only after a gated trial

| Field | Value |
|---|---|
| Status | **Proposed.** Nothing in a running goatnet changes on merge. The decision is the direction and the gates; the build follows a trial that passes them |
| Date | 2026-11-14 |
| Deciders | operator |
| Repo scope | goat core (the `spire` role, including its router trust sync timer, and the peering SPIRE role, `goat-peering-spire`) |
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

Three properties of spire-ha-agent, each stated in [its README](https://github.com/spiffe/spire-ha-agent/blob/main/README.md), fix the shape of the trial:

- **Broker mode.** spire-ha-agent has two modes. The default, `-mode=delegated`, does not serve federated bundles to workloads. `-mode=broker` does, and it uses the SPIFFE Broker API, which the README calls experimental. goat federates trust domains ([ADR 1157](1157-cross-sovereign-federation-edge-terminated.md)), so the trial runs `-mode=broker`. In this mode a workload receives a partner's bundle from whichever server answered, so both servers carry the same federation configuration.
- **The other server's bundle at startup.** spire-ha-agent starts only when both servers are reachable, unless each side already holds the other side's trust bundle. The spire-ha-agent project ships a service for this, also named `spire-trust-sync` (unit `spire-trust-sync@.service` in that repository), which copies each server's bundle to the other under a cross-trust name, `spire-ha`. The trial runs it. The two servers therefore share no datastore and no keys, and each holds the other's public bundle.
- **The union is served to workloads only.** spire-ha-agent serves the union of both servers' authorities on the Workload API socket. A SPIRE server's bundle endpoint, which is what a federated partner reads, serves that one server's authorities. No maintained SPIFFE component serves the union to partners: [spiffe/spire issue 7200](https://github.com/spiffe/spire/issues/7200) asked for one and was closed without it, with the advice to build a simple server. The trial therefore serves the union itself: spiffe-helper reads it from the spire-ha-agent Workload API socket, a web server on the node publishes it as a SPIFFE bundle document under an SVID of the site's trust domain, and the forward that `goat-peering-spire` sets up points at this endpoint in place of one server's bundle endpoint.

Two different things carry the name `spire-trust-sync`. goat's is a timer and script in the `spire` role (`spire-trust-sync.timer`, `spire-trust-sync.sh`) that keeps the Zenoh router's trust file current and rescues an agent whose SVID expired. The spire-ha-agent project's is the bundle copy between the two servers described above. This ADR calls the first "goat's router trust sync" and the second "the spire-ha-agent trust sync". Both run on a node that holds a SPIRE server.

### D2 — Adoption waits for a trial that passes every gate

The trial runs on two nodes of one site in a lab environment. Each gate is shown by a command and its output, recorded as a measurement.

| Gate | Passes when |
|---|---|
| G1 Loss of either server | With either server stopped, every SPIFFE-identified goat service renews its SVID past the one-hour lifetime with no person acting |
| G2 Cold start with one server down | With the spire-ha-agent trust sync running, a workload node is powered off and started again while one server is unreachable. spire-ha-agent starts and every workload on the node gets a valid SVID with no person acting. Shown once for each server being the one that is down |
| G3 Federation | spire-ha-agent runs in `-mode=broker`. A federated trust domain reads the union of both servers' authorities from the endpoint described in D1, reached through the forward `goat-peering-spire` sets up, and validates SVIDs issued by either server. It keeps doing so after a CA rotation on the first server and, separately, after a CA rotation on the second, and while either server is stopped. In the other direction, a goat workload receives the partner's bundle through spire-ha-agent from whichever server answers |
| G4 Router trust | The Zenoh router's trust file carries both servers' CAs and goat's router trust sync (`spire-trust-sync.timer` in the `spire` role) keeps it current as each rotates. It runs on the same node as the spire-ha-agent trust sync, and neither replaces the other's unit or files. The router matches certificate names only (F-424), so the trial states which names each server may issue and shows they are the same set |
| G5 Rebuild | A destroyed server is rebuilt from declared configuration, gets new keys, and rejoins without re-issuing identities on the surviving side |
| G6 Standing of the component | spire-ha-agent's README calls it early in development and not for production, and calls the Broker API that `-mode=broker` depends on experimental. Adoption needs either those statements to change upstream or the operator's recorded acceptance of them for this use. The trial records the SPIRE version whose agent serves the Broker API endpoint, and the serving of the union to partners as a part goat maintains |

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
- Each workload node runs two SPIRE agents and spire-ha-agent in broker mode in place of one agent. Each node that holds a SPIRE server also runs the spire-ha-agent trust sync.
- goat maintains the endpoint that serves the united bundle to federated partners, because no SPIFFE project component does. A partner that reads one server's bundle endpoint directly rejects SVIDs issued by the other server.
- ADR 1164 D5 no longer points at shared-datastore high availability; it points here.
- A component outside goat's control enters the identity path. G6 exists for that reason.
