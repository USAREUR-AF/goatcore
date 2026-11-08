---
name: Emit session-bus message when a finding ID is allocated
description: When you allocate a new F-NNN by writing a new finding file and adding the INDEX.md row, emit `ops/work/work bus info` immediately — before any other tool call. Closes the cross-session ID-collision gap that release-time emission cannot
type: feedback
---

# Emit finding-allocation to the session bus at record time

When you create a new finding:

1. Pick the next free `F-NNN` from `docs/findings/INDEX.md`.
2. Write the file at `docs/findings/F-NNN-<short-slug>.md`.
3. Append the INDEX.md row.
4. **Immediately** emit `ops/work/work bus info "F-NNN: <one-line topic>"` — *before* any commit, branch push, or follow-on edit.

## Why at record-time, not at release

Release-time emission is covered separately by [`feedback_bus_emit_at_release.md`](feedback_bus_emit_at_release.md) and fires when a track wraps. By then, two parallel sessions may already have committed the same F-NNN to different files; the merge becomes a semantic-collision rebase ceremony (rename one side, fix every cross-ref, force-push, reword commits).

Record-time emission prevents the collision. Another session reading the bus in its next prompt context sees `F-NNN claimed` and picks F-NNN+1.

## Origin

PR #15 shipped two findings filed as F-110 and F-111. While that PR was open, Block 79 closure independently allocated F-110 (ecdsa-ca-init pathlen) and F-111 (avahi-daemon) for unrelated observations. PR #15 had to be rebased with both findings renumbered F-112 + F-113, internal cross-refs updated in two files + INDEX + commit message. A single `work bus info` call from either side at allocation moment would have prevented the entire rebase.

## When this rule fires

- ANY new F-NNN allocation in `docs/findings/`.
- BEFORE any subsequent tool call that depends on the ID (commit, push, cross-link from ADR / design doc / CLAUDE.md bullet).

## Bus message format

- Severity: `info` — this is a coordination signal, not an alert.
- Body: `F-NNN: <short topic>` — scannable in another session's bus tail.
- Example: `ops/work/work bus info "F-112: macOS utun naming + root privilege for tun.CreateTUN"`.

## Composes with

- [`feedback_bus_emit_at_release.md`](feedback_bus_emit_at_release.md) — release-time, broader scope (substrate changes, ID renumberings, fleet-incident signals).
- [`feedback_no_halfway_worktree_authoring.md`](feedback_no_halfway_worktree_authoring.md) — file-level worktree discipline.

Same pattern applies to any other append-only ID allocation that another session might race against — `M-NNN` measurements, `H-NN` human-action register entries, ADR numbers. Findings are the first instance because the collision has actually happened; extend to those when the same evidence accumulates.
