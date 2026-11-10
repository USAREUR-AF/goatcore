# ADR 0839 — `[multi-agent-orchestration]` Continuous-handoff baton — commit-message-as-baton + `/handoff` skill + state-loss-coincident hook triggers

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-05-09 |
| Repo scope | Multi (every repo using Claude Code with multi-session work or quota-bounded accounts) |
| Supersedes | — |
| Superseded by | — |
| Related | [`0006-multi-agent-human-concurrency-governance.md`](0006-multi-agent-human-concurrency-governance.md), [`0017-claude-code-session-isolation-pilot.md`](0017-claude-code-session-isolation-pilot.md), [`../design/multi-agent-team-management.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/multi-agent-team-management.md) §14 |

## TL;DR (for a layperson)

When a long task spans multiple Claude Code sessions — the account hits its weekly cap, the laptop reboots, the operator clears context, an autonomous overnight run halts on a permission boundary — the file system survives but the *in-head* state (working theory, what was tried, what's next) is gone. We're adding a lightweight pattern: every commit message carries enough context to onboard a fresh agent (already standard discipline; minor reinforcement), plus an explicit `/handoff` skill that captures uncommitted in-flight state into a `wip(track): handoff baton` commit, plus three hook triggers that fire `/handoff` at the moments where state is about to be lost anyway. No always-on artifact, no daemon polling, no quota signal needed. Cost is one extra commit per natural pause point; benefit is a 5-10 minute cold-start saving every time a new session has to pick up.

## Context

### The gap

The session-isolation pilot ([ADR 0017](0017-claude-code-session-isolation-pilot.md), Block 48O) gave us **byte-level continuity** across sessions:

- Worktree on disk → survives.
- Branch + commits with `[track:]` trailers → survives.
- `active-work.yaml` track row (status + scope + eta + note) → survives.
- Memory files at `~/.claude/projects/<repo>/memory/` → path-bound, survives across accounts on the same workstation.
- Session bus JSONL → survives.
- `/iso resume <name>` → re-enters the right worktree.

What does NOT survive a session boundary (account-rollover, crash, `/clear`, weekly-cap shutdown, autonomous-worker permission-halt):

- TodoWrite list (in-conversation only)
- Plan-mode plan content (in-conversation only)
- Working hypothesis ("I think the cause is X")
- Ruled-out paths ("Already tried Y; failed because Z")
- "Stuck on" current blocker
- Intent behind uncommitted diffs (the diff itself survives; the *why* doesn't)

Today this gap is closed by an operator manually re-briefing the new session in 5–10 min. That's the bottom-tier fallback. We can do better without paying always-on cost.

### Why not a quota-aware handoff?

Claude Code does not expose any programmatic signal for daily / weekly / monthly subscription-quota approach. No CLI command, no env var, no file at `~/.claude/`, no hook event named after quota state. The harness shows a UI banner near the cap; the agent doesn't see it.

We considered building a "trigger handoff at 95% of cap" mechanism. Not buildable today. If Anthropic ever exposes a quota hook, it slots into this pattern as an additional trigger without changing the rest.

### Why not always-on artifact?

Cost-benefit: handoffs are rare (~1–2 per quarter under normal use). Saving 10 min × 1–2 events = 10–20 min/quarter benefit. Maintaining a file artifact updated on every commit (~200 commits/quarter) at 30s each = 100 min/quarter cost. **5–10× net negative.**

The only artifact pattern that pays: one fired at natural state-loss moments, not per tool-use.

### Why "state-loss-coincident" triggers work even without a quota signal

The events where in-head state is *actually* lost have substantial overlap with quota approaches:

- **Context compaction (auto)** — Claude Code fires `PreCompact` with `matcher=auto` when the context window approaches its limit. That's not a subscription-quota signal, but it IS a state-loss signal (post-compaction the agent operates on a summary, not the full transcript). Long sessions trigger it; long sessions are also the ones most likely to bump into weekly caps.
- **Session end** — `SessionEnd` fires on every session termination, including ones triggered by quota-hit shutdowns.
- **Stop with WIP** — every assistant-response end is a candidate handoff point. Most are no-ops (no WIP). The ones that aren't catch the residual cases.

These triggers don't *know* they're correlated with quota; they just fire when state is about to be lost regardless of cause. Quota-driven shutdown is one cause among several (laptop reboot, manual `/clear`, autonomous-worker halt, accidental session close); the trigger set covers all of them uniformly.

## Decision

Adopt **five composing pieces** as a self-contained pattern. Each independently revertable.

### 1. Commit-message-as-baton (zero-cost layer)

Every commit body on a tracked branch must contain enough handoff context that a fresh agent can resume without re-deriving:

- **Why** (motivation in 1–2 sentences — what problem is this commit attacking)
- **What changed** (already required by Conventional Commits / good hygiene)
- **What's next** (1–2 sentences — what the agent intends as the next step on this track; can be "this completes the track" if true)

This is mostly already-existing discipline — `[track:]` trailer is mandatory per ADR 0006, conventional-commits format per ADR 0109. The minor reinforcement is the **what's next** line, currently optional. Pre-commit hook gains a soft warning (not block) when the body is empty or omits a clear forward pointer on a tracked branch.

Cost: roughly zero — disciplined commit messages already carry this content. The marginal effort is naming it as a discipline, not adding new mechanism.

### 2. `/handoff` skill (`.claude/skills/handoff/SKILL.md`)

Project-installed under `.claude/skills/handoff/` so every Claude Code session that opens the repo gets `/handoff` automatically — no per-user install, available to any agent on first use.

Operator/agent-invokable skill that captures uncommitted in-flight state into a structured `wip(<track>): handoff baton — <one-liner>` commit.

Recipe:

1. Detect the active track (from current branch `track/<name>` or active-work.yaml owned-by-this-session row).
2. If working tree is clean and last commit body already carries handoff context → no-op (emit "no handoff needed; last commit is fresh") and exit.
3. If WIP exists → stage all files, compose a commit with body containing:
   - Current TodoWrite list (markdown checkbox form)
   - Active hypothesis ("currently working: …")
   - Ruled-out paths ("already tried: A failed because B")
   - Next concrete step ("next: …")
   - Files mid-edit + their state
4. Push the branch (no-op on master mode by default).
5. Update active-work.yaml track `note` field with `handoff at <commit-sha> @ <timestamp>` via `work update <track> --note "..."`.
6. Emit `HANDOFF_COMPLETE=<commit-sha>` directive line.

The skill itself is the recipe; the agent executes it (reading its own TodoWrite list and active hypothesis from session state).

### 3. PreCompact hook trigger (`.claude/hooks/precompact-handoff.sh`)

`matcher=auto`: fires when Claude Code is about to auto-compact context. This is the strongest available "state about to be summarized away" signal.

Hook emits `additionalContext` text instructing the agent: *if you have an active track and uncommitted work, run `/handoff` BEFORE compaction proceeds.* The agent decides whether to fire the skill or no-op (clean tree, no track, etc.).

`matcher=manual` is also handled — same emit, lower priority since user-invoked compaction is usually deliberate.

### 4. SessionEnd hook trigger (`.claude/hooks/session-end-handoff.sh`)

Last-chance write before the session terminates. Same pattern: emit the nudge, agent decides.

This is the only trigger that catches **abrupt** session ends (quota-hit shutdown, IDE crash, `/quit`). Other triggers fire mid-session.

### 5. Stop hook with throttle + WIP-check (reuses existing Stop hook)

Adds to the existing `.claude/settings.json` Stop hook: emit a handoff nudge ONLY when:

- An active track is owned by this session (check `work list --owner $(work whoami)` for active rows), AND
- `git diff --quiet` is non-zero (WIP exists), AND
- A throttle file at `.claude/handoff/<track>.last` is older than N minutes (default 30) — prevents per-response chatter on a series of quick commits.

Cheap when nothing's happening. Catches steady-state drift between explicit `/handoff` invocations.

### Self-discipline memory rule

Memory hook (`feedback_handoff_at_pause_points.md` (`ops/scripts/feedback-memory-hooks/`)) instructs agents to fire `/handoff` proactively at natural pause points:

- Before any long-running background task that might outlast the session
- Before `/clear` or `/iso exit`
- Before plan-mode exit when leaving WIP
- Any "I think I'm done for now / handing back to the operator" moment

This is the operator-zero-touch path: agent self-fires without waiting for a hook nudge.

## Alternatives considered

### Always-on per-tool-use artifact

Cost analysis above. 5–10× net negative. Rejected on cost-benefit grounds, not on technical feasibility.

### Separate file artifact at `.claude/handoff/<track>.md`

Cleaner discovery surface than `git log --grep`, but introduces a second artifact that drifts independently from commit history. Rejected — commit history is canonical, file would be redundant. (One-line throttle marker file at `.claude/handoff/<track>.last` is fine; it's a counter, not state.)

### Time-boundary heuristic (T+4.5h handoff before 5-hour rolling window resets)

Brittle (assumes session-start time alignment with quota window, which isn't guaranteed); doesn't help with the weekly cap; doesn't help with any non-quota state-loss event. Rejected as a primary trigger; available as an out-of-band cron a paranoid operator could add without changes to this design.

### Wait for Anthropic to expose a quota signal

The pattern needs to work today. If/when a quota hook arrives, it's an additional trigger; the pattern itself is unchanged.

### Operator-only manual `/handoff`

Discussed in conversation. Bottom-tier reliable fallback (always available), but leaves the org35x unused if the operator isn't watching. The hook triggers fire on operator's behalf without depending on operator vigilance.

## Consequences

### Acquired

- Cross-session resume time drops from ~5–10 min (operator re-brief) to ~30s–2 min (read latest handoff commit + diff).
- Account-rollover scenario (described in originating conversation 2026-05-09) becomes a non-event — new account picks up where old one left off via git history alone.
- Crash / accidental `/clear` / autonomous-worker halt all use the same recovery surface uniformly.
- `wip(track):` commits stay on per-track branches (per ADR 0017 default) and get squashed at merge — no permanent pollution of `main` history.

### Risk

- **Stop hook chatter.** If throttle is too short, agents waste cycles on no-op handoffs. Default 30 min should be safe; revisit if telemetry shows otherwise.
- **Master-mode WIP commits land on main.** When an operator deliberately works on master (per ADR 0017 master-mode), `wip(track):` commits stay in main history rather than getting squashed. Acceptable trade-off — master-mode is a deliberate choice, the audit trail is a feature, and master-mode work should be small/short anyway.
- **Hook nudges depend on the agent acting on `additionalContext`.** Same dependency as every other hook in our settings; not a new risk class. Memory rule + role-prompt registry reinforces the discipline.
- **TodoWrite state capture is by agent introspection, not direct API.** The agent has to look at its own todo list and write it into the commit body. Failure mode: forgotten todos. Mitigated by skill recipe explicitly listing it as a step, not by mechanism.

### Reversibility

- Skill: `rm -r .claude/skills/handoff/` (project-installed; no per-user uninstall to coordinate).
- Hooks: remove from `.claude/settings.json`.
- Memory rule: per-user file deletion.
- Commit-message discipline: revert ADR / pre-commit hook warning.

Each piece independently reversible.

## Pilot evaluation criteria

Promote Proposed → Accepted after **two of three signals** turn net-positive across three real cross-session resumes:

1. **Resume time delta.** Cold-start time (new agent start → first useful action on the track) measured before vs after. Target: ≥3 min reduction average.
2. **Re-derivation count.** Count of "I tried this and it didn't work — let me try again" cycles in the new agent's first 10 actions. Target: zero (i.e., the handoff baton successfully captured the ruled-out paths).
3. **Hook-nudge action rate.** Of the PreCompact + SessionEnd nudges that fire, what fraction result in an actual `/handoff` commit (vs the agent correctly no-opping for clean tree)? Target: ≥50% — too high means we're nagging on no-WIP, too low means we're missing real WIP. Rough sanity check, not a hard gate.

Three real resumes is the minimum sample. If telemetry is inconclusive after three, the pattern stays Proposed and we extend the sample window.

## Acronyms / glossary

- **WIP** — Work-in-progress (uncommitted diff in working tree)
- **Baton** — The structured handoff content that "passes" between agent sessions
- **Track** — Concurrency unit per ADR 0006 + 0017; one row in `active-work.yaml`

See `GLOSSARY.md` for project-wide term definitions.
