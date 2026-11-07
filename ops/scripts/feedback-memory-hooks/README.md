# feedback-memory-hooks

The seed memories for multi-agent sessions (master worktree read-only, handoff
at pause points, bus emits, and the rest) live in the agent-workflow plugin:
the dots repository, `claude/plugins/agent-workflow/memory-seeds/`. Copy them into
this repo's Claude Code memory directory with `aw-seed-memories`, which never
overwrites a memory already there:

```bash
ops/scripts/dots-tool aw-seed-memories            # or: make multi-agent-substrate-memory-install
ops/scripts/dots-tool aw-seed-memories --dry-run  # list what it would copy
```
