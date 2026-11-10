#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# repo-slug.sh — the GitHub repositories a checkout's pull requests live in.
#
#   ops/scripts/repo-slug.sh primary  [DIR]   # owner/repo of DIR's trunk remote
#   ops/scripts/repo-slug.sh pr-repos [DIR]   # every repo whose open PRs a review lists
#
# DIR defaults to the current directory's checkout, never this script's: the
# session tools that call it judge the clone a person is working in.
#
# primary: the remote named `usareur-af` when the clone has one, else `origin`,
# printed as owner/repo; `GH_REPO` wins when set (gh's own variable).
# pr-repos: `GOAT_PR_REPOS` (space-separated) when set; otherwise the primary,
# and, when DIR is an overlay (it carries goatcore.ref at its root), the three
# public code repositories it builds with.
#
# Exit 0 with output; exit 1 when no slug can be derived.
set -uo pipefail

MODE="${1:-primary}"
DIR="${2:-.}"
TOP="$(git -C "$DIR" rev-parse --show-toplevel 2>/dev/null || true)"

slug_of() {
  local url owner repo
  url="$(git -C "$TOP" remote get-url "$1" 2>/dev/null)" || return 1
  url="${url%/}"; url="${url%.git}"
  repo="${url##*/}"; owner="${url%/*}"; owner="${owner##*/}"; owner="${owner##*:}"
  [ -n "$owner" ] && [ -n "$repo" ] || return 1
  printf '%s/%s\n' "$owner" "$repo"
}

primary() {
  if [ -n "${GH_REPO:-}" ]; then printf '%s\n' "$GH_REPO"; return 0; fi
  [ -n "$TOP" ] || return 1
  if git -C "$TOP" remote get-url usareur-af >/dev/null 2>&1; then slug_of usareur-af
  else slug_of origin; fi
}

case "$MODE" in
  primary) primary ;;
  pr-repos)
    # shellcheck disable=SC2086 # a space-separated list, split on purpose
    if [ -n "${GOAT_PR_REPOS:-}" ]; then printf '%s\n' $GOAT_PR_REPOS; exit 0; fi
    p="$(primary)" || exit 1
    printf '%s\n' "$p"
    if [ -n "$TOP" ] && [ -f "$TOP/goatcore.ref" ]; then
      printf '%s\n' USAREUR-AF/goatcore USAREUR-AF/goatdocs USAREUR-AF/goatstuff
    fi ;;
  -h|--help) sed -n '3,16p' "$0" ;;
  *) echo "usage: repo-slug.sh primary|pr-repos [DIR]" >&2; exit 2 ;;
esac
