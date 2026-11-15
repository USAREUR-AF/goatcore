#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# c3-measure.sh: gate row C3 on a standing goatnet, as one command.
#
#   ops/goatbuilder/c3-measure.sh --env <goatnet> [--out <dir>] [-C <checkout>]
#
# It only reads: doctor, status, diff, and verify once for each component status
# names, each with -o json, then ops/goatbuilder/c3-agreement.py over the
# reports. It times each verb, and keeps the reports and record.json (the fields
# docs/design/goatbuilder-hardware-measurement.md keeps) in a private directory:
# --out, or a new directory under $TMPDIR when --out is missing. Those files name
# the goatnet's hosts, so they stay with the operator.
#
# It prints one summary line, the one the gate row records. The goatnet appears in
# it only as "<env>", and it names no host:
#   c3 <env>: AGREE; goatbuilder <version>; checkout <sha>; transport <kind>; reached R of N machine(s); M component(s)
# Exit: 0 agree, 3 disagree (C3 fails), 2 not a measurement (a read failed;
# fix the reach and run again), 64 a usage error.
#
# The binary is $GOATBUILDER, or goatbuilder on PATH. Needs python3.
set -euo pipefail

usage() { echo "usage: $0 --env <goatnet> [--out <dir>] [-C <checkout>]" >&2; exit 64; }
env_name="" out="" checkout="."
while [ $# -gt 0 ]; do
  case "$1" in
    --env) [ $# -ge 2 ] || usage; env_name="$2"; shift 2 ;;
    --out) [ $# -ge 2 ] || usage; out="$2"; shift 2 ;;
    -C) [ $# -ge 2 ] || usage; checkout="$2"; shift 2 ;;
    -h|--help) usage ;;
    *) echo "c3: unknown argument: $1" >&2; usage ;;
  esac
done
[ -n "$env_name" ] || usage
gb="${GOATBUILDER:-goatbuilder}"
here="$(cd "$(dirname "$0")" && pwd)"

if [ -z "$out" ]; then
  out="$(mktemp -d "${TMPDIR:-/tmp}/c3-XXXXXX")"
else
  if [ -e "$out" ] && [ -n "$(ls -A "$out" 2>/dev/null)" ]; then
    echo "c3: $out is not empty; give a new directory so this run's reports are not mixed with another's" >&2
    exit 64
  fi
  mkdir -p "$out"
fi
chmod 700 "$out"
echo "c3: keeping the reports and record.json in $out (private: they name the goatnet's hosts)" >&2

now() { python3 -c 'import time; print("%.3f" % time.time())'; }
times="$out/times.tsv"
: > "$times"
# run <label> <file> <goatbuilder args...>: one verb with -o json; its exit is recorded, never fatal.
run() {
  local label="$1" file="$2" t0 t1 rc
  shift 2
  t0="$(now)"
  rc=0
  "$gb" -o json -C "$checkout" "$@" > "$out/$file" 2> "$out/$file.stderr" || rc=$?
  t1="$(now)"
  printf '%s\t%s\t%s\t%s\n' "$label" "$file" "$rc" "$(python3 -c "print('%.3f' % ($t1 - $t0))")" >> "$times"
}

run doctor doctor.json doctor --env "$env_name"
run status status.json status --env "$env_name"
run diff diff.json diff --env "$env_name"

components="$(python3 - "$out/status.json" <<'PY'
import json, sys
try:
    doc = json.load(open(sys.argv[1]))
except (OSError, ValueError):
    sys.exit(0)
seen = set()
for c in doc.get("checks", []):
    parts = c.get("name", "").split("/")
    if len(parts) == 3 and parts[2] != "inventory":
        seen.add(parts[2])
print("\n".join(sorted(seen)))
PY
)"
verify_args=()
for c in $components; do
  run "verify:$c" "verify-$c.json" verify "$c" --env "$env_name"
  verify_args+=(--verify "$c=$out/verify-$c.json")
done

agree_rc=0
if [ -z "$components" ]; then
  echo "c3: status.json names no component (status did not read the goatnet; see $out/status.json.stderr)" > "$out/c3-agreement.txt"
  agree_rc=2
else
  python3 "$here/c3-agreement.py" --status "$out/status.json" --diff "$out/diff.json" \
    ${verify_args[@]+"${verify_args[@]}"} > "$out/c3-agreement.txt" 2>&1 || agree_rc=$?
fi

