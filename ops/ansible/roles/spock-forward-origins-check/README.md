# `spock-forward-origins-check` — refuse forward_origins on Spock subscriptions

A subscription with forward_origins set in a mesh replicates in a loop. With
`forward_origins` set to `'{all}'` on three or more sites, each change goes on
to the next site and comes back, without end. Every subscription still reads
`replicating` and `spock.exception_log` stays empty, so no other alert fires.
This role makes the install refuse such a subscription, and it installs the
metric that makes an alert fire if a subscription has it set.

Design: [`docs/design/spock-mesh-peering.md`](../../../../docs/design/spock-mesh-peering.md).

## When it runs

It runs as the first task before `spock-mesh-peering` and `ory-narrow-repset`, under the same
conditions as the role it precedes:

- `ops/ansible/site-bootstrap.yml`, the Phase 7B and Phase 7D plays;
- `ops/ansible/spock-mesh-wire.yml`, both plays.

## What it does

1. Refuses (GB-351) a `forward_origins` setting other than `[]` or `['none']`:
   `spock_mesh_forward_origins` and `ory_spock_forward_origins` from the site's group_vars.
2. Runs `files/spock-forward-origins.sh list` on this host and on each peer (the hosts in
   `mgmt_hosts` that set `participates_in_spock`, with `router_role` A or none). The script
   reads `spock.subscription.sub_forward_origins` in every database that has the spock
   schema, through `docker exec -u postgres <container> psql` (no password). The role refuses
   (GB-351), naming every subscription with forwarding set as `<host> <database>.<subscription>`.
3. With `spock_forward_origins_watch: true` (the Phase 7B tasks set it), installs
   `spock-forward-origins.timer`. Every minute it runs the same script in `prom` mode, which
   writes `spock_subscription_forward_origins{datname,subscription_name}` (1 when forwarding is
   set, else 0) to node-exporter's textfile directory, by temp file then `mv`. The alert
   `SpockSubscriptionForwardOriginsSet`
   ([`ops/compose/mgmt-host/monitoring/rules/spock-forward-origins.yml`](../../../compose/mgmt-host/monitoring/rules/spock-forward-origins.yml))
   reads it.

## Variables

| Variable | Default | Meaning |
|---|---|---|
| `spock_forward_origins_check_settings` | the two settings above | name → value; each must be `[]` or `['none']` |
| `spock_forward_origins_check_container` | `spock_mesh_postgres_container`, else `postgres-a` | this host's Postgres container, and the default for a peer |
| `spock_forward_origins_check_peers` | `true` | also read each peer's subscriptions |
| `spock_forward_origins_watch` | `false` | install the timer |
| `spock_forward_origins_watch_interval` | `60s` | how often the timer writes the metric |
| `spock_forward_origins_metrics_dir` | `node_exporter_textfile_host_dir`, else `/var/lib/node_exporter/textfile` | node-exporter's textfile directory |

## Fixing a refusal

On the host and database named, drop the subscription
(`SELECT spock.sub_drop('<subscription>');`) and run the playbook again. The role that owns it
creates it again with `forward_origins := ARRAY[]::text[]`.

## Tests

- `ops/postgres/spock/tests/forward-origins/test_three_sites.py` (CI: `postgres-spock.yml`,
  job `forward-origins`): three postgres-spock sites in a full mesh, no host ports. With
  `forward_origins` `'{all}'` this role refuses and names all six subscriptions. A setting of
  `[all]` is refused, `[]` and `[none]` pass, and with `forward_origins` empty it passes. The
  script's metric is 1 and then 0 for every subscription. The test writes no rows, so no loop
  runs.
- `ops/compose/mgmt-host/monitoring/tests/spock-forward-origins.test.yml` (promtool, CI:
  `goat-obs-checks.yml`).
