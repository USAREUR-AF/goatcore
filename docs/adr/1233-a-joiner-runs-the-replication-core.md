# ADR 1233 — A joiner runs the replication core: Traefik, NetBird management, signal and Ory are present at every site that replicates

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-13 |
| Deciders | operator (acceptance) |
| Repo scope | goat core (goatbuilder end-state catalog) |
| Amends | [ADR 1206](1206-goatnet-declared-end-state.md) item 2 (the founder rule: "a joiner does not run it, ADR 1084 §D3") |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1084](1084-block-64r-mgmt-repset-and-two-node-federation.md) §D3 (single-homed admin surfaces, any-node enrolment), [ADR 1165](1165-island-mode-multi-writer-every-site-admits.md) (lifts §D3 one write class at a time), [`../design/goatbuilder.md`](../design/goatbuilder.md) §`join-node`: the whole declared site, part (e), [`../design/goatnet-end-state.md`](../design/goatnet-end-state.md) |

## TL;DR (for a layperson)

goatbuilder keeps a list of the services each site of a goatnet should run, and checks each site against it. That list says that seven services run only on the first site of a goatnet, so on every site added later it expects them to be missing. In fact a later site runs most of them: it lets devices join, keeps them in sync, and signs people in, exactly as the first site does. This decision corrects the list for the four services that clearly run everywhere, keeps the administration screen on the first site as the earlier decision intends, and says that the last two services are placed only after someone looks at what a real later site runs. Until the list is corrected, the check reports some services a later site runs as "not applicable" instead of checking them.

## Context

The end-state catalog, `ops/goatbuilder/endstate/catalog.yaml`, marks seven services `present: {founder: true}`: traefik, netbird-mgmt, signal-server, netbird-dashboard, ory, kratos-selfservice-ui and mesh-coredns. `endstate.py` reads that mark as absent at any site with `goatnet_founder: false` (`presence()`, the founder branch). ADR 1206 item 2 gives the reason: "the goatnet's own control plane runs on the founder; a joiner does not run it, ADR 1084 §D3".

ADR 1084 §D3 does not say that. It keeps only the admin mutation surfaces (portal, id console, dashboard, `goat admin`) single-homed on one node. The second node "serves enrollment, peer sync, signal, and Ory login flows". `join-node` phase 13 deploys Postgres with Spock, NetBird management, signal and Ory (Kratos and Hydra) at a joiner, and every live joiner runs Postgres, signal, NetBird management, Traefik, Kratos and Hydra.

The effect was measured: with the model passed to the acceptance phase, a joiner's PASS count fell from 18 to 16, because rows for services the joiner runs read N/A. The designed `join-node` phase "the declared services" reads the same model, so with the catalog as it is it would leave those services out of its verification ([`goatbuilder.md`](../design/goatbuilder.md), `join-node` part (c)).

## Decision

This ADR amends ADR 1206 item 2 only. ADR 1206's other items and its body are unchanged; the founder rule is replaced by the rules below.

