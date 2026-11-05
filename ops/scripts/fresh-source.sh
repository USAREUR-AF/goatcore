#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# fresh-source.sh — contract clause C3 (fresh source) on a real run: refuse to
# touch a host from a checkout that is behind the source of record in what a
# deploy reads.
#
# docs/design/goatbuilder-conformance.md §C3: the declaration read is the
# current one on origin; a stale checkout is refused with the command to fix it.
# A deploy reads ops/ansible (roles, plays, group_vars, inventory, the rendered
# pins.yml) and ops/pins. Ansible renders whole files, so a role or group_vars
# file rendered from a tree missing a merged change silently removes what that
# change added on the host, and the play reports success.
#
# One check, both avenues (ADR 1201): ansible-preflight.sh runs it before every
# play; deploy-component.sh, rollback.sh, goat-standup.sh and the goatbuilder
# binary run it once before the first host is touched and pin the trunk commit
# it read (GOAT_FRESH_SOURCE_TRUNK), so every later play in the same run is
# judged against the same commit and a merge landing mid-run cannot stop a run
# half way.
#
#   behind <remote>/main in ops/ansible or ops/pins  -> REFUSE (GB-064), exit 1
#   ahead of it (this branch's own commits)          -> WARN, naming them, exit 0
#   current                                          -> exit 0
#   <remote> cannot be fetched                       -> REFUSE (GB-065), exit 1,
#                                                       unless --offline
#
# usage: fresh-source.sh [--repo DIR] [--remote NAME] [--offline] [--quiet]
#   --offline   an islanded site: do not fetch; judge against the cached
#               <remote>/main and say so. Also GOAT_DEPLOY_OFFLINE=1, which the
#               rollback-ledger role records in the row it writes.
#   --quiet     print nothing on success (the per-play call under a pinned trunk)
# Prints the trunk commit it judged against on stdout (empty when there is none).
#
# The trunk remote: --remote, else $GOAT_ANSIBLE_TRUNK_REMOTE, else a remote
# named `usareur-af` if one exists, else `origin` — the same order as
# ansible-preflight.sh. The trunk branch: $GOAT_ANSIBLE_TRUNK_BRANCH, else
# `main` — for a fork whose deploy trunk is another branch of its mirror.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
REMOTE=""; OFFLINE="${GOAT_DEPLOY_OFFLINE:-0}"; QUIET=0
while [ $# -gt 0 ]; do
  case "$1" in
    --repo) REPO="$2"; shift 2 ;;
    --remote) REMOTE="$2"; shift 2 ;;
    --offline) OFFLINE=1; shift ;;
    --quiet) QUIET=1; shift ;;
    -h|--help) sed -n '2,36p' "$0"; exit 0 ;;
    *) echo "fresh-source: unknown argument $1" >&2; exit 2 ;;
  esac
done
[ "$OFFLINE" = 1 ] || OFFLINE=0
PATHS=(ops/ansible ops/pins)

if [ -z "$REMOTE" ]; then
  if [ -n "${GOAT_ANSIBLE_TRUNK_REMOTE:-}" ]; then REMOTE="$GOAT_ANSIBLE_TRUNK_REMOTE"
  elif git -C "$REPO" remote get-url usareur-af >/dev/null 2>&1; then REMOTE=usareur-af
  else REMOTE=origin; fi
fi
BRANCH="${GOAT_ANSIBLE_TRUNK_BRANCH:-main}"
TRUNK="$REMOTE/$BRANCH"

branch="$(git -C "$REPO" symbolic-ref --quiet --short HEAD 2>/dev/null || echo "")"
if [ "$branch" = main ] || [ "$branch" = master ] || [ "$branch" = "$BRANCH" ]; then
  FIX="GOAT_ALLOW_MASTER_GIT_MUTATE=1 git -C $REPO pull --ff-only $REMOTE $BRANCH"
else
  FIX="git -C $REPO fetch $REMOTE $BRANCH && git -C $REPO rebase $TRUNK"
fi

