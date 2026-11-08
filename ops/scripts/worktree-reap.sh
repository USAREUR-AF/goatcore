#!/usr/bin/env bash
# worktree-reap.sh — reclaim stale per-session git worktrees safely.
#
# WHY THIS EXISTS
#   `work release` already tears down the worktree for a track via
#   cleanupWorktree() (Block 48Q, ops/work/main.go). But that only fires when
#   `work release <track>` is actually called AND the track metadata recorded
#   mode=worktree. In practice worktrees accumulate because:
#     - a PR squash-merges but nobody runs `work release` (track stays held);
#     - ad-hoc worktrees (image-bumps, pin-bumps, deploy shims, detached HEADs)
#       are created by hand, never registered as tracks, so release never
#       knows they exist;
#     - F-118 orphans: release ran but metadata mode!=worktree, so auto-cleanup
#       skipped with only a stderr warning.
#   This script is the periodic SWEEP that catches all of those. It is the
#   durable codification of the manual 51->3 cleanup .
#
# SAFETY CONTRACT (the whole point)
#   Never destroy work that exists only locally. A worktree is removed only
#   when its work is preserved elsewhere:
#     - merged into origin/main, OR
#     - already pushed to origin/<branch>, OR
#     - pushed by this script (a `wip:` snapshot is committed first if dirty).
#   A full local working-tree copy is expensive; a remote branch ref is cheap.
#   Preserve the work on the remote, then drop the local copy.
#
# WHAT GETS REAPED (the ACTION column = exactly what --apply will do)
#   REAP        merged to origin/main and clean; or a stale admin entry whose
#               dir is gone; or a detached HEAD already contained in
#               origin/main. Work is in main — safe. Merged-detection is
#               git-cherry (patch-id) PLUS, when gh is available, a merged-PR
#               head-SHA match — cherry alone misses multi-commit branches
#               that were squash-merged (they would HOLD forever).
#   PUSH+REAP   unmerged local commits and/or dirty tree AND idle beyond
#               --idle-days: push to origin (wip snapshot if dirty), then remove.
#   HOLD        safe but retained: unmerged-yet-pushed, or local work not idle
#               enough to reclaim. Shown so you can see it; not touched.
#   KEEP        never touched: the worktree you're standing in, or an open PR
#               (with --skip-open-prs).
#
# USAGE
#   ops/scripts/worktree-reap.sh                    # dry-run: show plan
#   ops/scripts/worktree-reap.sh --apply            # reap MERGED + stale only
#   ops/scripts/worktree-reap.sh --apply --idle-days 14
#                                                   # also reclaim idle (>14d)
#                                                   # safe/local-only worktrees,
#                                                   # pushing them to origin first
#   ops/scripts/worktree-reap.sh --skip-open-prs    # never touch live-PR worktrees (needs gh)
#   ops/scripts/worktree-reap.sh --apply --yes      # no confirmation prompt
#   ops/scripts/worktree-reap.sh --keep-branches    # remove worktrees, keep local branch refs
#
# NOTE: bare `grep` is shadowed by a shell-function shim in this environment
# (it execs the claude binary in subshells and silently kills loops). This
# script uses `command grep` throughout to bypass it.

set -uo pipefail

APPLY=0; ASSUME_YES=0; KEEP_BRANCHES=0; IDLE_DAYS=0; SKIP_OPEN_PRS=0

while [ $# -gt 0 ]; do
  case "$1" in
    --apply) APPLY=1 ;;
    --yes|-y) ASSUME_YES=1 ;;
    --keep-branches) KEEP_BRANCHES=1 ;;
    --skip-open-prs) SKIP_OPEN_PRS=1 ;;
    --idle-days) shift; IDLE_DAYS="${1:-0}" ;;
    --idle-days=*) IDLE_DAYS="${1#*=}" ;;
    -h|--help) command sed -n '2,/^set -uo/p' "$0" | command sed '$d; s/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
  shift
done

# Master worktree root (NOT --show-toplevel, which returns the child root when
# run from inside a worktree). `git worktree remove` must run from here.
COMMON_DIR="$(git rev-parse --git-common-dir 2>/dev/null)" || { echo "not in a git repository" >&2; exit 1; }
COMMON_DIR="$(cd "$COMMON_DIR" && pwd)"
MASTER_ROOT="$(dirname "$COMMON_DIR")"
CWD_REAL="$(pwd -P 2>/dev/null || pwd)"