1. **The replication core is present at every site that runs it.** traefik, netbird-mgmt, signal-server and ory lose the `founder: true` mark. Their presence comes from the pin table and their flags, as for any other service (ADR 1206 item 2, first half). A founder and a joiner are graded the same way for them.
2. **netbird-dashboard stays at the founder, as an admin surface.** It is one of ADR 1084 §D3's single-homed admin mutation surfaces, so a joiner does not run it until ADR 1165 lifts §D3 for its write class. That lift is declared in group_vars, and the model then reports the dashboard present at the joiner with no change to the catalog or to `goatbuilder`.
3. **The catalog gains an explicit admin-surface mark.** The rows for portal, id-admin-console, netbird-dashboard and onboard-runner carry it. The mark means: present at the site that holds the admin writer (today the founder), and at every site once the class's §D3 lift is declared in group_vars. A marked row that has its own flag or pin keeps that as the deciding declaration; the mark adds the reason the model gives when the row is absent at a joiner. The wording of `join-node`'s "not run here" list (portal, id-admin-console, netbird-dashboard, onboard-runner) comes from this mark in the catalog, never from a list in code, so the plan, the summary and the end-state grade cannot disagree ([`goatbuilder.md`](../design/goatbuilder.md), `join-node` part (d)). Neither the mark, nor an error text, nor the `core` machine class makes an admin surface impossible on a joiner.
4. **kratos-selfservice-ui and mesh-coredns are pending a live read.** This ADR does not place them. The design places them by reading a live joiner's running containers before the catalog changes, not by argument. That read is required and has not been done:

   > **Pending: the live read.** Before `catalog.yaml` changes, read the running containers on a live joiner's management host and record the result in `goatbuilder.md`, `join-node` part (e), as measured. A service found running there takes rule 1. A service not found there keeps its founder mark, and its catalog row says what the read found. Until the read is recorded, kratos-selfservice-ui and mesh-coredns keep `present: {founder: true}` and this ADR stays Proposed.

5. **The gate.** The change to `catalog.yaml` and `endstate.py` lands with a test that holds both halves of the design's gate: "the model of a joiner fixture marks present every service a live joiner runs, and acceptance with `--endstate` reads no row N/A for one of them." The gate cannot pass before the live read in item 4, because it names what a live joiner runs.

This ADR becomes Accepted in the pull request that changes the catalog and `endstate.py` and passes the gate. Acceptance is the operator's.

## Alternatives considered

- **Keep the founder rule and add each joiner service by hand to a joiner list.** It would make a second home for presence, which ADR 1206 item 1 rejects, and the list would drift from what `join-node` phase 13 deploys.
- **Remove the founder mark from all seven services.** It would report netbird-dashboard as missing on every joiner, which contradicts ADR 1084 §D3 while ADR 1165 has not lifted it, and it would place kratos-selfservice-ui and mesh-coredns without evidence.
- **Place kratos-selfservice-ui and mesh-coredns by argument.** §D3 puts Ory login flows on the second node, which suggests the self-service UI runs there, and mesh DNS could argue either way. The design refuses placement by argument because the model's job is to agree with what runs; one read of a live joiner settles it.
- **Leave the catalog as it is and treat the N/A rows as known.** A row that reads N/A for a running service is a grade that checks nothing, and the declared-services phase of `join-node` would skip verifying those services on every new site.

## Consequences

**Positive.** A joiner's end state and acceptance grade the replication core it runs, so a broken NetBird management, signal, Traefik or Ory at a joiner fails instead of reading N/A. The designed `join-node` phase verifies what phase 13 converged. The admin-surface placement has one home, the catalog, and its wording reaches the operator from there.

**Negative.** Until the live read is done, two services stay on the founder rule, so the gate in item 5 cannot be met and this ADR cannot be accepted. A joiner fixture must be kept in step with what live joiners run. A goatnet whose joiner today lacks one of the four core services will see that as a failure the first time it is graded, which is the intent but will look like a regression.

## Assessor view (SCA-R)

The change makes the configuration baseline (CM-2, CM-8) describe what each site actually runs, so continuous monitoring (CA-7) checks the identity and mesh control services at every site instead of reporting them not applicable. The admin mutation surfaces stay single-homed under ADR 1084 §D3, keeping one writer for policy, group, key and identity changes (AC-3, AC-6), and the catalog mark records that placement as a declared fact rather than an omission. No waiver depends on it. The residual risk is the two services not yet placed; they keep today's grading until the live read is recorded, and the ADR stays Proposed while it is open.

## Trigger conditions for revisit

- The live read finds a joiner running a service outside rule 1 that the catalog marks absent, or not running one of the four.
- ADR 1165 lifts §D3 for a write class: the admin-surface rows for that class then read present at every site through group_vars, and this ADR's item 2 is checked against it.
- A machine class or goatnet shape appears in which a site runs the replication core without Traefik in front of it.
