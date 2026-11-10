# ADR 0013 — `[multi-agent-orchestration]` Master worktree read-only-by-convention; per-session work happens in per-session worktrees

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-05-06 |
| Repo scope | Multi (every repo running multi-agent clusters or multi-session human work) |
| Supersedes | — |
| Superseded by | — |
| Related | [`0006-multi-agent-human-concurrency-governance.md`](0006-multi-agent-human-concurrency-governance.md), [`0011-cluster-worker-execution-environment.md`](0011-cluster-worker-execution-environment.md), [`0012-session-bus-event-layer.md`](0012-session-bus-event-layer.md), [`0016-ref-ownership-preflight.md`](0016-ref-ownership-preflight.md), [`../operations/cluster-worker-worktree-runbook.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/operations/cluster-worker-worktree-runbook.md) |

## TL;DR (for a layperson)

When a project uses git "worktrees" (separate folders that share the same git history), one folder is the original — the "master" worktree. It's tempting for everyone to do their work there because it's the most familiar one. The problem is that any git operation done in that folder can disrupt every other agent that is reading the same folder at the same time. This ADR makes a rule: the master worktree is for low-risk operations only (reading status, fetching from GitHub, looking at history). Real work — editing, committing, branching — happens in a per-session worktree that the agent owns alone. The few things that genuinely need the master worktree announce themselves first and take a soft lock so others know to wait.

## Context

[ADR 0011](0011-cluster-worker-execution-environment.md) adopted git worktrees as the substrate for cluster-worker isolation, with the operator's mainline checkout staying on `main` in the master worktree. In practice, sessions still default to the master worktree for any work that doesn't feel "track-shaped" — running ansible playbooks, working on cross-cutting docs, using the `work` CLI, anything where the per-track-worktree convention feels like overhead.

The 2026-05-06 PR-#2-merge-rebase incident exposed the cost: two sessions both treating the master worktree as their default editing space. Session A had 15 modified + 6 untracked files in flight when Session B ran a `git rebase` on local main. Stash + rebase succeeded mechanically but the master-worktree-as-shared-mutable-space pattern is what made the incident possible at all. With session-bus warnings ([ADR 0012](0012-session-bus-event-layer.md)) the disruption becomes detectable; with worktree isolation it becomes structurally impossible.

Worktrees are cheap (shared `.git` object store, only differing checkout files) but the convention to actually use them isn't enforced anywhere. ADR 0011 names the pattern for cluster workers; this ADR generalizes it to *every* concurrent session.

## Decision

The master worktree is **read-only by convention** for in-flight edits. Every session — captain, worker, operator-driven, autonomous — does its real work in a per-session worktree it owns alone.

**What is allowed in the master worktree (always-safe operations):**

| Operation | Why safe |
|---|---|
| `git fetch` | Updates remote-tracking refs only; doesn't mutate local branches or working tree |
| `git status` / `git log` / `git diff` / `git show` | Read-only |
| `git worktree list` / `git worktree add` / `git worktree remove` | Worktree provisioning is by definition multi-worktree-aware |
| `work list` / `work bus tail` | Read-only over the coordination substrate |
| Reading any file (cat / grep / IDE-open in read-only mode) | No mutation |

**What is NOT allowed in the master worktree without a soft lock:**

| Operation | Hazard |
|---|---|
| Any file edit (Edit, Write, IDE save) | Cross-session staging-area race |
| `git add` / `git commit` (any flavor) | Stages other sessions' edits + this session's together — the af119b9 scoop class |
| `git checkout <branch>` / `git switch` | Changes what the other sessions see as `main`'s working tree |
| `git rebase` / `git reset --hard` / `git pull` (anything that rewrites refs) | Cross-worktree blast radius via shared `.git/refs` |
| `git stash` of working-tree state | Sweeps in other sessions' uncommitted work — the 2026-05-06 incident |

**Soft-lock convention for unavoidable master-worktree operations.** Some operations genuinely need the canonical clone path (e.g. ansible playbooks that hardcode the path; build scripts that resolve sibling repos relatively). The soft-lock procedure:

1. Operator/agent emits a `work bus warn --scope master-worktree --msg "<intent + duration>"` (per [ADR 0012](0012-session-bus-event-layer.md)).
2. Wait for any other session to acknowledge by either (a) pausing their own master-worktree intent or (b) `work bus info --scope master-worktree --msg "yielding"`.
3. Perform the operation; emit `work bus info --scope master-worktree --msg "released"` when done.
4. The lock is *honor-system*; no kernel-level enforcement. The bus warning is the signal.

For workflows that genuinely cannot be moved out of the master worktree, capture the requirement as an F-NNN finding so the master-worktree carve-out shrinks over time.

**Per-session worktree pattern.** Every session that intends to mutate state opens with:

```
git worktree add ../worktrees/<session-name> -b track/<cluster>/<name> origin/main
cd ../worktrees/<session-name>
./ops/work/work pickup <track-name> ...
```

Session terminates with `git worktree remove`. Stale worktree cleanup is operator-driven via `git worktree prune` (already part of [`cluster-worker-worktree-runbook.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/operations/cluster-worker-worktree-runbook.md)).

