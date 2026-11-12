#!/usr/bin/env bash
# check-alerting-receiver-not-noop.sh — alerting-enabled vs all-routes-noop
# (coupled-fact guard #2, redteam remediation).
#
# THE COUPLED FACTS: an obs host runs Prometheus + Alertmanager ("alerting is
# on"), but every route in the rendered alertmanager.yml can resolve to the
# always-defined `webhook-noop` receiver, which has no integrations and
# drops every alert — alerts fire, reach Alertmanager, and reach NOBODY. The
# fall-through lives in roles/obs-host/templates/alertmanager.yml.j2 (and its
# twin roles/observability/templates/alertmanager.yml.j2):
#
#   critical_fanout_active = pagerduty_integration_key OR
#                            ntfy_alertmanager_webhook_url        (line ~52)
#   deadman_active         = deadman_webhook_url                  (line ~65)
#   route critical -> critical-fanout else webhook-noop           (lines ~68,98)
#   route Watchdog -> deadman         else webhook-noop           (line ~88)
#
# Every receiver var role-defaults to "" (roles/obs-host/defaults/main.yml
# lines 58-72), so a goatnet scope that never DECLARES them silently renders
# noop everywhere. That is exactly what group_vars/zagros_prod_obs_hosts shipped
# (verified live: `amtool config routes test severity=critical ->
# webhook-noop`, F-289).
#
# WHAT THIS GUARD PROVES (P6 honesty): this is the STATIC half. It fails when
# a route resolves to webhook-noop WITH STATIC CERTAINTY — no declaration at
# all, or a literally-empty one. A declaration via `lookup('env', ...)` counts
# as declared-but-runtime-resolved: the static verdict is "declared", never
# "delivering" — the runtime half is owned by the acceptance rows
# `deadman-switch-firing` / `deadman-switch-has-external-receiver`
# (ops/validation/acceptance-tests.yaml) plus a live paging test.
#
# Scope selection: a scope is an alerting scope if its group_vars directory
# name matches *_obs_hosts (the obs-host role's host groups) OR it declares
# any receiver var (the observability-role mgmt scopes). The Watchdog/deadman
# check applies to *_obs_hosts scopes only — only the obs-host template
# carries the deadman route.
#
# --self-test: proves the guard RED against broken fixtures and GREEN against
# a wired one before its verdict on the real tree is trusted (P6). CI runs
# the self-test before every real run.
set -euo pipefail

if [ "${1:-}" = "--self-test" ]; then
  self="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
  tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
  # RED A: obs scope with NO receiver declarations at all — the pre-remediation
  # zagros_prod_obs_hosts state, reconstructed. Critical AND Watchdog -> noop.
  mkdir -p "$tmp/redA/group_vars/broken_obs_hosts"
  cat > "$tmp/redA/group_vars/broken_obs_hosts/vars.yml" <<'EOF'
site_id: broken
site_role: observability
EOF
  if KNOWN_OPEN_FILE=/dev/null "$self" "$tmp/redA" >/dev/null 2>&1; then
    echo "SELF-TEST FAIL: guard passed an obs scope with zero receiver declarations"
    exit 1
  fi
  echo "self-test: RED proven — obs scope with no receiver declarations fails"
  # RED B: critical wired but the Watchdog/deadman route still falls to noop.
  mkdir -p "$tmp/redB/group_vars/halfwired_obs_hosts"
  cat > "$tmp/redB/group_vars/halfwired_obs_hosts/vars.yml" <<'EOF'
pagerduty_integration_key: "{{ lookup('env', 'PAGERDUTY_INTEGRATION_KEY') | default('', true) }}"
EOF
  if KNOWN_OPEN_FILE=/dev/null "$self" "$tmp/redB" >/dev/null 2>&1; then
    echo "SELF-TEST FAIL: guard passed an obs scope whose Watchdog route is noop-certain"
    exit 1
  fi
  echo "self-test: RED proven — deadman-less obs scope fails"
  # GREEN: critical + deadman both declared.
  mkdir -p "$tmp/green/group_vars/wired_obs_hosts"
  cat > "$tmp/green/group_vars/wired_obs_hosts/vars.yml" <<'EOF'
