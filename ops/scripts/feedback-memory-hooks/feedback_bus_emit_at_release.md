---
name: Emit session-bus messages at track release where warranted
description: Before releasing a track, evaluate whether other sessions/agents need to know about anything that happened, and emit ops/work/work bus {info|warn|urgent} messages for the warranted signals. Closes the cross-session-context-loss gap.
type: feedback
---

When releasing a track via `ops/work/work release <track>`, evaluate whether other sessions/agents need to know about anything from your work, and emit `ops/work/work bus {info|warn|urgent} "<msg>"` for each warranted signal before completing the release.

**Why:** Sessions run in parallel under ADR 0006 / 0017 concurrency governance. Each session is byte-isolated via its own worktree, but **discoveries** (substrate gotchas, ID renumberings, naming conventions, infrastructure quirks, recovered-from outages) don't propagate automatically. The session-bus (Block 48N, `ops/work/work bus` reads/writes append-only JSONL surfaced to every Claude Code session via the `UserPromptSubmit` hook) exists exactly to close this gap — but only if track captains remember to emit. Surfaced 2026-05-11 during Block 79 closure when (a) goat-client parallel session was uninformed about Block 79's release intent + had to ask 6 coordination questions, and (b) Block 79 and Block 80 independently picked F-107 / F-108 in their tracks because neither bus-warned the other about finding-ID allocation.

**How to apply:**

Before `work release`, ask: "If another session pulled main right now / started new work tomorrow, what would they hit cold that they should know about?"

**Emit a bus message when ANY of these are true:**

1. **Cross-track ID collision risk** — you allocated a new ADR / finding / block number that's not in main yet. `bus warn` so a peer doesn't grab the same ID.
2. **Substrate change peer sessions rely on** — you bumped a binary, renamed a path, changed an env-var convention, retired an artifact, added a `.gitignore` exception. `bus info` so others pulling main pick it up cleanly.
3. **Renumbering / breaking rename** — you renamed a file, moved a doc, changed a slug that other branches reference. `bus warn`.
4. **Active-fleet incident or recovery** — you noticed an outage, recovered an infrastructure piece, or documented a transient flaky-substrate condition that's relevant to anyone touching that surface. `bus info` (or `urgent` if it's still degraded).
5. **Operator-laptop / workflow gotcha** — a footgun you hit that the next operator would hit too (bundle-corruption pattern, bare-cat-append, stale binary, etc.). `bus warn`.
6. **Coordination signals for known peer sessions** — if `active-work.yaml` shows a parallel track that's plausibly affected by your work, emit specifically for them. `bus info`.

**Skip emit when:**

- Your work was self-contained to one track with no cross-session surface
- You only edited internal-to-this-track files (track-specific design docs, internal notes)
- The information already lives in the merged commit message that everyone will see via `git log`
- It's a trivial doc fix or cleanup with no behavioral impact

**Format:**

- Lead with the **what** (verb-shaped: "X closed", "Y renumbered", "Z added")
- Include the **commit hash / merge SHA** if it's on main, so consumers can `git show <sha>` to inspect
- Include **specific paths or names** consumers will need to grep / reference (file paths, env-var names, finding IDs)
- Keep messages dense — bus messages surface in other sessions' prompts via UserPromptSubmit, so longer = more attention-cost for that session

**Levels:**

- **`info`** — routine FYI, peer can act on it whenever convenient
- **`warn`** — peer should adjust their work or check for collision before continuing
- **`urgent`** — active incident requiring immediate attention from anyone reading

**Example invocations:**

```bash
# Substrate change that peer sessions will benefit from
./ops/work/work bus info "Block 79 closed + merged to main as ab633ab7. ECDSA root at ops/enrollment/ca/dev/root-cert.pem; wg-cp0-bundle-apply accepts both PEM types (dc3944fe)." --force

# ID-renumbering warning for peers with cached references
./ops/work/work bus warn "Findings renumbered: F-107 → F-110 + F-108 → F-111 in Block 79 (collision with Block 80's F-107/F-109 on main). Branches with cached refs to F-107-ecdsa-*.md or F-108-avahi-*.md need to update." --force

# Active incident worth flagging
./ops/work/work bus info "Control-plane outage 11:49-12:17 UTC today, not session-induced. Recovered." --force
```

**`--force` note:** the 60-second rate-limit on identical (level, owner, msg) tuples is meant for noisy retries. Closing-out emissions are intentional + unique; passing `--force` skips the rate-limit. Don't `--force` if you're emitting in a loop — that's the wrong shape.

**Composes with:** track-release in `ops/work/work release`, `[track: <name>]` commit trailer, `recently_completed.summary` field in `docs/project/active-work.yaml`. The summary is the durable record; bus messages are the synchronous push to peers who are awake right now.