## Alternatives considered

- **Status quo (master worktree as shared mutable space).** Rejected — the 2026-05-06 incident is evidence-of-cost. Mechanism for failure is structural, not disciplinary.
- **Hard lock on master worktree (e.g. `chmod` or filesystem-level).** Rejected — git itself needs write access to `.git/`; differentiating "git internals write" from "user file write" is brittle. Honor-system soft-lock with bus signal is sufficient.
- **Eliminate the master worktree entirely.** Rejected — git requires a master worktree; you cannot operate purely from secondary worktrees. The right framing is "use the master only for safe operations."
- **Per-clone separation (each session gets its own `git clone`).** Rejected — duplicates the object store, loses the cheap-worktree property, and creates cross-clone coordination needs that are worse than cross-worktree-in-one-clone.
- **Auto-spawn worktree on session start.** Considered, deferred to Block 48N ergonomics work — could be done by a Claude Code SessionStart hook. Not required for v1.

## Consequences

**Positive.**
- The 2026-05-06 incident class becomes structurally impossible: if every session is in its own worktree, there's no shared working tree for `git stash` to sweep.
- ADR 0011's worktree substrate gets uniform usage instead of opt-in.
- Per-session work has its own branch by construction → real PRs, real CI, real review surface.
- Composes with [ADR 0012](0012-session-bus-event-layer.md): the soft-lock signal IS a bus warning.

**Negative.**
- Operational friction: every session-start now includes `git worktree add` + `cd`. Mitigated by ergonomics work (auto-spawn helper) at Block 48N.
- Disk usage grows with worktree count (only working-tree files duplicate; .git stays shared). Bounded by stale-worktree cleanup discipline.
- Some ansible playbooks + build scripts hardcoded to the master-worktree path require either rewriting (path-independent) or invocation from the master worktree under soft-lock.

**Residual risk.**
- Honor-system enforcement — an agent that ignores the convention and edits in the master worktree silently becomes the next incident. Mitigated by [ADR 0012](0012-session-bus-event-layer.md) bus + [ADR 0016](0016-ref-ownership-preflight.md) ref pre-flight catching the *symptom* even if the convention is breached.
- The "what counts as the master worktree?" boundary is a single path string. If a session mistakes a worktree for the master (unlikely; `git worktree list` shows clearly which is which), it could still operate without a lock. Mitigated by shell prompt convention and runbook step 1 ("verify you are in a per-session worktree").

## Assessor view (SCA-R)

This ADR sits in **CM-2** (baseline configuration — per-session worktree is part of the multi-agent baseline), **CM-7** (least functionality — sessions confined to their own worktree directory), and **AC-4** (information flow enforcement — soft-lock signal mediates concurrent access to the canonical clone path). No CMVP-relevant crypto change. No data-handling change beyond making concurrent activity visible via the bus.

## Trigger conditions for revisit

