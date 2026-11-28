# Scheduling a guard that reads live state

A guard that reads the running mesh cannot run in PR CI, so it tends to run
nowhere. A guard
nobody runs is indistinguishable from no guard.

This doc is the decision about where such guards run, and — more importantly —
what a scheduled job on a mgmt host owes the person who later has to work out
whether it was involved in an incident.

## 1. Where it runs, and why not GitHub

| Option | Verdict |
|---|---|
| GitHub-hosted runner, token in repo secrets | **Rejected.** |
| Self-hosted runner on the mesh | Viable, unnecessary here. |
| **systemd timer on the goatnet's mgmt host** | **Chosen.** |
| Named person, named cadence, no automation | Rejected as the *whole* answer; kept as half of it. |

**Why not GitHub.** The check needs an admin-scoped NetBird PAT — listing setup
keys is admin-only and the setup key is the entire baseline. Actions secrets are readable by any
workflow on a same-repo branch. A repo secret would therefore convert *push
access to this repo* into *NetBird admin on a production goatnet*. That trades a
preventive control away to buy a detective one, which is a bad trade at any
detection value.

Note the tempting wrong reason to reject it: reachability. The NetBird
management API is the enrolment endpoint and is public by construction, so a
GitHub-hosted runner **is** a sufficient vantage point for this particular
check. Public reachability made the hosted runner *possible*; it never made it
*right*. Guards that read a mesh-internal surface (the portal, say) are excluded
by reachability as well — but do not reach for that argument when the real
objection is the credential.

**Why the mgmt host.** It already holds a token for this API. Running the audit
there adds no new credential exposure — the read happens where reads of this API
already happen.

**Why "a named person on a named cadence" is not the whole answer.** It is what
we already had. The check depended on somebody remembering, and nobody did.
But it survives as half the answer, because a timer **detects** and a person
must **act**. Automating
detection without naming a responder moves the silence one step later.

## 2. What a background job on a mgmt host owes you

The failure mode this guards against is not "the job breaks". It is *an incident
six months from now where the job is a suspect and there is no evidence either
way*. Each rule below exists to make the job cheap to exonerate.

**It must be visible when it is working, not only when it fails.** The audit
writes `mesh_membership_reconcile_last_run_timestamp_seconds` to the
node_exporter textfile collector on every successful read. A timer that died,
a host that lost its token, and a mesh that stopped answering then all look the
same honest way: a number that stops moving. A job whose only trace is a journal
line nobody greps is invisible by default.

**Its exit code must mean exactly one thing.** `0` = the mesh was read and no
declared-absent group has come back. `1` = a deliberate removal is present
again. `2` = the mesh was not read at all, which says *nothing* about
membership and must never be read as clean.

**It must not fail for a backlog.** The audit gates on reverted removals, not on
drift, because standing drift rows would leave the unit permanently failed —
and a permanently failed unit is muted, then ignored, then invisible.

**It must be provably not the cause.** The unit is read-only against the mesh
(four GETs) and confined against the host: `ProtectSystem=strict`,
`NoNewPrivileges`, an empty `CapabilityBoundingSet`, and `ReadWritePaths`
limited to its report directory and the textfile directory. Whatever changed on
that host, this did not change it.

**It must not collide.** `RandomizedDelaySec=600` and a schedule offset from the
host's other timers, plus `Nice=10` and idle IO. Two unrelated jobs firing on
the same second is how they end up in each other's incident timelines.

**`enabled: false` must tear it down.** Not skip — remove: stop, disable, delete
the units, the script, the env file, and the stale metric. A gate that only
declines to redeploy leaves a timer running on a host with nothing in the repo
admitting it exists, which is the precise shape of an untraceable background
process.

**Its credential must be its own.** A dedicated service user per goatnet
(`svc-zagros-membership-reconcile`, `svc-zagros-prod-membership-reconcile`), minted
explicitly by `ops/scripts/netbird-service-identities.py <goatnet> --mint`
(identity `membership-reconcile` in `ops/netbird/service-identities.yaml`) rather
than as a side effect of a playbook — creating a mesh admin is a decision, not a deploy
step.

Be honest about what that buys. NetBird has no read-only role that can list
setup keys, so the dedicated user is still `role: admin`. It is **not** least
privilege. It buys two real things: revocation without collateral (a shared token serves other surfaces), and attribution — its reads appear in the activity log as this service
user rather than as another anonymous admin uuid.

