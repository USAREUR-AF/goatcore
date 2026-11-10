#!/usr/bin/env bash
# check-phase-numbering.sh — every numbered sequence an operator resumes from is
# whole numbers from 1, with no gaps and no letters.
#
# Operator rule: phase numbers that skip, or carry letters (6b, 12m),
# confuse the people running them. This gate reads what each driver PRINTS —
# the plan an operator sees and resumes from with --from — not the source, so a
# phase added with the wrong number fails here whichever avenue adds it:
#
#   goat-standup.sh --plan        (lan and internet: together they cover 1..N)
#   peering-standup.sh --plan
#   goatbuilder join-node --dry-run   (each node a deployment lists, below)
#   goatbuilder federate --dry-run
#
# join-node plans the goatnets this checkout declares, so the nodes to plan come
# from the deployment, not from this file: every
# ops/ci/check-phase-numbering.d/*.joinnode file lists one per line as
#     <label>|<goatnet>|<site>|<class>
# (e.g. "joiner core|<goatnet>|<joiner site>|core" and
# "founder rescue|<goatnet>|<founder site>|breakglass"). A tree that declares no
# goatnet lists none. *.exclude files in the same directory add playbook
# pathspecs (one per line) the task-name check below leaves alone, beside the
# ones it names. Blank lines and `#` comments are ignored in both.
#
# goatbuilder standup/teardown and federate are held to the scripts line for line
# by the parity gates, so the scripts' plans stand for both avenues there.
#
# usage:
#   ops/ci/check-phase-numbering.sh <goatbuilder binary>   run every check
#   ops/ci/check-phase-numbering.sh --selftest             prove the patterns below
set -uo pipefail
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
EXT_DIR="$REPO/ops/ci/check-phase-numbering.d"
ext_entries() {  # ext_entries <suffix> — entry lines of every $EXT_DIR/*<suffix>
  local f
  for f in "$EXT_DIR"/*"$1"; do
    [ -f "$f" ] || continue
    sed -e 's/[[:space:]]*#.*$//' -e 's/^[[:space:]]*//' "$f" | grep -v '^$' || true
  done
}

# Shared patterns — the live checks and --selftest read the SAME regexes, so a
# green self-test means the checks match these exact shapes and nothing else.
#   NAME_RE           a task/play `name:` whose step carries a phase number an
#                     operator reads in the run output: "Phase 7", "Phase -1",
#                     "Phases 11-14", "Phase-7", "Phase 7B", "Phase 12m". A bare
#                     "# Phase N" COMMENT is not a name and is left — that is
#                     developer navigation (e.g. site-bootstrap.yml's own 0..8
#                     scheme), not the sequence anyone resumes from.
#   STANDUP_FROMTO_RE a goat-standup.sh --from/--to outside 1..14 (0 or 15..19):
#                     a command that resumes from a phase the 1..14 driver has
#                     not got. Tied to goat-standup.sh because goatbuilder
#                     join-node legitimately runs to 15.
#   OLD_LETTERED_RE   a retired lettered standup id with NO collision with
#                     site-bootstrap.yml's internal scheme: 9b/9c/11b/12m/13a..13d
#                     were only ever standup, so they are unambiguous; 7b/6a etc.
#                     are left because they are also internal-scheme navigation.
#   BARE_STEPID_RE    a script comment that opens with a lettered step id ("# 13a2.").
NAME_RE='^[[:space:]]*-?[[:space:]]*name:.*[Pp]hases? ?-?[0-9]'
STANDUP_FROMTO_RE='goat-standup\.sh[^&|;]*--(from|to) +(0|1[5-9])([^0-9]|$)'
OLD_LETTERED_RE='[Pp]hase ?(9[bc]|11b|12m|13[a-d][0-9]?)'
BARE_STEPID_RE='^#[[:space:]]*[0-9]+[a-z][0-9]?\.'

