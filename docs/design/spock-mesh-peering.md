# Spock mesh peering: subscriptions with forward_origins empty

| Field | Value |
|---|---|
| Status | **Built** (the install refusal, the metric and the alert); **measured** by a three-site Docker test and a promtool test; not yet measured on a deployment |
| Roles | [`spock-mesh-peering`](../../ops/ansible/roles/spock-mesh-peering/README.md), [`ory-narrow-repset`](../../ops/ansible/roles/ory-narrow-repset/README.md), [`spock-forward-origins-check`](../../ops/ansible/roles/spock-forward-origins-check/README.md) |
| Error | GB-351 |
| Alert | `SpockSubscriptionForwardOriginsSet` |

## The rule

`spock-mesh-peering` wires every pair of sites in a goatnet with two Spock subscriptions, one
each way. `ory-narrow-repset` does the same for the Ory databases. Every subscription is created
with `forward_origins` empty: a site applies the changes its provider made, and does not pass on
changes that the provider received from a third site.

A subscription with forward_origins set in a mesh replicates in a loop. With `forward_origins`
set to `'{all}'` on three or more sites, site A applies B's change and passes it to C, C passes it
back to B, and so on, without end. Every subscription still reads `replicating`, and
`spock.exception_log` stays empty. `SpockSubscriptionLag`, `SpockSubscriptionNotReplicating` and
`SpockApplyExceptionLogged` therefore stay silent while the loop runs. A full mesh needs no
forwarding, because every site already subscribes to every other site directly.

So the install refuses it, and an alert fires if a subscription has it set.

## The install refuses it

The playbooks that run the two roles (`ops/ansible/site-bootstrap.yml` Phase 7B and 7D, and
`ops/ansible/spock-mesh-wire.yml`) first run `spock-forward-origins-check`, under the same
conditions. It stops the run with GB-351:

1. when `spock_mesh_forward_origins` or `ory_spock_forward_origins` is anything other than `[]`
   or `['none']`;
2. when any subscription, in any database, on this host's Postgres or on a peer's, has
   `spock.subscription.sub_forward_origins` set (anything but empty or `none`). The message
   names each one as `<host> <database>.<subscription>`.

The second check catches a subscription made by hand, or one made before this check existed. The
roles create subscriptions only `IF NOT EXISTS`, so they would otherwise leave it as it is. The
fix is to drop the subscription and run the playbook again; the role then creates it with
`forward_origins` empty.

The catalog is read by `files/spock-forward-origins.sh`, through
`docker exec -u postgres <container> psql` over the local socket, so no password is passed or
printed.

## An alert fires if a subscription has it set

The Phase 7B task also installs `spock-forward-origins.timer` on the host. Every minute it runs
the same script, which writes `spock_subscription_forward_origins{datname,subscription_name}`
to node-exporter's textfile directory: 1 when the subscription has forward_origins set, else 0. The
file is written to a temp file and then moved into place. The mgmt host's Prometheus scrapes the
local node-exporter and the peer's, and loads every file in `monitoring/rules/`.

| Alert | Severity | Fires when | Status |
|---|---|---|---|
| `SpockSubscriptionForwardOriginsSet` | critical | `spock_subscription_forward_origins == 1` for 2m, one series per database and subscription | built ([`rules/spock-forward-origins.yml`](../../ops/compose/mgmt-host/monitoring/rules/spock-forward-origins.yml)); measured by its promtool test ([`tests/spock-forward-origins.test.yml`](../../ops/compose/mgmt-host/monitoring/tests/spock-forward-origins.test.yml)) |

The alert reads only the subscription setting. It does not wait for the loop's symptoms, so it
fires while the subscription still reads `replicating`.

## Status

| Part | Status |
|---|---|
| Subscriptions created with `forward_origins` empty by both roles | built |
| Install refusal of a non-empty setting, and of an existing subscription with forward_origins set (GB-351) | built; measured by [`test_three_sites.py`](../../ops/postgres/spock/tests/forward-origins/test_three_sites.py) (three postgres-spock sites in a full mesh, CI `postgres-spock.yml` job `forward-origins`): with `'{all}'` the role refuses and names all six subscriptions, and with `forward_origins` empty it passes |
| `spock_subscription_forward_origins` from the timer's script | built; measured against real Spock by the same test (1 for every subscription with `'{all}'`, 0 with it empty) |
| `SpockSubscriptionForwardOriginsSet` | built; measured by promtool |
| The timer running on a deployed mgmt host | built, not measured on a deployment |
| An alert on the loop's apply rate, as a second signal | not built |
| Demonstrating the loop itself in CI | not built: the test writes no rows, so no loop runs in CI |
| `spock_mesh_expected_subscriptions{datname}` written to node-exporter's textfile directory by this role and `ory-narrow-repset`, one per declared peer site for each database each wires; `SpockMeshSubscriptionsMissing` fires when fewer replicate, including a subscription that never started | built; measured on a five-site Docker mesh and by promtool; the files by `roles/spock-mesh-peering/tests/test-expected-metrics.sh` |
