# ADR 1109 — Distributed fleet management as a design dimension; convergence is pulled

| Field | Value |
|---|---|
| Status | **Accepted** (2026-11-24, operator; proposed 2026-08-02). Pull convergence is designed, not built: today every deploy is a push run by an operator (ADR 1198, `converger::Push`). D6-D9 (2026-11-24) settle how intent reaches a node |
| Date | 2026-08-02 |
| Deciders | maintainer |
| Repo scope | goat core (informs every consumer repo and every partner goatnet that deploys to nodes) |
| Supersedes / amends | **Amends [ADR 0224](0224-progressive-config-rollout-with-auto-revert.md) §3.6** — extends its staged-rollout *pattern* past the Phoenix-native boundary to substrate deploys (D4). Consumes [ADR 0206](0206-island-mode-by-construction.md), [ADR 0201](0201-enrollment-vs-mesh-op-separation.md), [ADR 0208](0208-phoenix-native-peer-discipline.md), [ADR 1060](1060-universal-deploy-rollback-ledger.md), [ADR 1081](1081-control-plane-storm-resilience.md) |
| Related | [`fleet-management.md`](../design/fleet-management.md) (the design doc this decides for) · `environment-parity.md` · [`universal-deploy-rollback.md`](../design/universal-deploy-rollback.md) · [`zenoh-brokerless-consequences.md`](../design/zenoh-brokerless-consequences.md) · ADR 0215 · [ADR 1016](1016-complexity-budget-substrate-encapsulation.md) · F-115 · F-177 |

## TL;DR (for a layperson)

We manage a lot of machines spread across sites that regularly lose their internet connection. Until now, the rules for how a change reaches all those machines were scattered across a dozen documents, so the same question got a different answer depending on which document you happened to open. This decision gives that topic a name and one home, and settles the one question everything else was waiting on: **machines fetch their instructions and update themselves, rather than a central server pushing changes out to them.** That is the opposite of how most commercial fleet products work, and the reason is that a site cut off from the world for a month still has to be able to run itself — a central server it must reach is a server it cannot reach exactly when it matters most.

## Context

Fleet management already exists in this project, unnamed and distributed across eleven mechanisms. The consequence is not a missing feature; it is that fleet-shaped decisions get re-made per mechanism, inconsistently:

- The fleet-wide **undo** primitive is live, universal, and CI-ratcheted to zero gap ([ADR 1060](1060-universal-deploy-rollback-ledger.md), live-fired M-075). The fleet-wide **do** primitive is `ops/ansible/coordinated-upgrade.yml`, a serial playbook whose only blast-radius control is `-e target=`.
- Two staged-rollout designs exist — [ADR 0224](0224-progressive-config-rollout-with-auto-revert.md) for configuration activation and `failure-isolation.md` §6 for a canary-site gate — and neither references the other. Neither is built.
- A node offline for months does not converge on power-up (F-115, active, engineered around by hand).
- Straggler remediation is an unbounded serial SSH fan-out (`peer-recover.sh`) in a project that has already measured why unbounded fan-out storms itself ([ADR 1081](1081-control-plane-storm-resilience.md)).

**The blocking question is push-versus-pull**, and the tree records it as explicitly open — backlog item 50J.2 states the fork verbatim ("Lambda fan-out vs DynamoDB Streams + per-relay agent that watches the stream and applies — pick one"). Staged rollout, offline convergence, and fleet remediation all wait on it. It is one decision that unblocks three designs, which is why this ADR exists now rather than after a mechanism is built.

## Decision

**D1 — Fleet management is a named design dimension** with one canon doc, [`fleet-management.md`](../design/fleet-management.md), covering identity/inventory, intent, convergence, change propagation, undo, observation, and remediation. Its fifteen principles carry explicit provenance (ADOPTED from a cited industry origin / DERIVED from a cited founding decision / ORIGINATED with the gap named). It owns *the dimension*; `environment-parity.md` continues to own *config alignment between environments* and is not duplicated.

**D2 — Convergence is pulled.** A node fetches signed, versioned intent from a **site-local replica** on a jittered interval and reconciles itself. The fleet plane publishes intent and collects reports; it never drives execution. Three constraints settle this and all point the same way:

1. **No delivery lifecycle.** The fabric has no acks, retries, or DLQ; `put` is fire-and-forget, and denial and drop are silent at both ends (F-177, F-178, [ADR 1022](1022-zenoh-denial-observability.md)). A push cannot know it landed, and making it know re-introduces the broker the project rejected.
2. **Load must decay under failure** ([ADR 1081](1081-control-plane-storm-resilience.md)). A push fan-out retried against unreachable nodes is the M-085 storm shape. A jittered pull with backoff and a circuit breaker decays by construction.
3. **Island mode** ([ADR 0206](0206-island-mode-by-construction.md)). Push converges the nodes reachable during the run; at the denied edge that is the wrong set. Pull converges a node whenever *the node* can reach intent — true with the WAN down, because the replica is site-local.

