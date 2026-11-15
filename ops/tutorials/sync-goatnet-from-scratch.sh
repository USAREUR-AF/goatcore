#!/usr/bin/env bash
# sync-goatnet-from-scratch.sh — keep the Hello Galaxy repository's copy of the worked
# example equal to this repository's (docs/design/goatnet-tutorials.md).
#
#   ops/tutorials/sync-goatnet-from-scratch.sh <clone of hello-galaxy-goatenv> [--apply] [--goatbuilder <binary>]
#
# ops/goatbuilder/fixtures/goatnet-from-scratch/ is the canonical copy: goatbuilder's
# tutor, the goatnet_from_scratch_* tests and ops/tutorials/check.sh read it on every
# pull request here. USAREUR-AF/hello-galaxy-goatenv carries the same step
# directories at its root, and its own checkpoints, for a reader who clones the
# example alone.
#
# The step directories: each file this repository holds is compared with the
# clone's (without --apply), or copied over it (with --apply). A file only the
# clone has is kept and named, never deleted: the clone has files of its own.
#
# The checkpoints are never copied. The clone rehearses tutorial 3 from a copied
# fixture, so its checkpoint names other paths than this repository's. With
# --goatbuilder <binary> (built at this checkout), the clone's own ./run checks
# them, or, with --apply, re-records them (./run <binary> --record), with
# GOAT_REPO set to this checkout. Without it they are not compared.
#
# --apply also sets the clone's GOATBUILDER_REF to this checkout's HEAD and leaves
# the commit to you. Run it in the same change that edits the fixture or
# re-records a checkpoint, then review the clone's diff.
#
# Exit 0 equal (or applied), checkpoints included; 1 a difference; 2 usage;
# 3 the step directories equal (or applied) but the checkpoints not compared
# (no --goatbuilder).
#
# Bash 3.2 portable.
set -euo pipefail

usage() { echo "usage: sync-goatnet-from-scratch.sh <clone of hello-galaxy-goatenv> [--apply] [--goatbuilder <binary>]" >&2; exit 2; }
dest=""; apply=0; gb=""
while [ $# -gt 0 ]; do
  case "$1" in
    --apply) apply=1 ;;
    --goatbuilder) [ $# -ge 2 ] || usage; gb="$2"; shift ;;
    -*) usage ;;
    *) [ -z "$dest" ] || usage; dest="$1" ;;
  esac
  shift
done
[ -n "$dest" ] || usage
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
src="$ROOT/ops/goatbuilder/fixtures/goatnet-from-scratch"
[ -x "$dest/run" ] || { echo "sync-goatnet-from-scratch.sh: $dest is not a clone of hello-galaxy-goatenv (no ./run)" >&2; exit 2; }
if [ -n "$gb" ]; then
  case "$gb" in /*) ;; *) gb="$(cd "$(dirname "$gb")" && pwd)/$(basename "$gb")" ;; esac
  [ -x "$gb" ] || { echo "sync-goatnet-from-scratch.sh: --goatbuilder $gb is not an executable" >&2; exit 2; }
fi

steps="00-goatlite 01-founder 02-first-federated-node 03-boundary-node"
fail=0
for p in $steps; do
  differs=0
  # Every file this repository holds for the step, by its path inside it.
  while IFS= read -r rel; do
    rel="${rel#./}"
    if [ "$apply" = 1 ]; then
      if ! cmp -s "$src/$p/$rel" "$dest/$p/$rel" 2>/dev/null; then
        mkdir -p "$(dirname "$dest/$p/$rel")"
        cp "$src/$p/$rel" "$dest/$p/$rel"
        echo "copied $p/$rel"
      fi
    elif ! cmp -s "$src/$p/$rel" "$dest/$p/$rel" 2>/dev/null; then
      echo "DIFF  $p/$rel"
      diff "$src/$p/$rel" "$dest/$p/$rel" 2>&1 | sed 's/^/      /' | head -20 || true
      differs=1; fail=1
    fi
  done <<EOF
$(cd "$src/$p" && find . -type f | LC_ALL=C sort)
EOF
  # A file only the clone has: kept, and said.
  if [ -d "$dest/$p" ]; then
    while IFS= read -r rel; do
      rel="${rel#./}"
      [ -n "$rel" ] || continue
      [ -e "$src/$p/$rel" ] || echo "kept  $p/$rel (only in the clone; not this repository's)"
    done <<EOF
$(cd "$dest/$p" && find . -type f | LC_ALL=C sort)
EOF
  fi
  [ "$apply" = 1 ] || [ "$differs" = 1 ] || echo "ok    $p"
done

if [ "$apply" = 1 ]; then
  git -C "$ROOT" rev-parse HEAD > "$dest/GOATBUILDER_REF"
  echo "GOATBUILDER_REF -> $(cat "$dest/GOATBUILDER_REF")"
fi

# The checkpoints: only through the clone's own rehearsal.
if [ -z "$gb" ]; then
  if [ "$apply" = 1 ]; then
    echo "SKIP  checkpoints: not re-recorded; pass --goatbuilder <binary built at this checkout> to re-record them with the clone's ./run"
  else
    echo "SKIP  checkpoints: not compared; pass --goatbuilder <binary built at this checkout> to check them with the clone's ./run"
  fi
  [ "$fail" = 1 ] && exit 1
  exit 3
fi
if [ "$apply" = 1 ]; then
  if (cd "$dest" && GOAT_REPO="$ROOT" ./run "$gb" --record); then
    echo "recorded checkpoints with the clone's ./run (review the clone's diff)"
  else
    echo "FAIL  checkpoints: the clone's ./run --record failed"; fail=1
  fi
elif (cd "$dest" && GOAT_REPO="$ROOT" ./run "$gb"); then
  echo "ok    checkpoints (the clone's ./run)"
else
  echo "DIFF  checkpoints: the clone's rehearsal does not match its checkpoints (./run above)"; fail=1
fi
exit "$fail"
