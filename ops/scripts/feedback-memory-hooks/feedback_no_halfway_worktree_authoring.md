---
name: No half-way worktree authoring — fully in or fully out, never split
description: When a task is operating on a per-session worktree branch, ALL filesystem writes anchor to the worktree path. Never author in the master worktree first and copy in. Author-in-master-then-cp leaves orphan untracked files on master that other concurrent sessions can grab and commit directly to main, bypassing your PR. Companion to feedback_master_worktree_readonly.md (commit-level invariant); this rule is the file-level extension.
type: feedback
---

# Master worktree's working tree is also shared substrate

`feedback_master_worktree_readonly.md` codifies the commit-level + branch-level invariant: never `git checkout -b` on the master worktree, never `git commit` there. The post-checkout + pre-commit hooks at `ops/git-hooks/` enforce that mechanically.

This entry codifies the **file-level** extension that the hooks do not enforce: the master worktree's *working tree* is also shared substrate visible to every concurrent session. Any session can `git add` an untracked or modified file in the master worktree and commit it directly to main — and the hook will not stop them, because committing-to-main-from-master is "legal" by the existing invariant.

The incident is the proof:

1. Session A authored four files (two ADRs + two design docs) via `Write` to absolute master-worktree paths, then `cp`'d to a per-session worktree, committed there as `ac0f9b9f`, and opened PR #10.
2. Session A never cleaned up the master-worktree copies. They sat as `??` (untracked) on master.
3. Session B started later, saw the four files as untracked on master, sha256-confirmed they matched Session A's branch, and committed them directly to main as `09fa5bc0`.
4. Session A's PR #10 was effectively pre-empted; closed unmerged.
5. No hook tripped. Both sessions followed the existing rules. The collision happened in the gap.

The half-way pattern (author in master → `cp` → commit in worktree → leave orphans behind) is **strictly worse** than either pure path. Pure-master mode accepts the cross-session-share risk but at least the bytes are visible and committable from one place. Pure-worktree mode isolates the bytes to your branch + PR. Half-way mode pays both costs without either upside — the master is dirty AND the worktree carries the same content, with the orphan-on-master inviting cross-session collision.

## Rule

When operating on a per-session-worktree task:

- **All filesystem writes anchor to the worktree path** (`.claude/worktrees/<track>/...` or `~/src/.worktrees/<track>/...` or whatever the task's worktree root is). Never `Write` or `Edit` against the master-worktree absolute path.
- **The master worktree stays read-only for that task.** `Read`, `git log`, `git status`, `cat` in `Bash` for cross-references — fine. `Write`, `Edit`, file-creating `Bash` commands — refuse, redirect to the worktree path.
- **No half-way.** Once the task is anchored to a per-session worktree, the master worktree gets no writes for the duration of that task.

## Transition rule (the one that blew up)

If you start authoring on master *intentionally* — e.g., you decided ad-hoc at task start, then mid-task realized this is actually track-shaped and need to provision a worktree — you MUST clean up the master worktree as part of the transition. Sequence:

1. Before creating the worktree, identify every file you've edited / written under the master root for this task (use `git -C <master-root> status --short` + your own session memory).
2. Create the per-session worktree (`/iso enter <track>` or `git worktree add` directly).
3. **Move** the work into the worktree, do not `cp`. Use `cp` then **immediately** `git restore <file>` (for tracked files) or `rm <file>` (for untracked files) on the master copy, in the same operation pair.

   `git stash push -u + git stash pop` looks tempting here but is forbidden: `refs/stash` is a single repo-wide ref shared across every worktree and every concurrent session, so another session can `git stash pop` and silently grab the bytes between your push and your pop. The PreToolUse destructive-git guard refuses stash mutations to make this structurally impossible.
4. **Verify** before continuing: `git -C <master-root> status --short` for the relevant paths returns clean (nothing `M`, nothing `??`) for files you authored. If anything remains, you haven't transitioned — keep cleaning until they're gone.
5. From that point on, the rest of the rule applies: all writes anchor to the worktree, master stays read-only.

Half-transition (you moved your branch to the worktree but left the bytes on master) is the **failure mode** — strictly worse than not transitioning at all.

## End-of-session check

Before declaring a per-session-worktree task done, run `git -C <master-root> status --short` and verify nothing task-related is left as untracked or modified there. If something is, clean it up (`git restore` / `rm`) — even if it's already committed in the per-session worktree. Orphans on master are the root cause of cross-session collisions; they have to go.

## Pre-flight check before any Edit / Write tool call

Mental check before invoking the tool:

- Does the absolute path start with the per-session-worktree root? → proceed.
- Does the absolute path start with the master worktree root? → stop. Switch the path to the worktree path. Then proceed.
- Am I currently anchored to a per-session worktree at all? → if no (this is genuinely an ad-hoc task you decided ad-hoc upfront), the rule doesn't apply; stay on master throughout. If yes, the master path is forbidden.

## Composes with

- `feedback_master_worktree_readonly.md` — the commit-level + branch-level rule that this is the file-level extension of. The hook enforces commit-level; this rule is the procedural enforcement of file-level until the substrate gap is closed.
- `feedback_auto_iso_enter_on_track_shaped_tasks.md` — auto-spawning the worktree at task start is the cleanest way to avoid the temptation to author-in-master-then-cp; it makes the worktree your default cwd from the start, before any Write tool reflex fires.
- ADR 0013 (post-2026-05-09 amendment) — codifies the file-level extension as decision-of-record.
- Block 48P cwd-anchor discipline (backlog) — when that lands, the substrate-level mechanical enforcement layer would refuse `Write` / `Edit` tool calls outside the active worktree path. This memory rule is the procedural complement until then.

## Why this is "net negative" not just "less efficient"

Pure-master mode: 1 location, owned by the active session, accept-the-share-risk. Cost = N (the share-risk).
Pure-worktree mode: 1 location, owned by the active session, isolated. Cost = ~0.
Half-way mode: 2 locations carrying identical bytes, both visible to other sessions, no clear owner. Cost = N (the share-risk on master) + complexity of `cp` synchronization + the fact that the orphan is now disowned by the original session's mental model. **The orphan disown is the multiplier** — once the bytes are on master and the session has "moved on" to the worktree, the original author no longer thinks of those bytes as "their" work, so cleanup never happens. The cross-session collision becomes the default outcome, not a rare race.

The asymmetry: there is no scenario where half-way mode produces a better outcome than either pure mode.
