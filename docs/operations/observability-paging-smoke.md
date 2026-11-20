# Observability paging smoke runbook

Turnkey verification that the alertmanager → PagerDuty notification path
is armed end-to-end. Run this:

- After the obs-host role is re-run on a site (or on first deploy)
- After PagerDuty integration-key rotation
- After alertmanager image bump
- During any site cutover-and-settle window
- Quarterly as a dry-run of the runbook itself

The discipline is intentionally narrow — verify the path works, not
that every alert rule fires correctly. Per-rule firing is verified at
soak-driver level (Block 52 / Block 27 / per-block soaks).

## Pre-flight

Verify these before firing the synthetic alert:

```bash
# (1) PagerDuty integration key is set in the operator's environment
echo "${PAGERDUTY_INTEGRATION_KEY:?PAGERDUTY_INTEGRATION_KEY not set in env}"

# (2) The site's obs-host group_var sources it from env
grep "pagerduty_integration_key" ops/ansible/group_vars/<site>_obs_hosts/vars.yml

# (3) The alertmanager container is up + config-loaded on the obs host
ssh <obs-host> "sudo docker ps --filter name=alertmanager --format '{{.Status}}'"

# (4) The deployed alertmanager.yml has pagerduty_configs.routing_key populated
ssh <obs-host> "sudo grep -A2 pagerduty_configs /opt/obs-host/alertmanager.yml | head -5"
# Expected: 'routing_key: "<32-char-hex>"' present (NOT empty string)
```

If any of (1)-(4) fails, fix that before running the smoke. Do not
synthetic-fire against a known-broken configuration — the failure mode
will be ambiguous between "config wrong" and "delivery path broken."

## Fire the synthetic alert

```bash
./ops/observability/alertmanager-paging-smoke.sh goat@<obs-host-ip>
```

The script prints the run ID and fires a synthetic critical alert
with auto-resolve at +5 min. Expected output ends with `HTTP 200`.

## Verify operator-side

Within ~1-2 min:

1. **PagerDuty UI** (your account → Service Directory →
   the relevant service → Incidents): a new incident titled "Synthetic
   paging smoke test (`<run-id>`)" should appear.

2. Within ~5-6 min the same incident should auto-resolve (the alert's
   `endsAt` triggers `send_resolved=true` on the receiver).

3. Optionally open the alertmanager UI on the obs host
   (`http://<obs-host-mesh-ip>:9093/#/alerts`, from a mesh peer; the obs
   host's exposure guard drops traffic to that address that does not arrive on
   the mesh interface) — the synthetic alert should
   appear in the firing list during its lifetime.

## Diagnostic ladder if the page doesn't land

Walk these in order. Stop at the first one that explains the failure.

### 1. Did alertmanager accept the alert?

```bash
ssh <obs-host> "sudo docker logs --since 5m alertmanager 2>&1 | grep 'Failed to validate alerts'"
```

If you see `start time must be before end time` or similar — the alert
was rejected at validation. Check the smoke script's date generation.
This was F-063's
near-cousin: macOS BSD date evaluates differently than Linux GNU date,
and remote-eval is required.

### 2. Did the alert match a route?

```bash
ssh <obs-host> "curl -sS 'http://localhost:9093/api/v2/alerts' | python3 -m json.tool"
```

Look for the synthetic alert. If `receivers` list is empty or contains
only `webhook-noop`, the route didn't match severity=critical.
Inspect `/opt/obs-host/alertmanager.yml` — the route's matchers may have
drifted from the obs-host role's template.

### 3. Did notify dispatch fire?

Default log level (info) emits errors only. To see successful
dispatches, raise log verbosity:

```bash
ssh <obs-host> "sudo docker exec alertmanager pkill -USR1 alertmanager"
# (alertmanager doesn't support log-level reload via signal — restart instead)
ssh <obs-host> "cd /opt/obs-host && sudo docker compose restart alertmanager"
# Then re-fire the smoke and watch logs:
ssh <obs-host> "sudo docker logs -f alertmanager 2>&1 | grep -iE 'notify|pagerduty'"
```

For permanent debug-level logging, the obs-host role needs to add
`--log.level=debug` to alertmanager's args. Don't leave debug on
indefinitely — log volume grows.

### 4. Did the dispatch reach PagerDuty?

If alertmanager logs show `notify` attempts but PD UI shows nothing:
- Verify the integration key in `/opt/obs-host/alertmanager.yml`
  matches the PD service's "Events API v2" Integration Key (NOT a
  legacy v1 key — same value works for both, but check the field name
  is `routing_key` not `service_key`).
- Verify outbound HTTPS to `events.pagerduty.com` is reachable from
  the obs host: `curl -I https://events.pagerduty.com` from the obs
  host or inside the alertmanager container.
- Check PD's incident-creation rate-limit — paid PD services rarely
  hit this, but worth confirming.

### 5. Did the dispatch resolve correctly?

If the page lands but doesn't auto-resolve at +5 min, check
`send_resolved: true` in the receiver config (it should be true per the
obs-host template) and verify alertmanager's clock is sane (`docker exec
alertmanager date -u` should match host UTC within seconds).

## Known issues

- **F-063 — nflog/silences permission-denied maintenance writes:**
  surfaces every 15 min in alertmanager logs as
  `caller=nflog.go:352 ... open /alertmanager/nflog.<hex>: permission
  denied`. **Not load-bearing for first-time delivery** but breaks
  restart-deduplication. Diagnose before patching;
  see F-063
  for context.

- **Default log level shows successes silently:** at default `info`
  level, alertmanager only logs notify ERRORS, not successes. Plan to
  raise to debug for one verification window when smoke-testing for
  the first time on a new deployment, then drop back.

- **PagerDuty empty-summary 400:** PD Events API v2 rejects alerts
  with empty summary. The obs-host receiver template handles this via
  fallback `description: "{{ or .CommonAnnotations.summary
  .CommonLabels.alertname \"(no summary)\" }}"`. Custom rules added
  without `summary` annotation will get the alertname as description.

## Sequencing notes

This runbook is the verification half. The arming half (operator
copies PD integration key into `.envrc.local` + obs-host role re-run
picks it up) is described in the
obs-host role tasks. If `.envrc.local` already has
`PAGERDUTY_INTEGRATION_KEY` and `obs-host` was recently re-run, the
arming half is already done — verify with the pre-flight (4) check.

For a site cutover window, expectation is: arming verified BEFORE
cutover begins, not during. Silent paging during operational landing
is unacceptable.
