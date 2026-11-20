# Smoketest

`smoketest` runs configured infrastructure probes and serves their results with
the dashboard in [`dashboard/`](dashboard/). The dashboard is plain JavaScript;
it has no separate build step.

The repository does not provide a supported deployment wrapper for this tool.
Deployment belongs to a goatnet's overlay, where its targets, credentials,
networking, and routing are defined. In particular, `docker-compose.yml` is an
integration artifact: it expects an existing image, external Traefik network,
mounted goatnet configuration, DNS, and routing. It is not a standalone local
quick start.

## Local startup

Prerequisites are Go 1.26.11 (the version declared by `go.mod`) and `curl` for
the checks below. The following procedure stays on loopback, uses synthetic
configuration, and does not need operational credentials.

From the repository root, build the binary and create a temporary state
directory:

```bash
cd ops/smoketest
go build -buildvcs=false -o ./smoketest ./cmd/smoketest
smoketest_state="$(mktemp -d)"
printf 'temporary state: %s\n' "$smoketest_state"
```

Create the goatnet configuration. Every address is loopback and no relay is
configured:

```bash
cat >"$smoketest_state/goatnet.yaml" <<'YAML'
domain: localhost
relays: []
host_mesh_ip: 127.0.0.1
relay_dns: []
expected_ips:
  - 127.0.0.1
YAML
```

Create a catalogue whose only probe calls the local server itself:

```bash
cat >"$smoketest_state/services.yaml" <<'YAML'
services:
  - name: local-smoketest
    description: Loopback-only smoketest API
    category: local
    checks:
      - type: http
        url: http://127.0.0.1:18080/api/run/status
        expect:
          status: 200
          body_contains: running
YAML
```

Start the server from `ops/smoketest`. The explicit listener prevents access
from non-loopback interfaces. Removing the three optional variables only for
this process keeps an existing shell configuration from replacing the
synthetic inputs or enabling the NetBird client.

```bash
env -u NETBIRD_API_TOKEN -u NETBIRD_DOMAIN -u SMOKETEST_CONFIG \
  ./smoketest serve \
  --addr 127.0.0.1:18080 \
  --config "$smoketest_state/goatnet.yaml" \
  --catalogue "$smoketest_state/services.yaml" \
  --results-file "$smoketest_state/results.jsonl" \
  --dashboard-dir "$PWD/dashboard"
```

Keep that process running. In a second terminal, open
<http://127.0.0.1:18080/> or verify the dashboard and read-only API responses.
The dashboard's **Run Smoke Test** button sends the selected quick/full mode but
no category filter, so the runner selects every registered probe category. For
this synthetic example, use the `categories=service` API request below instead.

```bash
curl --fail http://127.0.0.1:18080/
curl --fail http://127.0.0.1:18080/api/run/status
curl --fail http://127.0.0.1:18080/api/catalogue
curl --fail http://127.0.0.1:18080/api/services
```

To exercise the results file without contacting any non-loopback target, start
only the catalogue probe and wait for `running` to become `false`:

```bash
curl --fail --request POST \
  'http://127.0.0.1:18080/api/run?categories=service'
curl --fail http://127.0.0.1:18080/api/run/status
curl --fail http://127.0.0.1:18080/api/results/latest
```

The completed run is appended to `$smoketest_state/results.jsonl`. Stop the
server with `Ctrl-C`; it does not create or stop any other process.

## Using real configuration

The `--config` file supplies `domain`, `relays`, `host_mesh_ip`, `relay_dns`,
and `expected_ips`. `--catalogue` supplies the services exposed by
`/api/catalogue` and probed by `/api/services` or the `service` category.
`NETBIRD_API_TOKEN` is optional; when absent, `/api/peers` returns 503 while the
dashboard and other APIs continue to work.

The checked-in `services.yaml` describes integration targets and is not a safe
offline fixture. Run it only in an environment whose overlay supplies and
authorizes those targets. The `run` command prints probe results but does not
write the server's JSONL results file:

```text
smoketest run --config <goatnet.yaml> --catalogue <services.yaml>
```
