#!/usr/bin/env bash
# parity-fresh-source.sh — the parity gate for contract clause C3 (fresh source)
# on a real run (goatbuilder-conformance.md §C3; ADR 1201: both avenues stay
# functional and agree).
#
# A scratch trunk is made from this checkout's HEAD, a clone of it is left one
# commit behind (the commit touches ops/ansible), and each mutating verb is run
# for real on both avenues from that clone:
#
#   deploy     deploy-component.sh <c> --goatnet <g>   | goatbuilder deploy
#   rollback   rollback.sh <c> 1                       | goatbuilder rollback <c> 1
#   standup    goat-standup.sh <site>                  | goatbuilder standup <env> --site <site>
#
# Each side must refuse with exit 1 and the same GB-064 refusal naming the same
# commit, and print nothing from past the check (no transport, no deploy, no
# phase). Then the trunk is made unreachable and deploy must refuse with GB-065
# on both sides. No host is touched: every run stops at the check.
#
# usage: parity-fresh-source.sh <goatbuilder binary> [component goatnet site-env:site]
#        (defaults: netvis zagros_dev zagros_dev:zagros-dev)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
BIN="${1:?usage: parity-fresh-source.sh <goatbuilder binary> [component goatnet env:site]}"
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"
COMP="${2:-netvis}"; GOATNET="${3:-zagros_dev}"; ENVSITE="${4:-zagros_dev:zagros-dev}"
SENV="${ENVSITE%%:*}"; SITE="${ENVSITE#*:}"

S="$(mktemp -d "${TMPDIR:-/tmp}/parity-c3.XXXXXX")"
trap 'rm -rf "$S"' EXIT
export GIT_LFS_SKIP_SMUDGE=1 GOAT_ANSIBLE_TRUNK_REMOTE=origin GOAT_MESH_PROBE=0
unset GOAT_FRESH_SOURCE_TRUNK GOAT_DEPLOY_OFFLINE
G=(-c user.name=parity -c user.email=parity@localhost)

# --- a trunk one commit ahead of the clone, in ops/ansible -----------------------
# The trunk is a fresh root commit of HEAD's ops/ tree, so the gate works the
# same from a shallow CI checkout (which can be neither cloned nor pushed from
# whole) as from a full one; everything the drivers read lives under ops/.
# An assembled tree (.goat-assembled at its root) holds the overlay uncommitted
# over goatcore's HEAD, and the overlay is where its goatnets are declared, so
# there the trunk is seeded from the working tree's ops/ (tracked and untracked,
# ignored files left out); from HEAD alone neither avenue would find a goatnet
# and both would stop before the check under test.
mkdir -p "$S/seed"
if [ -f "$REPO/.goat-assembled" ]; then
  git -C "$REPO" ls-files -z -co --exclude-standard -- ops | (cd "$REPO" && tar --null -T - -cf -) | tar -x -C "$S/seed"
else
  git -C "$REPO" archive HEAD ops | tar -x -C "$S/seed"
fi
git -C "$S/seed" init -q -b main
git -C "$S/seed" add -A
git "${G[@]}" -C "$S/seed" commit -q -m "parity: the checkout under test"
git init -q --bare "$S/o.git"
git -C "$S/o.git" symbolic-ref HEAD refs/heads/main
git -C "$S/seed" push -q "$S/o.git" main
git clone -q "$S/o.git" "$S/stale"
BLOB="$(printf 'c3 parity probe\n' | git -C "$S/o.git" hash-object -w --stdin)"
GIT_INDEX_FILE="$S/idx" git -C "$S/o.git" read-tree main
GIT_INDEX_FILE="$S/idx" git -C "$S/o.git" update-index --add --cacheinfo "100644,$BLOB,ops/ansible/.c3-parity-probe"
TREE="$(GIT_INDEX_FILE="$S/idx" git -C "$S/o.git" write-tree)"
SUBJECT="c3 parity: a role change this checkout has not pulled"
NEXT="$(git "${G[@]}" -C "$S/o.git" commit-tree "$TREE" -p main -m "$SUBJECT")"
git -C "$S/o.git" update-ref refs/heads/main "$NEXT"

