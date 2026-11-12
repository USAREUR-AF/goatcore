---
adr: 0112
title: Tech-strategy refinement — workload-shape language boundary (per-byte hot path vs per-request control)
status: Accepted
date: 2026-04-30
audiences:
  technical: how to pick Go or Rust for a new long-running service in this project
  layperson: why the relay rewrite is not a sign that we should rewrite everything in Rust
  scar: how the language boundary maps to assurance posture and supply-chain hygiene
repo_scope: Multi (project-wide)
glossary: docs/adr/GLOSSARY.md
---

# ADR 0112 — Tech-strategy refinement: workload-shape language boundary

## Status

Accepted.

## Context

The project's tech strategy
(`../design/tech-strategy.md` §"Language
Selection") established a Go / Rust split, refined 2026-04-24:

> 1. If it's the data path (transport, conditioning, validation,
>    interceptors, Zenoh-adjacent) and is a long-running daemon — **Rust.**
> 2. If it's a control-plane HTTP service or CLI — **Go.**

This split has worked. Existing Go services (signal-server, gatewatch,
db-policy-engine, ntp-health-agent, admit-audit, mgmt-host-bootstrap)
are healthy at their workload shapes. Existing Rust work
(zenoh-plugins, audit-logger interceptor, schema-enforcer, the
in-flight Block 33 relay rewrite) is healthy.

In 2026-04 the relay rewrite (Block 33,
[ADR 0709](0709-block-33-relay-rewrite-architecture.md)) surfaced
F-017 — the Go
runtime is the structural cap on the netbird-relay's per-byte
throughput, not CPU. This led to a strategy question: does F-017
invalidate the broader Go boundary? Should we be migrating other
services to Rust?

## Decision

**The strategy as written is correct. F-017 confirms the split rather
than invalidating it. We refine the rule from "data-path adjacency"
to a sharper test: workload shape.**

The relay was the only true *data-plane* Go service in the project —
sustained per-packet I/O, multi-Mbps per connection, thousands of
concurrent connections — exactly the regime in which Go's runtime
overhead matters. Every other Go service in the system is
request-shaped: low-to-moderate qps, sub-millisecond GC pauses
invisible at that scale, broad library needs (HTTP / DB / SSH / AWS),
fast iteration prized. F-017 does not lurk in those services — they
are not the regime.

### Refined decision test

| Workload shape | Default | Rationale |
|---|---|---|
| **Per-byte / per-packet hot path, sustained throughput, predictable-memory matters** | **Rust** | F-017 regime — Go runtime caps you |
| **Sidecar / interceptor / shim *on the data path* (even if logically thin)** | **Rust** | Lives inside the Zenoh process or relay process; ABI + memory profile matters; matches Zenoh's Rust-native interceptor pattern |
| **Per-request control service**, qps measured in single-to-low-hundreds, broad library needs | **Go** | Iteration speed, ecosystem breadth, deployment simplicity dominate |
| **One-shot CLI / Ansible-callable binary** | **Go** | Static binary, fast to write, no async ceremony |
| Browser UI | **Plain HTML + JavaScript, no build step**; TypeScript for a frontend too complex for that | [engineering-baseline §12.4](https://github.com/USAREUR-AF/goatdocs/blob/main/docs/design/engineering-baseline.md) sets the rule and its CI check |
| Ansible / scripts / ad-hoc data work | **Python** | unchanged |
| Infra / config | **Terraform + Ansible** | unchanged |

### What changes from the prior wording

The prior rule used adjacency framing — "data path / Zenoh-adjacent
→ Rust." That works in practice but admits a fuzzy edge case: a
service like the chaos peer (Block 38) lives in the fabric, declares
schemas, emits Zenoh, and could plausibly be read as "Zenoh-adjacent
→ Rust." Workload-shape framing is unambiguous: chaos is request-
shaped (probes fire on a schedule, return verdicts), so it is Go.

The shift is **not** a re-categorization. It is a sharpening of the
rule so that future contributors do not have to argue about whether
something is "data path adjacent." The test now is:

> "Does this service spend most of its CPU time in a per-byte
> sustained hot path?" Yes → Rust. No → Go (unless it's UI / glue /
> infra, in which case use the rest of the table).

### Three forward-looking masterstrokes — language assignments