if [ "${1:-}" = --selftest ]; then
  t=0; bad=0
  chk() { # chk <label> <want 1|0> <regex> <string>
    local g=0; printf '%s' "$4" | grep -qE "$3" && g=1; t=$((t + 1))
    [ "$g" = "$2" ] || { bad=$((bad + 1)); echo "  FAIL [$1]: want=$2 got=$g :: $4"; }
  }
  chk name-plain   1 "$NAME_RE" '    - name: "Phase 7 — smoke"'
  chk name-neg     1 "$NAME_RE" '- name: "zagros-prod-bootstrap: Phase -1 — sentinel gate"'
  chk name-plural  1 "$NAME_RE" '- name: "nova: Phases 11-14 — decks"'
  chk name-hyphen  1 "$NAME_RE" '    - name: "smoke on Phase-7 invariants"'
  chk name-letter  1 "$NAME_RE" '    - name: "Phase 7B — spock mesh"'
  chk name-m       1 "$NAME_RE" '    - name: "Phase 12m — first login"'
  chk name-words   0 "$NAME_RE" '    - name: "sentinel gate | refuse while a sentinel survives"'
  chk name-comment 0 "$NAME_RE" '# Phase 7B — inter-site Spock auto-mesh'
  chk from-0       1 "$STANDUP_FROMTO_RE" 'ops/scripts/goat-standup.sh zagros --from 0 --to 0'
  chk from-15      1 "$STANDUP_FROMTO_RE" 'goat-standup.sh zagros-prod --from 15'
  chk to-19        1 "$STANDUP_FROMTO_RE" 'goat-standup.sh uk --to 19 --yes'
  chk from-ok1     0 "$STANDUP_FROMTO_RE" 'goat-standup.sh zagros --from 1 --to 14'
  chk from-ok14    0 "$STANDUP_FROMTO_RE" 'goat-standup.sh zagros --from 14'
  chk joinnode-ok  0 "$STANDUP_FROMTO_RE" 'goatbuilder join-node zagros --from 15 --yes'
  chk old-11b      1 "$OLD_LETTERED_RE" 'needs SPIRE (phase 11b)'
  chk old-12m      1 "$OLD_LETTERED_RE" 'first login is phase 12m'
  chk old-13a      1 "$OLD_LETTERED_RE" '### Phase 13a — synthetic fleet'
  chk old-9c       1 "$OLD_LETTERED_RE" 'relay ingress (phase 9c)'
  chk old-7b-left  0 "$OLD_LETTERED_RE" 'site-bootstrap Phase 7b snitch probe'
  chk old-plain    0 "$OLD_LETTERED_RE" 'the router is phase 9'
  chk bare-13a2    1 "$BARE_STEPID_RE" '# 13a2. goat-zenoh-portal'
  chk bare-phase   0 "$BARE_STEPID_RE" '# Phase 12. goat-zenoh-portal'
  if [ "$bad" = 0 ]; then echo "ok   selftest: $t patterns, all correct"; exit 0; fi
  echo "FAIL selftest: $bad of $t wrong"; exit 1
fi

BIN="${1:?usage: check-phase-numbering.sh <goatbuilder binary> | --selftest}"
fail=0

# check <name> <require-all: 1|0> — stdin is a plan; the "=== Phase <id> [" headers
# are read in order. Every id is a whole number, they never go down, the first is
# 1, and (require-all) every number from 1 to the last appears.
CHECK_PY='
import re, sys
name, require_all = sys.argv[1], sys.argv[2] == "1"
ids = re.findall(r"=== Phase (\S+) \[", sys.stdin.read())
bad = [i for i in ids if not i.isdigit()]
if not ids:
    print(f"FAIL {name}: the plan printed no phases"); sys.exit(1)
if bad:
    print(f"FAIL {name}: phase ids that are not whole numbers: {sorted(set(bad))}"); sys.exit(1)
ns = [int(i) for i in ids]
if ns[0] != 1:
    print(f"FAIL {name}: the first phase is {ns[0]}, not 1"); sys.exit(1)
down = [(a, b) for a, b in zip(ns, ns[1:]) if b < a]
if down:
    print(f"FAIL {name}: phases go backwards: {down[:3]}"); sys.exit(1)
seen = sorted(set(ns))
gaps = [n for n in range(1, seen[-1] + 1) if n not in seen]
if require_all and gaps:
    print(f"FAIL {name}: phases skip {gaps} (1..{seen[-1]} must all appear)"); sys.exit(1)
print(f"ok   {name}: phases {seen[0]}..{seen[-1]}" + ("" if not gaps else f" (this profile skips {gaps})"))
'
check() { python3 -c "$CHECK_PY" "$1" "$2"; }

standup_lan="$("$REPO/ops/scripts/goat-standup.sh" phasenumbercheck --plan --profile lan 2>/dev/null)"
standup_net="$("$REPO/ops/scripts/goat-standup.sh" phasenumbercheck --plan --profile internet 2>/dev/null)"
printf '%s\n' "$standup_lan" | check "goat-standup.sh --plan (lan)" 0 || fail=1
printf '%s\n' "$standup_net" | check "goat-standup.sh --plan (internet)" 1 || fail=1
# Together the two profiles cover every phase: a number neither prints is a gap.
printf '%s\n%s\n' "$standup_lan" "$standup_net" | python3 -c '
import re, sys
ns = sorted({int(i) for i in re.findall(r"=== Phase (\d+) \[", sys.stdin.read())})
gaps = [n for n in range(1, ns[-1] + 1) if n not in ns]
print(("FAIL goat-standup.sh: no profile prints phases %s" % gaps) if gaps else "ok   goat-standup.sh: lan + internet cover 1..%d" % ns[-1])
sys.exit(1 if gaps else 0)' || fail=1

