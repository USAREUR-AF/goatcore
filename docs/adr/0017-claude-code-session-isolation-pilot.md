# ADR 0017 — `[multi-agent-orchestration]` Claude Code session-isolation pilot — wrapper + slash command + work-CLI metadata + master-worktree hooks

| Field | Value |
|---|---|
| Status | Proposed (pilot) |
| Date | 2026-05-08 |
| Repo scope | Multi (every repo using Claude Code with multi-agent or multi-session work) |
| Supersedes | — |
| Superseded by | — |
| Related | [`0006-multi-agent-human-concurrency-governance.md`](0006-multi-agent-human-concurrency-governance.md), [`0011-cluster-worker-execution-environment.md`](0011-cluster-worker-execution-environment.md), [`0013-master-worktree-read-only-by-convention.md`](0013-master-worktree-read-only-by-convention.md), [`0014-local-main-as-trunk-mirror-invariant.md`](0014-local-main-as-trunk-mirror-invariant.md), `../reports/2026-05-08-master-worktree-hook-spike.md`, `../multi-agent/README.md` |

## TL;DR (for a layperson)

When multiple Claude Code sessions run on the same machine, they share the master git working folder. If one session changes branches there, every other session is suddenly looking at a different code state. We're adopting a launcher script (`claude-session`) that provisions a per-session isolated working folder by default, plus a slash command (`/iso`) for the operator to toggle modes mid-session, plus mechanical git hooks that refuse changes to the master folder, plus metadata in our work tracker so other sessions can see who's working where. Pilot for now; rollback plan attached.

## Context

[ADR 0013](0013-master-worktree-read-only-by-convention.md) declared the master worktree read-only **by convention**. The 2026-05-08 zenoh-bump incident proved convention alone isn't enough at our reliability level. A top-level Claude session ran `git checkout -b zenoh-1.9-bump` in the master worktree; HEAD swung for every concurrent agent operating in the same shared working tree; two parallel agents committed against the wrong branch; recovery cost ~3h.

