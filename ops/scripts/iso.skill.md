---
name: iso
description: Toggle a Claude Code session between worktree-mode (isolated per-session worktree) and master-mode (operating on master HEAD). Subcommands enter, exit, status, resume.
allowed-tools: Bash
audience: ai
---

# /iso — operator toggle for session isolation mode

This skill controls the per-session worktree state for the current Claude Code session. Composed with the master-worktree hooks (which enforce that off-main work happens in a worktree) and (optionally) the `claude-session` CLI launcher.

**For VSCode-launched sessions (typical operator path):** Claude Code starts in the master worktree by default. `/iso enter` is the **primary entry point** — invoke it as the first action of any session that may write or commit. The CLI wrapper `claude-session` is only relevant for terminal-launched sessions; VSCode users don't need it.

## When to use

- **`/iso enter <name>`** — provision a per-session worktree at `.claude/worktrees/<name>` on branch `track/<name>` from origin/main, register the track, and operate from there for the rest of the session.
- **`/iso enter --autonomous <name>`** — same as above, plus copies the overnight allowlist into the per-session worktree's `.claude/settings.local.json` so the agent runs end-to-end without operator permission prompts. Hard deny list still applies (cloud apply, destructive, main push, merge). Permission denial = structural halt per `ops/work/roles/autonomous-worker.md`. Use for overnight runs (zenoh bump, airdefenders cluster).
- **`/iso exit`** — return to master worktree (suspend the active track via `work update --status paused`, don't release).
- **`/iso resume <name>`** — re-enter an existing per-session worktree (already-provisioned).
- **`/iso status`** — show current operating directory + branch + track state + dirty flag + autonomous-mode flag.

Master mode collisions follow status-quo behavior — no soft warnings, no hard locks (per net-positive governance call: don't add new mechanism for a failure mode that already exists).

## Enter dirty-handling

If the master worktree has uncommitted changes when `/iso enter` is invoked, the skill refuses and prompts the operator: commit on a track branch (via the worktree being provisioned, or via `/handoff` to capture as a `wip(<track>):` baton commit per ADR 0839) / discard explicitly. Avoids silent state shuffling. Stash is not an option — the PreToolUse destructive-git guard refuses stash mutations (refs/stash is repo-global, cross-session collision).

**Exception — `docs/project/active-work/` is excluded from this dirty check.** The `ops/work/work pickup|release` CLI writes tracked coordination yaml there in the *master* worktree, so a routine pickup (yours or a concurrent session's) leaves master dirty. Without the exclusion, `/iso enter` refused on that routine churn (observed). That yaml is the work CLI's, not the code being isolated, so it's ignored; any other uncommitted tracked change still refuses.

## Exit dirty-handling

If the per-session worktree has uncommitted changes when `/iso exit` is invoked, prompt before switching: commit (either a normal commit on `track/<name>` or a `wip(<track>):` baton via `/handoff` per ADR 0839), or refuse. Don't lose work. Stash is not an option — the PreToolUse destructive-git guard refuses stash mutations.

## Committing from a track worktree

`/iso enter <name>` puts you on `track/<name>`, which makes the
`[track: <name>]` trailer **mandatory on every commit you make there**.
`drift-check` fails the PR otherwise, and it checks every commit between
the PR base and head — one unlabelled commit anywhere in the branch
fails the whole PR. Merge commits are exempt; non-`track/*` branches are
not checked at all.

Two ways to get it right:

```bash
# the tool appends the trailer for you
ops/work/work commit <name> -m "fix(scope): subject"

# or by hand — the trailer is the last line of the message
git -C <worktree> commit -s -F <msgfile>   # msgfile ends: [track: <name>]
```

Already pushed without it? `git commit --amend` + `git push
--force-with-lease`. If the branch is shared with someone else, ask on
the PR before rewriting their commit — the session bus is scoped to one
clone (`docs/design/session-bus.md`) and will not reach a contributor on
another machine.

Canon: [ADR 0006](../../docs/adr/0006-multi-agent-human-concurrency-governance.md)
· contributor-facing summary in `CONTRIBUTING.md` under **Commits**.

## Operating-directory directive

Every successful `/iso <subcommand>` emits a directive line that the parent agent honors for subsequent file operations (Read / Write / Edit absolute paths):

```
OPERATING_DIRECTORY=<absolute path>
MODE=<worktree|master>
TRACK=<name|none>
BRANCH=<branch|main>
```

The parent agent reads this and updates its mental model. Bash invocations inherit the new cwd mechanically (cwd persists across Bash tool calls per Claude Code's documented behavior). File tools (Read/Write/Edit) need absolute paths constructed against the new operating directory — relies on operator-side discipline reinforced by memory hook.

## Subcommand reference

### `enter [--autonomous] <name>`

```bash
# Parse args: optional --autonomous flag, then required <name>
AUTONOMOUS=0
NAME=""
for arg in "$@"; do
  case "$arg" in
    --autonomous) AUTONOMOUS=1 ;;
    *) [ -z "$NAME" ] && NAME="$arg" ;;
  esac
done
if [ -z "$NAME" ]; then
  echo "usage: /iso enter [--autonomous] <name>"
  exit 1
fi

REPO_ROOT="$(git rev-parse --show-toplevel)"
abs_git=$(cd "$(git rev-parse --git-dir)" && pwd)
abs_common=$(cd "$(git rev-parse --git-common-dir)" && pwd)
if [ "$abs_git" != "$abs_common" ]; then
  echo "/iso enter must be run from master worktree (currently in: $(pwd))"
  exit 1
fi

# Refuse on dirty master — but IGNORE work-tracking churn under
# docs/project/active-work/. The `ops/work/work pickup|release` CLI writes
# tracked coordination yaml there IN THE MASTER worktree, so a routine pickup
# (yours or a concurrent session's) dirties master and would otherwise block
# every /iso enter (the failure hit 2026-07-21: a modified mesh-control-path
# track + two release moves left master dirty, refusing enter). That yaml
# round-trips through the work CLI, not the worktree being provisioned, so it is
# not the state /iso is protecting. The positive `.` pathspec is required — a
# lone `:(exclude)` matches nothing. Everything else still refuses.
if ! git diff --quiet -- . ':(exclude)docs/project/active-work' \
   || ! git diff --cached --quiet -- . ':(exclude)docs/project/active-work'; then
  echo "/iso enter refuses: master worktree has uncommitted changes (outside work-tracking)."
  echo "  commit (via /handoff for a wip(<track>): baton per ADR 0839) or discard"
  echo "  before entering worktree mode. Stash is banned (refs/stash is repo-global)."
  echo "  (Churn under docs/project/active-work/ is ignored — that's the work CLI's.)"
  exit 1
fi

WORKTREE_PATH=".claude/worktrees/${NAME}"
BRANCH="track/${NAME}"
if [ ! -d "${REPO_ROOT}/${WORKTREE_PATH}" ]; then
  git fetch origin main --quiet
  git worktree add "${REPO_ROOT}/${WORKTREE_PATH}" -b "${BRANCH}" origin/main
fi

# Register track
"${REPO_ROOT}/ops/work/work" pickup "${NAME}" \
  --note "/iso enter${AUTONOMOUS:+ --autonomous}; worktree at ${WORKTREE_PATH}" 2>/dev/null || true

# Write per-worktree env anchor (anti-drift discipline per 2026-05-08 audit)
WORKTREE_ABS="${REPO_ROOT}/${WORKTREE_PATH}"
cat > "${WORKTREE_ABS}/.iso-mode.env" <<ENVEOF
# /iso enter anchor — sourced by agents to verify worktree context.
# Per ops/work/roles/autonomous-worker.md cwd-discipline: every Bash
# invocation in this worktree should start with: cd "\$WORKTREE_ROOT" && ...
WORKTREE_ROOT=${WORKTREE_ABS}
TRACK=${NAME}
BRANCH=track/${NAME}
MODE=worktree
AUTONOMOUS=${AUTONOMOUS}
ENVEOF

# Install overnight allowlist if --autonomous
if [ "$AUTONOMOUS" = "1" ]; then
  ALLOWLIST="${REPO_ROOT}/ops/scripts/permissions/overnight-allowlist.json"
  if [ -f "$ALLOWLIST" ]; then
    mkdir -p "${REPO_ROOT}/${WORKTREE_PATH}/.claude"
    cp "$ALLOWLIST" "${REPO_ROOT}/${WORKTREE_PATH}/.claude/settings.local.json"
    echo "/iso enter --autonomous: installed overnight allowlist at .claude/settings.local.json"
    echo "  hard-deny still applies (cloud apply, destructive, main push, merge)"
    echo "  permission denial = structural halt; see ops/work/roles/autonomous-worker.md"
    echo "  rollback this worktree: rm .claude/settings.local.json"
  else
    echo "warning: --autonomous requested but allowlist missing at ${ALLOWLIST}"
    echo "         continuing without allowlist; agent will hit interactive prompts"
  fi
fi

cd "${REPO_ROOT}/${WORKTREE_PATH}"

cat <<EOF
OPERATING_DIRECTORY=${REPO_ROOT}/${WORKTREE_PATH}
MODE=worktree
TRACK=${NAME}
BRANCH=${BRANCH}
AUTONOMOUS=${AUTONOMOUS}
EOF
```

### `exit`

```bash
# Refuse on dirty worktree
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "/iso exit refuses: worktree has uncommitted changes."
  echo "  commit (via /handoff for a wip(<track>): baton per ADR 0839) or discard"
  echo "  before exiting to master. Stash is banned (refs/stash is repo-global)."
  exit 1
fi

REPO_ROOT="$(git rev-parse --show-toplevel)"
# Detect we're in a child worktree
abs_git=$(cd "$(git rev-parse --git-dir)" && pwd)
abs_common=$(cd "$(git rev-parse --git-common-dir)" && pwd)
if [ "$abs_git" = "$abs_common" ]; then
  echo "/iso exit: already in master worktree, nothing to exit"
  exit 0
fi

# Mark track paused (don't release — operator does that explicitly)
TRACK_NAME=$(basename "${REPO_ROOT}")
COMMON_REPO=$(dirname "$abs_common")
"${COMMON_REPO}/ops/work/work" update "${TRACK_NAME}" \
  --status "paused:iso-exit" 2>/dev/null || true

cd "${COMMON_REPO}"

# Worktree-hygiene nudge: if the working set has grown past the 5-10 target,
# suggest the reaper sweep (dry-run by default; never auto-reaps here).
WT_COUNT=$(git -C "${COMMON_REPO}" worktree list | command grep -c . || echo 0)
if [ "${WT_COUNT}" -gt 10 ]; then
  echo "note: ${WT_COUNT} worktrees on disk (target 5-10). Review with:"
  echo "  ops/scripts/worktree-reap.sh            # dry-run plan"
  echo "  ops/scripts/worktree-reap.sh --apply    # reap merged/stale; see docs/design/worktree-hygiene.md"
fi

cat <<EOF
OPERATING_DIRECTORY=${COMMON_REPO}
MODE=master
TRACK=none
BRANCH=main
EOF
```

### `status`

```bash
REPO_ROOT="$(git rev-parse --show-toplevel)"
abs_git=$(cd "$(git rev-parse --git-dir)" && pwd)
abs_common=$(cd "$(git rev-parse --git-common-dir)" && pwd)

if [ "$abs_git" = "$abs_common" ]; then
  echo "MODE=master"
  echo "OPERATING_DIRECTORY=${REPO_ROOT}"
  echo "BRANCH=$(git branch --show-current)"
else
  echo "MODE=worktree"
  echo "OPERATING_DIRECTORY=${REPO_ROOT}"
  echo "BRANCH=$(git branch --show-current)"
fi
echo "DIRTY=$(git diff --quiet && git diff --cached --quiet && echo 'no' || echo 'yes')"
echo "AUTONOMOUS=$([ -f "${REPO_ROOT}/.claude/settings.local.json" ] && echo 'yes' || echo 'no')"
```

### `resume <name>`

```bash
NAME="$1"
WORKTREE_PATH=".claude/worktrees/${NAME}"
REPO_ROOT="$(git rev-parse --show-toplevel)"
abs_git=$(cd "$(git rev-parse --git-dir)" && pwd)
abs_common=$(cd "$(git rev-parse --git-common-dir)" && pwd)
COMMON_REPO=$(dirname "$abs_common")

if [ ! -d "${COMMON_REPO}/${WORKTREE_PATH}" ]; then
  echo "/iso resume: worktree ${WORKTREE_PATH} does not exist; use /iso enter to create"
  exit 1
fi

"${COMMON_REPO}/ops/work/work" update "${NAME}" --status "active" 2>/dev/null || true
cd "${COMMON_REPO}/${WORKTREE_PATH}"

cat <<EOF
OPERATING_DIRECTORY=${COMMON_REPO}/${WORKTREE_PATH}
MODE=worktree
TRACK=${NAME}
BRANCH=track/${NAME}
EOF
```

## Installation

**Symlink it, do not copy it.** A copy stops tracking this file the moment
this file changes, and nothing announces the drift — the personal install on
the operator laptop was found to be a 21 July snapshot, two
months stale, missing the `[track:]` section below. The in-repo skill
(`.claude/skills/iso/SKILL.md`) has always been a symlink and never drifted.

```bash
mkdir -p ~/.claude/skills/iso
ln -sfn "$(git rev-parse --show-toplevel)/ops/scripts/iso.skill.md" ~/.claude/skills/iso/SKILL.md
# verify next session: type /iso status
```

Per-user, not per-repo. Operator tool that's useful across goat substrate adoptions.

The symlink points into a clone, so it serves whatever that clone has checked
out. Keep that clone's `main` current — a trunk 237 commits behind serves a
skill 237 commits old. `ops/scripts/ansible-preflight.sh` already refuses to
run from a stale trunk for the same reason.

## Compose with — does not replace

- `claude-session` launcher: sets the entry mode at session start; `/iso` toggles mid-session.
- `ops/git-hooks/`: enforces master-readonly mechanically; `/iso` is operator UX on top.
- Memory hook `feedback_master_worktree_readonly.md`: advisory backstop reminding the model to honor `OPERATING_DIRECTORY` for file-tool absolute paths.

## Pilot status — may be rolled back

This skill is part of the multi-agent-substrate-export pilot. Rollback per ADR 0013 amendment + ADR `0017-claude-code-session-isolation-pilot.md`. To revert, remove `~/.claude/skills/iso/SKILL.md` only if it is still the symlink to this checkout, then use `rmdir ~/.claude/skills/iso` to remove the directory only if empty. Preserve any other skills or user files there.
