# ADR 0011 — `[multi-agent-orchestration]` Cluster worker execution environment — git worktrees v1, cloud sandbox queued

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-05-05 |
| Repo scope | Multi (every repo running multi-agent clusters) |
| Supersedes | — |
| Superseded by | — |
| Related | [`0006-multi-agent-human-concurrency-governance.md`](0006-multi-agent-human-concurrency-governance.md), [`0007-multi-agent-orchestration-hierarchy.md`](0007-multi-agent-orchestration-hierarchy.md), [`0008` (Backlog)](INDEX.md), [`0109-git-collaboration-trunk-based.md`](0109-git-collaboration-trunk-based.md), [`../design/cluster-worker-execution-environment.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/cluster-worker-execution-environment.md), [`../design/multi-agent-team-management.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/multi-agent-team-management.md) |

## TL;DR (for a layperson)

When the substrate spawns a captain agent and three worker agents to build something in parallel, all four agents need their own copy of the source code so they don't trip over each other's edits. The cheapest way to give them that today is a feature of git called "worktrees" — each agent gets a separate folder on the same laptop that points at the same git history but holds a different in-progress branch. This lets us run real per-worker pull requests without buying any cloud infrastructure, and it stays available even when we eventually move long-running agents to a cloud machine. The cloud-machine option is queued for later; worktrees are the day-1 substrate.

## Context

[ADR 0006](0006-multi-agent-human-concurrency-governance.md) + [0007](0007-multi-agent-orchestration-hierarchy.md) + [ADR 0008 (Backlog)](INDEX.md) describe the substrate's shape — captain + worker hierarchy, per-track branches, cluster-integration patterns A and B, AI PR review layering. [ADR 0109](0109-git-collaboration-trunk-based.md) requires PR-gated merges to `main` once the >3-active-contributor threshold trips. Block 66 operationalizes branch-protection readiness.

What none of those decisions name is **where the worker agents physically execute**. Today the operator runs Claude Code inside VS Code; the working tree is a single checkout on `main`; there is no second checkout. If a captain spawns three worker agents in this session, all four share the same `.git/HEAD`. Pattern A becomes single-threaded by working-tree contention; Pattern B is unusable (no second checkout to integrate into); the PR-gated branch protection becomes advisory because nothing actually opens a PR — workers commit straight to `main` because that's the only branch checked out.

This is a real prerequisite to firing the Block 48D airdefenders-epic-wrath pilot. The kickoff memo §5 worker-spawn-plan claims "3 workers in parallel" without operational detail on isolation.

## Decision

Adopt git worktrees as the substrate's worker-execution-environment v1, available for any cluster captain at any time. Plan for a long-running cloud execution sandbox (Option III) as a deferred upgrade, queued under Block 48M with explicit trigger conditions.

**Three options enumerated, decision per option:**

| Option | Status | Use case |
|---|---|---|
| **(I) Local git worktrees** | **Adopted v1; always-available** | Default substrate for captain + worker isolation; works for clusters of up to ~3-4 concurrent workers on a developer laptop |
| **(II) GitHub Actions runner** | Named, deferred | Scale-out path when laptop CPU/memory ceiling becomes binding (likely Phase-3 push at 6-10 concurrent workers); not v1 because API-key custody + cost-control + ergonomics need their own design pass |
| **(III) Long-running cloud execution sandbox** | **Queued; planned migration** | Persistent agent execution — workers run while operator's laptop sleeps; closer to "how Cloudflare runs Codex"; planned under Block 48M |

**Operational shape for Option I:**