The three concurrent design proposals
(Block 36 time-quality,
Block 37 replay,
Block 38 chaos) get language
assignments under this rule:

| Block | Service | Workload shape | Language |
|---|---|---|---|
| 36 | Time-quality propagation (schema-enforcer extension + producer SDK helpers + lineage attribute) | On-data-path interceptor | **Rust** (extends existing zenoh-plugins) |
| 37 | Replay substrate (`goat-replay`) | Stream reconstruction emitting Zenoh sessions | **Rust** (data-path-adjacent; shares OpenObserve-client / audit-client primitives with existing Rust interceptors) |
| 38 | Chaos peer (`goat-chaos`) | Periodic probes, broad library needs | **Go** |

## Consequences

### Positive

- **Predictable boundary going forward.** New services have an unambiguous language assignment.
- **F-017 contained.** Block 33 absorbs the data-plane Rust complexity at the place where it pays off most. The team does not have to maintain Rust expertise across every service.
- **Vendor handoff alignment.** The vendor-handoff plan (`../project/vendor-handoff-pickup-plan.md`) names the relay as a candidate for handoff to a more-resourced org. Concentrating the deepest Rust work at handoff candidates is consistent with the small-team-with-leverage posture.
- **Iteration speed preserved.** Control-plane Go services keep the iteration speed that has carried the project this far.

### Negative

- **Two-language ecosystem.** The team maintains Go + Rust toolchains, supply chains, CI matrices. Already true today; no change.
- **Bilingual interfaces.** Some services span both (e.g., the schema-enforcer has Rust core + Go SDK helpers for Go-language producers). Already true; no change.
- **Edge cases will still surface.** A future service that mixes shapes (e.g., a Go HTTP API in front of a per-byte data-path component) requires judgment. The rule does not eliminate judgment — it narrows where judgment is needed.

### Neutral

- **No retroactive rewrites.** Existing Go services stay Go. Existing Rust services stay Rust. Migration triggers are workload-shape changes (a service that was request-shaped becoming data-shaped, the way the relay's load profile changed in production), not language preference.
- **Python / Terraform / Ansible unchanged.** This ADR refines only the Go / Rust boundary. The browser-UI row follows [engineering-baseline §12.4](https://github.com/USAREUR-AF/goatdocs/blob/main/docs/design/engineering-baseline.md).

## Alternatives considered

### A. Migrate everything to Rust

Rejected. Most services are not in the F-017 regime; the iteration-speed cost of Rust for control-plane HTTP work would be large with no payoff. The team's Rust expertise is concentrated where it matters most (Block 33); spreading it thin would dilute that.

### B. Stay with the prior wording and tolerate the fuzzy edge

Rejected (mildly). The prior wording works for current contributors who built the system. New contributors and vendor handoff recipients benefit from a sharper rule. Sharpening is cheap; tolerating ambiguity costs every future judgment call.

### C. Pick by service-author preference

Rejected. The project's posture (small team, long-lived systems, supply-chain discipline, vendor handoff) makes the cost of fragmentation higher than the cost of a single rule. Author preference matters in the small (style, idioms within a language) but not in the large (which language).

### D. Re-categorize control-plane Go services as candidates for Rust

Rejected. F-017 is workload-shape-specific; control-plane services do not exhibit the Go-runtime overhead pattern that surfaced in the relay. Rewriting on speculation would burn engineering time on a non-problem.

## Implementation tracker

This ADR is a strategy refinement, not a build effort. Implementation
is the §-update to `../design/tech-strategy.md`
in the same change as this ADR (line numbers may drift — search for
"Decision test" in the Language Selection section).

The three masterstrokes that this ADR's language assignments shape
are tracked in:

- Block 36 — time-quality attestation
- Block 37 — replay substrate
- Block 38 — continuous architectural chaos

## Cross-refs

- `../design/tech-strategy.md` §"Language Selection" — the document this ADR refines
- ADR [`0709`](0709-block-33-relay-rewrite-architecture.md) — Block 33 architecture, the F-017 response
- F-017 — the trigger finding
- ADR [`0501`](0501-rust-interceptors.md) — Rust interceptors as integration extension point (still stands)
- `../project/vendor-handoff-pickup-plan.md` — handoff candidate framing

## Glossary pointer

See `GLOSSARY.md` for project acronyms used in this ADR.
