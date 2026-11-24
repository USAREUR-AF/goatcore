# goat-pod-api: carriage on a pod

| Field | Value |
|---|---|
| Status | Built (`ops/goat-pod-api`); runs on a pod host as `goat-pod-api.service`, installed by the pod's first-boot |
| Canon | [ADR 1251](../adr/1251-every-source-keeps-its-identity-through-a-pod-carriage-rules-provenance-at-entry-and-pod-local-authority.md), [`pod-carriage-and-provenance.md`](../design/pod-carriage-and-provenance.md) §4 and §7.4 |

goat-pod-api lets devices that hold their own goat certificate publish and read through a pod under their own slot. It pulls
the pod's signed rule generation from the portal, adds the pod's local rules, renders the pod router's ACL, restarts the
router, and waits for it to be healthy. When it is not, an addition rolls back to the last known good configuration; a
removal runs the base configuration, which carries no source, or stops the router, and is tried again. While the pod is cut
off, the people named as its local authority change its rules through the local API; each change is signed into a ledger
the portal receives on reconnection.

## Install

Releases: a `goat-pod-api/v<semver>` tag (repository admins only) runs `.github/workflows/goat-pod-api-release.yml`, which
publishes `goat-pod-api-linux-amd64`, `goat-pod-api-linux-arm64` and `gatepost.html` with `SHA256SUMS` and a keyless cosign
signature over it (the shape of every goatcore binary release, `.github/release-notes/goat-pod-api.md`). No release has been
cut yet.

When the profile sets `POD_API_SIGNER_CERT`, the pod repository's `host/first-boot.sh` installs the release its
`host/goat-pod-api.pin` names (`host/install-goat-pod-api.sh`): it checks the cosign signature against this workflow's
identity, then the pinned digest of `SHA256SUMS`, then each asset's digest, and refuses on any mismatch. To run a build of
your own instead, put it on the host's `PATH` and set `POD_API_INSTALL=0` in the profile:

```sh
cd ops/goat-pod-api && CGO_ENABLED=0 go build -o goat-pod-api .
```

first-boot writes `<state>/pod-api/config.json` and the systemd unit. The configuration keys:

| Key | Meaning |
|---|---|
| `pod_slot` | the pod's slot |
| `state_dir` | holds `generation.json` (last good generation), `central-archive.json`, `ledger.jsonl` and `ledger.jsonl.ack` (how far the portal holds the ledger, and its answer per entry) |
| `portal_url`, `portal_ca` | portal base URL (empty: no pulls, no uploads) and the CA for its TLS certificate (empty: system roots) |
| `portal_token_file` | file holding the pod's portal API key (the goat profile's `portal.token`), sent as a bearer token; required with `portal_url` |
| `portal_signer_cert` | certificate whose key signs generations; its CN must equal the generation's `signer` |
| `pod_cert`, `pod_key`, `ca_roots` | the pod's leaf (server certificate for the local API, ledger signing key, sent as `pod_cert` with each upload) and the roots that verify sign-in certificates. The key must be ECDSA P-256 |
| `listen` | local API addresses, for example `["127.0.0.1:8447", "<mesh address>:8447"]` |
| `router.*` | `base_config` (first-boot's template rendering with the carriage markers), `config` (live), `last_good`, `applied_sha_file`, `container` |
| `pull_interval_seconds`, `cutoff_after_seconds` | 60 and 300 by default; the pod reports itself cut off after the second without portal contact |
| `max_rule_days`, `reactivation_days` | 90 and 183 by default; the longest local end date and the reactivation window |
| `health_publish` | `{"enabled": true}` publishes carriage health on `ops/pod/health/<pod-slot>` with `goat pub` |
| `gatepost_page` | optional file that replaces the built-in Gatepost page served at `/`. The binary carries `ops/fieldkit/gatepost/gatepost.html` (CI checks the copy matches) |

## Commands

| Command | Does |
|---|---|
| `goat-pod-api serve --config <file>` | runs the service: pull, upload, re-render on expiry, local API |
| `goat-pod-api render --config <file> --out <file>` | renders the router configuration from saved state, restarts nothing (first-boot uses it) |
| `goat-pod-api adopt --config <file> <generation.json>` | verifies and applies a generation file, for a pod with no portal link |
| `goat-pod-api status --config <file>` | prints the status from saved state |

## Local API

HTTPS with client-certificate sign-in. Any certificate the pod's CA roots verify may read; only a certificate whose CN the
held generation names in `local_authority` may change rules. Routes and bodies: design §7.4.

| Question | Route |
|---|---|
| Is the pod connected, how old are its rules, what does it carry? | `GET /v1/status` |
| Which rules, central and local, with end dates? | `GET /v1/rules` |
| Let a device use this pod | `POST /v1/rules {"source": "<slot>.g<N>", "until": "...", "reason": "..."}` |
| Extend or reactivate a rule | `POST /v1/rules/<id>/extend` or `/reactivate` with `{"until", "reason"}` |
| Cut a device off this pod now | `POST /v1/sources/<slot>/revoke {"reason"}`; it holds until a later allow or reactivate, or until an org lead decides it. A decision, either way, frees central rules only: earlier local rules for the device stay cut off |
| Local changes not yet ratified, and those that lapsed | `GET /v1/pending` |

For example, from the pod host with the local authority's certificate:

```sh
curl --cacert ca-roots.pem --cert lead.pem --key lead.key https://127.0.0.1:8447/v1/status
```

## Turning it off, and on again

On the pod: zagros-moon-pod `docs/router-config.md`, "Turn carriage off, and on again" (run on a pod on 2026-11-24, off and on,
with a carried device). It puts the router on its base configuration, keeps the ledger and the held generation, and leaves
the portal and the core alone; the core side is [`pod-carriage.md`](pod-carriage.md).

## When something is wrong

| Symptom | Meaning and action |
|---|---|
| `status.last_pull_error` names a signature | the generation does not verify against `portal_signer_cert`; confirm the signer certificate through the onboarding channel. The pod keeps the generation it holds |
| `... is lower than the N this pod holds` | the portal served an older generation; the pod refuses it. A goat-side operator checks the portal |
| `generation N was rejected by the router earlier` | the generation only added sources and the router did not come up with it; the previous configuration is running. The pod waits for a newer generation. Router log: `docker logs goat-moon-zenoh-router` |
| an answer `{"error": {"code": ...}}` | the code, what, why and fix name the refusal; codes are listed in design §7.4 |
| `POST` answers `202` with `warning` | the change is in the ledger but the router refused the rendering. After an addition the previous configuration is running. After a removal no source is carried (next row) |
| `status.suspended` is true | a removal could not be applied, so the pod carries no source: the router runs the base configuration, or is stopped when that failed too. `last_apply_error` says which; `docker logs goat-moon-zenoh-router` says why. The service tries again at each tick |
| the service will not start: `ledger line N ...` or `the ledger was truncated` | the ledger was edited or shortened; the pod refuses to run on it. Keep the file for the goat-side operator |
| `last_upload_error` names `409` | the portal holds a different entry at that sequence number; the pod never rewrites one. A goat-side operator compares the two |