1. **Worktree provisioning per track.** Captain runs `git worktree add ../worktrees/<track-name> -b track/<cluster>/<sub-block>-<role>` to create an isolated checkout per worker.
2. **Worker session spawned in worktree.** Worker is a Claude Code SDK session (or sub-agent invocation) whose `cwd` is the worktree path. Worker commits to its own branch.
3. **PR opened from worker.** Worker (or captain on worker's behalf) runs `gh pr create` against `main` (or `cluster/<name>` for Pattern B). PR is real; CI runs; reviewer gates apply.
4. **Captain integration.** Captain reviews + merges (or queues via merge queue once enabled). Worktree is removed via `git worktree remove ../worktrees/<track-name>` after PR merge.
5. **Cleanup discipline.** Stale worktrees are an operational burden; `git worktree prune` runs as part of `work cluster status` rollup.

The interactive operator session in VS Code stays on `main` (the captain's "default" working tree); workers run in `../worktrees/*` siblings. Captain can switch between worktrees freely; operator's mainline checkout is never blocked by worker activity.

**Concurrency ceiling for Option I**: ~3-4 concurrent workers per laptop is the soft ceiling. Beyond that, Option II or III is required. The exact ceiling is operator-machine-specific; not a substrate property.

**PR-gate enforcement during pilots**: airdefenders-epic-wrath pilot (and subsequent pilots) MUST flip branch protection on `main` for the duration of the pilot per [ADR 0109](0109-git-collaboration-trunk-based.md) trigger conditions. Otherwise the substrate test isn't testing real PR-shape.

Full mechanics live in the design doc at [`../design/cluster-worker-execution-environment.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/cluster-worker-execution-environment.md). Operator runbook lives at [`../operations/cluster-worker-worktree-runbook.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/operations/cluster-worker-worktree-runbook.md).

## Alternatives considered

- **Single shared working tree (today's state).** Rejected — PRs are advisory, Pattern B unusable, branch protection meaningless. Substrate test isn't testing the substrate.
- **Multiple independent local clones (`git clone` per worker).** Rejected — worktrees give the same isolation with shared `.git` (cheaper, faster, less disk); per-clone has no advantage at this scale.
- **GitHub Actions as v1 (Option II).** Rejected for v1 — API-key custody (Anthropic key in repo secrets or self-hosted-runner env), cost-control (Claude API spend × N workers + runner minutes), and debugging ergonomics worse than local-worktree make this premature. Adopt later when laptop ceiling binds.
- **Cloud execution sandbox as v1 (Option III).** Rejected for v1 — IAM provisioning + persistent state + operational substrate maturity costs more than worktrees deliver. Defer until Options I + II reveal what they can't handle.
- **Devcontainer / VS Code Remote Containers per worker.** Considered; deferred. A devcontainer per worker IS a way to layer Option I — useful if workers need different toolchains. Not load-bearing at v1; revisit if a worker's environment requirements diverge from main.

## Consequences

**Positive.**
- Pilot can fire without buying infrastructure.
- PRs become real (per worker, per branch) — Block 66 readiness work isn't wasted.
- Pattern B (cluster-integration branch) becomes usable.
- Captain can switch worktrees without disturbing operator's mainline checkout.
- No vendor lock-in; worktrees are git stdlib.
- The path to Option III is purely additive (worktrees stay available even when the cloud sandbox lands).

**Negative.**
- Laptop CPU + memory ceiling for parallelism (~3-4 concurrent workers).
- Long-running workers compete with operator's interactive session.
- If laptop sleeps, work pauses. Not a fit for overnight execution.
- Operator-to-worktree manual provisioning is the load-bearing convention until automated by `work` CLI.

**Residual risk.**
- Stale worktrees accumulating (operational burden) — mitigated by `work cluster status` checks + `git worktree prune` discipline.
- Worker writes to a shared resource that's not under git (e.g., a dev database) — out of scope for this ADR; covered in design doc §"Shared-state hazards."
- Operator unintentionally edits a worktree directory thinking it's mainline — UI accident; mitigated by directory naming convention (`worktrees/*`) + shell prompt.

## Assessor view (SCA-R)

This ADR sits in **CM-2** (baseline configuration — git-worktree convention is part of the multi-agent baseline), **CM-7** (least functionality — workers are confined to their worktree directory), and **AC-3** (access enforcement — PR-gated merges to `main` are the access-enforcement mechanism for the substrate's mainline). No CMVP-relevant crypto change. No data-handling change.

## Trigger conditions for revisit

- Laptop concurrency ceiling becomes binding (operator observes >3-4 concurrent workers thrashing) → upgrade to Option II or III.
- Workers need to run during operator's off-hours → Option III becomes load-bearing; activate Block 48M.
- Phase-3 push at 6-10 concurrent workers across multiple clusters → Option II at minimum, likely Option III.
- A worker's toolchain requirements diverge enough from mainline that a separate environment is cleaner than per-worker venv/asdf hacks → adopt devcontainer-per-worktree pattern.
- Stale-worktree operational burden grows past what `work cluster status` checks handle → automate worktree lifecycle in the `work` CLI.

---

*See `GLOSSARY.md` for terms and acronyms.*
