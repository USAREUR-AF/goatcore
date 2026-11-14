# ADR 1233 — Every site that replicates is declared to run every service; what a joining site does not run yet is a named gap

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-13; revised 2026-11-14 |
| Deciders | operator (acceptance) |
| Repo scope | goat core (goatbuilder end-state catalog, `endstate.py`, `join-node`, acceptance) |
| Amends | [ADR 1206](1206-goatnet-declared-end-state.md) item 2 (the founder rule: "a joiner does not run it, ADR 1084 §D3") |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1165](1165-island-mode-multi-writer-every-site-admits.md) (every site fully island-mode capable; it retires ADR 1084 §D3 one write class at a time), [ADR 1084](1084-block-64r-mgmt-repset-and-two-node-federation.md) §D3 (the earlier rule that kept the admin surfaces on one node), [`../design/goatbuilder-v1-gate.md`](../design/goatbuilder-v1-gate.md) group I, [`../design/goatbuilder.md`](../design/goatbuilder.md) §`join-node`: the whole declared site, parts (d) and (e), [`../design/goatnet-end-state.md`](../design/goatnet-end-state.md) |

## TL;DR (for a layperson)

goatbuilder keeps a list of the services each site of a goatnet should run, and checks each site against it. The list said that some services run only on the first site, so on every site added later it expected them to be missing. That is wrong: every site of a goatnet must be able to carry on alone when it is cut off from the others, so every site must run every service it needs to do that.

This decision corrects the list. Every site that replicates is declared to run all of them: the services that keep devices joined and in sync and sign people in, the administration screens, the enrolment engine and the site's own name server. A later site that does not run one of them yet is not "fine by design". It is shown as a gap, "declared, not yet built at this site", together with the step of the build plan that closes it.

## Context

The end-state catalog, `ops/goatbuilder/endstate/catalog.yaml`, marked seven services `present: {founder: true}`: traefik, netbird-mgmt, signal-server, netbird-dashboard, ory, kratos-selfservice-ui and mesh-coredns. `endstate.py` read that mark as absent at any site with `goatnet_founder: false`, and ADR 1206 item 2 gave the reason "a joiner does not run it, ADR 1084 §D3". The portal, the id console and the onboarding engine were absent at a joiner because the joiner's flags and pins leave them off, and `join-node` (#612) printed that absence as "single-homed at the founder (ADR 1084 §D3)".

That writes today's gaps into the declared end state. The decision of record is that every site is fully island-mode capable ([ADR 1165](1165-island-mode-multi-writer-every-site-admits.md), gate group I of [`goatbuilder-v1-gate.md`](../design/goatbuilder-v1-gate.md)). ADR 1084 §D3, which kept the admin mutation surfaces on one node, is the earlier rule that ADR 1165 retires one write class at a time. It is not the target, and a declared end state may not say founder-only for anything a site cut off from the others needs.

**The live read, measured 2026-11-13.** The running containers were read on the management hosts of two joined sites of a standing goatnet. Both run admit-audit, consistency-health, the zenoh router, fabric-crawler, fabric-health, federation-proxy, NetBird management, mesh-health, Hydra, Kratos, Postgres, presence-health, the Kratos self-service UI, signal, time-health and Traefik. Neither runs the NetBird dashboard, the portal, the id console, the onboarding engine or mesh DNS yet.

## Decision

This ADR amends ADR 1206 item 2 only. ADR 1206's other items and its body are unchanged.

