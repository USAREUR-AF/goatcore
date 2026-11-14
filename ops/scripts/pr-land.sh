#!/usr/bin/env bash
# pr-land.sh — commit, push, open the PR, wait for its checks, merge it, and
# return the merge commit. The leg every actuating driver (fastlane.sh, rc.sh)
# needs and used to print for the operator to do by hand.
#
# usage:
#   pr-land.sh --worktree <path> --branch <name> --title <t> --body-file <f> \
#              [--paths <p> ...] [--dry-run]
#
# The worktree is a linked worktree on <branch> (the git guard refuses bare git
# on the master tree; every git call here carries -C). --paths defaults to
# everything modified/added under the worktree. Merge is squash with branch
# deletion; this repository does not allow auto-merge, so the merge is
# performed here after the checks report, with --admin as the fallback the
# repository's protection needs. A failed check stops the leg and names it.
set -euo pipefail
WT=""; BR=""; TITLE=""; BODY=""; DRY=0; PATHS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --worktree) WT="$2"; shift 2 ;;
    --branch)   BR="$2"; shift 2 ;;
    --title)    TITLE="$2"; shift 2 ;;
    --body-file) BODY="$2"; shift 2 ;;
    --paths)    shift; while [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; do PATHS+=("$1"); shift; done ;;
    --dry-run)  DRY=1; shift ;;
    -h|--help)  sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "pr-land: unknown arg $1" >&2; exit 2 ;;
  esac
done
[ -n "$WT" ] && [ -n "$BR" ] && [ -n "$TITLE" ] && [ -n "$BODY" ] || { echo "pr-land: --worktree, --branch, --title, --body-file are required" >&2; exit 2; }
REPO_SLUG="$(git -C "$WT" remote get-url origin | sed -E 's#.*[:/]([^/]+/[^/]+)\.git$#\1#')"
say() { printf '    pr-land: %s\n' "$*"; }
if [ "$DRY" = 1 ]; then
  say "DRY: would commit $( [ ${#PATHS[@]} -gt 0 ] && echo "${PATHS[*]}" || echo "all changes") on $BR in $WT, push, open PR '$TITLE' on $REPO_SLUG, wait for checks, squash-merge"
  exit 0
fi
if [ ${#PATHS[@]} -gt 0 ]; then git -C "$WT" add -- "${PATHS[@]}"; else git -C "$WT" add -A; fi
# A staged change of nothing but detached signatures is a re-sign of an
# unchanged body (ECDSA is fresh every time): not a change to land. Five such
# PRs merged in one morning (#2019 #2023 #2024 #2025 #2028) before
# the drivers stopped re-signing unchanged intents; this is the chokepoint.
if ! git -C "$WT" diff --cached --quiet && [ -z "$(git -C "$WT" diff --cached --name-only | grep -v '\.sig$' || true)" ]; then
  git -C "$WT" reset -q -- . ; git -C "$WT" checkout -q -- . 2>/dev/null || true
  say "only signatures changed — nothing to land"
fi
git -C "$WT" fetch -q origin main
if git -C "$WT" diff --cached --quiet && git -C "$WT" diff --quiet origin/main -- 2>/dev/null; then
  SHA="$(git -C "$WT" rev-parse --short=12 origin/main)"
  say "nothing to land — the worktree matches main ($SHA)"
  printf '%s\n' "$SHA"
  exit 0
fi
if git -C "$WT" diff --cached --quiet; then say "nothing to commit"; else
  # subject = --title, body = --body-file; -F "$BODY" alone made the body's
  # first line the subject and lost the title.
  MSG="$(mktemp)"; { printf '%s\n\n' "$TITLE"; cat "$BODY"; } > "$MSG"
  # a track/* branch carries the ADR 0006 trailer on every commit (drift-check asserts it)
  case "$BR" in track/*) printf '\n[track: %s]\n' "${BR#track/}" >> "$MSG" ;; esac
  git -C "$WT" commit -q -s -F "$MSG"; rm -f "$MSG"
fi
# judge the push by its exit status; a hook's advisory text may contain "error".
git -C "$WT" push -q -u origin "$BR" || { echo "pr-land: push failed" >&2; exit 1; }
N="$(gh pr list -R "$REPO_SLUG" --head "$BR" --json number -q '.[0].number' 2>/dev/null || true)"
if [ -z "$N" ]; then
  URL="$(gh pr create -R "$REPO_SLUG" --base main --head "$BR" --title "$TITLE" --body-file "$BODY" 2>&1 | tail -1)"
  N="${URL##*/}"
fi
say "PR #$N — waiting for checks"
sleep 45
gh pr checks "$N" -R "$REPO_SLUG" --watch --interval 20 >/dev/null 2>&1 || true
if gh pr checks "$N" -R "$REPO_SLUG" 2>/dev/null | awk -F'\t' '{print $2}' | grep -q '^fail$'; then
  echo "pr-land: PR #$N has failing checks:" >&2
  gh pr checks "$N" -R "$REPO_SLUG" 2>/dev/null | awk -F'\t' '$2=="fail"{print "  " $1 "  " $4}' >&2
  exit 1
fi
gh pr merge "$N" -R "$REPO_SLUG" --squash --delete-branch 2>/dev/null \
  || gh pr merge "$N" -R "$REPO_SLUG" --squash --delete-branch --admin 2>&1 | tail -1 >&2
git -C "$WT" fetch -q origin main
SHA="$(git -C "$WT" rev-parse --short=12 origin/main)"
say "merged #$N — main is $SHA"
printf '%s\n' "$SHA"
