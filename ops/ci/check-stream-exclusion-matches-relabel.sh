#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# check-stream-exclusion-matches-relabel.sh — assert that every declared
# netview stream-exclusion list uses names the Prometheus relabel actually
# produces.
#
# WHY THIS EXISTS: two facts have to agree and nothing made them.
#
#   1. `ops/compose/goat-obs/prometheus/prometheus.yml` carries a
#      `metric_relabel_configs` block on the traefik job that rewrites the
#      `service` and `router` labels at INGEST, stripping a legacy prefix.
#      The prefix is the goatnet's `obs_traefik_legacy_name_prefix`
#      (group_vars); every goatnet that declares one gets the relabel.
#
#   2. netview excludes SSE endpoints from its latency percentiles, because a
#      stream's duration is its held-open lifetime and not a response time.
#      The exclusion is a regex matched against that same `service` label,
#      declared in THREE places: the argparse default in
#      `ops/netview/netview-poll.py`, the role default in
#      `roles/netview/defaults/main.yml`, and any per-goatnet group_vars
#      override.
#
# Changing (1) changes the label values out from under (2). The exclusion
# stops matching and DOES NOT FAIL — the streams silently rejoin the
# percentile set and the page reports a held-open SSE lifetime as a response
# time. That is the whole argument for this file: a comment saying "keep these in sync" is not a
# mechanism.
#
# WHAT IT CHECKS: for each prefix the relabel strips, no declared exclusion
# list may contain a name carrying that prefix. The check derives the prefix
# FROM the relabel block rather than hardcoding it, so renaming the relabel
# moves this guard with it instead of leaving it asserting a dead rule.
#
# Deliberately NOT checking that the excluded services exist: a goatnet that
# does not run signal has no such series, and a guard that demanded one would
# fail on a correct config.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

PROM="ops/compose/goat-obs/prometheus/prometheus.yml"
POLLER="ops/netview/netview-poll.py"
ROLE_DEFAULTS="ops/ansible/roles/netview/defaults/main.yml"

fail=0
note() { printf '  %s\n' "$*"; }

[ -f "$PROM" ] || { echo "check-stream-exclusion: FAIL — $PROM missing"; exit 1; }

# Derive the stripped prefixes from the relabel block itself. We want the
# regex values of rules whose target_label is `service` or `router` and whose
# replacement is the first capture group — i.e. the "strip a prefix" shape.
# read -r in a while loop rather than mapfile: bash 3.2 (macOS system bash) has
# no mapfile, and this script runs on operator laptops as well as CI.
PREFIXES=()
while IFS= read -r _line; do
  PREFIXES+=("$_line")
done < <(
  python3 - "$PROM" <<'PY'
import glob, re, sys
text = open(sys.argv[1]).read()
# ^nova-(.*)$ -> nova-      ; tolerate quoting and anchors
seen = []
for m in re.finditer(r"regex:\s*['\"]?\^([A-Za-z0-9_-]+)-\(\.\*\)\$?['\"]?", text):
    p = m.group(1) + "-"
    if p not in seen:
        seen.append(p)
# The prefix as a variable: regex: '^{{ <var> }}-(.*)$'. The values are whatever
# the goatnets declare for that variable in group_vars, so the guard follows
# the declaration instead of standing down when the literal left the template.
for m in re.finditer(r"regex:\s*['\"]?\^\{\{\s*([A-Za-z0-9_]+)\s*\}\}-\(\.\*\)\$?['\"]?", text):
    decl = re.compile(r"^" + re.escape(m.group(1)) + r":\s*['\"]?([A-Za-z0-9_-]+)['\"]?\s*(?:#.*)?$", re.M)
    for gv in sorted(glob.glob("ops/ansible/group_vars/**/*.yml", recursive=True)):
        for v in decl.findall(open(gv).read()):
            if v + "-" not in seen:
                seen.append(v + "-")
for p in seen:
    print(p)
PY
)

# Drop blank lines defensively. An empty element here would be catastrophic
# rather than merely wrong: the prefix is interpolated into a grep pattern, so
# an empty prefix matches EVERY declaration and the guard fails the whole tree
# while claiming the relabel strips "". Covered by the negative test that
# removes the relabel rules — the guard must stand DOWN in that case, not
# indict every correct value in the repo.
filtered=()
for p in ${PREFIXES+"${PREFIXES[@]}"}; do
  [ -n "$p" ] && filtered+=("$p")
done
PREFIXES=(${filtered+"${filtered[@]}"})

if [ "${#PREFIXES[@]}" -eq 0 ]; then
  echo "check-stream-exclusion: OK — no prefix-stripping relabel rules found; nothing to keep in sync"
  exit 0
fi

# Collect every declared exclusion list. Each entry is "<where>\t<value>".
declarations=$(
  { grep -n -- '--stream-services' "$POLLER" 2>/dev/null | sed "s|^|$POLLER:|"
    grep -rn 'netview_stream_services:' "$ROLE_DEFAULTS" ops/ansible/group_vars/ 2>/dev/null
  } | sort -u
)

for prefix in "${PREFIXES[@]}"; do
  # A declared name carrying the stripped prefix can never match post-relabel.
  hits=$(printf '%s\n' "$declarations" \
    | grep -E "[\"'|=]${prefix}" \
    | grep -v '^[[:space:]]*$' \
    | sort -u || true)

  if [ -n "$hits" ]; then
    if [ "$fail" -eq 0 ]; then
      echo "check-stream-exclusion: FAIL — exclusion names carry a prefix the relabel strips"
      echo
      note "The traefik job in $PROM rewrites \`service\` at ingest, removing"
      note "\"${prefix}\". A stream-exclusion regex containing \"${prefix}\" therefore matches"
      note "nothing, and the failure is SILENT: the SSE endpoints rejoin the latency"
      note "percentiles and netview reports a held-open stream lifetime as a response"
      note "time. Drop the prefix from the value(s) below, or remove the relabel rule."
      echo
    fi
    printf '%s\n' "$hits" | sed 's/^/    /'
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  echo
  note "All three homes must agree: $POLLER (argparse default),"
  note "$ROLE_DEFAULTS, and any group_vars override."
  exit 1
fi

echo "check-stream-exclusion: OK — no exclusion name carries a stripped prefix (${PREFIXES[*]})"
