# obs-host (per-site observability + storage Orin)

> Anchor: ADR 0826
> · Companion role: [observability/](../observability/) (legacy single-host).

## Scope

Phase 1 (this role today):
- Prometheus (scrapes mgmt-host-pair exporters via wg-cp0 / LAN)
- Alertmanager (paging fan-out — same template as the legacy role)
- Grafana (dashboards; reverse-proxied via mesh-internal DNS once enrolled)
- node_exporter (Orin-C's own metrics)

Phase 2 (Block 17D + Block 27 follow-on, NOT this role):
- OpenObserve (wide-event log substrate, Block 27)
- MinIO (audit-integrity bucket per Block 17D + telemetry-otlp bucket per Block 27)
- OTel Collector (active-passive at the per-site obs tier)

## Required vars

`obs_scrape_targets` — list of dicts with `{name, addr, job}`. See
`group_vars/vega_obs_hosts/vars.yml`
for the vega instance. Pre-wg-cp0 the addr is a LAN IP; post-wg-cp0
it's the mgmt-host's wg-cp0 internal address.

## Why a separate role from `observability/`

The legacy `observability` role co-locates Prometheus + Alertmanager
with the mgmt-host stack and scrapes a single peer (`peer_node_exporter_target`).
ADR 0826 splits observability OFF the mgmt-hosts — Orin-C scrapes BOTH
mgmt-hosts, doesn't run any postgres/signal/management of its own, and
later picks up MinIO + OpenObserve + OTel that the legacy role doesn't
need.

Merging the two is a refactor we'll do once OpenObserve + MinIO land
on Orin-C and the legacy role's "co-located" use case has been
thoroughly retired (probably never — nethost dev / integdev / vega cloud-PoC
all keep the legacy single-host shape).

## Blackbox probing and TLS certificate observability

Off by default (`obs_blackbox_enabled: false`). When set, the role adds a
`blackbox-exporter` container, a `blackbox-https` scrape job, and the
`tls-cert-renewal.yml` rule file — as one unit, deliberately.

**Why they move together.** `tls-cert-renewal.yml` carries
`TlsCertExpiryUnobserved: absent(probe_ssl_earliest_cert_expiry{job="blackbox-https"})`.
That expression is TRUE on any Prometheus that does not scrape blackbox, so
listing the rule flatly in `obs_rule_files` would fire a permanent critical on
every obs host without an exporter. It is appended by the flag instead, in the
`obs_rule_files_all` fact that both the install loop and the did-they-load
assertion read.

**Why it exists.** With no blackbox job `probe_ssl_earliest_cert_expiry` has no
series and the renewal alerts cannot fire. Certificates renew unattended via
Traefik's ACME resolver, and Traefik renews at 30 days remaining; a renewal that
quietly stops therefore has a month of runway before the certificate dies. See ADR 1140.

| Variable | Meaning |
|---|---|
| `obs_blackbox_enabled` | Master switch. Adds exporter + job + rule file. |
| `obs_blackbox_targets` | Full `https://…` URLs including health path. One per vhost — coverage is per-target. |
| `obs_blackbox_resolve_ip` | Mesh IP the probe names resolve to, rendered into `extra_hosts`. Required for mesh-zone targets. |
| `obs_blackbox_module` | Probe module, default `http_gate` (no redirect-following, per ADR 1133). |
| `site_dns_zone` | Required when enabled — the shared blackbox module file interpolates it. |

**Coverage is per-target, and the gap is silent.** A vhost with no entry in
`obs_blackbox_targets` has an unwatched certificate while the alerts read green
from its neighbours, because `absent()` is false as long as any one target
still reports a cert. `ops/ci/check-blackbox-covers-registry.sh` fails the build
when a `probe_gate` service in the site's `isitup_services` has no probe target
in the obs group's vars.

The role's preflight fails closed on an enabled exporter with no targets, and on
an enabled exporter with no `site_dns_zone` — both otherwise surface as a
critical alert 30 minutes after a deploy that looked successful.

**Not the full ADR 1133 canary.** This is the gate plane only. goat-obs also
runs `blackbox-surface-app`, probing each service's own container over a shared
docker network; an obs host on its own machine has no such network, so a surface
that answers at the gate while broken behind it reads healthy here.

## Alert rules, and the metric that has to exist for one to fire

`obs_rule_files` is a **role default**, so a rule file added for one goatnet loads on every
obs host in the fleet. `obs_scrape_targets` is a **per-goatnet list that starts empty**. The
fleet-wide half grows; the per-goatnet half does not.

That asymmetry is F-449: rules can load over metric families a Prometheus was never
told to scrape. **A rule with no metric is exactly as
silent as no rule, and it reads greener** — an unfed alert and a healthy system are the same
colour on every dashboard.

Two things now stand between that and a repeat:

- **`ops/ci/check-obs-rules-have-scrapers.sh`** fails the build when a rule file in this
  goatnet's effective rule set keys on a metric family no job in its `obs_scrape_targets`
  produces. Gaps can be declared in `ops/pins/obs-metric-sources.yaml` with a reason and an
  `until:` date, the same contract `parity-capabilities.yaml` uses for deviations.
- **`obs_rule_files_exclude`** drops individual files from the default list without restating
  it. Use it when the site does not run the thing a rule watches — a site with no
  audit-subscriber and no MinIO, where `audit-subscriber.yml`'s six rules could never fire.
  Prefer it over overriding `obs_rule_files`: an overridden list silently stops inheriting
  every rule file added later, which is the same quietly-falls-behind shape as F-449 itself.

**What the guard does NOT check:** that the exporter emits the metric NAME the rule uses. It
checks that the producing JOB is scraped. Only a live vantage catches that, and that
is a mgmt-host timer, not CI.

## Reaching a metrics endpoint from off-box (ADR 1161)

This role's Prometheus is on a **different machine** from the services it watches, so anything
the co-located `goat-obs` stack reaches by container name — `traefik:8082`,
`postgres-exporter:9187`, `cadvisor:8080` — is unreachable here. Where the endpoint exists, the
fix is to publish it on the mgmt host's **mesh address** (`traefik_metrics_bind`), never
`0.0.0.0`. Where the container does not exist at all (cadvisor, process-exporter,
postgres-exporter on the split-obs goatnets), it needs an agent on the mgmt hosts and is
recorded as a cross-goatnet row in `ops/pins/parity-capabilities.yaml` rather than assumed.

Publish the port **before** adding the scrape row: a target with nothing behind it pages
within minutes.

## Dashboards, and why a site installs only some of them

This role provisions Grafana dashboards from `ops/compose/goat-obs/grafana/dashboards/`
— the same source the co-located stack uses, so a panel fix reaches both instead of
being made twice. `obs_grafana_dashboards_enabled` defaults **true**, so a new
goatnet gets them without asking.

**Shipping all of them is the other half of the same mistake.** A container dashboard
on a goatnet with no cadvisor answers "how are the containers" with blank graphs, and
a blank graph reads as idle rather than as "nothing is collecting this". So a site
names what it cannot populate in `obs_grafana_dashboards_exclude` — the same contract
as `obs_rule_files_exclude`, for the same reason: naming the absence keeps the site
inheriting everything added later.

`ops/ci/check-obs-dashboards-have-metrics.sh` enforces it. For each obs group it
resolves every dashboard's metric families through `ops/pins/obs-metric-sources.yaml`
to the job that produces them, and asks whether that group's `obs_scrape_targets`
names it.

- **Fails** when an installed dashboard can draw *nothing* — zero families served.
- **Notes** when it is under half populated, because "is a partial view useful or
  misleading" is an editorial call, and a guard that makes editorial calls gets
  argued with and then switched off.

It scores against **declared config, not a live host**, so it works for a goatnet
that does not exist yet. That is the point: standing up a new environment cannot
repeat this.

### Rollout: mesh-only ports, and what that changes

Grafana (3000), Prometheus (9090) and Alertmanager (9093) publish on loopback
and the host's mesh (wt0) address only, and an nftables table
(`inet goat_obs_expose`, loaded by `goat-obs-expose-guard.service`) drops
traffic to them on the mesh address unless it arrives on wt0. Binding the mesh
address alone is not enough: a LAN client that routes the mesh address via the
host's LAN address would reach them. `goat-obs-expose-guard-check.timer`
re-applies the table if a ruleset flush removes it or changes its drop rules.
That leaves a window: after a root-level flush (a start or reload of
`nftables.service`, or `nft -f /etc/nftables.conf`, which begins with
`flush ruleset`), the three ports are open to such a LAN client until the timer
next fires, up to about 65 s (60 s interval plus 5 s accuracy; measured 40 to
58 s). The check does not notice an `accept` rule inserted ahead of the drops;
`systemctl reload goat-obs-expose-guard` restores the table exactly.

- The play stops if wt0 has no IPv4 address. Enrol the host on the mesh before
  the obs leg, set `obs_bind_address`, or set `obs_bind_all: true` (this also
  removes the guard).
- After a reboot the stack cannot start until NetBird has given wt0 its
  address, and Docker does not retry a failed start. `goat-obs-stack.service`
  waits for the address and then runs `docker compose up -d --force-recreate`.
  It recreates rather than starts because a container whose start failed for
  want of the address has lost its network, so a plain start is not enough.
  It gives up after 3 failed attempts in 10 minutes (one container that cannot
  start would otherwise recreate the stack every 30 s). If the stack is down
  after a reboot: `systemctl status goat-obs-stack` (still waiting means no
  mesh address yet); after fixing the cause,
  `systemctl reset-failed goat-obs-stack && systemctl start goat-obs-stack`,
  or `cd /opt/obs-host && sudo docker compose up -d --force-recreate`.
- Prometheus' `/-/reload` and `/-/quit` are off (`obs_prometheus_web_lifecycle`);
  reload with `docker kill --signal HUP prometheus`.
- Containers on this host must reach the stack by compose service name or
  loopback, not by the mesh address (the guard drops that path).

### Standing up a new obs host

1. Write `obs_scrape_targets` for what the site actually runs.
2. Run `ops/ci/check-obs-dashboards-have-metrics.sh`. It names every dashboard the
   site cannot draw.
3. Exclude those, each with the reason and what would have to arrive to bring it back.
4. Run `ops/ci/check-obs-rules-have-scrapers.sh` for the alert half of the same
   question.

## Changing a receiver without re-running the stack

`ops/ansible/obs-alertmanager-render.yml -e target=<site>_obs_hosts` writes this
role's `alertmanager.yml` and makes the running Alertmanager load it. It touches
nothing else on the host: no clock, no docker engine, no Prometheus, no Grafana.
Use it when the change is a receiver: a heartbeat address, a paging key, a
webhook. `obs-host-bootstrap.yml` writes the same file through the same task
(`tasks/alertmanager-render.yml`) as part of the whole stack.

The play includes this role's task files, so the role's defaults stay below the
site's settings. The role's dependency, the rollback ledger, appends a row only
when the image ref changes, so a receiver change writes none.

The play refuses before it writes anything (`tasks/alertmanager-guard.yml`,
error `X-ANSIBLE-006`):

- `deadman_webhook_url` is set and `deadman_webhook_token` resolved empty. A
  watcher refuses a heartbeat without its token and pages for the host. A site
  whose watcher runs with no token sets `deadman_webhook_token_optional: true`.
- The new config lacks a receiver, or a receiver's credential, that the running
  config has (`files/alertmanager-receiver-guard.py`, run on the controller; it
  prints receiver names and notifier kinds only). A receiver's secret comes from
  the deploy environment, and an environment that lacks one renders the receiver
  without it. To remove a receiver on purpose, pass
  `-e obs_alertmanager_allow_receiver_loss=true`.

After the write the play compares the file the container reads with the file on
the host, recreates the Alertmanager container alone when they differ (a
single-file bind mount keeps the old file after a rewrite), and fails unless
Alertmanager reports the config loaded.
