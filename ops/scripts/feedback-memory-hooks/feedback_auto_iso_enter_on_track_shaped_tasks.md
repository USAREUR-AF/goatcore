---
name: agent self-spawns per-session worktree on track-shaped tasks
description: When the operator gives you a task that is multi-commit, multi-hour, or substrate-touching, your FIRST action is `/iso enter <track-name>` to provision the per-session worktree before any other tool call. The operator should not have to ask. Pickup is mandatory not optional when the triggers fire — make it your reflex, not a deliberation step.
type: feedback
---

# Auto-`/iso enter` on track-shaped tasks

The operator's UX expectation is "type a task, Claude does it." They should not need to know about per-session worktrees, the `/iso` skill, or substrate isolation. **You handle isolation transparently.**

## Rule

When the operator gives you a task that matches ANY of these triggers — even one — your **first action** is `/iso enter <track-name>` (or `/iso enter --autonomous <track-name>` if the task is overnight-shaped) before any Read/Write/Edit/Bash invocation:

- Multi-commit work (the task will involve more than one logical commit)
- Multi-hour work (>30 min of focused engineering)
- Edit to load-bearing docs (CLAUDE.md, ADR, implementation-plan.md, active-work.yaml, design docs in `docs/design/`)
- Substrate edit (Ansible roles, terraform, mgmt-host compose, work CLI, hooks, build scripts)
- Cluster-shaped (the task involves multiple workers under a captain)
- Operator says any of: "do the zenoh bump", "land the PR for X", "fix the bug in module Y end-to-end", "build feature Z", "refactor module M"

If unsure, **err toward `/iso enter`**. Cost is ~5s. Cost of skipping is the zenoh-bump scenario (master HEAD hijacked, parallel sessions confused, ~3h recovery).

## How to derive the track name

Pick a name from the task description, lowercase + hyphenated, ≤30 chars:

- "do the zenoh bump" → `zenoh-1.9-bump` (pick the version specifically)
- "fix F-074 bootstrap-bundle absence" → `f074-bootstrap-bundle-fix`
- "build the airdefenders 2 cluster" → `block-67-airdefenders-2-captain`
- "refactor the policy engine" → `policy-engine-refactor`

Don't ask the operator to confirm the track name — pick a reasonable one and proceed. The operator can correct mid-flight if needed.

## Use --autonomous when

- The operator says any of: "overnight", "while I'm out", "kick off", "let it run", "see it through to a PR"
- The task estimate is >2 hours
- The operator is clearly handing off (not pair-programming)

`--autonomous` installs the overnight allowlist + activates the halt-on-permission-boundary protocol. Cost of using it inappropriately is small (you might halt + escalate when a quick approval would have sufficed). Cost of NOT using it for a real overnight run is bigger (operator gets pinged at 2am for a routine `cargo test`).

## Don't use /iso enter when

- The task is genuinely a one-line fix or read-only inspection (typo correction, "what does this code do?", "show me the design doc for X")
- The operator is asking a question, not assigning work
- You're already inside a per-session worktree (check `pwd` — if it's under `.claude/worktrees/`, you're in one)
- The operator explicitly says "do this on master" or "I want to see live state" (rare; respect)

When in doubt: `/iso enter`. Reversing is `/iso exit` then `git worktree remove` if you decide it wasn't needed. Cheap.

## Per-task workflow

```
operator: "do the zenoh bump"

your first action:
  /iso enter --autonomous zenoh-1.9-bump

your second action onwards:
  Operating directory is now the worktree.
  Use absolute paths (cd "$WORKTREE_ROOT" && ...).
  Do the work end-to-end.
  Push branch + open PR.
  ops/work/work release zenoh-1.9-bump
```

The operator types the first line. You handle the rest including the substrate provisioning.

## Caveat — first-time-after-install

If the substrate was just installed (`make multi-agent-substrate-install` ran for the first time in this clone), the post-checkout hook may refuse `git checkout` operations against branches other than main on the master worktree. That's by design. `/iso enter` uses `git worktree add` which is hooks-permitted. You should never need to `git checkout` on master anyway under this discipline.

## Caveat — operator-explicit-master

If the operator EXPLICITLY says "do this on master" or "I want to see what's actually on main right now", honor it. That's a deliberate choice. Don't override.