# --- the trunk commit to judge against ------------------------------------------
SHA=""
if [ -n "${GOAT_FRESH_SOURCE_TRUNK:-}" ]; then
  # Pinned by the driver that ran this check before the first host: no fetch.
  SHA="$(git -C "$REPO" rev-parse --verify --quiet "${GOAT_FRESH_SOURCE_TRUNK}^{commit}" 2>/dev/null || echo "")"
  [ -n "$SHA" ] || { echo "fresh-source: GOAT_FRESH_SOURCE_TRUNK=$GOAT_FRESH_SOURCE_TRUNK is not a commit in $REPO" >&2; exit 2; }
elif [ "$OFFLINE" = 1 ]; then
  SHA="$(git -C "$REPO" rev-parse --verify --quiet "$TRUNK^{commit}" 2>/dev/null || echo "")"
  if [ -z "$SHA" ]; then
    echo "fresh-source: OFFLINE — no cached $TRUNK in this checkout, so there is nothing to judge it against; proceeding as an offline run." >&2
    exit 0
  fi
  echo "fresh-source: OFFLINE — not fetching $REMOTE; judging this checkout against the cached $TRUNK at $(git -C "$REPO" rev-parse --short "$SHA") ($(git -C "$REPO" log -1 --format=%cr "$SHA")). The run is recorded as offline." >&2
else
  # BatchMode and a connect timeout: an unreachable remote is an answer, not a hang.
  if ! GIT_SSH_COMMAND="${GIT_SSH_COMMAND:-ssh -o BatchMode=yes -o ConnectTimeout=10}" \
       git -C "$REPO" fetch --quiet "$REMOTE" "$BRANCH" 2>/dev/null; then
    {
      echo "fresh-source: GB-065 REFUSE — could not fetch $REMOTE $BRANCH, so this checkout cannot be shown current with the source of record (contract C3); refusing before any host is touched."
      echo "  fix: restore the connection to $REMOTE and re-run."
      echo "  On an islanded site, re-run with --offline: the run is then judged against the cached $TRUNK and recorded as offline."
    } >&2
    exit 1
  fi
  SHA="$(git -C "$REPO" rev-parse --verify --quiet "$TRUNK^{commit}" 2>/dev/null || echo "")"
  [ -n "$SHA" ] || { echo "fresh-source: GB-065 REFUSE — fetched $REMOTE, but it has no $BRANCH branch to judge against." >&2; exit 1; }
fi

# --- behind: the trunk carries changes to what a deploy reads that HEAD lacks -----
MISSING="$(git -C "$REPO" log --format='%h %s' "HEAD..$SHA" -- "${PATHS[@]}" 2>/dev/null || true)"
if [ -n "$MISSING" ]; then
  N="$(printf '%s\n' "$MISSING" | grep -c .)"
  {
    echo "fresh-source: GB-064 REFUSE — this checkout is behind $TRUNK in what a deploy reads (${PATHS[*]}); refusing before any host is touched (contract C3)."
    echo "  HEAD $(git -C "$REPO" rev-parse --short HEAD) (${branch:-detached HEAD}); $TRUNK $(git -C "$REPO" rev-parse --short "$SHA")"
    echo "  $N commit(s) on $TRUNK not in this checkout change those paths (newest first):"
    printf '%s\n' "$MISSING" | head -10 | sed 's/^/    /'
    if [ "$N" -gt 10 ]; then echo "    … and $((N - 10)) more"; fi
    echo "  A role, play or group_vars file rendered from this tree would remove on the host what those commits added, and the play would report success."
    echo "  fix: $FIX"
  } >&2
  exit 1
fi

# --- ahead: this branch's own commits go out with the run -------------------------
if [ "$QUIET" = 0 ]; then
  AHEAD="$(git -C "$REPO" log --format='%h %s' "$SHA..HEAD" 2>/dev/null || true)"
  if [ -n "$AHEAD" ]; then
    N="$(printf '%s\n' "$AHEAD" | grep -c .)"
    {
      echo "fresh-source: WARN — this checkout carries $N commit(s) not on $TRUNK; they are deployed with this run:"
      printf '%s\n' "$AHEAD" | head -5 | sed 's/^/    /'
      if [ "$N" -gt 5 ]; then echo "    … and $((N - 5)) more"; fi
    } >&2
  fi
fi
echo "$SHA"
