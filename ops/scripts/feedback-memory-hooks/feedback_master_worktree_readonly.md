---
name: master worktree is read-only by convention + hook enforcement
description: Always operate from a per-session worktree, never the master worktree, when the task involves commits or branch operations. The master worktree HEAD is shared across every Claude Code session on this machine; mutating it breaks parallel sessions. Hooks at .git/hooks/{post-checkout,pre-commit} structurally enforce; this memory entry is the reflex-layer reinforcement.
type: feedback
---

# Master worktree HEAD is shared substrate

The master worktree at the repo root is **shared across every Claude Code session on this machine**. Its HEAD, branches, and dirty files are visible to every concurrent session. Mutating it is a multi-agent-collision failure mode.

## Rule

- **Never `git checkout -b <name>`, `git checkout <branch>`, or `git switch <branch>`** in the master worktree. The post-checkout hook in `ops/git-hooks/` will revert and exit non-zero, but the reflex should be to not do it in the first place.
- **Never commit on the master worktree** unless explicitly maintaining the trunk-mirror invariant per ADR 0014 (≤2 unpushed commits, fast-forward only).
- **First action on any task involving commits or branch ops**: launch via `claude-session <name>` or run `/iso enter <name>` mid-session to provision a per-session worktree at `.claude/worktrees/<name>` on branch `track/<name>`.

## Why

- ADR 0011 — cluster-worker execution environment: per-session worktrees are mandatory for isolation
- ADR 0013 — master worktree read-only by convention (now enforced by post-checkout hook per the 2026-05-08 spike)
- incident: zenoh-bump session ran `git checkout -b zenoh-1.9-bump` in master worktree; parallel sessions committed against the swung HEAD; recovery cost ~3h + lost commits

## How to apply

1. **For VSCode-launched sessions**: `/iso enter <name>` is the **primary entry path** — invoke as the first action of any session that may write. The CLI wrapper `claude-session` is the equivalent for terminal-launched sessions.
2. **At task start (mid-session)**: if you realize work is non-trivial, run `/iso enter <name>` to switch to per-session worktree.
3. **Cwd discipline — anti-drift**:
   - `/iso enter` writes `.iso-mode.env` at the worktree root with `WORKTREE_ROOT=<absolute-path>`
   - **Every Bash invocation MUST start with `cd "$WORKTREE_ROOT" && ...`** (or use fully absolute paths; never bare relative `cd`)
   - **For git ops, prefer `git -C "$WORKTREE_ROOT"`** — bypasses cwd dependency entirely
   - If `pwd` ever returns a path outside `$WORKTREE_ROOT`, **halt** rather than continue — drift outside your worktree is a track-level halt
4. **For file operations**: when `/iso` emits `OPERATING_DIRECTORY=<path>`, use that path as the prefix for all subsequent absolute paths in Read/Write/Edit tool calls. Never use master-worktree absolute paths from a worktree session.
5. **Cwd persists across Bash tool calls** within a session — once `cd` into the worktree, subsequent Bash invocations inherit. **But persistence is brittle**: the zenoh-bump session demonstrated relative `cd` drifting back to launch directory. Always-anchor with absolute paths.

## Triggers for pickup (not just discipline)

Per ADR 0006 + the SessionStart hook's enumerated triggers (strict-OR, not gestalt):
- >1 anticipated commit
- >30 min anticipated duration
- any edit to CLAUDE.md / ADR / impl-plan / active-work / design-doc
- substrate edit (Ansible roles, terraform, mgmt-host compose, work CLI, hooks)

If ANY trigger matches, pickup is mandatory not optional. Cost of pickup is ~30s; cost of skipping is the zenoh-bump scenario.
