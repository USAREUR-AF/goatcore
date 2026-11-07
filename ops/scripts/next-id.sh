#!/usr/bin/env bash
# next-id.sh — runs `next-id` from dots (agent-tools/bin/next-id); see ops/scripts/dots-tool.
s="${BASH_SOURCE[0]}"
while [ -L "$s" ]; do l="$(readlink "$s")"; case "$l" in /*) s="$l" ;; *) s="$(dirname "$s")/$l" ;; esac; done
exec "$(dirname "$s")/dots-tool" next-id "$@"
