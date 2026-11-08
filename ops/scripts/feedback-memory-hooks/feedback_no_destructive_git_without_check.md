---
name: never run destructive git without verifying tree state first + a PreToolUse hook structurally enforces
description: Refuses `git reset --hard`, `git push --force` (without --force-with-lease), `git branch -D`, `git checkout .`, `git restore .` (without --staged), `git clean -f`, `rm -rf .git`, `git worktree remove --force`, and `git stash` mutations (push/save/pop/apply/drop/clear/branch/create/store) at the Bash tool boundary. A 2026-05-13 incident wiped multi-file unstaged WIP via `git reset --hard origin/main`; the 2026-05-06 incident class is cross-session stash collision (refs/stash is a single repo-wide ref shared across every worktree). The hook backs the reflex; this memory makes the reflex itself correct so the override is rarely needed.
type: feedback
---

# Destructive git operations are catastrophic; verify before invoking

A `git reset --hard` against a working tree carrying unstaged WIP destroys those changes irrecoverably (no reflog for unstaged content). Same class: `git push --force` (overwrites remote without lease check), `git branch -D` (deletes unmerged work), `git checkout .` / `git restore .` (discards all unstaged), `git clean -f` (deletes untracked), `rm -rf .git` (destroys repo), `git worktree remove --force` (drops uncommitted work in sibling worktree).

## Rule

- **Never run a destructive git operation without first checking the tree state.** `git status` + `git diff` first — confirm what is about to be lost.
- **Use the existing coordination primitives, not stash, to preserve WIP.** The per-session worktree is already the isolation — uncommitted state in your worktree is invisible to other sessions (ADR 0013, ADR 0017), so most "preserve before destructive op" cases are solved by *not* running the destructive op. For cross-session resumption use the `/handoff` skill (`wip(<track>): handoff baton` commit on your track branch — ADR 0839). For a recovery point before a branch-wiping op, note the SHA or tag it; the reflog also holds it 90 days.
- **`git stash` mutations are banned.** `refs/stash` is a single repo-wide ref shared across every worktree and every concurrent session; another session can `git stash pop` and silently grab WIP it doesn't own (the incident class). The PreToolUse destructive-git guard refuses the mutating subcommands. Read-only `git stash list` / `git stash show` remain allowed.
- **Treat the PreToolUse refusal as load-bearing**, not friction. If the hook denies a command, *stop and read the message* — it points at the right primitive (worktree, /handoff, reflog, tag). Re-evaluate whether the destructive action is what you actually want.

## Why

The hook (`.claude/hooks/pretooluse-destructive-git-guard.sh`) refuses the destructive patterns at the Bash tool boundary. It exists because of a 2026-05-13 incident where the agent ran `git reset --hard origin/main` against a tree carrying user WIP across multiple files (CLAUDE.md, docs/adr/0840, docs/design/goat-client.md, docs/project/implementation-plan.md) and the user lost work they had not explicitly asked to discard. The hook is structural enforcement; this memory rule is the reflex layer that should make the hook a backstop, not a routine speedbump.

## How to apply

1. **Before every destructive git op**: read the current `git status` output (or recall from a recent call). If there is anything unstaged or untracked that the operator might care about, ask first. To preserve across the op, use the existing primitives — note/tag the SHA (reflog also holds it 90 days), or commit `wip(<track>): ...` on your track branch. Don't `git stash` (banned — the stash list is repo-global and other sessions can pop your WIP).
2. **If the PreToolUse hook denies a command**: do NOT immediately retry with the override sentinel. Read the refusal reason. The hook lists safer alternatives — try those first. Only use `GOAT_ALLOW_DESTRUCTIVE_GIT=1` after confirming with the operator that the destructive action is intended.
3. **Override sentinel discipline**: the operator override `GOAT_ALLOW_DESTRUCTIVE_GIT=1 <command>` is per-invocation by design — it must appear in the command string itself, not the parent shell. Treat each override as a deliberate user-confirmed escape, not a blanket waiver.
4. **Rebase conflict recovery is non-destructive by default**: `git rebase --abort` and `git rebase --continue` are recovery operations, not destructive ones — the hook allows them.
5. **Force push only with lease**: `git push --force-with-lease` is allowed because it refuses to overwrite remote work that has moved past your last fetch. Plain `--force` / `-f` is denied.

## Composition

- Structural enforcement: PreToolUse hook at `.claude/hooks/pretooluse-destructive-git-guard.sh` + regression fixture at `.claude/hooks/tests/destructive-git-guard-tests.sh`
- Reflex layer: this memory rule
- Operator override: `GOAT_ALLOW_DESTRUCTIVE_GIT=1` prefix per-invocation
- Pairs with `feedback_master_worktree_readonly.md` (mechanical worktree isolation) and `feedback_no_halfway_worktree_authoring.md` (file-level invariant) — together those three close the master-tree damage class.