- Soft-lock contention becomes frequent enough to warrant kernel-level enforcement (advisory `flock` on a sentinel file in `.git/`) → upgrade lock mechanism.
- A workflow surfaces that genuinely cannot be moved out of the master worktree AND cannot tolerate soft-lock waiting → per-workflow exception captured as F-NNN; design doc §"master-worktree carve-outs" updated.
- Auto-spawn worktree ergonomics ship at Block 48N → tighten this ADR's "soft-lock" prose to assume auto-spawn is the default path.
- Per-clone separation becomes load-bearing (e.g. for security isolation across operator identities) → revisit "per-clone vs per-worktree" decision.

## Amendment 2026-05-08 — flip "by convention" → structurally enforced via post-checkout + pre-commit hooks

**What changed.** ADR 0013's original framing was "read-only by convention" — operator + agent discipline, no mechanical enforcement. The 2026-05-08 zenoh-bump incident demonstrated that convention alone is insufficient at our agent reliability level: a top-level Claude session ran `git checkout -b zenoh-1.9-bump` in the master worktree, swinging HEAD for every concurrent agent operating in the same shared working tree. Two parallel agents committed against what they thought was main but had actually been hijacked. Recovery cost ~3h + recovery via cherry-pick of stranded commits.

The spike report at `../reports/2026-05-08-master-worktree-hook-spike.md` walks through the failure mode, the wrong-primitive attempt (`reference-transaction` doesn't reliably catch `git checkout <existing-branch>` when source and target point at the same commit), and the right-primitive solution (`post-checkout` that resets BOTH HEAD and working tree).

**This amendment flips the rule to "read-only structurally enforced via post-checkout + pre-commit hooks."** Mechanical layer alongside the existing convention layer + the existing per-session worktree pattern (ADR 0011 + this ADR's original decision).

**What's enforced:**

- `post-checkout` hook in `../../ops/git-hooks/post-checkout` — fires after any checkout in the master worktree. If HEAD has moved off `main`, immediately reverts both HEAD and working tree to `main` via `git symbolic-ref HEAD refs/heads/main` + `git read-tree -u --reset HEAD`, exits non-zero with a loud message pointing at the launcher. Inert in child worktrees.
- `pre-commit` hook in `../../ops/git-hooks/pre-commit` — refuses commits in the master worktree when HEAD is not on `main`. Belt-and-suspenders alongside `post-checkout`.

Both hooks use `git rev-parse --git-dir != --git-common-dir` to detect "is this a child worktree?" and exit 0 if so — child worktrees behave normally. Empirically validated in the spike.

**Caveat — `git reset --hard <feature-branch>` not caught.** Doesn't trigger `post-checkout` (no checkout fires; HEAD's symref doesn't move; `main` is rewritten locally). Mitigation:

- GitHub branch protection on `main` (linear history, no force-push) blocks the destructive push
- Local damage is recoverable via `git reflog`
- Acceptable for v1; documented in the install README at `../../ops/git-hooks/README.md`

**Composes with:**

- **ADR 0011** — per-session worktrees (the structural pattern this amendment defends)
- **ADR 0014** — local-main-as-trunk-mirror invariant (≤2 unpushed commits + commit-time DCO/lint; orthogonal layer)
- **ADR 0006** — concurrency governance (track pickup/release; the pickup decision triggers worktree provision)
- **`claude-session` launcher** at [`../../ops/scripts/claude-session`](../../ops/scripts/claude-session) — provides the "or use" path that the hook's error message points at
- **`/iso` slash command** at [`../../ops/scripts/iso.skill.md`](../../ops/scripts/iso.skill.md) — operator UX for mid-session toggling between modes
- **Memory hook** at `feedback_master_worktree_readonly.md` (`ops/scripts/feedback-memory-hooks/`) — advisory backstop reinforcing path discipline at agent reflex layer

**Pilot status — may be rolled back.**

The amendment + hooks ship as a pilot under track `multi-agent-substrate-export`. Per [Block 48I net-positive-only governance meta-rule](https://github.com/usareur-af/goatdocs/blob/main/docs/design/multi-agent-team-management.md), adoption requires 2+ pilot runs showing net-positive vs status quo.

**Pilot evaluation criteria** (also noted in `../multi-agent/README.md`):

- **Operator-intervention rate.** How often the operator must manually fix git state. Healthy: 0–2 per cluster/session.
- **Friction at session start.** How long the wrapper takes to provision a worktree (~5–15s expected).
- **False positives.** How many times the post-checkout hook fires on legitimate operations (target: zero in tested scenarios; document each as a finding).
- **Recovery from rollback test.** Can `rm` the hooks and `rm` the wrapper and have the substrate behave exactly as today? (verified at install).

**Rollback procedure** (per [ADR `0017-claude-code-session-isolation-pilot.md`](0017-claude-code-session-isolation-pilot.md) when authored):

```bash
# Revert hooks
mv .git/hooks/post-checkout .git/hooks/post-checkout.disabled
mv .git/hooks/pre-commit .git/hooks/pre-commit.disabled
# OR remove entirely:
rm .git/hooks/post-checkout .git/hooks/pre-commit

# Revert wrapper (operator-installed)
rm ~/bin/claude-session 2>/dev/null

# Revert /iso skill (per-user)
[ -L ~/.claude/skills/iso/SKILL.md ] && rm ~/.claude/skills/iso/SKILL.md
rmdir ~/.claude/skills/iso 2>/dev/null  # only if empty
rm ~/.claude/skills/iso.md 2>/dev/null  # copy left by an older install

# Revert memory hook (per-repo)
rm ~/.claude/projects/<this-repo>/memory/feedback_master_worktree_readonly.md
# also remove the line from MEMORY.md

# active-work.yaml mode field is non-load-bearing; ignore it or revert
```

**Trigger to roll back**: 2+ consecutive sessions or cluster runs show net-negative on any evaluation criterion.

**Trigger to promote to Accepted final**: 2+ consecutive sessions or cluster runs show clean operation, plus at least one Pattern-3 cluster run with successful captain integration.

## Amendment 2026-05-09 — file-level invariant; no half-way authoring on master

The 2026-05-08 amendment closed the **commit-level + branch-level** gap (post-checkout + pre-commit hooks refuse master HEAD off main, refuse commits to a non-main branch on master). The 2026-05-09 incident exposed a **file-level** gap that the hooks do not enforce: the master worktree's working tree is shared substrate visible to every concurrent session, and any session can `git add` an untracked / modified file there and commit it directly to main — legally, by the existing invariant — regardless of whether another session has the same bytes on a feature branch with an open PR.

### What happened (2026-05-09)

1. Session A (Block 76/77 scaffolding) authored four files via `Write` to absolute master-worktree paths: `docs/adr/0840-*.md`, `docs/adr/0841-*.md`, `docs/design/snitch-app.md`, `docs/design/goat-client.md`.
2. Session A `cp`'d the four files into a per-session worktree at `.claude/worktrees/block-76-77-scaffolding/`, committed them as `ac0f9b9f`, opened PR #10.
3. Session A never cleaned up the master-worktree copies. They sat as `??` (untracked) on master for ~6 hours.
4. Session B started later, observed the four files as untracked on master, sha256-confirmed they matched `ac0f9b9f` on the worktree branch, and committed them directly to main as `09fa5bc0`. Session B's commit body explicitly notes the byte-identical relationship to the worktree branch but states the rationale as "INDEX.md and impl-plan already reference these ADRs (broken in-tree links)" — i.e., Session B was closing what looked like a documentation gap, not racing Session A. Session B did not run `gh pr list` before committing.
5. PR #10 was effectively pre-empted (closed unmerged the next session).
6. **No hook tripped.** The 2026-05-08 amendment hooks (post-checkout + pre-commit) cover branch + HEAD operations, not direct-to-main commits whose content matches a sibling worktree-branch.

### File-level invariant (added by this amendment)

**On a per-session-worktree task, all filesystem writes anchor to the worktree path. The master worktree is read-only — at the file level, not just the branch level — for the duration of that task.**

The four sub-rules:

| # | Rule |
|---|---|
| F1 | When a per-session worktree exists for the active task, every `Write` / `Edit` / file-creating `Bash` operation targets a path under that worktree's root. The master worktree path is forbidden as a write target. |
| F2 | Reads against the master worktree (`Read`, `git status`, `git log`, `cat`, `grep`) remain allowed — that's how cross-references and verification work. |
| F3 | If a session realizes mid-task that work is track-shaped after already authoring on master (the "transition case"), the session MUST move the bytes off master atomically as part of provisioning the per-session worktree. Use `cp` to the worktree immediately followed by `git restore` (tracked) or `rm` (untracked) on master. **Never just `cp` and leave the master copy.** Stash-then-pop is forbidden — `refs/stash` is repo-global across every worktree and every concurrent session, so another session can pop the bytes between your push and your pop (the PreToolUse destructive-git guard refuses stash mutations). |
| F4 | At the end of any per-session-worktree task, before reporting done, the session verifies `git -C <master-root> status --short` shows nothing task-related as `??` or `M`. Anything left there is an orphan that another session can pick up and commit. |

Half-way authoring (write on master → `cp` to worktree → leave orphan on master) is the failure mode this invariant exists to prevent. It is **strictly worse** than either pure path:

- **Pure-master mode:** 1 location, accept the cross-session-share risk. Cost = N (share-risk).
- **Pure-worktree mode:** 1 location, isolated. Cost = ~0.
- **Half-way mode:** 2 locations carrying identical bytes, no clear owner because the original author has mentally "moved on" to the worktree. Cost = N (share-risk on master) + cross-session collision becomes the default rather than a rare race.

There is no scenario where half-way mode produces a better outcome than either pure mode.

### Companion sibling rule for the cross-session second-actor

The file-level invariant is one-sided in coverage — it constrains the *originating* session (the one with the worktree branch + PR). The second-actor session that observes the orphan and considers committing it has its own gap to close: **before any direct-to-main commit on master, run `gh pr list` and check for active PRs touching the same paths.** If a PR exists with the same content, the second actor should add to / coordinate with the PR rather than pre-empting it via direct-to-main commit. This is procedural (not mechanical) until the substrate enforcement in the next subsection lands.

### Substrate-level enforcement (queued, not in this amendment)

The procedural rule is the immediate fix. The substrate gap remains and warrants mechanical enforcement in a follow-on:

1. **Pre-commit hook on master extended to refuse commits whose content matches any active worktree-branch HEAD.** Scans `git worktree list` + checks each branch HEAD's tree for the staged paths; refuses if any matches. Closes the exact 2026-05-09 failure mode (the hook would have refused `09fa5bc0` because its content matched `ac0f9b9f` on a live branch). Composes with the existing pre-commit hook.
2. **`work doctor`-style drift check** (cron or pre-prompt): flags untracked / modified files on master that match active worktree-branch trees; surfaces leaks for operator-fired cleanup before they get committed.
3. **Tool-use hook on `Write` / `Edit`** that checks active worktree context (e.g., presence of `.iso-mode.env` or per-session env-var) and refuses paths under the master worktree root when a per-session worktree is active. Cleanest agent-side gate but requires Claude Code harness-level hook support that may not be available at all surfaces today (per the Block 48P cwd-anchor backlog entry — mechanical Bash-tool enforcement is queued; Write / Edit equivalents would be sibling work).

Items 1 + 2 are mechanical and worth implementing as a follow-on (Block 48-style hardening). Item 3 closes the gap by construction at agent-side rather than at git-side. All three compose.

### Distributable memory hook

Procedural enforcement until substrate items 1-3 land: `feedback_no_halfway_worktree_authoring.md` (`ops/scripts/feedback-memory-hooks/`) — installed alongside the existing `feedback_master_worktree_readonly.md` hook by the same operator-fired install path. Future Claude Code sessions in this repo pick up the rule automatically as part of the memory-hook bundle.

### Pilot status update

The 2026-05-09 incident is a third evaluation signal for the original Block 48O pilot: per-piece rollback per ADR 0017 §"Pilot evaluation criteria" was not triggered because the **commit-level** invariant the hooks enforce did hold (Session A correctly used a per-session worktree + branch + PR; Session B did not commit on master HEAD off main). The gap exposed is a **separate failure mode** the pilot did not target. This amendment closes the conceptual gap; substrate enforcement closes the mechanical gap.

---

*See `GLOSSARY.md` for terms and acronyms.*
