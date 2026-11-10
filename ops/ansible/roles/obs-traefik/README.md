# obs-traefik

Public-facing TLS terminator on Orin-C for the operator IdP and every obs/utility consumer per [ADR 0842](../../../../docs/adr/0842-dedicated-operator-idp-for-observability.md).

## What this is (and is not)

This is a **standalone Traefik** running in its own compose project at `/opt/obs-traefik/` on Orin-C. It fronts:

- operator-IdP Kratos (`auth.ops.<site>.netbird-prod.goat.example`) → `host.docker.internal:4435`
- operator-IdP Hydra (`oauth2.ops.<site>.netbird-prod.goat.example`) → `host.docker.internal:4446`
- Future obs consumers (Grafana, MinIO console, OpenObserve, Forgejo, Ollama, Prom, Alertmanager UIs)

It is **not** the Orin-A Traefik — that lives in the mgmt-host compose project and uses the [`traefik`](../traefik/) role. The `obs-traefik` role is parallel, on a different host, with its own compose.

## File-provider directory model

Traefik static config (entryPoints, providers, log) lives in `/opt/obs-traefik/traefik.yml`.

Dynamic config lives in `obs_traefik_dynamic_dir` (`defaults/main.yml`), mounted into the container at the same path:

- `00-middlewares.yml` — named-middleware catalog (forwardauth-operator-idp, future redirects)
- `route-<name>.yml` — one file per route, rendered by this role from the `obs_traefik_routes` group_var

Future consumer roles (operator-idp-kratos consumer routes, Grafana, MinIO, etc.) drop their own `route-<consumer>.yml` files into the dynamic dir as they're added — the role's `obs_traefik_routes` collects them at the orchestrator layer.

Traefik file provider auto-reloads on file changes; no SIGHUP / restart needed for route updates.

## Required group_var

`obs_traefik_routes` — list of route dicts. Each:

| Key | Type | Purpose |
|---|---|---|
| `name` | string | Route + cert directory identifier |
| `hosts` | list | Host header rules |
| `backend` | URL | `host.docker.internal:<port>` |
| `cert_src` | path | Operator workstation path to cert PEM |
| `key_src` | path | Operator workstation path to key PEM |
| `middlewares` | list | (optional) Named middlewares from catalog |

Pre-flight asserts the cert + key files exist on the operator workstation before any host-side state changes.

## Operator-fired cert ceremony

Before flipping `operator_idp_enabled` to true and running the orchestrator:

```
cert-sign \
  -key-type ecdsa-p256 \
  -san auth.ops.vega.netbird-prod.goat.example \
  -san oauth2.ops.vega.netbird-prod.goat.example \
  -san 198.18.0.9 \
  -san 192.168.1.253 \
  -san 127.0.0.1 \
  -out-cert <operator state dir>/vega-c/operator-idp-cert.pem \
  -out-key  <operator state dir>/vega-c/operator-idp-key.pem
```

(Concrete invocation per vega's group_vars; substitute SAN list per site.)

## ForwardAuth middleware (for non-OIDC consumers)

The `forwardauth-operator-idp` middleware in the catalog is for consumers that don't speak OIDC natively (Prometheus, Alertmanager, Ollama UI). Routes that include it in `middlewares: [forwardauth-operator-idp]` get authn at the Traefik tier; the request body is forwarded to operator-IdP Hydra's `/userinfo` endpoint with the bearer token; on success the request passes through to the backend with `X-User-Email` + `X-User-Groups` headers populated. Consumer-side authz (which group strings unlock what) remains the consumer's responsibility.

### Phase 1 limitation (today)

The `/userinfo` ForwardAuth pattern works for **API access** (caller already has a bearer token) but does NOT redirect unauthenticated browser users to the OIDC login flow. A browser hitting `prometheus.vega.*` with no token gets a 401, not a redirect.

Today this is acceptable for:

- Prometheus federation pulls from Grafana (Grafana already has a token; passes through Authorization header)
- Alertmanager API calls from operator scripts (operator obtains a token via `curl -X POST .../oauth2/token` first)
- Ollama API calls from authenticated programmatic clients

It is NOT acceptable for direct UI access. Operators currently SSH-tunnel + use loopback to access Prometheus / Alertmanager UIs.

### Phase 2 follow-up (deferred — 21H.4c)

Deploy oauth2-proxy as a sidecar to obs-traefik (or as its own compose project). oauth2-proxy handles the OIDC redirect dance + cookie + ForwardAuth response; the middleware's `address:` is then re-pointed at oauth2-proxy's `/oauth2/auth` endpoint. Browser users get redirected to operator-IdP login; on success oauth2-proxy sets a cookie and Traefik passes through with `X-User-Email` etc. populated.

Phase 2 is queued as a follow-up because it adds another container to the obs-host budget and the API-only pattern covers operator workflows in the near term.

## Tests

`tests/syntax-check.sh` — Ansible parse-only smoke + register-uniqueness lint. Behavioral coverage rides on the operator-fired 21H.3 bringup against real Orin-C.