# EVERY remote, not just origin. Since USAREUR-AF/goatcore became primary
# a checkout's `origin` may be the MIRROR: in the goat-maintainer clone
# origin is goat-maintainer and the primary is the `usareur-af` remote. A branch pushed
# to the primary then has no refs/remotes/origin/<branch>, so this script read
# it as "local-only: N unmerged" and HELD every worktree — failing safe, but
# accumulating exactly what it exists to reclaim.
REMOTES="$(git -C "$MASTER_ROOT" remote 2>/dev/null)"
[ -z "$REMOTES" ] && REMOTES="origin"
for _r in $REMOTES; do
  git -C "$MASTER_ROOT" fetch "$_r" main --quiet 2>/dev/null || \
    echo "warning: could not fetch $_r/main; merged-detection may be stale" >&2
done

# The GitHub repo slug for a remote, for `gh --repo`. Without this, `gh pr list`
# queries whatever repo the cwd points at, which after the flip is the wrong one.
remote_slug() {
  git -C "$MASTER_ROOT" remote get-url "$1" 2>/dev/null \
    | sed -E 's|\.git/?$||; s|^.*[:/]([^/]+/[^/]+)$|\1|'
}
NOW="$(date +%s)"

# Plan rows: action<TAB>branch<TAB>path<TAB>reason
PLAN="$(mktemp)"; trap 'rm -f "$PLAN"' EXIT

# live_pids_in DIR — pids whose current directory is DIR or below it, as
# "pid(comm)" words; empty when none. lsof where present (macOS, most Linux),
# /proc otherwise. Never fails the run: no probe means no pids.
live_pids_in() {
  local dir="$1" out=""
  if command -v lsof >/dev/null 2>&1; then
    out="$(lsof -a -d cwd -Fpcn 2>/dev/null | awk -v d="$dir/" '
      /^p/ {pid=substr($0,2)} /^c/ {comm=substr($0,2)}
      /^n/ {n=substr($0,2) "/"; if (index(n,d)==1) printf "%s(%s) ", pid, comm}')"
  elif [ -d /proc ]; then
    local l t
    for l in /proc/[0-9]*/cwd; do
      t="$(readlink "$l" 2>/dev/null)" || continue
      case "$t/" in "$dir/"*) out="$out${l#/proc/}"; out="${out%/cwd}($(cat "${l%/cwd}/comm" 2>/dev/null)) " ;; esac
    done
  fi
  printf '%s' "${out% }"
}

