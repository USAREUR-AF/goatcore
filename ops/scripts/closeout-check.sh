#!/usr/bin/env bash
# closeout-check.sh — the cleanliness half of a close-out, run rather than assumed.
#
# The failure this exists for: a session BELIEVING it finished. Close-outs have
# reported "clean" while leaving worktrees on disk, uncommitted files, unmerged
# PRs and key material in a scratchpad. The verdict is only worth something if
# the checks actually ran, so this script is the running of them.
#
# usage:
#   ops/scripts/closeout-check.sh                # whole-repo verdict
#   ops/scripts/closeout-check.sh --track <name> # also judge one track's PR/branch
#
# Exit 0 = CLEAN. Exit 1 = NOT CLEAN, with every outstanding item named.
set -uo pipefail

TRACK=""
[ "${1:-}" = "--track" ] && TRACK="${2:-}"

R=$(git rev-parse --show-toplevel 2>/dev/null) || { echo "not a git repo" >&2; exit 2; }
MASTER=$(git -C "$R" worktree list --porcelain | awk '/^worktree /{print $2; exit}')
# The repository this checkout's pull requests live in: GH_REPO, else the
# clone's trunk remote (ops/scripts/repo-slug.sh), else repo.source in
# ops/goatbuilder/goatbuilder.yaml (project-setting.sh).
REPO="$("${BASH_SOURCE[0]%/*}/repo-slug.sh" primary "$R" 2>/dev/null)"
[ -n "$REPO" ] || REPO="$("${BASH_SOURCE[0]%/*}/project-setting.sh" repo source 2>/dev/null)"
[ -n "$REPO" ] || { echo "closeout-check: no repository: set GH_REPO=<owner>/<name>, or declare repo.source in ops/goatbuilder/goatbuilder.yaml" >&2; exit 2; }
OUT=0
note() { printf '  %s\n' "$*"; }
bad()  { printf '  OUTSTANDING: %s\n' "$*"; OUT=1; }

# Every network call runs under a bound (#1356): an unbounded gh or git
# remote call left the script silent for 40 minutes. A call that hits the
# bound is OUTSTANDING, never a pass, and so is a call that fails for any
# other reason. timeout(1) is GNU; macOS has it as gtimeout via coreutils.
# A stock macOS has neither, so without them a shell watchdog holds the
# bound instead: it runs the call in the background, and at the bound kills
# the call's children and then the call, and reports 124 as timeout(1) does.
# timeout(1) kills the call's whole process group; the watchdog reaches one
# level down, which covers gh and git's remote helpers.
NET_TIMEOUT="${CLOSEOUT_NET_TIMEOUT:-20}"
TIMEOUT_BIN=""
if command -v timeout >/dev/null 2>&1; then TIMEOUT_BIN=timeout
elif command -v gtimeout >/dev/null 2>&1; then TIMEOUT_BIN=gtimeout; fi
watchdog_bounded() {
  local pid dog rc marker
  # A marker file, not the watchdog's exit code, records the timeout. The dog
  # writes it BEFORE it kills the call, so whichever of the dog and the parent
  # wins the kill/wait race, a fired watchdog is recorded exactly once. The old
  # `wait "$dog"; [ $? -eq 124 ]` lost that race under load (#1529): the
  # parent's `kill -TERM "$dog"` could reach the dog's TERM trap before its
  # `exit 124`, so the dog exited 0 and a real timeout was misreported as the
  # call's SIGTERM exit (143). No mktemp: this path also runs on the minimal
  # PATH of the no-timeout-binary fallback, where printf and `[ -s ]` are
  # builtins and only a writable TMPDIR is needed.
  "$@" &
  pid=$!
  marker="${TMPDIR:-/tmp}/closeout-wd.$$.$pid"
  : > "$marker"
  # The watchdog's output goes to /dev/null so its sleep never holds open
  # the pipe of a $(...) that called bounded.
  ( s=""
    trap '[ -n "$s" ] && kill "$s" 2>/dev/null; exit 0' TERM
    sleep "$NET_TIMEOUT" & s=$!
    wait "$s"
    printf timeout > "$marker"
    command -v pkill >/dev/null 2>&1 && pkill -TERM -P "$pid" 2>/dev/null
    kill -TERM "$pid" 2>/dev/null
    exit 0 ) >/dev/null 2>&1 &
  dog=$!
  wait "$pid"
  rc=$?
  kill -TERM "$dog" 2>/dev/null
  wait "$dog" 2>/dev/null
  [ -s "$marker" ] && rc=124
  rm -f "$marker"
  return "$rc"
}
bounded() {
  if [ -n "$TIMEOUT_BIN" ]; then "$TIMEOUT_BIN" "$NET_TIMEOUT" "$@"; else watchdog_bounded "$@"; fi
}
timed_out() { [ "$1" -eq 124 ]; }

echo "==> closeout-check ${TRACK:+(track: $TRACK) }on $REPO"
[ -z "$TIMEOUT_BIN" ] && note "no timeout/gtimeout on PATH — a shell watchdog bounds the network calls below"
note "fetching origin (bound ${NET_TIMEOUT}s)"
bounded git -C "$R" fetch origin --quiet 2>/dev/null
rc=$?
if timed_out "$rc"; then bad "git fetch origin timed out after ${NET_TIMEOUT}s — origin/main comparisons below may be stale"
elif [ "$rc" -ne 0 ]; then bad "git fetch origin exited $rc — origin/main comparisons below may be stale"; fi

echo
echo "1. worktrees"
WT=$(git -C "$R" worktree list --porcelain | grep -c '^worktree ')
if [ "$WT" -le 1 ]; then note "only the master worktree — nothing to reap"; else
  git -C "$R" worktree list | tail -n +2 | while read -r p rest; do note "$p $rest"; done
  bad "$((WT-1)) worktree(s) still on disk (ops/scripts/worktree-reap.sh --apply, after confirming content is in main)"
fi

echo
echo "2. uncommitted work in the master tree"
DIRTY=$(git -C "$MASTER" status --porcelain | grep -v 'active-work' | wc -l | tr -d ' ')
if [ "$DIRTY" -eq 0 ]; then note "clean (active-work yaml excluded — that churn is normal)"; else
  git -C "$MASTER" status --short | grep -v 'active-work' | head -12 | while read -r l; do note "$l"; done
  bad "$DIRTY uncommitted/untracked entr(ies) in $MASTER"
fi

echo
echo "3. open PRs"
if command -v gh >/dev/null 2>&1; then
  MINE=""
  ME=$(bounded gh api user --jq .login 2>/dev/null)
  rc=$?
  if [ "$rc" -eq 0 ]; then
    MINE=$(GH_REPO="$REPO" bounded gh pr list --state open --limit 60 --json number,title,author,headRefName \
             --jq ".[] | select(.author.login == \"$ME\") | \"#\(.number) \(.headRefName) — \(.title)\"" 2>/dev/null)
    rc=$?
  fi
  if timed_out "$rc"; then bad "gh timed out after ${NET_TIMEOUT}s — PR check NOT run (this is NOT a pass)"
  elif [ "$rc" -ne 0 ]; then bad "gh exited $rc — PR check NOT run (this is NOT a pass)"
  elif [ -z "$MINE" ]; then note "none open under $ME"; else
    echo "$MINE" | while read -r l; do note "$l"; done
    bad "$(echo "$MINE" | wc -l | tr -d ' ') PR(s) still open"
  fi
else
  note "gh not on PATH — PR check skipped (this is NOT a pass)"; OUT=1
fi

echo
echo "4. remote track/* branches"
# Scoped to THIS close-out. A repo this size carries ~100 abandoned track
# branches from other sessions and other months; judging them here would make
# every close-out "not clean" for work that is not yours. With --track, that
# branch is judged and the rest are counted. Without it, everything is counted.
#
# Judgement is by CONTENT, not reachability: squash-merge makes the merged
# commit a new object, so --is-ancestor and --contains fail by construction on
# branches that ARE merged.
LSR=$(bounded git -C "$R" ls-remote origin 'refs/heads/track/*' 2>/dev/null)
rc=$?
BR=$(echo "$LSR" | awk '{print $2}' | sed 's|refs/heads/||')
TOTAL=$(echo "$BR" | grep -c . || echo 0)
if timed_out "$rc"; then bad "git ls-remote timed out after ${NET_TIMEOUT}s — track branches NOT checked"
elif [ "$rc" -ne 0 ]; then bad "git ls-remote exited $rc — track branches NOT checked (this is NOT a pass)"
elif [ "$TOTAL" -eq 0 ]; then note "none"; else
  MINE=""
  [ -n "$TRACK" ] && MINE=$(echo "$BR" | grep -x "track/$TRACK" || true)
  if [ -n "$MINE" ]; then
    files=$(git -C "$R" diff --name-only "origin/main...origin/$MINE" 2>/dev/null)
    resid=$(git -C "$R" diff --name-only origin/main "origin/$MINE" -- $files 2>/dev/null)
    if [ -z "$resid" ]; then
      note "$MINE — content IS in main (squash-merged); delete it"
      bad "$MINE still exists on origin after merge"
    else
      n=$(echo "$resid" | grep -c .)
      bad "$MINE carries $n file(s) not in main, e.g. $(echo "$resid" | head -3 | tr '\n' ' ')"
    fi
  elif [ -n "$TRACK" ]; then
    note "track/$TRACK — no branch on origin (merged and deleted, or never pushed)"
  fi
  note "$TOTAL track/* branch(es) on origin in total — repo hygiene, not this close-out"
  note "sweep separately: ops/scripts/worktree-reap.sh"
fi

echo "5. the named track's work is in main"
if [ -n "$TRACK" ]; then
  PRS=$(GH_REPO="$REPO" bounded gh pr list --head "track/$TRACK" --state all --limit 10 \
          --json number,state,title --jq '.[] | "#\(.number) \(.state) \(.title)"' 2>/dev/null)
  rc=$?
  if timed_out "$rc"; then bad "gh timed out after ${NET_TIMEOUT}s — track/$TRACK PR state NOT checked"
  elif [ "$rc" -ne 0 ]; then bad "gh exited $rc — track/$TRACK PR state NOT checked (this is NOT a pass)"
  elif [ -z "$PRS" ]; then note "no PR was opened from track/$TRACK"; else
    echo "$PRS" | while read -r l; do note "$l"; done
    echo "$PRS" | grep -q ' OPEN ' && bad "the PR from track/$TRACK is still open"
  fi
  Y="$R/docs/project/active-work/tracks/${TRACK}.yaml"
  [ -f "$Y" ] && bad "track yaml still under tracks/ — ops/work/work release $TRACK moves it to recently_completed/"
else
  note "no --track given; skipped"
fi

echo
echo "6. key material left in scratchpads"
SCRATCH="${TMPDIR:-/tmp}"
# Prune before reading, not after (#1356): build trees under the scratch dir
# (cargo target/, node caches) run to gigabytes, and grepping them first then
# filtering the paths read every byte. A PEM key file is small; the size cap
# keeps images and archives out of the read, and it means a key pasted into a
# file of 10 MiB or more is not seen.
KEYS=$(find "$SCRATCH" \( -name target -o -name node-compile-cache -o -name node_modules -o -name .git \) -prune \
         -o -type f -size -10240k \
         ! -name '*.rs' ! -name '*.go' ! -name '*.py' ! -name '*.sh' ! -name '*.bats' \
         ! -name '*.ts' ! -name '*.js' ! -name '*.md' ! -name '*.json' -print0 2>/dev/null \
       | xargs -0 grep -lE '^-----BEGIN ([A-Z ]+ )?PRIVATE KEY-----' 2>/dev/null | head -5)
if [ -z "$KEYS" ]; then note "none found under $SCRATCH"; else
  echo "$KEYS" | while read -r l; do note "$l"; done
  bad "private key material in the scratchpad — remove it"
fi

echo
echo "7. open items — the close-out is not written until every one is classified"
note "every open item carries a SCOPE and a CLASS, an owner, and a resolution a reader can look up:"
note "  Scope: THIS SESSION | OUTSIDE THIS SESSION"
note "  Class: URGENT (human action now; listed first)  | OPEN (this session's, untracked)"
note "         ASSIGNED (owner + tracked home)           | BY DESIGN (ladder rung, soak, debt — say what resolves it)"
note "URGENT and OPEN are only ever THIS SESSION, and either one makes the verdict NOT CLEAN"
note "regardless of the repo checks above. An item you cannot classify is OPEN."
note ""
note "Two groups, this session first; class is a column, scope is the heading:"
note "  ## This session            | Class | Item | Owner | Resolves by |"
note "  ## Outside this session    | Class | Item | Owner | Resolves by |   (or: none)"

echo
if [ "$OUT" -eq 0 ]; then
  echo "==> REPO CLEAN — worktrees reaped, nothing uncommitted, no PR open, track branches carry no content missing from main."
  echo "    The verdict is CLEAN only if the open-items table has no THIS SESSION row classed URGENT or OPEN."
else
  echo "==> REPO NOT CLEAN — every line marked OUTSTANDING above must appear in the open-items table under its true scope."
  echo "    Other sessions' worktrees and PRs go under Outside this session, ASSIGNED to their owner. The verdict is"
  echo "    NOT CLEAN only if a This-session row is URGENT or OPEN, or an OUTSTANDING line above is this session's."
fi
exit "$OUT"
