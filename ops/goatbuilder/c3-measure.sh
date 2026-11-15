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
# Exit: 0 the three verbs agree on every machine; 3 they disagree (C3 fails);
# 2 not a measurement: a read failed, a verb timed out, a machine was not
# reached ("NOT A MEASUREMENT: reached R of N machine(s)"), or the agreement
# script or the summary failed. Fix what it names and run again. 64 a usage error.
#
# What it changes besides its own directory: on a goatnet reached through an SSM
# door (ops/scripts/door-open.sh) while this machine is not on the mesh, status,
# diff and verify open the door, as they do when run by hand. The script reads
# the door's state first, and closes it at exit if this run opened it; a door
# that was already open is left open. Nothing on a host, in a ledger or in the
# goat context changes.
#
# Each verb is stopped after C3_VERB_TIMEOUT seconds (default 600); a verb that
# is stopped leaves an unreadable report, and the run is not a measurement.
#
# Values are scrubbed from record.json, the summary line and the kept *.stderr
# files: the shapes of known credentials, and the value of every environment
# variable that ops/ci/check-no-live-secret.sh treats as a live secret. The
# verbs' reports (*.json) are kept as the verbs wrote them: they name the
# goatnet's hosts, and stay private with the rest of the directory.
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

# The environment variables check-no-live-secret.sh would treat as live secrets,
# by name: their values are scrubbed below. The pattern, the length floor and the
# name-shape test are read from that script, not copied.
live_secret="$here/../ci/check-no-live-secret.sh"
eval "$(sed -n '/^MIN_LEN=/p;/^MAX_NAME_SEGMENT=/p;/^CANDIDATE_NAME_RE=/p;/^is_name_shaped()/,/^}/p' "$live_secret")"
secret_names=""
while IFS= read -r name; do
  eval "val=\${$name-}"
  [ -n "$val" ] && [ "${#val}" -ge "$MIN_LEN" ] || continue
  is_name_shaped "$val" && continue
  secret_names="$secret_names $name"
done < <(compgen -e | grep -E "$CANDIDATE_NAME_RE")
unset val

# The door: its state before the verbs run, so a door this run opens is closed
# again at exit, and one the operator had open is left as it was.
door="$checkout/ops/scripts/door-open.sh"
door_env="${env_name//_/-}"
door_was=""
if [ "${GOAT_DOOR_OPEN:-1}" != 0 ] && [ -x "$door" ]; then
  if "$door" "$door_env" --status >/dev/null 2>&1; then door_was=open; else door_was=closed; fi
fi
# shellcheck disable=SC2329 # invoked by the EXIT trap below
close_door() {
  if [ "$door_was" = closed ] && "$door" "$door_env" --status >/dev/null 2>&1; then
    if "$door" "$door_env" --close >/dev/null 2>&1; then
      echo "c3: closed the door this run opened" >&2
    else
      echo "c3: this run opened the door and could not close it: $door <goatnet> --close" >&2
    fi
  fi
}
trap close_door EXIT

now() { python3 -c 'import time; print("%.3f" % time.time())'; }
times="$out/times.tsv"
: > "$times"
timeout_s="${C3_VERB_TIMEOUT:-600}"
case "$timeout_s" in ''|*[!0-9]*|0) echo "c3: C3_VERB_TIMEOUT must be a whole number of seconds above 0" >&2; exit 64 ;; esac
# run <label> <file> <goatbuilder args...>: one verb with -o json, stopped after
# C3_VERB_TIMEOUT seconds (exit 124); its exit is recorded, never fatal.
run() {
  local label="$1" file="$2" t0 t1 rc
  shift 2
  t0="$(now)"
  rc=0
  python3 - "$timeout_s" "$out/$file" "$out/$file.stderr" "$gb" -o json -C "$checkout" "$@" <<'PY' || rc=$?
import os, signal, subprocess, sys
limit, out, err, argv = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4:]
with open(out, "wb") as o, open(err, "wb") as e:
    p = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=o, stderr=e, start_new_session=True)
    try:
        sys.exit(p.wait(timeout=limit))
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        p.wait()
        e.write(f"c3: timed out after {limit}s and was stopped\n".encode())
        sys.exit(124)