work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
FED_WORK="$work" "$REPO/ops/scripts/peering-standup.sh" hello-ally --plan 2>/dev/null | check "peering-standup.sh --plan" 1 || fail=1
# The phase rows from -o json (the human table prints them as headings).
phase_rows() { python3 -c 'import json,sys
for c in json.load(sys.stdin)["checks"]:
    if c["name"] == "phase": print(c["detail"])'; }
# A peering must be declared (C1), and the repository's own config declares none.
"$BIN" -C "$REPO" -o json --mock "$REPO/ops/goatbuilder/fixtures/goatnet-from-scratch/03-boundary-node" federate hello-ally --work "$work" --dry-run 2>/dev/null | phase_rows | check "goatbuilder federate --dry-run" 1 || fail=1
while IFS='|' read -r label jn_env jn_site jn_class; do
  [ -n "$label" ] || continue
  "$BIN" -C "$REPO" -o json join-node "$jn_env" --site "$jn_site" --node phasecheck-1 --class "$jn_class" --dry-run 2>/dev/null | phase_rows | check "goatbuilder join-node --dry-run ($label)" 1 || fail=1
done < <(ext_entries .joinnode)

# A lettered phase id written anywhere a driver prints a phase header.
# Every .rs file under crates/goatbuilder/src, nested modules included (bash 3.2 has no globstar).
if find "$REPO/crates/goatbuilder/src" -name '*.rs' -print0 | xargs -0 grep -nE '=== Phase [0-9]+[a-z]' "$REPO/ops/scripts/goat-standup.sh" "$REPO/ops/scripts/peering-standup.sh"; then
  echo "FAIL a phase header carries a letter (above)"; fail=1
fi

# An Ansible task or play name that numbers its own steps. An operator reads these
# in the run's output beside the standup's phases, so a play's "Phase 7B" / "Phase -1"
# / "Phases 6-10" is read as a second, conflicting phase number. Name the step for
# what it does instead. Left as they are:
#   - integdev-bootstrap.yml — teardown-only legacy env; a guard refuses any edit of
#     its grandfathered NetBird user seed.
#   - nova-lan-*.yml — a closed-LAN orchestrator, a SEPARATE driver with its
#     own phase scheme (not goat-standup). A deployment's own drivers of that
#     kind are listed in ops/ci/check-phase-numbering.d/*.exclude.
#   - mgmt-host.yml — `phase2` is a deploy-mode var (docker-compose.phase2.yml), the
#     full-stack-vs-postgres-only switch, not a sequence anyone resumes from.
name_excl=()
while IFS= read -r f; do [ -n "$f" ] && name_excl+=( ":!$f" ); done < <(ext_entries .exclude)
if git -C "$REPO" grep -nE "$NAME_RE" -- 'ops/ansible/*.yml' 'ops/ansible/*.yaml' \
     ':!ops/ansible/integdev-bootstrap.yml' ':!ops/ansible/nova-lan-*.yml' ':!ops/ansible/mgmt-host.yml' \
     ${name_excl[@]+"${name_excl[@]}"}; then
  echo "FAIL an Ansible task name carries a phase number (above): name the step in words"; fail=1
fi

# Docs and scripts that banner themselves as the pre-2026-10-29 numbering (with a
# pointer to the old-to-new table) are intentionally left in the old numbers; the
# two checks below skip them, and the ADR/finding/measurement records.
banner_excl=()
while IFS= read -r f; do [ -n "$f" ] && banner_excl+=( ":!$f" ); done < <(
  git -C "$REPO" grep -lE 'before 2026-10-29' -- 'docs/**' 'CLAUDE.md' '.claude/**' 2>/dev/null)

# A goat-standup.sh resume (--from/--to) pointed at a phase outside 1..14.
if git -C "$REPO" grep -nE "$STANDUP_FROMTO_RE" -- 'ops/scripts/*' 'docs/**' 'CLAUDE.md' '.claude/**' \
     ':!docs/adr/*' ':!docs/findings/*' ':!docs/measurements/*' ${banner_excl[@]+"${banner_excl[@]}"}; then
  echo "FAIL a goat-standup.sh --from/--to names a phase outside 1..14 (above)"; fail=1
fi

# A retired lettered standup id (9b/9c/11b/12m/13a..13d) in a live doc, script or
# CLAUDE.md — the 1..14 scheme has no letters. Bannered and record files are skipped.
if git -C "$REPO" grep -nE "$OLD_LETTERED_RE" -- \
     'ops/scripts/goat-standup.sh' 'ops/scripts/peering-standup.sh' \
     'docs/operations/*' 'docs/design/*' 'docs/project/*' 'CLAUDE.md' '.claude/**' \
     ':!docs/adr/*' ':!docs/findings/*' ':!docs/measurements/*' ${banner_excl[@]+"${banner_excl[@]}"}; then
  echo "FAIL a retired lettered standup id survives (above): the 1..14 scheme has no letters"; fail=1
fi

# A standup/peering script comment that opens with a lettered step id ("# 13a2.").
if grep -nE "$BARE_STEPID_RE" "$REPO/ops/scripts/goat-standup.sh" "$REPO/ops/scripts/peering-standup.sh"; then
  echo "FAIL a script comment is a lettered step id (above): the 1..14 scheme has no letters"; fail=1
fi

exit "$fail"