## 3. Operating it

```bash
# install / update
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES aws-vault exec usareur-af --no-session -- \
  ../scripts/ansible-preflight.sh -i inventory/mgmt-hosts-mesh.ini \
  mesh-membership-reconcile.yml -e target=zagros_hosts

# remove it, properly
... mesh-membership-reconcile.yml -e target=zagros_hosts \
    -e mesh_membership_reconcile_enabled=false

# on the host
systemctl list-timers mesh-membership-reconcile.timer
journalctl -u mesh-membership-reconcile.service -n 50
cat /var/lib/goat/membership-reconcile/last-run.txt
```

| Series | Read it as |
|---|---|
| `mesh_membership_reconcile_last_run_timestamp_seconds` | Stops moving ⇒ the audit is dead. This is the one to alert on. |
| `mesh_membership_reverted_total` | Non-zero ⇒ a deliberate removal came back. Expected 0 by construction. |
| `mesh_membership_drift_total` | The backlog. Non-zero today and not an alarm. |
| `mesh_membership_unknown_baseline_total` | Peers the check *cannot speak about*. Not clean, not dirty. |

The timer's output is discoverable, not pushed; reading it is the named-owner half of §1.

**And do not treat the metric as the whole answer.** That is why the unit *also* fails: a red
`systemctl --failed` is a second, independent surface that does not depend on
the Prometheus path being healthy. Two surfaces is not belt-and-braces here, it
is the minimum — a guard whose only signal rides one pipeline inherits every
outage of that pipeline.

## 4. Adding another guard — the contract

A second guard is its own role that **conforms to this contract**, not an entry
in a shared list. That is a deliberate reversal of a proposal made on the
session bus on 2026-08-14, and the reasons are worth stating because the
opposite instinct is the usual correct one:

- **The first instance has not run in anger yet.** At the time of writing, the
  membership audit has never fired on a real mgmt host. Generalising an
  abstraction over one unproven implementation and one unbuilt one is the wrong
  order, whatever the line count says.
- **The differences are not config.** The provenance guard (ADR 1130) needs two
  credentials on two planes, a portal service key written into the portal's
  hot-reload key directory, a rendered allowlist file, and a per-goatnet
  enforce/report-only flag. A list-driven role covering that needs conditional
  *install* logic per entry, which is where list-driven ansible roles stop being
  simpler than two roles.

So: **two roles, one contract, and fold at three.** Two bespoke roles is
tolerable; three is the duplication
[ADR 1016](../adr/1016-complexity-budget-substrate-encapsulation.md) exists to
prevent. The fold is a named follow-up once both have run in production long
enough to show which parts are genuinely common.

Conform to these, and folding later is mechanical rather than a redesign:

| | |
|---|---|
| **Units** | `<guard>.service` + `<guard>.timer`, `Type=oneshot`, `Documentation=` pointing at this doc. |
| **Exit codes** | `0` read + clean · `1` the condition the guard exists to catch · `2` **the source could not be read**, which says nothing and must never be taken as clean. |
| **Never fail on a backlog** | Gate on the regression, not on standing debt. A permanently failed unit is muted, then ignored, then invisible. |
| **Metrics** | `<guard>_last_run_timestamp_seconds` written to the node_exporter textfile dir on every successful read, plus one gauge per condition counted. Atomic write. Stamped only after all reads succeed. |
| **Report** | Human-readable last run at a fixed path, and echoed to the journal so `journalctl -u <guard>` is self-sufficient. |
| **Confinement** | Read-only against its source; `ProtectSystem=strict`, empty `CapabilityBoundingSet`, `ReadWritePaths` limited to report + metrics dirs. |
| **Politeness** | `Nice=10`, idle IO, `RandomizedDelaySec` ≥ 300, schedule offset from the host's other timers. |
| **Teardown** | `<guard>_enabled: false` stops, disables and removes units, script, env file **and the stale metric**. Never merely skip. |
| **Credentials** | Dedicated service identity per guard per goatnet, minted by an operator-run script, never as a playbook side effect (the one exception is the bootstrap identity, which a founder's standup mints: [ADR 1274](../adr/1274-standup-mints-the-bootstrap-netbird-token-through-the-management-image.md), Proposed). Refuse to install unconfigured rather than deploying a unit that fails nightly. |
| **Config on the host** | Anything a reviewer would want to see — allowlists, exception lists — is rendered from `group_vars`, never hand-edited on the box. |
