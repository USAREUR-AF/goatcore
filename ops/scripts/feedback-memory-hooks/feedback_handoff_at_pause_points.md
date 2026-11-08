---
name: agent fires /handoff at natural pause points without operator nudge
description: Fire `/handoff` proactively at moments where in-head state is about to be lost — before long background tasks, before plan-mode exit with WIP, before `/clear` or `/iso exit`, on autonomous-worker permission-halt, at any "I think I'm done for now" moment. Don't wait for a hook nudge. Per ADR 0839 the cost is one extra commit per pause; the benefit is a 5-10 min cold-start saving for the next agent session.
type: feedback
---

# Self-fire `/handoff` at natural pause points

The `/handoff` skill captures TodoWrite + active hypothesis + ruled-out paths + next concrete step + WIP files into a `wip(<track>): handoff baton` commit. It exists because file-system state survives a session boundary but in-head state vaporizes.

PreCompact / SessionEnd / Stop-with-throttle hooks will nudge you when state is about to be lost. **Don't wait for the nudge.** Self-fire whenever you'd lose continuity if the session ended right now.

## Pause-point triggers

Fire `/handoff` proactively when ANY of these is about to happen:

- You're about to start a long-running background task (>30 min) — build, soak run, fleet rollout — that might outlast the session.
- You're exiting plan mode with non-trivial ruled-out paths and a clear forward step.
- You're about to `/clear` (operator-instructed or self-decided).
- You're about to `/iso exit` with WIP in the worktree.
- You're an autonomous worker hitting a permission boundary and about to halt — fire `/handoff` BEFORE writing the F-NNN halt note, so the operator picks up not just the boundary but your full reasoning state.
- The operator says "pause for now" / "let's come back to this" / "done for the day" with WIP in the tree.
- Any "I think I'm done for now / handing back" moment with non-trivial in-head state.

## Reason

Per ADR 0839:

- Cost is ~30 seconds of agent time + one git commit per pause point.
- Benefit is a 5-10 minute cold-start saving for the next session, which can be a different account (after a quota-shutdown rollover), a different machine, or just you-tomorrow after the context window vaporizes.
- The pattern works without any quota signal — the natural pause points already coincide with where state-loss matters.

## When NOT to fire

- Working tree is clean AND the last commit body already names a clear "next:" step. Skip — `/handoff` is a no-op in this case.
- You're not on a tracked branch (no `track/<name>` checkout, no active-work.yaml row owned by this session).
- You're going to commit a real fix in the next 30 seconds — the real commit body carries the same content. Don't double-commit.
- Pure read-only inspection. No state to capture.

## Per-task workflow

```
operator: "great work, let's pause for now and pick up tomorrow"

your action:
  /handoff
  → captures TodoWrite + hypothesis + ruled-out + next step into wip baton commit
  → updates active-work.yaml track note with handoff sha + timestamp
  → emits HANDOFF_COMPLETE=<sha>
  → operator acknowledges; session ends gracefully
```

```
operator: "kick off the site rebuild"
(implies: long task, likely overnight)

your action:
  /iso enter --autonomous site-rebuild
  ... do work ...
  ... hit a permission boundary on a destructive operation ...
  /handoff       # capture full state
  ... write F-NNN halt note ...
  ... stop ...

operator (next morning):
  /iso resume site-rebuild
  git log --grep="handoff baton" -1 --format=full   # see your full state
  ... approves boundary, resumes you ...
```

## Don't over-think it

When in doubt, fire. The no-op path is fast (~2s). The cost of NOT firing when you should is the next agent re-deriving 5-10 min of context you already had. Asymmetric cost — err toward firing.

## Composes with

- `/handoff` — the skill itself (project-installed at `.claude/skills/handoff/`, available to every Claude Code session in this repo automatically)
- [`/iso`](../iso.skill.md) — re-entry path; new agent runs `/iso resume <track>` then `git log --grep="handoff baton" -1`
- PreCompact / SessionEnd / Stop hooks — automated nudges (this rule means you fire BEFORE the nudge fires, not because of it)
- ADR 0839 — full pattern + rationale + cost-benefit math