1. **Every service a cut-off site needs is declared present at every site that replicates.** The catalog's `present: {every_site: true}` covers the replication core (traefik, netbird-mgmt, signal-server, ory, kratos-selfservice-ui), the admin surfaces (netbird-dashboard, portal, id-admin-console, onboard-runner) and mesh-coredns. A founder and a joiner are graded the same way. Where a row also names a pin or a flag, that declaration still decides at the site.
2. **No row is founder-only.** The catalog has no `founder` presence rule and no mark that means "present at the founder". `endstate.py check` refuses a row that declares either.
3. **What a joining site does not run yet is a named gap.** A row the end state declares at every site but that a joining site does not run yet carries `gap: {row, why}` in the catalog. At a joining site it is graded `GAP`: "declared, not yet built at this site: <why> (gate row <row>)". It is never N/A and never "not run here by design". The site's own flag turning the service on closes the gap; a pin, which only names a version, does not. The gaps today:
   - **The admin surfaces** (netbird-dashboard, portal, id-admin-console, onboard-runner), gate rows **I1** and **I4**: an admin surface cannot be turned on at a second site before its store is at the multi-writer schema (I1) and its write class is switched on there (I4).
   - **Mesh DNS** (mesh-coredns), gate row **I3**: join-node does not deploy mesh DNS yet, and a site cut off from the others needs its own resolver. Its absence on both joined sites read above is a defect, closed when `join-node` deploys it.
4. **The gap reaches every reader from the catalog.** The end-state grade, acceptance (a row a gap service owns reads `SKIPPED` with the gap's text, not N/A), and `join-node`'s list of what the site does not run yet all print the catalog's text and gate row. Nothing in code names the gaps.
5. **A service a site turns off is still a declared absence.** An add-on a site's flag turns off (`isitup_enabled: false`) stays N/A with its flag as the reason. That is the site's own declaration, not a gap.
6. **The gate.** The change lands with a test holding: the model of a joiner fixture marks present every service a live joiner runs, and acceptance with `--endstate` reads no row N/A for one of them; every gap is graded `GAP` with its gate row, in the grade, in acceptance and in `join-node`'s list; and no catalog row is founder-only.

This ADR becomes Accepted in the pull request that changes the catalog and `endstate.py` and passes the gate. Acceptance is the operator's.

## Alternatives considered

- **Keep a founder rule for the admin surfaces and mesh DNS, as the first draft of this ADR did.** It writes today's gaps into the end state as design, contradicts ADR 1165, and leaves a cut-off site without its admin surfaces or its resolver while every grade reads green.
- **Grade a gap as a failure.** Every joined site would read red until gate group I is built, so the red would stop carrying information. A gap is counted and printed with its gate row and does not by itself make a site red; it stops being a gap only when the service is built there.
- **Grade a gap as N/A.** N/A says the service does not apply to the site. That is the claim this ADR removes.
- **Turn the admin surfaces on at a joiner now.** Two writers on stores that are not at the multi-writer schema diverge; I1 and I4 come first.

## Consequences

**Positive.** Each site's end state says what the site must run to carry on alone, and what it does not run yet is visible as work with a named gate row, in the grade, in acceptance and in `join-node`'s plan. The replication core at a joiner is graded, so a broken NetBird management, signal, Traefik or Ory there fails instead of reading N/A.

**Negative.** A joined site's grade carries gaps until gate group I is built, and acceptance reports their rows as skipped. Mesh DNS on the joined sites is a defect to fix in `join-node` (I3), not a declared state.

## Assessor view (SCA-R)

The configuration baseline (CM-2, CM-8) describes what each site must run to operate alone, so continuous monitoring (CA-7) checks the identity and mesh control services at every site and reports each missing service as a named gap rather than as not applicable. Until I1 and I4 are built, the admin mutation surfaces still have one writer for policy, group, key and identity changes (AC-3, AC-6), and the gap text says why. No waiver depends on it. The residual risk is the gaps themselves: a site cut off before gate group I is built cannot administer itself or resolve mesh names, and the grade says so.

## Trigger conditions for revisit

- `join-node` deploys mesh DNS (I3): its gap mark is removed.
- An admin surface's store reaches the multi-writer schema and its class is switched on (I1, I4): its gap mark is removed, and its flag turns it on at every site.
- A live read finds a joined site running a service the catalog marks as a gap there, or not running one it declares built.