version="$("$gb" --version 2>/dev/null | head -n 1 || true)"
commit="$(git -C "$checkout" rev-parse --verify -q HEAD 2>/dev/null || echo "no-commit")"

python3 - "$out" "$env_name" "$agree_rc" "$version" "$commit" <<'PY'
import json, os, re, sys

out, env_name, rc, version, commit = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4], sys.argv[5]

# Values shaped like a credential never enter record.json or the summary line.
TOKEN = re.compile(
    r"gh[opsu]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{20,}|(?:AKIA|ASIA)[0-9A-Z]{16}"
    r"|xox[abpr]-[A-Za-z0-9-]{10,}|glpat-[A-Za-z0-9_-]{20,}"
    r"|eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}"
    r"|-----BEGIN [A-Z ]*PRIVATE KEY-----")
redacted = []


def scrub(v):
    if isinstance(v, str):
        s, n = TOKEN.subn("<redacted: token-shaped>", v)
        if n:
            redacted.append(n)
        return s
    if isinstance(v, list):
        return [scrub(x) for x in v]
    if isinstance(v, dict):
        return {k: scrub(x) for k, x in v.items()}
    return v


def load(name):
    try:
        return json.load(open(os.path.join(out, name)))
    except (OSError, ValueError):
        return None


lines = [l.rstrip("\n") for l in open(os.path.join(out, "c3-agreement.txt"))]
c3_transport = next((l for l in lines if l.startswith("c3: transport: ")), "")
agree = next((l for l in lines if l.startswith("c3: agree on ")), "")
m = re.search(r"reached (\d+) of (\d+) machine\(s\)", agree)
reach = m.group(0) if m else "not measured"
m = re.search(r"(\d+) component\(s\)", agree)
ncomp = m.group(1) if m else None

times, reports = {}, []
for row in open(os.path.join(out, "times.tsv")):
    label, fname, vrc, secs = row.rstrip("\n").split("\t")
    times[label] = float(secs)
    reports.append({"verb": label, "file": fname, "exit": int(vrc)})

doctor = load("doctor.json") or {}
doctor_transport = [c for c in doctor.get("checks", [])
                    if re.search(r"transport|reach", c.get("name", ""))]

# The transport's kind only: the word after "via" in diff's transport row. The
# rest of that row names member groups and door commands, which stay private.
kind = "not recorded"
diff_transport = next((c.get("detail", "") for c in (load("diff.json") or {}).get("checks", [])
                       if c.get("name", "").endswith("/transport")), "")
m = re.search(r"\bvia (\w[\w-]*)", diff_transport)
if m:
    kind = m.group(1)

if rc == 0:
    verdict = "AGREE"
elif rc == 3:
    n = sum(1 for l in lines if l.startswith("c3: DISAGREE "))
    verdict = f"DISAGREE on {n} machine/component pair(s)" if n else "DISAGREE: a component was not compared"
else:
    verdict = "NOT A MEASUREMENT"
parts = [verdict, version or "goatbuilder version unknown", f"checkout {commit[:12]}", f"transport {kind}", reach]
if ncomp is not None:
    parts.append(f"{ncomp} component(s)")
line = "c3 <env>: " + "; ".join(parts)
# The goatnet's name, and any host the reports name, never reach the line.
names = {env_name, env_name.replace("_", "-")}
for doc in (load("status.json"), load("diff.json")):
    for c in (doc or {}).get("checks", []):
        for p in c.get("name", "").split("/"):
            if len(p) > 2:
                names.add(p)
for n in sorted(names, key=len, reverse=True):
    if n and n not in ("transport", "inventory", "diff", "status") and n in line[len("c3 <env>: "):]:
        line = line[: len("c3 <env>: ")] + line[len("c3 <env>: "):].replace(n, "<name>")
line = scrub(line)

record = scrub({
    "env": env_name,
    "build": {"goatbuilder_version": version, "checkout_commit": commit},
    "transport": {"doctor": doctor_transport, "diff": diff_transport, "c3": c3_transport},
    "reach": reach,
    "result": {"exit": rc, "c3_lines": lines},
    "times_s": times,
    "reports": reports,
    "summary": line,
})
with open(os.path.join(out, "record.json"), "w") as fh:
    json.dump(record, fh, indent=2)
    fh.write("\n")
for l in lines:
    print(scrub(l), file=sys.stderr)
if redacted:
    print(f"c3: {sum(redacted)} token-shaped value(s) left out of record.json", file=sys.stderr)
print(line)
PY
exit "$agree_rc"
