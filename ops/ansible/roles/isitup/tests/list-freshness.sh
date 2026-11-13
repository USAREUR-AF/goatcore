#!/usr/bin/env bash
# list-freshness.sh: the list-only path and the stale-list check.
#
# What it holds:
#   - isitup-rows.yml (the list-only play) and isitup-rows-render.yml parse;
#   - the row list renders from a declaration that sets NO OIDC client secret
#     and NO cookie secret (fixture/group_vars), so the list-only path needs
#     neither;
#   - the digest the role stamps is the sha256 of the canonical row list, and
#     the plain-jinja2 harness the other tests use computes the same one;
#   - ops/scripts/isitup-list.py check exits 0 for a meta.json carrying the
#     declared list, 1 with a plain message for one that is behind, and 3 for a
#     meta.json written before the stamp existed.
#
# The render runs ansible-playbook against fixture/inventory.ini: one local
# host, no remote connection. Skips without ansible-playbook.
set -euo pipefail
cd "$(dirname "$0")"

if ! command -v ansible-playbook >/dev/null 2>&1; then
  echo "[isitup/list-freshness] SKIP (ansible-playbook not on PATH)"
  exit 0
fi

ANSIBLE_DIR="$(cd ../../.. && pwd)"
TOOL="$(cd ../../../../scripts && pwd)/isitup-list.py"
FIXTURE="$(pwd)/fixture/inventory.ini"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
export ANSIBLE_NOCOLOR=1

fail=0
pass() { echo "[isitup/list-freshness] PASS  $*"; }
flunk() { echo "[isitup/list-freshness] FAIL  $*"; fail=1; }

for play in isitup-rows.yml isitup-rows-render.yml; do
  if (cd "$ANSIBLE_DIR" && ansible-playbook -i 'localhost,' -c local -e target=localhost "$play" --syntax-check) >"$WORK/syntax.log" 2>&1; then
    pass "$play parses"
  else
    flunk "$play does not parse"; cat "$WORK/syntax.log"
  fi
done

# No sign-in secret anywhere in the environment of the render.
if env -u GOAT_DEPLOY_SOURCE_TREE python3 "$TOOL" render listtest -i "$FIXTURE" --json >"$WORK/declared.json" 2>"$WORK/render.err"; then
  pass "the row list renders with no OIDC client secret and no cookie secret set"
else
  flunk "the row list did not render"; cat "$WORK/render.err"
  exit 1
fi

python3 - "$WORK" "$(pwd)" <<'PY' || fail=1
import hashlib, json, os, sys

work, here = sys.argv[1], sys.argv[2]
fail = 0
def check(ok, msg):
    global fail
    print("[isitup/list-freshness] %s  %s" % ("PASS" if ok else "FAIL", msg))
    fail |= (not ok)

d = json.load(open(os.path.join(work, "declared.json"), encoding="utf-8"))
ids = [r["id"] for r in d["rows"]]
check(ids == ["portal", "docs", "board"], "the fixture declares portal, docs and board: %s" % ids)
canonical = json.dumps(d["rows"], sort_keys=True, separators=(",", ":"), ensure_ascii=True)
check(d["sha256"] == hashlib.sha256(canonical.encode()).hexdigest(),
      "the stamped digest is the sha256 of the canonical row list")
check(isinstance(d["source"], dict), "the stamp names its source as an object: %s" % d["source"])

# The harness the other tests render with must agree with the role, or those
# tests would be pinning a list the deploy never produces.
try:
    import yaml
    sys.path.insert(0, here)
    from render_services_env import render, rows_sha256
except ImportError:
    print("[isitup/list-freshness] SKIP  harness comparison (python3 jinja2 or PyYAML not importable)")
else:
    ctx = yaml.safe_load(open(os.path.join(here, "fixture/group_vars/listtest_hosts/vars.yml"), encoding="utf-8"))
    rows, _, _ = render(ctx)
    check(rows == d["rows"], "the plain-jinja2 harness renders the same rows as the role")
    check(rows_sha256(rows) == d["sha256"], "and computes the same digest")

def meta(rows, stamp):
    m = {"rows": {i: {"lifecycle": "live", "source": "core", "owner": "", "retire_after": ""} for i in rows}}
    if stamp is not None:
        m["rendered"] = stamp
    return m

stamp = {"sha256": d["sha256"], "rows": len(ids), "at": "2026-01-01T00:00:00Z", "source": {"commit": "a" * 40}}
json.dump(meta(ids, stamp), open(os.path.join(work, "current.json"), "w"))
# A page still carrying a row the declaration no longer has.
json.dump(meta(ids + ["removed-service"], dict(stamp, sha256="0" * 64)), open(os.path.join(work, "behind.json"), "w"))
# A meta.json from before the stamp existed: rows only.
json.dump(meta(ids, None), open(os.path.join(work, "unstamped.json"), "w"))
sys.exit(fail)
PY

expect() {  # <meta file> <exit code> <text the output must carry> <what it shows>
  local rc=0 out
  out="$(python3 "$TOOL" check listtest -i "$FIXTURE" --meta "$WORK/$1" 2>&1)" || rc=$?
  if [ "$rc" = "$2" ] && printf '%s' "$out" | grep -q -- "$3"; then
    pass "$4 (exit $rc)"
  else
    flunk "$4: wanted exit $2 and '$3', got exit $rc:"; printf '%s\n' "$out"
  fi
}
expect current.json 0 "listtest: current" "a page carrying the declared list is current"
expect behind.json 1 "on the page, not declared.*removed-service" "a page behind the declaration is reported, with the row that should be gone"
expect unstamped.json 3 "carries no list stamp" "a meta.json from before the stamp is reported as unknown, not as current"

rc=0; out="$(python3 "$TOOL" check listtest -i "$FIXTURE" --meta - <"$WORK/behind.json" 2>&1)" || rc=$?
if [ "$rc" = 1 ]; then pass "the meta.json is read from standard input with --meta -"; else flunk "--meta - gave exit $rc: $out"; fi

exit "$fail"
