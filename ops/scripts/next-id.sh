#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# next-id.sh — what ADR and finding numbers are already claimed, anywhere,
# and what is genuinely free.
#
# WHY THIS EXISTS. the same mistake happened five times in
# about eighteen hours:
#
#   F-322   claimed by three sessions; one retracted, the ID stays burned
#   F-324   claimed by three sessions inside six minutes
#   ADR 1128  claimed twice — mesh-bandwidth-attribution merged first, and
#             logical-first-fabric-time is still sitting unmerged on a branch
#             that was correct when it was cut
#   F-329 / F-331  claimed on branches that exist only on the mirror, so a
#             session searching origin reported them free
#
# None of those were carelessness. The repo has TWO remotes and hundreds of
# branches, and the honest answer to "what is the next number" lives across all
# of them. Everyone was grepping a subset — usually `docs/adr/` on their own
# checkout, which shows only what has merged — and the subset is always
# optimistic. One session put it exactly right: searching refs/remotes/origin
# "was true of origin and false of the repo."
#
# So: one command, both remotes, every ref, including unmerged work.
#
#   ops/scripts/next-id.sh              # next free ADR + finding
#   ops/scripts/next-id.sh --collisions # only the numbers claimed twice
#   ops/scripts/next-id.sh --fetch      # git fetch --all first (slower, current)
#
# CLAIM BEFORE YOU WRITE. Run this, take the number it gives, and say so on the
# session bus BEFORE creating the file — claiming afterwards reports a collision
# rather than preventing one, which is what two sessions learned the hard way.
# It still is not a lock: two people can run this within a minute of each other
# and both get the same answer. It removes the large window, not the small one.

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO"

MODE="${1:-}"
if [ "$MODE" = "--fetch" ]; then
  echo "fetching all remotes (this is the slow, current answer)…" >&2
  git fetch --all --quiet
  MODE=""
fi

# Every remote-tracking ref on every remote. Local-only, so no network: run
# --fetch first if you need today's branches rather than your last fetch.
refs="$(git for-each-ref --format='%(refname)' refs/remotes/)"
nrefs="$(echo "$refs" | grep -c . || true)"

# One pass: collect "<id> <file>" for every ADR and finding on every ref.
# `git ls-tree -r --name-only` on a directory is cheap; the cost is the ref
# count, which is why this is a script rather than something people retype.
# ONE pass over every ref, recording "<kind> <id> <slug> <ref>" — the ref is
# captured here rather than re-derived per collision, which is what the first
# version did (743 ls-trees per collision, and it silently reported only the
# first one).
rows="$(
  for r in $refs; do
    short="${r#refs/remotes/}"
    git ls-tree -r --name-only "$r" -- docs/adr docs/findings 2>/dev/null |
      sed -n -E "s|^docs/adr/([0-9]{4})-(.*)\\.md$|ADR \\1 \\2 $short|p;
                 s|^docs/findings/(F-[0-9]{3})-(.*)\\.md$|FND \\1 \\2 $short|p"
  done | sort -u
)"

adr_ids="$(echo "$rows" | awk '$1=="ADR"{print $2}' | sort -u)"
fnd_ids="$(echo "$rows" | awk '$1=="FND"{print $2}' | sort -u)"

# A number claimed by two DIFFERENT slugs is a collision. The same slug on many
# branches is one document on many branches — dedupe on (kind,id,slug) FIRST,
# then count by (kind,id).
collisions="$(
  echo "$rows" | awk '{print $1, $2, $3}' | sort -u |
    awk '{print $1, $2}' | uniq -c | awk '$1>1{print $2"|"$3}'
)"

if [ -n "$collisions" ]; then
  echo "COLLISIONS — one number, two different documents:"
  echo "$collisions" | while IFS='|' read -r kind id; do
    [ -z "${id:-}" ] && continue
    echo "  $kind $id"
    # One line per DOCUMENT, not per ref: a doc living on forty branches is
    # still one doc, and printing forty lines buries the thing you must decide.
    # What decides it is whether the doc is on a main ref — that is "merged".
    echo "$rows" | awk -v k="$kind" -v i="$id" '$1==k && $2==i {
        n[$3]++
        if ($4 ~ /\/main$/) merged[$3]=1
        if (!($3 in ex)) { ex[$3]=$4; order[++c]=$3 }
      }
      END {
        for (j=1; j<=c; j++) {
          sl=order[j]
          printf "      %-52s %s  (%d ref%s, e.g. %s)\n", sl,
                 (sl in merged ? "MERGED  <- keeps the number" : "unmerged <- this one moves"),
                 n[sl], (n[sl]==1 ? "" : "s"), ex[sl]
        }
      }' 
  done
  echo
  echo "  RULE: merged beats unmerged. Renaming a MERGED id breaks every inbound"
  echo "  link, its INDEX row and any CLAUDE.md pointer; renaming an unmerged one"
  echo "  is a rebase. So the unmerged claimant moves — and gets TOLD, because"
  echo "  they were not wrong when they cut the branch; the number was free then."
  echo "  A ref ending in /main is merged. Anything else is not."
  echo
fi

[ "$MODE" = "--collisions" ] && exit 0

next_after() { # highest numeric id + 1
  echo "$1" | tr -d 'F-' | sort -n | tail -1 | awk '{printf "%d\n", $1+1}'
}

adr_next="$(next_after "$adr_ids")"
fnd_next="$(printf 'F-%03d\n' "$(next_after "$fnd_ids")")"

echo "scanned $nrefs remote-tracking refs across: $(git remote | tr '\n' ' ')"
echo "  ADRs claimed:     $(echo "$adr_ids" | grep -c .)  highest $(echo "$adr_ids" | sort -n | tail -1)"
echo "  findings claimed: $(echo "$fnd_ids" | grep -c .)  highest $(echo "$fnd_ids" | sort | tail -1)"
echo
echo "NEXT FREE — claim on the bus before you write the file:"
echo "  ADR      $adr_next"
echo "  finding  $fnd_next"
echo
echo "(local refs only; re-run with --fetch for today's branches. A retracted"
echo " finding still burns its number — F-322 is a retracted row, not free.)"