FAILS=0
# The refusal, reduced to what both avenues print: the code line and the commit.
# `|| true` on every grep: a side that printed no refusal is a difference to
# report, not a reason for set -e to end the gate silently.
reduce() { { grep -oE 'GB-06[45] REFUSE — [^;]*|could not fetch [a-z]+ main|c3 parity: [a-z ]+' || true; } | sort -u; }
# Anything that would mean the run went past the check.
past() { grep -E 'via (mesh|door|direct)|deploying |=== Phase|DRY:|rolled back|ssh ' || true; }

judge() {   # <verb> <script output file> <script rc> <binary output file> <binary rc>
  local verb="$1" so="$2" src="$3" bo="$4" brc="$5" a b
  a="$(reduce < "$so")"; b="$(reduce < "$bo")"
  if [ "$src" != 1 ] || [ "$brc" != 1 ]; then
    echo "FAIL $verb: exit script=$src binary=$brc, both must be 1"; FAILS=$((FAILS+1)); return
  fi
  if [ -z "$a" ] || [ "$a" != "$b" ]; then
    echo "FAIL $verb: the refusals differ"; echo "  script: $a" | head -5; echo "  binary: $b" | head -5; FAILS=$((FAILS+1)); return
  fi
  if [ -n "$(past < "$so")$(past < "$bo")" ]; then
    echo "FAIL $verb: a side went past the check"; past < "$so" | head -3; past < "$bo" | head -3; FAILS=$((FAILS+1)); return
  fi
  echo "ok   $verb: both refuse — $(printf '%s' "$a" | head -1)"
}

run() {   # <out file> <cmd...>  -> rc on stdout
  local out="$1"; shift
  ( cd "$S/stale" && "$@" ) > "$out" 2>&1 && echo 0 || echo $?
}

SC="$S/stale/ops/scripts"
rc1="$(run "$S/d.s" "$SC/deploy-component.sh" "$COMP" --goatnet "$GOATNET")"
rc2="$(run "$S/d.b" "$BIN" deploy --env "$GOATNET" --component "$COMP" --ack parity --yes)"
judge "deploy (behind)" "$S/d.s" "$rc1" "$S/d.b" "$rc2"

if [ -f "$S/stale/ops/ansible/roles/$COMP/deploy-history.yml" ]; then
  rc1="$(run "$S/r.s" "$SC/rollback.sh" "$COMP" 1)"
  rc2="$(run "$S/r.b" "$BIN" rollback "$COMP" 1 --ack parity --yes)"
  judge "rollback (behind)" "$S/r.s" "$rc1" "$S/r.b" "$rc2"
else
  echo "skip rollback: $COMP has no deploy-history.yml"
fi

rc1="$(run "$S/s.s" "$SC/goat-standup.sh" "$SITE")"
rc2="$(run "$S/s.b" "$BIN" standup "$SENV" --site "$SITE" --ack parity --yes)"
judge "standup (behind)" "$S/s.s" "$rc1" "$S/s.b" "$rc2"

git -C "$S/stale" remote set-url origin "$S/no-such-trunk.git"
rc1="$(run "$S/u.s" "$SC/deploy-component.sh" "$COMP" --goatnet "$GOATNET")"
rc2="$(run "$S/u.b" "$BIN" deploy --env "$GOATNET" --component "$COMP" --ack parity --yes)"
judge "deploy (trunk unreachable)" "$S/u.s" "$rc1" "$S/u.b" "$rc2"

if [ "$FAILS" -gt 0 ]; then
  echo "parity-fresh-source: $FAILS difference(s); outputs:"; for f in "$S"/*.s "$S"/*.b; do echo "--- $f"; tail -15 "$f"; done
  exit 1
fi
echo "parity-fresh-source: both avenues refuse a stale checkout and an unreachable trunk identically, before any host"