pagerduty_integration_key: "{{ lookup('env', 'PAGERDUTY_INTEGRATION_KEY') | default('', true) }}"
deadman_webhook_url: "http://100.127.0.217:9099/"
EOF
  KNOWN_OPEN_FILE=/dev/null "$self" "$tmp/green" >/dev/null
  echo "self-test: GREEN proven — wired obs scope passes"
  echo "OK: alerting-receiver-not-noop self-test — red-first proof holds"
  exit 0
fi

[ $# -eq 0 ] && cd "$(git rev-parse --show-toplevel)"
ANSIBLE_DIR="${1:-ops/ansible}"
KNOWN_OPEN="${KNOWN_OPEN_FILE:-ops/ci/coupled-fact-known-open.txt}"

python3 - "$ANSIBLE_DIR" "$KNOWN_OPEN" <<'PY'
import glob, os, re, sys

ansible_dir = sys.argv[1]
known_open_path = sys.argv[2] if len(sys.argv) > 2 else ""

# Shrink-only baseline (ops/ci/coupled-fact-known-open.txt): obs scopes already
# tracked as open (deadman leg) are grandfathered to a NOTE so the guard ships
# green and blocks NEW introductions; a grandfathered scope that no longer
# reproduces FAILS ("you fixed it — remove the line"), so the set only shrinks.
KNOWN_OPEN_DEADMAN = set()
for _line in (open(known_open_path, encoding="utf-8") if known_open_path and os.path.exists(known_open_path) else []):
    _line = _line.strip()
    if not _line or _line.startswith("#"):
        continue
    parts = _line.split("|")
    if len(parts) >= 2 and parts[0].strip() == "alerting-deadman":
        KNOWN_OPEN_DEADMAN.add(parts[1].strip())
known_open_deadman_hit = set()

# Mirrors alertmanager.yml.j2: critical_fanout_active (line ~52) and
# deadman_active (line ~65). warning-tier fan-out is deliberately not gated
# here — a goatnet may legitimately run critical-only paging.
CRITICAL_KEYS = ["pagerduty_integration_key", "ntfy_alertmanager_webhook_url"]
DEADMAN_KEY = "deadman_webhook_url"
TRIGGER_KEYS = CRITICAL_KEYS + [
    DEADMAN_KEY,
    "deadman_webhook_token",
    "slack_webhook_url",
    "smtp_smarthost",
    "email_digest_recipients",
    "warning_to_pagerduty",
]


def strip_value(raw):
    v = raw.strip()
    if v.startswith(("'", '"')):
        q = v[0]
        end = v.find(q, 1)
        if end != -1:
            return v[1:end]
    return re.split(r"\s+#", v, maxsplit=1)[0].strip()


def scan(path):
    """Top-level scalar assignments: key -> (line, value). Regex, not
    yaml.safe_load — values carry Jinja and must be read as written. A key
    assigned twice keeps the LAST assignment (YAML last-wins), which matters:
    zagros_prod_obs_hosts declares deadman_webhook_url twice."""
    out = {}
    with open(path, encoding="utf-8") as fh:
        for n, line in enumerate(fh, 1):
            if line.startswith((" ", "\t", "#", "-")) or not line.strip():
                continue
            m = re.match(r"^([a-z][a-z0-9_]*)\s*:\s*(.*?)\s*$", line.rstrip("\n"))
            if not m or m.group(2) == "" or m.group(2).startswith("#"):
                continue
            out[m.group(1)] = (n, strip_value(m.group(2)))
    return out


def scopes():
    found = {}
    for d in sorted(glob.glob(f"{ansible_dir}/group_vars/*/")):
        name = os.path.basename(d.rstrip("/"))
        if name == "_examples":
            continue
        for p in sorted(glob.glob(f"{d}*.yml")):
            for k, (line, val) in scan(p).items():
                found.setdefault(name, {})[k] = (p, line, val)
    for p in sorted(glob.glob(f"{ansible_dir}/group_vars/*.yml")):
        name = os.path.basename(p)[:-4]
        for k, (line, val) in scan(p).items():
            found.setdefault(name, {})[k] = (p, line, val)
    return found


def env_var(val):
    m = re.search(r"lookup\(\s*'env'\s*,\s*'([A-Z0-9_]+)'", val or "")
    return m.group(1) if m else None


def declared(keys, key):
    """A declaration exists and is not literally empty. env-lookup values are
    declared-but-runtime-resolved (the honest static limit)."""
    hit = keys.get(key)
    if hit is None or hit[2] == "":
        return None
    return hit


fail = 0
checked = 0
for scope, keys in sorted(scopes().items()):
    is_obs = scope.endswith("_obs_hosts")
    if not is_obs and not any(declared(keys, k) for k in TRIGGER_KEYS):
        continue  # no alerting stack configured from this scope
    checked += 1

    # Critical route (alertmanager.yml.j2 lines ~52/68/98).
    crit = [k for k in CRITICAL_KEYS if declared(keys, k)]
    if not crit:
        print(f"FAIL: group_vars/{scope}: alerting is configured but NO critical receiver is declared")
        print(f"      ({' / '.join(CRITICAL_KEYS)} all absent or literally empty; role defaults are \"\",")
        print( "      roles/obs-host/defaults/main.yml:58-72). The rendered critical route resolves to")
        print( "      webhook-noop with static certainty — alerts fire, reach Alertmanager, reach nobody")
        print( "      (amtool config routes test severity=critical -> webhook-noop). Declare at")
        print( "      least one vendor receiver var for this scope.")
        fail = 1
    else:
        envs = [f"{k} <- env {env_var(keys[k][2])}" for k in crit if env_var(keys[k][2])]
        if envs:
            print(f"note: group_vars/{scope}: critical receivers declared runtime-resolved ({'; '.join(envs)}) — "
                  "static verdict is 'declared', not 'delivering' (acceptance suite owns the live half)")

    # Watchdog/deadman route — obs-host scopes only (that template carries it,
    # alertmanager.yml.j2 lines ~65/88).
    if is_obs and not declared(keys, DEADMAN_KEY):
        if scope in KNOWN_OPEN_DEADMAN:
            known_open_deadman_hit.add(scope)
            print(f"KNOWN-OPEN (tracked, shrink-only): group_vars/{scope}: no {DEADMAN_KEY} — Watchdog route "
                  f"renders webhook-noop. Grandfathered in {known_open_path}; remove that line when wired.")
        else:
            print(f"FAIL: group_vars/{scope}: obs-host scope with no {DEADMAN_KEY} declared — the Watchdog")
            print( "      route renders webhook-noop, making the dead-man's switch decorative: a dead")
            print( "      Prometheus stops the heartbeat and no external watcher ever notices (the")
            print( "      acceptance row deadman-switch-has-external-receiver is the runtime twin of this")
            print( "      check). Wire an off-host absence-watcher endpoint.")
            fail = 1

# Shrink-only: a grandfathered obs scope that now declares a deadman must be removed.
for scope in sorted(KNOWN_OPEN_DEADMAN - known_open_deadman_hit):
    print(f"FAIL: {scope} is grandfathered in {known_open_path} for a missing deadman, but it now declares")
    print( "      one (or no longer scans as an obs scope) — the fix landed. Remove its line (set only shrinks).")
    fail = 1

if fail:
    sys.exit(1)
print(f"OK: no alerting scope routes critical/Watchdog to webhook-noop with static certainty "
      f"({checked} alerting scope(s) checked; env-declared receivers remain UNKNOWN until deploy — P6)")
PY
