# Feedback memory hooks (operator-installed)

These are advisory memory hooks that fire at every Claude Code session start. They reinforce the multi-agent substrate's discipline at the model's reflex layer, alongside the mechanical enforcement (`ops/git-hooks/`) and the structural pattern (per-session worktrees).

## Files

- `feedback_master_worktree_readonly.md` — never `git checkout -b` or commit on master worktree; provision per-session worktree first; cwd-anchor discipline for relative paths
- `feedback_auto_iso_enter_on_track_shaped_tasks.md` — agent's first action on any track-shaped task is `/iso enter <name>` (or `/iso enter --autonomous <name>`); operator should not have to ask. Closes the operator-UX gap that the substrate is "transparent" only if the agent reflexes to it.
- `feedback_no_halfway_worktree_authoring.md` — never `Write` to absolute master-worktree paths and `cp` into a worktree; anchor writes inside the worktree to start with. Companion to ADR 0013 amendment file-level invariant.
- `feedback_bus_emit_at_release.md` — at track release / session close-out, evaluate whether other sessions need to know about substrate changes / ID renumberings / fleet-incident signals / operator-laptop footguns. Emit `ops/work/work bus {info|warn|urgent}` for the warranted signals.
- `feedback_findings_emit_on_record.md` — when filing a new F-NNN finding, emit a session-bus `info` event so parallel sessions don't independently allocate the same finding ID (the F-107 double-allocation incident).
- `feedback_handoff_at_pause_points.md` — at natural pause points (PreCompact, SessionEnd, Stop-with-throttle), write a `wip(<track>): handoff baton` commit so a different session can resume without re-deriving context. Block 48T continuous-handoff substrate.
- `feedback_pr_title_scope_lowercase.md` — PR titles use single lowercase scope; finding/block IDs go in the subject, not the scope. The validator (`.github/workflows/pr-hygiene.yml`) rejects uppercase + commas, and sessions have repeatedly tripped this — bake the convention in.
- `feedback_no_destructive_git_without_check.md` — never run `git reset --hard`, `git push --force` (without `--force-with-lease`), `git branch -D`, `git checkout .`, `git restore .` (without `--staged`), `git clean -f`, `rm -rf .git`, or `git worktree remove --force` without verifying tree state first. Structural enforcement at `.claude/hooks/pretooluse-destructive-git-guard.sh` refuses these patterns at the Bash tool boundary; operator override via `GOAT_ALLOW_DESTRUCTIVE_GIT=1 <command>` per-invocation.

## Install

Per-user, per-repo (memory hooks are scoped to the working directory):

```bash
# Find your memory dir for this repo (Claude Code generates one per project)
ls ~/.claude/projects/ | grep -i $(basename $(pwd))
# Output: -Users-<you>-src-github-com-<org>-<repo>

MEMORY_DIR=~/.claude/projects/-Users-$USER-src-github-com-$(git remote get-url origin | sed -E 's|.*github\.com://([^/.]+).*|\1-\2|')/memory
# Verify:
ls $MEMORY_DIR/MEMORY.md

# Copy the hook
cp ops/scripts/feedback-memory-hooks/feedback_master_worktree_readonly.md "$MEMORY_DIR/"

# Add to MEMORY.md index (~/.claude/...) — append this line:
echo '- [feedback_master_worktree_readonly.md](feedback_master_worktree_readonly.md) — never `git checkout -b` on master worktree; provision per-session worktree first' >> "$MEMORY_DIR/MEMORY.md"
```

## Verify

Next Claude Code session: ask "what feedback memory entries are loaded?" and confirm the entry appears.

## Bypass

Operator can remove the entry from `MEMORY.md` index to disable without deleting the file. The model only loads entries indexed in `MEMORY.md`.

## Rollback (pilot may be reverted)

Per ADR `0017-claude-code-session-isolation-pilot.md`, rollback procedure if pilot proves net-negative:

```bash
rm "$MEMORY_DIR/feedback_master_worktree_readonly.md"
# also remove the line from MEMORY.md
```

The mechanical enforcement (hooks) and the launcher are independently revertable.
