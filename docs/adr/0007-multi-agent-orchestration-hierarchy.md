# ADR 0007 — Hierarchical orchestration for 6-10 parallel agent threads (program / cluster-captain / worker tiers)

| Field | Value |
|---|---|
| Status | Proposed (2026-05-01) |
| Date | 2026-05-01 |
| Repo scope | Multi |
| Sub-series | `[multi-agent-orchestration]` |
| Supersedes | — |
| Superseded by | — |
| Related | [`0006-multi-agent-human-concurrency-governance.md`](0006-multi-agent-human-concurrency-governance.md), `0008-cluster-branching-and-ai-review-tooling.md`, `0009-worker-discipline-mechanisms.md`, [`0010-boundary-classification-escalation-and-medium.md`](0010-boundary-classification-escalation-and-medium.md), [`0109-git-collaboration-trunk-based.md`](0109-git-collaboration-trunk-based.md) |

## TL;DR (for a layperson)

When two or three AI agents work alongside a human on the same project, the existing rules — branches, pull requests, and a shared "who's working on what" file (ADR 0006) — are enough. When the project tries to run six to ten agents in parallel to compress two months of work into eight weeks, those rules stop being enough on their own. Coordination overhead and silent disagreements between agents become the bottleneck, not how fast each agent can write code. **We add a hierarchy on top of the existing rules.** A single human stays in charge of the program. Each themed cluster of work (security improvements, identity overhaul, video infrastructure) gets a captain — sometimes another agent, sometimes a human — that owns the cluster and decomposes it into workable pieces. Each piece is assigned to a worker agent that does the actual writing. The captain integrates the workers' output and reports up; the human at the top makes the strategic calls. Three tiers, no deeper. Just enough structure to scale to ten threads without anyone stepping on anyone else.

## Context

The Phase-3 push (see `../project/implementation-plan.md` §"Two-month execution cadence") compresses critical production hardening + FIPS compliance + Ory IdP wire-up + video fabric + Zenoh feature maturity + the eight security-ascent blocks into two calendar months. Realistic execution requires 6-10 concurrent threads against a shared trunk.

[ADR 0006](0006-multi-agent-human-concurrency-governance.md) added a thin governance layer above git for 2-3 concurrent threads. It works in practice — verified mid-session 2026-05-01 by `commit ea02085` (goat-relay parallel track) landing during this substrate-authoring work without collision. The pattern fails to scale to 6-10 threads in three observable ways:

1. **No hierarchical decomposition.** Every agent is peer-level under the human. With 10 threads, the human becomes the bottleneck (the manager-of-Devins ceiling Cognition's [2025 Devin annual review](https://cognition.ai/blog/devin-annual-performance-review-2025) reports — captain capacity ceiling at 4 concurrent worker-PRs per human reviewer).
2. **No interface contracts between clusters.** When Block 33 relay shares `goat-relay-protocol` with security-ascent's Block 40 reproducible builds, the coupling is implicit. Two threads can produce conflicting designs at the seam.
3. **No decision-deduplication.** Two agents can independently raise the same architectural question, draft different ADRs, and land them both. ADR sub-series tags help discoverability but do not prevent parallel drafting.

The Stanford April 2026 ([arXiv 2604.02460](https://arxiv.org/abs/2604.02460)) result is load-bearing for *what* we orchestrate: multi-agent only beats single-agent for *parallel execution on distinct work*, not for redundantly re-thinking single hard questions. The full SOTA grounding lives at `../research/agent-team-management-sota-2026-05.md`.

## Decision

Adopt the **orchestrator-workers pattern** layered on top of the existing constitutional shared substrate (the project instruction file loaded by every captain and worker as the project constitution).

**Three tiers, capped:**

1. **Program orchestrator** — single entity (human + assistant pair). Owns: cross-cluster prioritization, decision-dedup arbitration, daily status review, scope-shape calls. Does NOT write code in clusters it's orchestrating.
2. **Cluster captain** — one per Block-cluster (or per ADR sub-series). Agent or human. Owns: decomposing the cluster into tracks, spawning workers, integrating outputs, surfacing escalations.
3. **Worker** — one per active track (sub-block). Agent or human. Owns: a single block of executable work end-to-end.

**Strict three-tier ceiling.** Workers do not spawn sub-workers. Three tiers is the production-validated nesting cap per Claude Code's published guidance — "shallow nesting (2-3 levels max) to avoid token blowup." A worker that needs help returns to its captain.

**Interface contracts between clusters.** Each cluster authors a contract per [`../design/multi-agent-team-management.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/multi-agent-team-management.md) §3.1 naming: outputs (named artifacts), inputs (named artifacts from upstream), verification (how downstream confirms upstream is ready), versioning (how breaking changes signal). Lives in `../project/active-work.yaml` under `cluster_contracts:`.

**Decision-deduplication via narrow multi-agent debate.** When two captains independently raise the same architectural question, debate-then-judge resolves: each sketches a candidate design; program orchestrator (or debate-judge agent) picks one, OR directs synthesis. Hard rule: an ADR number cannot be claimed by two threads — pending-decisions queue is the dedup layer. Multi-agent debate is **not** used for execution, only for decision-dedup, per the M3MAD-Bench finding (debate plateaus + can be subverted by misleading consensus).

**Role specialization via prompt registry.** Six v1 roles at `ops/work/roles/`: cluster-captain, researcher, planner, implementer, reviewer, integrator. Each role's prompt inherits CLAUDE.md by reference (constitution); role layers specialization on top. Captaincy template at `ops/work/roles/captaincy-template.md` is constant; per-cluster kickoff memo at `../project/cluster-kickoffs/<cluster-name>.md` is filled per spawn from `TEMPLATE.md`.

**Pattern stability hedge.** Codify *patterns* in this ADR + the design doc + tooling extensions to `ops/work/`, NOT framework adoption. The AutoGen → AG2 / Microsoft Agent Framework split (3-way fork in 18 months) is the cautionary tale; Claude Agent SDK is our harness, but if it forks the substrate's *patterns* survive.

## Alternatives considered

- **Microsoft Agent Framework graph workflows.** Production-grade, but adopting MS Agent Framework as substrate would require alien runtime + framework lock-in. We lift the *patterns* (Magentic-One planner-with-ledger, explicit graph-based workflow) without the runtime.
- **AutoGen / AG2 GroupChatManager.** Mid-fork stability concern; the 2024 split between original team (now AG2) and Microsoft (now Agent Framework) is exactly the cautionary tale.
- **LangGraph supervisor + state-graph.** Tool-call-pattern (LangChain's current recommendation over the supervisor library) is good; the runtime + reducer state-graph is a different shape than goat's git-as-state.
- **OpenAI Agents SDK handoff.** Chained-conversation model differs from goat's parallel-decomposed-execution model. Handoff *pattern* is useful at month-1 → month-2 cluster-captain rotation; not the routine within-cluster execution path.
- **Flat peer-level agents (no hierarchy).** Today's ADR 0006 default. Doesn't scale past ~3 threads; manager bottleneck cited above.
- **Deeper hierarchy (4+ tiers).** Rejected per Claude Code published guidance + token-blowup risk + observed inconsistent-activation anti-pattern (workers ignoring sub-sub-spawned agents).

## Consequences

**Positive.** 6-10 concurrent threads become coordinable through a single human program orchestrator at ~30-60 min/day review cadence. Three tiers map cleanly onto a three-tier git topology (`main` / `cluster/<name>` / `track/<...>` per ADR 0008). Substrate is framework-portable — if Claude Agent SDK changes, patterns survive. Composes with existing primitives (ADR 0006 pickup-track-release; Block-clustering pattern; ADR sub-series tags) rather than replacing them.

**Negative.** Captain-context bloat is a known failure mode (Cognition's published 2× efficiency factor was *isolate context, hand off final-form deliverables*). Mitigation in [ADR 0009](0010-boundary-classification-escalation-and-medium.md) worker-discipline mechanisms (context-firewall principle); pilot-validated in Block 48D. Operator (human program orchestrator) capacity bottleneck is real — staged rollout (3-4 clusters in month 1, ramp to full in month 2) hedges. Substrate authoring + tooling extensions are ~9-11 days of upfront work before any cluster execution; that gating is by design (per the substrate-spike rationale) but the time is real.

**Residual risk.** Pattern fitness in real-world conditions hasn't been demonstrated yet; Block 48D pilot is the first test. Failure mode would be 48D revealing the substrate doesn't quite fit, requiring an additional ~3-day refinement loop before scale-out. Captured in `../project/implementation-plan.md` Block 48E.

## Assessor view (SCA-R)

This ADR documents the orchestration substrate for an AI-augmented development workflow. It does not establish new system controls; it defines the human-and-agent operational pattern for *building* the system. **Control-family relevance:**

- **CM-3 (Configuration Change Control).** The three-tier hierarchy + `[track:]` commit trailer + cluster integration branches per ADR 0008 provide auditable change attribution at agent + human granularity.
- **AT-2 (Awareness training) / AT-3 (Role-based training).** The role-prompt registry (`ops/work/roles/`) is the operational analog of role-based training for agent participants; each role's prompt is the load-bearing artifact specifying acceptable behavior for that role.
- **PM-32 (Risk framing).** Decision-dedup protocol prevents architectural-decision drift across parallel threads; pending-decisions queue is the auditable artifact of "what was open, what was resolved, by whom."

Every agent action results in a `[track:]`-attributed commit on a signed-commit-required branch. Residual risk: the "agent acted outside its role-prompt" failure mode is mitigated by reviewer-role gates on every PR + Block 48I worker-discipline mechanisms; not eliminated. Compensating control is the human program orchestrator's daily review.

## Trigger conditions for revisit

- Block 48D pilot reveals the three-tier hierarchy doesn't fit (specifically: workers consistently need to spawn sub-workers; or captains can't manage 3+ concurrent workers without context bloat).
- Post-pilot tooling-decision evidence (CodeRabbit-vs-Copilot per ADR 0008, captain-intervention-effectiveness per ADR 0009) shifts the substrate's operational shape.
- Claude Agent SDK ships a substantively different orchestration primitive that obsoletes patterns in this ADR; or Anthropic publishes guidance contradicting the orchestrator-workers shape.
- Project participant count grows past ~15 contributors (multi-program-orchestrator becomes the question); or shrinks back to ≤2 contributors (substrate simplification becomes the question, fall back to ADR 0006 alone).
- Concurrent-thread count consistently exceeds 12 in production (the substrate's tested ceiling per Block 48D); or stays consistently below 4 (the substrate's break-even point — ADR 0006 alone may suffice).
- Stanford April 2026 single-agent-superior result is empirically refuted at goat's scale and shape; or extended to also cover parallel-execution work (in which case multi-agent itself becomes the question).

## Glossary

See `GLOSSARY.md` for project-specific terms (`multi-agent-orchestration` sub-series tag; cluster kickoff memo; `Mainline tenacity`; `Fast/slow inchworm`).

## Status flip criterion

Proposed → Accepted on three signals:

1. Block 48D pilot completes with green verdict (a demo-application cluster captain runs end-to-end via 2-3 worker agents — re-targeted from security-ascent 2026-05-05; pilot deliverables = 6 schema migrations + repset config + 6 JSON Schemas + 2 INTERFACE.md docs drafted by the captain + workers without human authoring intervention beyond review + accept; substrate-touch discipline holds with substrate edits accounted for in kickoff memo §11 decision log).
2. Block 48E refinement pass incorporates pilot lessons; design doc + this ADR updated with concrete observations.
3. At least one second cluster-captain pilot (any of Block 33C / Block 21 / Block 27) succeeds against refined substrate without further pattern changes.
