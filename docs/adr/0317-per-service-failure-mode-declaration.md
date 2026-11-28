# ADR 0317 — Per-service failure-mode declaration

| Field | Value |
|---|---|
| Status | Proposed; amended 2026-11-26 — ntp-admit-sidecar removed from the initial scope (retired) |
| Date | 2026-05-04 |
| Repo scope | goat core (each consumer-repo with load-bearing services follows the same declaration discipline) |
| Supersedes | — |
| Superseded by | — |
| Related | [`0006-multi-agent-human-concurrency-governance.md`](0006-multi-agent-human-concurrency-governance.md), [`0204-multi-site-active-active.md`](0204-multi-site-active-active.md), [`0208-phoenix-native-peer-discipline.md`](0208-phoenix-native-peer-discipline.md), [`0224-progressive-config-rollout-with-auto-revert.md`](0224-progressive-config-rollout-with-auto-revert.md), [`0822-wide-event-observability-otlp-openobserve.md`](0822-wide-event-observability-otlp-openobserve.md), [`1011-findings-register.md`](1011-findings-register.md), [`1013-continuous-architectural-chaos.md`](1013-continuous-architectural-chaos.md), [`1014-codex-pattern-adr-to-enforced-lint.md`](1014-codex-pattern-adr-to-enforced-lint.md). Prior art: [Cloudflare, "Code Orange: Fail Small, Fail Complete," May 2026](https://blog.cloudflare.com/code-orange-fail-small-complete/) §"Reducing Failure Impact: Graceful Degradation Patterns." |

## TL;DR (for a layperson)

When one of our services loses access to something it depends on — a database, a config registry, a peer service — what does it do? Today this is implicit, decided by whoever wrote the code, and easy to get wrong. Cloudflare adopted a discipline of having every service declare in writing what it does in each failure scenario, with three named choices (use the last good data, run with reduced functionality, refuse the request cleanly). We adopt that discipline and add a fourth choice unique to our architecture: tear down and rebuild from a clean copy. Declarations live in one matrix so an assessor or new contributor can read the whole posture in one place, and our continuous chaos system tests that the declared behavior matches reality.

## §1 Vocabulary

- **Failure mode** — the named behavior a service exhibits when a specific dependency is unreachable or unhealthy. One service may declare different modes for different dependencies.
- **Fail-stale** — continue serving using the last known-good copy of the unreachable dependency's data. Used when the data is recent enough that staleness is preferable to refusal.
- **Fail-open** — continue serving requests with the affected functionality reduced or disabled. The service still responds; some features degrade.
- **Fail-closed** — refuse the request cleanly with a structured error. Used when serving without the dependency would compromise correctness or security.
- **Fail-rebuild** — tear down the service's local state and rebuild from supply-chain-delivered code, then rejoin. Available only to Phoenix-native peers (ADR 0208); the cruft test guarantees a clean rebuild path.
- **Declaration matrix** — the single table in `docs/design/threat-model.md` §"Failure-mode declaration matrix" listing every load-bearing service, its dependencies, and its declared mode per dependency.

## §2 Context

Three forces converged.

**1. Today's failure-mode posture is implicit and unaudited.** A reader cannot answer "what does the schema-enforcer do when the schema registry is unreachable?" without reading the schema-enforcer's source. The same question for db-policy-engine, audit-logger plugin, and the WASM module loader has no central source. An SCA-R asking "show me your degradation strategy" gets a reading list, not a matrix.

**2. Block 38 (continuous architectural chaos) needs a target spec.** ADR 1013 commits Block 38 to encode every load-bearing architectural invariant as a runnable probe. Without per-service failure-mode declarations, the chaos probes have no canonical "what should happen?" to compare against. Declarations make probes possible.

**3. Phoenix-native peer discipline gives us a failure mode no one else has.** The standard taxonomy in industry literature (and in Cloudflare's post) names three modes: stale, open, closed. Phoenix-native peers (ADR 0208) can additionally declare *fail-rebuild* — a mode that requires the cruft test, supply-chain delivery, and stateless reconstruction. Non-Phoenix substrate cannot use this mode. Naming it explicitly here both extends the standard taxonomy and reinforces the value of the Phoenix discipline.

## §3 Decision

Adopt **per-service failure-mode declaration** as a required artifact for every load-bearing service.

### 3.1 Every load-bearing service declares one mode per dependency

For each dependency (named: database, peer service, config registry, audit substrate, NTP source, identity provider, Spock replication peer, Zenoh router, etc.), the service declares one of: fail-stale, fail-open, fail-closed, or fail-rebuild. A service with three dependencies has three declarations; the modes may differ per dependency.

The declaration is an engineering commitment: the service's code must implement the declared behavior, and the chaos probes (Block 38) test that it does.

### 3.2 Fail-rebuild requires Phoenix-native discipline

A service may declare fail-rebuild for a dependency only if that service satisfies the cruft test from ADR 0208 — its identity is supply-chain-rooted, its discovery is capability-based, its state is reconstructible from the substrate, and a clean rebuild of the service from supply chain is operationally possible without manual intervention.

If a service is not Phoenix-native, fail-rebuild is unavailable. Such services choose from the three standard modes only.

### 3.3 Declarations live in a single matrix

The matrix lives in `docs/design/threat-model.md` §"Failure-mode declaration matrix." It is a sibling to the existing controls-traceability matrix (Block 53), not a separate document. One row per (service, dependency) pair. Columns: service, dependency, declared mode, rationale, chaos-probe ID (Block 38).

A new service is not "load-bearing" until its rows appear in the matrix. The schema-enforcer interceptor (Block 23) flags PRs that add new services without matrix rows; the check warns until enough services are migrated, then blocks.

### 3.4 Initial scope: twelve named services

The matrix populates at v1 with rows for: netbird-mgmt, signal-server, schema-enforcer, db-policy-engine, ntp-health-agent, admit-audit, gatewatch, a demo game service, mgmt-host-bootstrap, goat-relay, audit-logger plugin, WASM module loader. These are the twelve services this ADR's first implementation pass commits to declaring.

Additional services land per-PR thereafter. The list is not closed.

### 3.5 Chaos-probe coverage is the verification mechanism

For every (service, dependency, mode) tuple in the matrix, Block 38 either has or commits to having a runnable probe that:

1. Induces the dependency unreachability.
2. Observes the service's behavior.
3. Asserts the declared mode is the observed mode.
4. Emits a structured event into Block 27 if observed ≠ declared.

A drift between declared and observed is a finding (F-NNN per ADR 1011) that triggers either fixing the code to match the declaration or updating the declaration to match the code, decided by the service owner.

### 3.6 Out of scope

- **Substrate failure modes.** Spock itself, Zenoh routers, Postgres, Caddy/Traefik are not "services" in this ADR's sense. Their failure behavior is governed by their upstream design and our deployment configuration, not by this declaration discipline. Block 38 covers them via separate substrate-invariant probes.
- **Compound dependency failures.** A service with three dependencies has three single-dependency declarations; what it does when two fail simultaneously is not in this matrix. Block 55 (combined-failure validation) covers compound-failure scenarios separately.

## §4 Alternatives considered

- **Status quo: implicit failure modes.** Rejected — the asymmetry between asking "what's our security posture?" (an answer exists) and "what's our degradation posture?" (no answer exists) is a real gap.
- **Per-service docstrings only.** Rejected — no central inspection surface; an assessor reads N source files instead of one matrix.
- **Only fail-rebuild for everything.** Rejected — too coarse; some services lose acceptable behavior (a service holding an audit log shouldn't rebuild on a transient registry blip).
- **Mode declared per request, not per dependency.** Rejected — granularity too high; matrix becomes unreadable. Per-dependency is the right balance for the audience the matrix serves (assessors, chaos-probe authors, new contributors).

## §5 Consequences

**Positive:**
- Single matrix for the whole degradation posture; readable in minutes.
- Chaos probes (Block 38) gain a canonical spec to test against.
- Fail-rebuild made first-class; reinforces and exercises Phoenix discipline (ADR 0208).
- New services join the discipline at PR time via the schema-enforcer flag check (warns now, blocks later).

**Negative / accepted trade-offs:**
- **Discipline cost at PR time.** Adding a new load-bearing service requires writing two to five matrix rows. Mitigated by a row template in the matrix doc.
- **Drift risk between declaration and code.** Mitigated by Block 38 probe coverage; finding-on-drift makes the gap visible.
- **Compound-failure declarations not in this matrix.** Block 55 covers the gap; cross-link in the matrix preamble points there.

## §6 Assessor view (SCA-R)

This ADR maps to NIST SP 800-171 rev 2 §3.6 (incident response, specifically 3.6.2 — track, document, and report incidents) and §3.13 (system and communications protection, specifically 3.13.5 — implement subnetworks for publicly accessible system components separated from internal networks). The matrix gives an assessor a single-table answer to "how does this system behave when a component fails?" — which is the question every assessment asks and most systems answer with prose.

The fail-rebuild mode is novel; an assessor reviewing it should also review ADR 0208 (Phoenix-native peer discipline) to confirm the cruft test is enforced in practice. Without that enforcement, fail-rebuild is a paper claim.

## §7 Trigger conditions for revisit

- **Three or more services discover, via Block 38 probes, that their declared mode does not match observed behavior within a 60-day window.** The declaration discipline is not catching the right cases; revisit.
- **A declared fail-stale mode causes harm because "stale" was longer than the rationale assumed.** Add a staleness-bound column to the matrix.
- **Fail-rebuild is invoked in production and a service does not return to healthy state within its declared rebuild SLA.** Phoenix discipline has a gap; revisit ADR 0208 or downgrade the service from fail-rebuild eligibility.
- **A new dependency class emerges that none of the four modes fits.** Extend the taxonomy; do not let services use ad-hoc "other" modes.

## References

- `docs/design/threat-model.md` — hosts the failure-mode declaration matrix.
- `docs/project/implementation-plan.md` Block 63B — execution plan.
- [Cloudflare, "Code Orange: Fail Small, Fail Complete," May 2026](https://blog.cloudflare.com/code-orange-fail-small-complete/) — prior art for the three-mode taxonomy this ADR extends.
- [ADR 0208](0208-phoenix-native-peer-discipline.md) — fail-rebuild availability depends on this.
- [ADR 1013](1013-continuous-architectural-chaos.md) — Block 38 chaos probes verify declared modes.
- [ADR 1011](1011-findings-register.md) — drift between declared and observed becomes an F-NNN finding.
- See GLOSSARY.md for acronym expansions used in this ADR.