**Push is retained as a declared exception** for operations requiring coordinated ordering across hosts — today exactly one, `coordinated-upgrade.yml`'s stop-all → migrate-once → verify → restart-serially sequence. Those operations are **explicitly unavailable in island mode**, stated rather than papered over: a cut-off site cannot run a coordinated multi-host migration, because coordination is what it lacks. This is [ADR 0201](0201-enrollment-vs-mesh-op-separation.md)'s classification applied to fleet operations.

**D3 — One convergence engine.** The pull loop does not reconcile host state itself; it invokes the existing idempotent Ansible play locally when fetched intent differs from last-applied. What is new is the *trigger* and the *transport*, not the actuator. This preserves [`engineering-baseline.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/engineering-baseline.md) §2.2/§12.3 ("drift is fixed by re-running") and avoids two authorities over one host.

**D4 — Extend ADR 0224's staged-rollout pattern past its §3.6 Phoenix-native boundary to substrate deploys.** 0224 §3.6 scopes itself to Phoenix-native peers and routes substrate (Spock, Zenoh routers, Postgres, Caddy/Traefik, kernel, OS packages) to traditional Ansible procedures, because its cohort derivation depends on peers declaring consumed key expressions. But substrate deploys are where fleet-wide blast radius actually lives. Cohorts for substrate waves derive instead from the **site and node-class dimensions of the identity grammar** (ADR 1043) — still derived, never a maintained cohort registry (0224 §3.2 preserved). **0224's deferred-activation mechanism does not extend**: substrate cannot defer activation, so a substrate wave stages at the deploy step and its safety property is health-gated automatic revert, not delayed activation. Waves are per-site, halt automatically, and **do not advance on `unknown`** — a node that has not reported counts against promotion, never toward it.

**D5 — An operation class declares four axes before it is built:** transport (`pull` default / `push` argued exception), partition-mode (`activate-on-arrival` / `hold-until-rejoin`, 0224 §3.4), artifact class (`image`/`config`/`data`/`cert`/`forward-only`, [ADR 1060](1060-universal-deploy-rollback-ledger.md) dispatch — where undo is a lie the class refuses with a restore or rotate pointer), and blast unit (`node`/`site`/`fleet`). All four already exist in canon, in three documents; nothing composed them, which is how a `fleet`-blast-unit operation came to exist with no declared partition-mode.

No new daemon, store, or network listener is authorized by D1-D5; D8 (2026-11-24) names the site registry and owes its own promotion. Sized per ADR 0215 for 6 sites and ~3,000 peers at the design center.

## Amendment 2026-11-24 — how intent reaches a node (D6-D9)

The pull loop in D2 needs four things the original decision left open: which document a node fetches, who signs it, where a site keeps its copy, and how a change moves through the fleet. The shape below is the one demonstrated in a lab of about 30 machines in *GitOps Reconciliation at the Edge* (draft): desired state signed once at the source, carried down a chain of registries, verified by every machine that uses it, applied by Ansible on the machine itself. D6-D9 take that shape and bind it to what goat already has.

**D6 — One signed document: `IntentV1`.** A node fetches its goatnet's `IntentV1` ([ADR 1110](1110-typed-consistency-foundation-and-console.md); one `intent.json` per goatnet, kept in the environment repository), which already carries a monotonic `serial`, a `content_hash` and every pinned value per member. The release each node class should run is added to `IntentV1` as a per-class entry (class, release digest); there is no second signed index. A node refuses a serial lower than the one it last applied and accepts a deliberate revert only as a new, higher serial.

**D7 — The signer is the goatnet's own offline certificate authority.** `IntentV1` is signed with the existing intent signer (`ops/enrollment/cmd/intent-sign`) under the goatnet's offline CA. No new key material and no separate signing keypair are introduced. A node verifies the signature against that CA before it reads anything else in the document.

**D8 — A site keeps its own copy in a small pull-through OCI registry; Forgejo keeps its role.** Each site (and, where present, each pod) runs a lightweight OCI registry, zot being the reference, that pulls from an ordered list of upstreams, so a change crosses each network link once and a site cut off from its upstream keeps serving what it holds. Nodes pull from the nearest registry that answers. Trust is in the signed content, never in the registry: a node verifies every artifact it pulls, so a stale or hostile store can withhold a change but cannot make a node apply one. Forgejo on the site observability host keeps its git and package roles; git remains the one source of record (ADR 1198). The site registry is a new always-on service; its [ADR 0213](0213-convenience-vs-core-services.md) promotion is owed in the change that builds it.

**D9 — Rings now, regions later.** A change moves through canary and stable rings that share one serial counter, so every ring runs a generation that exists in the tree. Rings compose with D4's per-site waves as a further cohort axis, derived from identity-grammar dimensions, not a maintained list. Higher groupings than a site (region, theater) are rollout seams only, to be named when a goatnet has more than one; they are not topology.

**Not changed by D6-D9.** D3 stands: the node's agent is a timer that fetches, verifies and runs the existing Ansible play locally; it does not reconcile host state itself and is not a second engine. The first node class to converge by pull is the observability host: it carries no fabric role, it already has its own standup leg, and a failed pull there degrades monitoring only. 1.0 of `goatbuilder` ships push only; the pull converger arrives as a second implementation of its `Converger` trait ([`goatbuilder.md`](../design/goatbuilder.md)).

## Alternatives considered

- **Central fleet controller / desired-state server** (Rancher Fleet, Salt master, Tanium, Fleet+osquery). The industry default. Rejected — [ADR 0206](0206-island-mode-by-construction.md) rejects cross-site quorum coordinators and centralized service discovery by name, and a site must manage its own nodes cut off for 30 days. The permitted residue is a *derived read-only projection* at the rear plane ([ADR 1065](1065-graph-engine-property-graph-read-model.md) stance): it may render, it may not decide.
- **SaaS fleet management** (Intune, JAMF, Datadog-style agents). Rejected — [`deliberate-deltas.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/deliberate-deltas.md) §1.1 disqualifies cloud as a store for data-at-rest, and fleet state is data-at-rest. The only qualifying shape is the PagerDuty carve-out's (named exception, own ADR, strict metadata-only boundary); no such proposal is made.
- **Keep push, add acknowledgement.** Rejected — an ack layer over a fabric with no delivery lifecycle is the broker by another name ([`zenoh-brokerless-consequences.md`](../design/zenoh-brokerless-consequences.md)), and it does not fix the storm shape or the reachable-set problem.
- **A bespoke reconciling agent as a second convergence engine.** Rejected — two authorities over one host contradicts the baseline's re-run discipline, and a new always-on privileged daemon owes an [ADR 0213](0213-convenience-vs-core-services.md) core-service promotion with its staffing implication. D3 gets the property by re-triggering the existing actuator instead.
- **Read ADR 0224 through to substrate implicitly.** Rejected — 0224 §3.6 states its boundary deliberately and gives the mechanical reason. Extending it silently would leave two documents disagreeing about scope; D4 makes it an amendment.
- **Leave push-versus-pull open and decide it inside the first mechanism.** Rejected — it is the fork three separate designs are blocked on; deciding it inside one of them means the other two inherit an implicit answer.