emit() { # path  branch(may be empty)  head
  local path="$1" branch="$2" head="$3"
  [ -z "$path" ] && return
  [ "$path" = "$MASTER_ROOT" ] && return                      # never master

  local real; real="$(cd "$path" 2>/dev/null && pwd -P)" || real="$path"
  case "$CWD_REAL/" in "$real/"*)                             # never our own cwd
    printf 'KEEP\t%s\t%s\tcurrent worktree (cwd)\n' "${branch:-(detached)}" "$path" >> "$PLAN"; return ;;
  esac

  if [ ! -d "$path" ]; then                                  # stale admin entry
    printf 'REAP\t%s\t%s\tdir gone from disk (prune)\n' "${branch:-(detached)}" "$path" >> "$PLAN"; return
  fi

  # A worktree that is the cwd of a running process is in use whatever its
  # merge state says: an ansible play, a go test, another session's shell.
  # Removing it mid-run kills the process with a bare "No such file or
  # directory" that names no cause (a live deploy died this way).
  local live; live="$(live_pids_in "$real")"
  if [ -n "$live" ]; then
    printf 'KEEP\t%s\t%s\tin use by running process(es) %s\n' "${branch:-(detached)}" "$path" "$live" >> "$PLAN"; return
  fi

  local dirty; dirty="$(git -C "$path" status --porcelain 2>/dev/null)"

  if [ -z "$branch" ]; then                                  # detached HEAD
    if git -C "$MASTER_ROOT" merge-base --is-ancestor "$head" origin/main 2>/dev/null && [ -z "$dirty" ]; then
      printf 'REAP\t(detached %.10s)\t%s\tdetached HEAD already in origin/main\n' "$head" "$path" >> "$PLAN"
    else
      printf 'KEEP\t(detached %.10s)\t%s\tdetached HEAD not in main / dirty — inspect manually\n' "$head" "$path" >> "$PLAN"
    fi
    return
  fi

  if [ "$SKIP_OPEN_PRS" = "1" ] && command -v gh >/dev/null 2>&1; then
    local pr; pr="$(gh pr list --state open --head "$branch" --json number --jq '.[0].number' 2>/dev/null)"
    [ -n "$pr" ] && { printf 'KEEP\t%s\t%s\topen PR #%s\n' "$branch" "$path" "$pr" >> "$PLAN"; return; }
  fi

  # Age of the branch's tip (for idle gating).
  local last age; last="$(git -C "$path" log -1 --format=%ct "$branch" 2>/dev/null || echo "$NOW")"
  age=$(( (NOW - last) / 86400 ))

  # Merged? patch-id based (squash/rebase safe) — same technique as work-release.
  # Merged into ANY remote's main counts as merged — the remotes are two views
  # of one history, and work merged on the primary is safe even before the
  # mirror is synced.
  local unmerged=999 merged_into="" _u
  for _r in $REMOTES; do
    git -C "$MASTER_ROOT" show-ref --verify --quiet "refs/remotes/$_r/main" || continue
    _u="$(git -C "$MASTER_ROOT" cherry "$_r/main" "$branch" 2>/dev/null | command grep -c '^+ ' || true)"
    _u="${_u:-999}"
    if [ "$_u" -lt "$unmerged" ]; then unmerged="$_u"; merged_into="$_r/main"; fi
  done
  [ "$unmerged" = 999 ] && unmerged=0

  # git-cherry cannot see a multi-commit branch that was squash-merged: the
  # squashed commit's patch-id matches no individual commit, so every commit
  # reads "+ unmerged" and the worktree HOLDs forever (sweep: 10 of
  # 16 provably-merged worktrees were invisible this way). Second proof: a
  # MERGED PR whose head SHA equals this branch tip — everything local was in
  # exactly what merged, so the work is in main.
  local squash_pr=""
  if [ "$unmerged" -gt 0 ] && command -v gh >/dev/null 2>&1; then
    local tip; tip="$(git -C "$MASTER_ROOT" rev-parse "refs/heads/$branch" 2>/dev/null)"
    if [ -n "$tip" ]; then
      for _r in $REMOTES; do
        local _slug; _slug="$(remote_slug "$_r")"
        [ -z "$_slug" ] && continue
        squash_pr="$(gh pr list --repo "$_slug" --state merged --head "$branch" --json number,headRefOid \
          --jq ".[] | select(.headRefOid == \"$tip\") | .number" 2>/dev/null | command head -1)"
        [ -n "$squash_pr" ] && { unmerged=0; merged_into="$_slug"; break; }
      done
    fi
  fi

  # Present on ANY remote, and are we ahead of it? Safe on either remote is safe.
  local has_remote=0 ahead=0 safe_on=""
  for _r in $REMOTES; do
    if git -C "$path" show-ref --verify --quiet "refs/remotes/$_r/$branch"; then
      local _ahead; _ahead="$(git -C "$path" rev-list --count "$_r/$branch..$branch" 2>/dev/null || echo 0)"
      if [ "$has_remote" -eq 0 ] || [ "$_ahead" -lt "$ahead" ]; then
        has_remote=1; ahead="$_ahead"; safe_on="$_r/$branch"
      fi
    fi
  done

  local idle_ok=0
  { [ "$IDLE_DAYS" -gt 0 ] && [ "$age" -gt "$IDLE_DAYS" ]; } && idle_ok=1

  if [ "$unmerged" -eq 0 ] && [ -z "$dirty" ]; then
    local how="fully merged to ${merged_into:-origin/main}"
    [ -n "$squash_pr" ] && how="squash-merged via PR #$squash_pr (tip == merged PR head)"
    printf 'REAP\t%s\t%s\t%s (idle %dd)\n' "$branch" "$path" "$how" "$age" >> "$PLAN"
  elif [ -z "$dirty" ] && [ "$has_remote" -eq 1 ] && [ "$ahead" -eq 0 ]; then
    # Unmerged but already safe on origin. Reclaim only when idle.
    if [ "$idle_ok" = 1 ]; then
      printf 'PUSH+REAP\t%s\t%s\tsafe on %s, idle %dd — drop local copy\n' "$branch" "$path" "$safe_on" "$age" >> "$PLAN"
    else
      printf 'HOLD\t%s\t%s\tsafe on %s, idle %dd (reclaim with --idle-days)\n' "$branch" "$path" "$safe_on" "$age" >> "$PLAN"
    fi
  else
    # Local-only state (unmerged unpushed commits and/or dirty). Reclaim only
    # when idle; when we do, push (snapshotting dirt) before removing.
    local what="local-only:"
    [ "$unmerged" -gt 0 ] && what="$what ${unmerged} unmerged"
    [ "$ahead" -gt 0 ] && what="$what ${ahead} unpushed"
    [ -n "$dirty" ] && what="$what $(printf '%s\n' "$dirty" | command grep -c . ) dirty"
    if [ "$idle_ok" = 1 ]; then
      printf 'PUSH+REAP\t%s\t%s\t%s, idle %dd — push then drop\n' "$branch" "$path" "$what" "$age" >> "$PLAN"
    else
      printf 'HOLD\t%s\t%s\t%s, idle %dd (push+reclaim with --idle-days)\n' "$branch" "$path" "$what" "$age" >> "$PLAN"
    fi
  fi
}