PY
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
  case "$agree_rc" in
    0|2|3) ;;
    *) echo "c3: c3-agreement.py failed (exit $agree_rc): not a measurement" >> "$out/c3-agreement.txt" ;;
  esac
fi
# A verb that was stopped leaves a report that cannot be trusted, even where the
# agreement script could read it: the run is not a measurement.
while IFS="$(printf '\t')" read -r label _ vrc _; do
  if [ "$vrc" = 124 ]; then
    echo "c3: $label timed out after ${timeout_s}s and was stopped: not a measurement" >> "$out/c3-agreement.txt"
    agree_rc=2
  fi
done < "$times"

version="$("$gb" --version 2>/dev/null | head -n 1 || true)"
commit="$(git -C "$checkout" rev-parse --verify -q HEAD 2>/dev/null || echo "no-commit")"

final=0
C3_SECRET_NAMES="$secret_names" C3_MIN_LEN="$MIN_LEN" \
python3 - "$out" "$env_name" "$agree_rc" "$version" "$commit" <<'PY' || final=$?
import json, os, re, sys

out, env_name, rc, version, commit = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4], sys.argv[5]

# Values shaped like a credential never enter record.json or the summary line.
TOKEN = re.compile(
    r"gh[opsu]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{20,}|(?:AKIA|ASIA)[0-9A-Z]{16}"
    r"|xox[abpr]-[A-Za-z0-9-]{10,}|glpat-[A-Za-z0-9_-]{20,}"
    r"|eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}"
    r"|-----BEGIN [A-Z ]*PRIVATE KEY-----")
redacted = []


# The live secrets of the operator's environment, by the names
# check-no-live-secret.sh picks (the shell passes the names, never the values).
LIVE = sorted(((n, os.environ[n]) for n in os.environ.get("C3_SECRET_NAMES", "").split()
               if len(os.environ.get(n, "")) >= int(os.environ.get("C3_MIN_LEN", "16"))),
              key=lambda nv: len(nv[1]), reverse=True)


def scrub(v):
    if isinstance(v, str):
        s = v
        for name, val in LIVE:
            if val in s:
                redacted.append(s.count(val))
                s = s.replace(val, f"<redacted: ${name}>")
        s, n = TOKEN.subn("<redacted: token-shaped>", s)
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

m = re.search(r"reached (\d+) of (\d+) machine", reach)
if rc == 0 and m and m.group(1) == m.group(2):
    verdict = "AGREE"
elif rc == 0:
    # The three verbs agree, but C3 is about every machine: a machine not reached
    # is unknown under all three, which is agreement about nothing.
    verdict = f"NOT A MEASUREMENT: {reach}" if m else "NOT A MEASUREMENT: no machine reached"
    rc = 2
elif rc == 3:
    n = sum(1 for l in lines if l.startswith("c3: DISAGREE "))
    verdict = f"DISAGREE on {n} machine/component pair(s)" if n else "DISAGREE: a component was not compared"
else:
    verdict = "NOT A MEASUREMENT"
    rc = 2
parts = [verdict, version or "goatbuilder version unknown", f"checkout {commit[:12]}", f"transport {kind}"]
if reach not in verdict:
    parts.append(reach)
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
# The verbs' stderr is kept beside the reports; it is scrubbed as the record is.
for name in sorted(os.listdir(out)):
    if name.endswith(".stderr"):
        path = os.path.join(out, name)
        with open(path, encoding="utf-8", errors="replace") as fh:
            text = fh.read()
        clean = scrub(text)
        if clean != text:
            with open(path, "w", encoding="utf-8") as fh:
                fh.write(clean)
for l in lines:
    print(scrub(l), file=sys.stderr)
if redacted:
    print(f"c3: {sum(redacted)} credential value(s) left out of record.json and the kept stderr", file=sys.stderr)
print(line)
sys.exit(rc)
PY
case "$final" in
  0|2|3) ;;
  *)
    # The summary itself failed: say so in the one line, never exit 1.
    echo "c3 <env>: NOT A MEASUREMENT: the summary could not be made (exit $final); the reports are in the private directory"
    final=2 ;;
esac
exit "$final"
