#!/usr/bin/env bash
# worktree-reap.sh — runs `worktree-reap` from dots (agent-tools/bin/worktree-reap); see ops/scripts/dots-tool.
s="${BASH_SOURCE[0]}"
while [ -L "$s" ]; do l="$(readlink "$s")"; case "$l" in /*) s="$l" ;; *) s="$(dirname "$s")/$l" ;; esac; done
exec "$(dirname "$s")/dots-tool" worktree-reap "$@"
