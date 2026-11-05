# rest-pub-bridge — mTLS publish sidecar for non-Rust publishers

**Status:** built; nova-a live-fire pending. [ADR 1023](../adr/1023-zenoh-rest-pub-bridge-for-non-rust-publishers.md).
**Code:** [`ops/zenoh-shims/rest-pub-bridge/`](https://github.com/usareur-af/goatstuff/tree/main/ops/zenoh-shims/rest-pub-bridge).

> **Update (2026-07):** an official cgo `zenoh-go` shipped in **Zenoh 1.9.0** (2026-04), so the "no maintained Go client for Zenoh 1.x" premise below is now historical. The bridge is unchanged and remains the right tool wherever a **CGO-free static image** or **zero-code-change** matters (demogame keeps it). Native `zenoh-go` is now an option for *new* Go publishers that can take the cgo/zenoh-c build+runtime dependency — see grant-keyed-encryption.md §8 M2.1 for how that choice plays out for the data-access E1 key-service.

## Problem

The nova Zenoh fabric authenticates publishers by X.509 client certificate presented during the native TLS handshake. The router's `access_control` matches the leaf Common Name (`CN=<vendor>-trial-peer`) to a write rule scoped to `<vendor>/**`. After the [ADR 1020](../adr/1020-zenoh-federated-peering-trust-model.md) ingress/egress split, **publish is granted only to identified per-vendor subjects** — a publisher with no presented cert has no identity and is dropped at ingress (silently — F-177).

A publisher that cannot open a native Zenoh session therefore cannot publish. The motivating case is **demogame**: it is Go, and at the time (early 2026) there was no maintained Go client for Zenoh 1.x (see the 2026-07 update above), so it published over the router's REST plugin (`HTTP PUT :8000`) — a surface with no client-cert requirement. Once the ACL split landed, those identity-less PUTs stopped flowing and the game's data disappeared from panoscope. See `demogame-multiplayer.md` §"Wire-publish path".

## Shape

`rest-pub-bridge` is a small Rust sidecar that **mimics the Zenoh REST plugin's PUT contract** on the front and **speaks native mTLS Zenoh** on the back:

```
   publisher (Go)                    rest-pub-bridge                 zenoh router
   ┌──────────────┐  HTTP PUT  /key  ┌───────────────┐  native mTLS  ┌──────────┐
   │ ZENOH_REST_  │ ───────────────► │  holds vendor │ ────────────► │ access_  │
   │ ENDPOINT  ──────────────────────► leaf; re-puts │  CN=<vendor>- │ control  │
   │ → sidecar    │   body = value   │  on a session │  trial-peer   │ admits   │
   └──────────────┘                  └───────────────┘               └──────────┘
        plaintext HTTP over docker net          mTLS, ACL-matchable identity
```

The publisher needs **zero code change** — it just repoints `ZENOH_REST_ENDPOINT` from the router to the sidecar. The bridge holds the per-vendor leaf, opens a native client session (the synth-vendor / audit-subscriber / panoscope mTLS idiom verbatim — `mode=client`, `tls/` endpoint, `transport/link/tls` with `enable_mtls=true`, `verify_name_on_connect=false`), and re-emits each PUT as a `session.put(key, body)`.

### Wire contract

| Request | Effect |
|---|---|
| `PUT /<keyexpr>` (body = value) | publish `value` to `<keyexpr>` |
| `POST /<keyexpr>` | same (accepted for parity) |
| `GET /health` | `200 "ok"` (smoketest liveness) |

The URL path minus the leading `/` is the key-expression; the request `Content-Type`, when present, becomes the Zenoh encoding.

### Configuration

| Flag / env | Default | Notes |
|---|---|---|
| `--listen` / `REST_PUB_BRIDGE_LISTEN` | `0.0.0.0:8000` | HTTP listener |
| `--endpoint` / `REST_PUB_BRIDGE_ENDPOINT` | `tls/goat-zenoh-router:7447` | `tls/` selects native mTLS |
| `--cert` / `REST_PUB_BRIDGE_CERT` | — | client leaf PEM (required for `tls/`) |
| `--key` / `REST_PUB_BRIDGE_KEY` | — | client key PEM |
| `--root-ca` / `REST_PUB_BRIDGE_ROOT_CA` | — | trust root PEM |
| `--allow-prefix` / `REST_PUB_BRIDGE_ALLOW_PREFIX` | `` (any) | comma-separated key-prefix allowlist |

### Identity + ACL — unchanged machinery

The bridge introduces **no new identity path**. The leaf is minted by the same portal/bootstrap flow (`bootstrap-demogame-portal.sh`, `CN=<vendor>-trial-peer`, URI-SAN `urn:goat-nova:vendor=<vendor>`), and the ACL grant comes from adding the vendor to `zenoh_seed_vendors` — which renders the subject (`cert_common_names: ["<vendor>-trial-peer"]`), the write rule (`put`/`delete` ingress on `<vendor>/**`), and the storage. For demogame: `CN=demogame-trial-peer` → `demogame/**`.

## Security

- **Two hops, two trust levels.** publisher→sidecar is plaintext HTTP over the docker network (the same trust boundary the prior direct-REST path used). sidecar→router is mTLS — a net improvement over the old identity-less path.
- **`--allow-prefix` (defense in depth).** The bridge holds a real vendor cert, so a compromised or buggy upstream must not be able to ask it to publish outside the vendor namespace. The router ACL already scopes the cert to `<vendor>/**`; the allow-prefix is the belt to that suspenders. Set it to the vendor prefix (`demogame/`).
- **No embedded secrets.** The image carries no identity material; the Ansible role bind-mounts the cert tree read-only at deploy time (uid 65532).

## Deployment

Deployed as a sidecar in the publisher's compose project, on the shared router network (`goat-zenoh_default`). For demogame this is wired in `demogame-docker-compose.yml.j2`; the role (`roles/demogame/`) pre-flights the cert material, pins the sidecar image in the compose `.env` (rollback via `rollback.sh`, [ADR 0859](../adr/0859-easy-rollback-engineering-principle.md)), and the game's `demogame_zenoh_rest_endpoint` points at `http://demogame-zenoh-sidecar:8000`.

**Turnkey order for a new publisher:**
1. Mint the vendor leaf (portal / bootstrap script); place cert + key on the host.
2. Add the vendor to `zenoh_seed_vendors`; re-render the router (`zenoh-deploy.yml`).
3. Add the sidecar service to the publisher's compose + repoint its publish endpoint; deploy.
4. Confirm topics flow in panoscope.

## Reuse

The bridge is generic — any non-Rust (e.g. Go) publisher in the same bind reuses the binary by mounting its own vendor leaf and setting `--allow-prefix` to its namespace. This closes the language-boundary gap ([ADR 0112](../adr/0112-tech-strategy-workload-shape-language-boundary.md)) with one shared piece rather than per-service bespoke code. The production goat-net target remains a native maintained-language publisher; the bridge serves the trial fabric and any transition.

## Limits

- The sidecar is on the publish hot path; a sidecar outage stops that publisher's Zenoh output (the app itself is unaffected — demogame' publish loop is non-blocking and drops on error).
- One container per non-Rust publisher (small, distroless).
- Not a subscribe path — publish only. Consumers that need to *read* from Go are out of scope here.