## Consequences

- **Positive:** three blocked designs (staged rollout, offline convergence, fleet remediation) gain a shared answer; the fleet gains a per-node convergence path that works with the WAN down; the fleet-wide `do` primitive stops being structurally less safe than the `undo` primitive it pairs with.
- **Negative / cost:** pull loses cross-host orchestration ordering and is harder to debug than a push run whose output an operator watches — a documented, accepted tradeoff. A poll interval is a doctrine deviation from "ride operational traffic; probe only when quiet" ([`fabric-health-surface.md`](../design/fabric-health-surface.md) §1a rule 5), named and bounded by the decay requirement. D4 admits substrate into a rollout pattern that was deliberately scoped away from it, which widens what the pattern must survive.
- **Neutral:** nothing here is built. The first buildable increment is a convergence-reporting arm on the existing `EnvHealthV1` envelope; the roadmap and its gates are in [`fleet-management.md`](../design/fleet-management.md) §7, tracked as `fleet-management` in `backlog.md`. The first consumer-facing increment (waves) owes a complexity-leak entry in its own change-set per [ADR 1016](1016-complexity-budget-substrate-encapsulation.md) D7.

## Trigger conditions for revisit

- A coordinated-ordering operation class grows beyond the single retained push exception — the push/pull split is then load-bearing enough to re-argue rather than assert.
- Measured convergence latency at the ADR 0215 design center (~3,000 peers) fails the operational need, per [ADR 1012](1012-load-testing-discipline.md)'s no-measured-ceiling-no-claim rule.
- A site-local pull replica proves unmaintainable at a site with no local storage tier, which would force the intent transport back toward the fabric.
- P3 (islanded intent origination) reaches build, which requires a per-fact rejoin-reconciliation policy this ADR does not define.
