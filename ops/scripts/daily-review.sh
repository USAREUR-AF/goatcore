#!/usr/bin/env bash
# daily-review.sh — the start-of-day and end-of-day review-and-act (ADR 1177).
#
# Graduation is never automated: an unwatched change reaching production is
# the risk the ladder exists to prevent. What is automated is the LOOKING.
# This script shows, from a current main, everything that needs a person and
# the exact command for each, so nothing soaking, owed or stale gets lost.
#
#   ops/scripts/daily-review.sh [--ack <name>] [--json]
#
# Morning: run it, act on ACT NOW, note SOAKING dates in your day.
# Evening: run it again; a record you opened today must show under SOAKING
# with its evidence filled, or under ACT NOW with its command run.
# Canon: docs/design/release-graduation.md §9, CONTRIBUTING.md "Daily cadence".
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
ACK="${GOAT_ACK:-$(git -C "$REPO" config user.name 2>/dev/null || echo '<you>')}"; JSON=0
while [ $# -gt 0 ]; do case "$1" in --ack) ACK="$2"; shift 2 ;; --json) JSON=1; shift ;; -h|--help) sed -n '2,15p' "$0"; exit 0 ;; *) echo "unknown flag $1" >&2; exit 2 ;; esac; done

# --- 1. the tree must be current: a stale checkout reviews yesterday's ladder ---
git -C "$REPO" fetch -q origin 2>/dev/null || true
HEAD="$(git -C "$REPO" rev-parse --short HEAD)"; MAIN="$(git -C "$REPO" rev-parse --short origin/main 2>/dev/null || echo '?')"
BEHIND="$(git -C "$REPO" rev-list --count HEAD..origin/main 2>/dev/null || echo '?')"
if [ "$JSON" = 1 ]; then
  python3 "$REPO/ops/release/graduate.py" review --json --ack "$ACK" \
    | python3 -c "import sys,json; d=json.load(sys.stdin); d['tree']={'head':'$HEAD','origin_main':'$MAIN','behind':'$BEHIND'}; print(json.dumps(d,indent=2))"
  exit 0
fi
W="${COLUMNS:-$(tput cols 2>/dev/null || echo 100)}"
echo "DAILY REVIEW  $(date -u '+%Y-%m-%d %H:%MZ')  main $MAIN  $ACK"
if [ "$BEHIND" != "0" ]; then
  echo "  !! this checkout ($HEAD) is $BEHIND commit(s) behind origin/main: the ladder below may be stale — git pull --ff-only"
fi
echo

# --- 2. the ladder: every open record, sorted by what a person must do -------
python3 "$REPO/ops/release/graduate.py" review --no-header --ack "$ACK"
echo

# --- 2b. the declared registry each goatnet is actually serving ---------------
# Wired in because this drift is only ever found by looking, and nothing looked.
# Measured: ZAGROS served a roster generated carrying 245
# people while the repository held 355, for a week, with every surface reading
# green — two people rendered "not onboarded" the whole time. The comparison
# already existed: `goat access strays` reports the served `source_digest` and
# says which side is behind. It was simply never part of the routine.
#
# Iterates the goatnets that HAVE a committed artifact rather than a hard-coded
# list, so a new goatnet joins this check by having one. The operator context is
# `<goatnet>-op`: the plain context answers 403 to a cross-identity query, and a
# 403 here must read as "could not ask", never as "no drift".
printf 'REGISTRY  the roster each goatnet is serving          goat access strays --context <env>-op\n'
for RJ in "$REPO"/docs/operators/*/roster.json; do
  [ -e "$RJ" ] || continue
  GN="$(basename "$(dirname "$RJ")")"
  goat access strays --context "${GN}-op" --json </dev/null 2>/dev/null \
    | RJ="$RJ" GN="$GN" python3 -c '
import calendar, json, os, sys, time
gn = os.environ["GN"]
try:
    served = (json.load(sys.stdin) or {}).get("freshness") or {}
except Exception:
    served = {}
try:
    local = json.load(open(os.environ["RJ"]))
except Exception:
    local = {}

def age_days(stamp):
    try:
        return (time.time() - calendar.timegm(time.strptime(stamp, "%Y-%m-%dT%H:%M:%SZ"))) / 86400.0
    except Exception:
        return None

if not served:
    print("  %-11s could not ask - no operator context, or the portal did not answer" % gn)
    raise SystemExit
a = age_days(served.get("generated_at", ""))
head = "  %-11s serving %d members, %s" % (
    gn, served.get("member_count", 0),
    ("%.0fd old" % a) if a is not None else "age unknown")
sd, ld = served.get("source_digest", ""), local.get("source_digest", "")
if sd and ld and sd == ld:
    print(head + " - matches this checkout")
elif sd and ld:
    # Deliberately does not say WHICH is behind: this checkout is as likely to
    # be the stale one, and asserting a direction from two digests is a guess.
    print(head + " - DIFFERS from this checkout (served %s, local %s) - one is behind" % (sd[:8], ld[:8]))
else:
    print(head + " - no digest to compare")
'
done
echo

# --- 3. tracks with something to say, and your newest PRs ----------------------
"$REPO/ops/work/work" list 2>/dev/null | awk -F'\t' -v w="$W" '
  NR>1 && $3 ~ /^active:/ {n++; note[n]=sprintf("  %-30s %s", $1, substr($3,8))}
  NR>1 && $3 == "active" {plain++}
  NR>1 && $3 ~ /^paused/ {p++}
  END {printf "TRACKS  %d active · %d paused          ops/work/work list\n", n+plain, p
       for(i=1;i<=n;i++) print substr(note[i],1,w-1)
       if(plain) printf "  (%d more active with no status note)\n", plain
       if(!n && !plain) print "  none active"}'
echo
echo
# Every repository this checkout's work lands in: the clone's own, and in an
# overlay checkout the public code repositories too (ops/scripts/repo-slug.sh).
PR_REPOS="$("$HERE/repo-slug.sh" pr-repos "$REPO" 2>/dev/null || "$HERE/project-setting.sh" repo source 2>/dev/null)"
pr_list() {   # updatedAt<TAB>number<TAB>state<TAB>draft<TAB>title, every repo; 1 if none answered
  local r tag ok=1 n
  n="$(printf '%s\n' $PR_REPOS | grep -c .)"
  for r in $PR_REPOS; do
    tag=""; [ "$n" -gt 1 ] && tag="${r#*/}#"
    gh pr list --repo "$r" --author @me --state open --limit 40 --json number,title,isDraft,mergeStateStatus,updatedAt \
      --jq '.[] | "\(.updatedAt)\t\(.number)\t\(.mergeStateStatus)\t\(if .isDraft then "draft" else "" end)\t\(.title)"' \
      2>/dev/null | awk -F'\t' -v OFS='\t' -v t="$tag" '{$2=t $2; print}' || continue
    ok=0
  done
  return $ok
}
if PRS="$(set -o pipefail; pr_list | sort -r | cut -f2-)"; then
  printf '%s\n' "$PRS" | awk -F'\t' -v w="$W" 'BEGIN{m["BLOCKED"]="needs review"; m["CLEAN"]="mergeable"; m["BEHIND"]="behind main"; m["DIRTY"]="conflicts"; m["UNSTABLE"]="checks failing"; m["UNKNOWN"]="…"}
       NF>1 {n++; if(n<=8){st=($2 in m)?m[$2]:$2; if($3!="")st=st" "$3; line[n]=sprintf("  %-6s %-14s %s", ($1 ~ /#/ ? $1 : "#" $1), st, $4)}}
       END{k=(n<8?n:8); printf "PRS  %d open, newest %d          gh pr list --author @me\n", n, k; for(i=1;i<=k;i++) print substr(line[i],1,w-1); if(!n) print "  none"}'
else
  echo "PRS  (gh did not answer — gh pr list --author @me)"
fi
echo
echo "evening: ops/scripts/closeout-check.sh, then the close-out's open-items table (this session / outside this session)"
