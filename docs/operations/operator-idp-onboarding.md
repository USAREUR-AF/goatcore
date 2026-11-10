# Operator IdP — onboarding runbook

Block 21H.2 artifact per [ADR 0842](../adr/0842-dedicated-operator-idp-for-observability.md). Step-by-step guide for adding, updating, and removing operator identities in the per-site operator IdP.

## When to use this runbook

- Adding a new operator (new SRE / analyst / fleet-admin joining the team)
- Adjusting an existing operator's group memberships
- Revoking an operator (departure / lost token / compromise)
- Recovery for an operator who lost their FIDO2 token

## What's NOT in this runbook

- Site-Hydra identities (fleet-admin / mesh users) — those live in the **site Kratos** on Orin-A, not the operator IdP. Different flow, different runbook.
- WebAuthn token issuance (the operator buys + owns their YubiKey).
- Cluster-secret rotation for the operator IdP (separate runbook when needed).

## Pre-flight

Operator IdP must be deployed + running. Verify:

```
ssh <orin-c-host> 'curl -fsS http://127.0.0.1:4436/admin/health/ready && echo " kratos OK"'
ssh <orin-c-host> 'curl -fsS http://127.0.0.1:4447/health/ready && echo " hydra OK"'
```

Both should return 200 + the `OK` echo. If either fails, the operator-IdP isn't up — bring it up via `<site>-obs-bootstrap.yml` per Block 21H.1 first.

## Add a new operator

SSH into the per-site Orin-C and run the enrollment script. Two examples:

**Trunk-owner / superuser (admin on everything):**

```
ssh <orin-c-host>
sudo /usr/local/bin/operator-idp-enroll.sh \
  --email person-13@example.org \
  --name "Alice Admin" \
  --groups obs-admin
```

**SRE (Grafana admin, MinIO read-only, no Forgejo):**

```
sudo /usr/local/bin/operator-idp-enroll.sh \
  --email person-19@example.org \
  --name "Bob SRE" \
  --groups grafana-admin,minio-readonly,prometheus-readonly,alertmanager-admin
```

Script output ends with a one-time recovery link. Share it OOB (Matrix DM / phone call / in-person). **Do not email it** — link is valid for 1 hour and grants the holder full FIDO2-enrollment authority for that operator.

## Group reference

| Group | Effect |
|---|---|
| `obs-admin` | Admin on every consumer (superuser) |
| `obs-readonly` | Read-only on every consumer |
| `grafana-admin` / `grafana-readonly` | Grafana scope |
| `minio-admin` / `minio-readonly` | MinIO scope |
| `openobserve-admin` / `openobserve-readonly` | OpenObserve scope |
| `forgejo-admin` / `forgejo-user` | Forgejo scope (admin = repo admin; user = read+push to assigned repos) |
| `prometheus-readonly` | Prometheus UI scope (no admin tier — Prometheus has no native auth) |
| `alertmanager-admin` | Alertmanager UI scope |
| `ollama-user` | Ollama UI scope |

Multiple groups compose by union — an operator with `["obs-readonly", "grafana-admin"]` has Grafana admin + read on everything else.

To extend the group list, edit:

- `ops/ansible/roles/operator-idp-kratos/templates/identity-schema.json.j2` (the `enum` array under `traits.groups.items`)
- `ops/scripts/operator-idp-enroll.sh` (the `ALLOWED_GROUPS` bash array — keep in sync)
- The consumer role that recognizes the new group (Grafana / MinIO / etc.)

Adding without updating all three = silent rejection at one of the layers.

## New-operator workflow (what the recipient does)

1. Receive the recovery link from the trunk-owner.
2. Open it in a browser on a wg-cp0-onboarded laptop. Operator IdP serves only on the wg-cp0 mesh — links don't work over the public internet.
3. Kratos lands the operator in the settings flow. Click "Set up Passkey."
4. Plug in FIDO2 token (e.g., YubiKey). Follow the WebAuthn prompts.
5. Done. The operator can now log into any operator-IdP-fronted consumer (Grafana, MinIO, OpenObserve, Forgejo) using their token.

