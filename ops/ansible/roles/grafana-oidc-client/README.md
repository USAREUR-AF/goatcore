# grafana-oidc-client

Provisions an OAuth2 client for Grafana at the operator-IdP Hydra per [ADR 0842](../../../../docs/adr/0842-dedicated-operator-idp-for-observability.md).

## What this is (and is not)

This role only manages the **Hydra-side** OAuth2 client config (idempotent GET-then-POST-or-PUT). It does NOT manage the Grafana container — that's the [`obs-host`](../obs-host/) role's job. The matching Grafana env-var changes (GF_AUTH_GENERIC_OAUTH_*) live in the obs host group's `group_vars/<site>_obs_hosts/vars.yml` and are consumed by obs-host.

The split is deliberate: this role is the only thing that talks to Hydra; obs-host is the only thing that runs Grafana. Re-running either is safe.

## Required env var

```
export <SITE>_C_OPERATOR_IDP_GRAFANA_CLIENT_SECRET=$(openssl rand -hex 32)
```

For a site named `zagros`: `ZAGROS_C_OPERATOR_IDP_GRAFANA_CLIENT_SECRET`.

The same secret value MUST be set in:

- the env-var above (consumed by this role)
- Grafana's `GF_AUTH_GENERIC_OAUTH_CLIENT_SECRET` env (consumed by obs-host)

Drift between the two = OIDC handshake fails with HTTP 401 from Hydra. The group_vars layer wires both from the same env var so they stay in sync.

## F-077 + F-078 mitigations

Built in:

- `skip_consent: true` per-client (F-077 — Hydra's selfservice-ui-node consent UI silently grants zero scopes when checkboxes unticked; first-party clients skip it).
- `access_token_strategy: jwt` per-client (F-078 — even though operator-IdP Hydra defaults to JWT at the issuer level, explicit-per-client is belt-and-braces).

## Group → Grafana role mapping

The `grafana_oidc_role_jmespath` default maps the JWT `groups` claim to Grafana built-in roles:

| Operator groups | Grafana role |
|---|---|
| `obs-admin` OR `grafana-admin` | Admin |
| `obs-readonly` OR `grafana-readonly` | Viewer |
| (no matching group) | login fails |

There is intentionally no mapping to Grafana's "Editor" role — operator IdP groups are coarser than Grafana's three tiers.

## Sequencing

This role runs AFTER operator-idp-hydra is reachable on Orin-C (Phase 7c). Orchestrator wires it into Phase 7e (after obs-traefik 7d) so the route → backend chain is complete before any operator can hit the OIDC flow.

## Tests

`tests/syntax-check.sh` — Ansible parse-only smoke + register-uniqueness lint. Behavioral coverage rides on operator-fired 21H.3 bringup.
