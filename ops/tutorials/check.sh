#!/usr/bin/env bash
# check.sh — keep the goatnet tutorials true (docs/design/goatnet-tutorials.md).
#
#   ops/tutorials/check.sh <goatbuilder binary> [--record]
#
# For every tutorial under docs/tutorials/ that has a section headed
# "Rehearse it on the fixture" or "Rehearse it on the worked example" (a
# leading step number such as "2." is ignored), run that section's ```bash blocks exactly as
# written (one shell, top to bottom, from the repository root, `goatbuilder`
# resolved to the binary given), normalise what varies run to run (the
# millisecond timing, the scratch directory), and compare the output with the
# worked example's recorded checkpoint:
#
#   ops/goatbuilder/fixtures/goatnet-from-scratch/checkpoints/<tutorial>.txt
#
# A difference fails: either the page drifted from the tool or the tool from
# the page, and a reader diffing their own run against the checkpoint would be
# misled either way. --record rewrites the checkpoints (review the diff).
#
# Bash 3.2 portable; needs python3 for the extraction.
set -euo pipefail

GB="${1:?usage: check.sh <goatbuilder binary> [--record]}"
RECORD="${2:-}"
case "$GB" in /*) ;; *) GB="$(cd "$(dirname "$GB")" && pwd)/$(basename "$GB")" ;; esac
[ -x "$GB" ] || { echo "check.sh: $GB is not an executable goatbuilder" >&2; exit 2; }
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CHECKPOINTS="$ROOT/ops/goatbuilder/fixtures/goatnet-from-scratch/checkpoints"
SECTION="Rehearse it on the fixture|Rehearse it on the worked example"
fail=0; ran=0

# The section's bash blocks. A function, not a heredoc inside $(...): bash 3.2
# (macOS's /bin/bash) misparses the backticks of the fences in that form.
extract() {
  python3 - "$1" "$2" <<'PY'
import re, sys
text = open(sys.argv[1], encoding="utf-8").read().splitlines()
want = sys.argv[2].split("|")
start = level = None
for i, l in enumerate(text):
    m = re.match(r"^(#+)\s+(.*)$", l)
    if m and re.sub(r"^\d+\.\s*", "", m.group(2).strip()) in want:
        start, level = i, len(m.group(1)); break
if start is None:
    sys.exit(0)
out, infence, keep = [], False, False
for l in text[start + 1:]:
    m = re.match(r"^(#+)\s+", l)
    if m and not infence and len(m.group(1)) <= level:
        break
    if l.startswith("```"):
        if infence:
            infence = False
        else:
            infence, keep = True, l.strip() in ("```bash", "```sh")
        continue
    if infence and keep:
        out.append(l)
print("\n".join(out))
PY
}

for page in "$ROOT"/docs/tutorials/[0-9][0-9]-*.md; do
  [ -f "$page" ] || continue
  name="$(basename "$page" .md)"
  script="$(extract "$page" "$SECTION")"
  [ -n "$script" ] || continue
  ran=$((ran + 1))
  scratch="$(mktemp -d)"
  # `goatbuilder` at the start of a command is the binary under test; the
  # reader's own copy is on PATH, ours may not be.
  shim="$scratch/bin"; mkdir -p "$shim"; ln -s "$GB" "$shim/goatbuilder"
  out="$(cd "$ROOT" && PATH="$shim:$PATH" TMPDIR="$scratch" bash -euo pipefail -c "$script" 2>&1)" || {
    echo "FAIL  $name: the rehearsal exited non-zero"; echo "$out" | sed 's/^/      /' | tail -20; fail=1; rm -rf "$scratch"; continue; }
  norm="$(printf '%s\n' "$out" | sed -E -e "s#${scratch}[^ ]*#<scratch>#g" -e 's/ in [0-9]+ ms/ in <n> ms/g')"
  rm -rf "$scratch"
  want="$CHECKPOINTS/$name.txt"
  if [ "$RECORD" = "--record" ]; then
    mkdir -p "$CHECKPOINTS"; printf '%s\n' "$norm" > "$want"; echo "recorded $name -> ${want#"$ROOT"/}"; continue
  fi
  if [ ! -f "$want" ]; then
    echo "FAIL  $name: no checkpoint at ${want#"$ROOT"/} (run with --record and review it)"; fail=1; continue
  fi
  if diff -u "$want" <(printf '%s\n' "$norm") > /dev/null; then
    echo "ok    $name: the rehearsal matches its checkpoint ($(wc -l < "$want" | tr -d ' ') lines)"
  else
    echo "FAIL  $name: the rehearsal no longer matches ${want#"$ROOT"/}"
    diff -u "$want" <(printf '%s\n' "$norm") | sed 's/^/      /' | head -40
    fail=1
  fi
done

[ "$ran" -gt 0 ] || { echo "check.sh: no tutorial has a \"$SECTION\" section" >&2; exit 1; }
exit "$fail"