cur_path="" cur_branch="" cur_head=""
while IFS= read -r line; do
  case "$line" in
    "worktree "*) cur_path="${line#worktree }" ;;
    "HEAD "*)     cur_head="${line#HEAD }" ;;
    "branch "*)   cur_branch="${line#branch refs/heads/}" ;;
    "detached")   cur_branch="" ;;
    "")           emit "$cur_path" "$cur_branch" "$cur_head"; cur_path="" cur_branch="" cur_head="" ;;
  esac
done < <(git -C "$MASTER_ROOT" worktree list --porcelain)
emit "$cur_path" "$cur_branch" "$cur_head"

echo
printf '%-10s %-46s %s\n' ACTION BRANCH REASON
printf '%-10s %-46s %s\n' '------' '------' '------'
command sort "$PLAN" | while IFS=$'\t' read -r act br path reason; do
  printf '%-10s %-46s %s\n' "$act" "$br" "$reason"
done
echo
echo "summary: $(command awk -F'\t' '{c[$1]++} END{for(a in c) printf "%s=%d ", a, c[a]}' "$PLAN")"
[ "$IDLE_DAYS" -eq 0 ] && echo "(no --idle-days: only merged/stale worktrees will be reaped; HOLD rows retained)"

if [ "$APPLY" != "1" ]; then
  echo; echo "(dry-run — re-run with --apply to execute)"
  exit 0
fi

if [ "$ASSUME_YES" != "1" ]; then
  printf '\nProceed with --apply? [y/N] '; read -r ans
  case "$ans" in y|Y|yes|YES) ;; *) echo "aborted."; exit 0 ;; esac
fi

reap_worktree() { git -C "$MASTER_ROOT" worktree remove --force "$1" 2>/dev/null || rm -rf "$1"; }
del_branch() { [ "$KEEP_BRANCHES" = 1 ] && return; [ -z "$1" ] && return
  git -C "$MASTER_ROOT" branch -D "$1" >/dev/null 2>&1 || true; }

while IFS=$'\t' read -r act br path reason; do
  case "$act" in
    KEEP|HOLD) ;;
    REAP)
      reap_worktree "$path" && echo "REAPED      $br"
      case "$br" in "(detached"*) ;; *) del_branch "$br" ;; esac ;;
    PUSH+REAP)
      echo "PRESERVE    $br — pushing to origin before removal"
      if [ -n "$(git -C "$path" status --porcelain 2>/dev/null)" ]; then
        git -C "$path" add -A
        git -C "$path" commit -s --no-verify \
          -m "wip(${br#track/}): snapshot uncommitted work before worktree reap" >/dev/null 2>&1 || true
      fi
      if git -C "$path" push -u origin "HEAD:$br" >/dev/null 2>&1; then
        reap_worktree "$path" && echo "REAPED      $br (preserved at origin/$br)"
        del_branch "$br"
      else
        echo "  SKIP — push failed for $br; worktree left in place (no data dropped)"
      fi ;;
  esac
done < <(command sort "$PLAN")

git -C "$MASTER_ROOT" worktree prune
echo; echo "done. remaining worktrees:"; git -C "$MASTER_ROOT" worktree list