The spike at `../reports/2026-05-08-master-worktree-hook-spike.md` validated `post-checkout` + `pre-commit` hooks as the right mechanical primitive (`reference-transaction` doesn't reliably catch `git checkout <branch>` when source and target point at the same commit).

The substrate now has the right shape on paper (ADRs 0006, 0011, 0013, 0014, 0016 + Block 48 cluster pattern + `ops/work` CLI). Operationally, the gap is between "discipline exists in docs" and "discipline executes reliably under load." This pilot closes that gap.

## Decision

Adopt **five composing pieces** as a pilot bundle. Each independently revertable; together they make the master-HEAD-mutation failure mode structurally impossible.

### 1. Master-worktree git hooks (`ops/git-hooks/`)

`post-checkout` + `pre-commit` hooks installed in `.git/hooks/` of the master worktree:

- `post-checkout`: refuses HEAD off `main`; reverts HEAD AND working tree (both load-bearing); exits non-zero with operator-facing error
- `pre-commit`: refuses commits when HEAD is not on `main`
- Both inert in child worktrees via `git rev-parse --git-dir != --git-common-dir` check

Spike-validated. Code copy-verbatim from the spike report. Install via `ops/git-hooks/install.sh`. Bypass via per-clone hook rename for emergencies.

### 2. `claude-session` launcher ([`ops/scripts/claude-session`](../../ops/scripts/claude-session))

Bash launcher that takes a session name and:

- Provisions `.claude/worktrees/<name>` on branch `track/<name>` from `origin/main` (default mode)
- Registers track via `ops/work/work pickup` (best-effort)
- `cd`s into the worktree
- `exec claude` (or operator-handles if `claude` not on PATH)

Three modes:
- **worktree** (default) — provisions per-session worktree; takes a `<name>`
- **`--master`** — explicit master mode; no worktree; optional `<name>` registers a master-mode track for visibility
- **`--read-only`** — no track, no worktree; pure inspection

Master-mode collision risk follows status-quo behavior (no soft warnings, no hard locks per net-positive call). Visibility is the value-add.

### 3. `/iso` slash command ([`ops/scripts/iso.skill.md`](../../ops/scripts/iso.skill.md))

Operator-side toggle for mid-session mode change. Subcommands: `enter <name>`, `exit`, `resume <name>`, `status`. Composes with the launcher.

Mechanical for Bash invocations (cwd persists across Bash tool calls within a session). Advisory for Read/Write/Edit tools (operator-side discipline reinforced by memory hook). Each successful subcommand emits an `OPERATING_DIRECTORY=...` directive that the parent agent honors for subsequent file operations.

Per-user installation at `~/.claude/skills/iso/SKILL.md`, a symlink to the tracked file.

### 4. Work CLI metadata extension

`ops/work/work pickup` gains:

- `--master` flag for explicit master-mode registration
- Auto-detection of `worktree_path` + `branch` + `host` from the invoking shell's `cwd` + `git worktree list` + `hostname`
- New optional fields in active-work.yaml track rows: `mode: worktree|master`, `worktree_path: <path>`, `branch: <branch>`, `host: <hostname>`

Auto-detection means zero ceremony — no flags needed beyond `--master` for explicit master mode.

### 5. Memory hook (`feedback_master_worktree_readonly.md` (dots agent-tools, `feedback-memory-hooks/`))

Advisory backstop loaded into Claude's memory each session. Reinforces:
- Master worktree HEAD is shared substrate; never `git checkout -b` or `git switch` there
- First action on tasks involving commits/branches: `claude-session` or `/iso enter`
- For file tools after `/iso` toggle: honor `OPERATING_DIRECTORY` directive

Per-repo, per-user, operator-installed via the README at [`ops/scripts/feedback-memory-hooks/README.md`](../../ops/scripts/feedback-memory-hooks/README.md).

## Pilot evaluation criteria

Per [Block 48I net-positive-only governance meta-rule](https://github.com/usareur-af/goatdocs/blob/main/docs/design/multi-agent-team-management.md), pilot adoption requires 2+ runs showing net-positive vs status quo. Three signals are visible to the operator without deep tooling:

### Signal 1 — operator-intervention rate per session/cluster

How often the operator must fix git state. Healthy: 0–2 per cluster of 6–10 workers. Trouble: more than that.

### Signal 2 — captain integration-phase duration (clusters only)

After all workers report done, how long does the captain spend merging worker branches? Healthy: 10–30 min for 6–10 worker branches with disjoint scope. Trouble: hours of conflict resolution → workers had overlapping scope (process gap, not substrate gap).

### Signal 3 — worker branch CI green-rate before integration (clusters only)

Each worker's branch should pass CI on its own before captain merges. Healthy: >90% green-on-first. Trouble: workers committing broken code that integration only catches post-merge.

For single-session work (Patterns 1 + 2), Signals 2 + 3 are not applicable; Signal 1 alone gates evaluation.

### Pilot data collection

`docs/reports/cluster-pilot-data.md` is the append-only record. Each cluster run records:

```
cluster name: <name>
session count: <count>
worker count (if cluster): <count>
total duration: <hh:mm>
captain integration phase (if cluster): <duration>
worker green-rate (if cluster): <count green / count total>
operator interventions: <count + brief description>
verdict: net positive / net neutral / net negative
notes: <any failure modes observed>
```

Three rows = enough evidence to decide.

## Rollback procedure

Each piece independently revertable:

```bash
# Hooks
rm .git/hooks/post-checkout .git/hooks/pre-commit
# OR rename for temporary disable:
mv .git/hooks/post-checkout .git/hooks/post-checkout.disabled

# Wrapper (operator-installed)
rm ~/bin/claude-session 2>/dev/null

# /iso skill (per-user)
[ -L ~/.claude/skills/iso/SKILL.md ] && rm ~/.claude/skills/iso/SKILL.md
rmdir ~/.claude/skills/iso 2>/dev/null  # only if empty
rm ~/.claude/skills/iso.md 2>/dev/null  # copy left by an older install

# Memory hook (per-repo memory dir)
rm ~/.claude/projects/<repo-prefix>/memory/feedback_master_worktree_readonly.md
# also remove the line from MEMORY.md

# active-work.yaml schema fields are optional + non-load-bearing; existing
# tracks unaffected if the new fields are absent. Roll back work-CLI changes
# by reverting the Go binary if shipped; the YAML stays valid either way.
```

**Trigger to roll back**: 2+ consecutive sessions / cluster runs show net-negative on any evaluation criterion. Roll back the offending piece, document the failure mode, redesign.

**Trigger to promote to Accepted final**: 2+ consecutive runs show clean operation, including at least one Pattern-3 cluster fire (Block 48-style) with successful captain integration.

## Three patterns the pilot enables

Documented in detail at `../multi-agent/README.md`. Briefly:

1. **Master-mode sessions** — quick fixes / inspection / merge-conflict resolution on master HEAD. `claude-session --master <name>` registers visibility track. Status-quo collision risk carries forward.
2. **Per-session worktree** — multi-commit scoped work in isolation. `claude-session <name>` is the canonical pattern. Standard branch-and-PR flow.
3. **Cluster of agents** — captain + workers under Block 48 substrate. Captain runs in its own worktree; spawns workers via Agent tool `isolation: "worktree"`. Each worker on its own branch (per ADR 0011 / git's worktree-branch invariant). Captain integrates worker branches into integration branch, opens single PR.

The pilot validates all three concurrently — the user's stated dogfood plan post-bundle-ship.

## Alternatives considered

- **Convention only (status quo).** Failed empirically on 2026-05-08. Rejected.
- **`reference-transaction` hook.** Spike-tested and rejected — doesn't catch `git checkout <existing-branch>` when source and target match (no OID change for HEAD).
- **No master mode at all (worktree-only).** Adds friction for legitimate quick-fix use cases (typos, merge-conflict resolution on main). Rejected — net-positive call says "don't add new mechanism for a status-quo failure mode."
- **Auto-cleanup on session exit.** Fragile — what if Claude Code crashed mid-work, the work isn't done, auto-cleanup would lose it. Rejected. Operator handles cleanup explicitly.
- **Auto-provision worktree at every Claude Code session start (Design B).** Cleanest end-state but requires harness/wrapper change outside this repo. Deferred to "Phase 2" once the launcher is proven.

## Consequences

**Positive.**

- Today's master-HEAD-mutation failure mode becomes structurally impossible (mechanical layer)
- Per-session worktrees become the default reflex (advisory layer)
- Cross-session visibility into who's working where (metadata layer)
- All three patterns from user's dogfood plan supported on common substrate
- Each piece revertable; rollback is `rm`

**Negative.**

- Operational friction at session start (wrapper run, ~5-15s for `git worktree add`)
- Memory hook adds to existing 80+ memory entries (small dilution risk per existing memory entries)
- Operator must learn new commands (`claude-session`, `/iso`)
- Active-work.yaml schema gains optional fields; legacy tooling unaffected if it ignores them
- Pilot bundle commits + ADR carries non-trivial doc weight; if rolled back, ADR carries the postmortem

**Neutral.**

- Master mode collision risk unchanged from status quo (deliberate per net-positive call)

## Trigger conditions for revisit

- Pilot data shows net-negative on 2+ runs → roll back offending piece, redesign
- Pattern-3 cluster integration friction high → author worker-branch integration runbook (deferred from this bundle per net-positive scoping)
- File-tool advisory layer fails repeatedly → consider mechanical wrapper around Read/Write/Edit
- Auto-provision-at-session-start (Design B) becomes viable → adopt as Phase 2

---

*See `GLOSSARY.md` for terms and acronyms.*