## Update an operator's groups

The simplest path is via Kratos admin API directly (no script wrapper yet — TBD if needed):

```
# Find the identity ID
ssh <orin-c-host>
curl -fsS http://127.0.0.1:4436/admin/identities | jq '.[] | select(.traits.email=="person-13@example.org") | .id'

# PATCH the groups (RFC 6902 JSON Patch)
curl -fsS -X PATCH "http://127.0.0.1:4436/admin/identities/<id>" \
  -H 'Content-Type: application/json-patch+json' \
  -d '[{"op": "replace", "path": "/traits/groups", "value": ["grafana-admin", "minio-admin"]}]'
```

Operator's existing JWT continues to carry the OLD groups until expiry (default 8h per [ADR 0842 §"Implications"](../adr/0842-dedicated-operator-idp-for-observability.md)). For immediate effect, also revoke active sessions:

```
curl -fsS -X DELETE "http://127.0.0.1:4436/admin/identities/<id>/sessions"
```

## Revoke an operator

Two options depending on intent:

**Soft revoke** — operator may return; preserve identity record:

```
# Revoke all sessions (forces re-auth on next access)
curl -fsS -X DELETE "http://127.0.0.1:4436/admin/identities/<id>/sessions"
# Strip group memberships (now they auth but get no access)
curl -fsS -X PATCH "http://127.0.0.1:4436/admin/identities/<id>" \
  -H 'Content-Type: application/json-patch+json' \
  -d '[{"op": "replace", "path": "/traits/groups", "value": []}]'
```

(Note: empty groups violates the schema's `minItems: 1` constraint — for a true zero-access identity, set `groups: ["obs-readonly"]` and treat that as the soft-revoke shape, OR the identity itself should be hard-deleted below.)

**Hard revoke** — operator departed permanently:

```
curl -fsS -X DELETE "http://127.0.0.1:4436/admin/identities/<id>"
```

Identity record removed; FIDO2 credential entries cascade-deleted; future logins return 404.

## Recovery — operator lost their FIDO2 token

```
# Trigger a fresh recovery link for the existing identity
curl -fsS -X POST "http://127.0.0.1:4436/admin/recovery/link/code/admin" \
  -H 'Content-Type: application/json' \
  -d "{\"identity_id\":\"<id>\"}" | jq -r '.recovery_link'
```

Share OOB. Operator follows the link, removes the lost token's credential record from their account settings, and registers their backup token.

## Total-token-loss event

If every operator on the site has lost their token simultaneously (theft, fire, mass departure), the recovery path is local-admin break-glass on each obs/utility consumer (Grafana `admin` / MinIO root user / etc.) per [ADR 0842 §"Local break-glass preserved"](../adr/0842-dedicated-operator-idp-for-observability.md). Then re-bootstrap the operator IdP (rebuild from `pg_dump` if available, OR fresh init via `<site>-obs-bootstrap.yml`) and re-enroll operators from scratch.

This is acceptable because operator-IdP data is small + reproducible — see [ADR 0842 §"Backup grade"](../adr/0842-dedicated-operator-idp-for-observability.md).

## Drift checks

```
# Every identity on the site
curl -fsS http://127.0.0.1:4436/admin/identities | jq '.[] | {email: .traits.email, name: .traits.name, groups: .traits.groups}'

# Identities without WebAuthn credentials (should be empty after enrollment completes)
curl -fsS 'http://127.0.0.1:4436/admin/identities?include_credential=webauthn' \
  | jq '.[] | select(.credentials.webauthn == null) | .traits.email'
```

## Related

- [ADR 0842](../adr/0842-dedicated-operator-idp-for-observability.md) — operator IdP decision
- Block 21H — implementation tracking
- `ops/scripts/operator-idp-enroll.sh` — enrollment script
- `ops/ansible/roles/operator-idp-kratos/` — Kratos role (schema + config)
