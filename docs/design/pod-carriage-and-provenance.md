# Pod carriage and provenance

| Field | Value |
|---|---|
| Status | Designed, not built ([ADR 1251](../adr/1251-every-source-keeps-its-identity-through-a-pod-carriage-rules-provenance-at-entry-and-pod-local-authority.md), Accepted); build tracked in goatcore#1025 |
| Scope | How a source keeps its identity on any path; carriage rules; provenance at entry; a cut-off pod's local authority |
| Related | [`pod-targeted-client-bundles.md`](pod-targeted-client-bundles.md) (bundles, superseded on route binding: routers are hints), [`fieldkit-and-bleating-edge.md`](fieldkit-and-bleating-edge.md) (operator tooling), [`federated-trust-model.md`](federated-trust-model.md) |

## 1. The model

A source (person, device, or keyless feed) has four layers.

| Layer | What it is | Where it lives | Changes when |
|---|---|---|---|
| Identity | Slot, certificate, portal key | Issued at enrollment | Rotation or retirement |
| Catalog | Topics, formats, schemas | Portal, keyed by slot | Its owner edits topics |
| Carriage | Rules for which pods may forward its data | Registry; rendered into router ACLs | Its owner changes policy |
| Route | Which router or peer it uses now | The source and routers, live | Whenever it moves |

Only one question depends on the path: when a router forwards data it did not authenticate itself, may it carry that slot?
On a direct link the receiver sees the source's own certificate, so no rule is involved.

## 2. Carriage rules

| Kind | Example | End date |
|---|---|---|
| Org (default) | Pods run by org X may carry devices of org X | None; created at org onboarding |
| Named | Pod Z may carry device D | Required |
| Temporary | Exercise pods may carry org X devices | Required |

- Consent once per rule: the source's custodian grants, the pod's custodian accepts; for the org rule both are the org.
- A device can opt out of the org rule.
- Expired rules are archived and kept. The owner can reactivate one in a single step within the reactivation window (six
  months by default, a goatnet setting); after that it is approved like a new rule. A named or temporary rule runs at most 90
  days by default, also a goatnet setting.
- Rendering: a generator produces each pod's ACL (each covered source's own certificate under its own slot; only the pod's
  certificate on the pod's slot) and the core router's ACL (a pod's connection may write the slots its rules cover). Pods pull
  a signed generation. A drift check compares rendered ACLs with the rules.
- A pod's read access is rendered from the read grants of the sources it may carry, never `**`
  ([ADR 1123](../adr/1123-client-demand-must-not-amplify.md)).

### Reading through a pod

Keys mean the same thing everywhere in one environment, so a device behind a pod subscribes with exactly the key it would use
at the core. No key translator exists or is needed.

| Step | What happens |
|---|---|
| Device subscribes at the pod | The pod's ACL admits the subscription if the device is a source the pod carries and the core would admit that source's read on the key. A bare `**` subscription stays the named grant `zenoh_fabric_wide_read` |
| Pod forwards interest | The pod declares upstream only the subscriptions its connected sources declared. It never subscribes to the whole keyspace on their behalf ([ADR 1123](../adr/1123-client-demand-must-not-amplify.md)) |
| Core delivers | The core sees the pod's certificate on the read, as on any read. Confidential data stays encrypted to its grant holders ([ADR 1208](../adr/1208-sovereign-environments-release-namespaces-and-grant-encryption.md)), so a sample forwarded to a device that holds no grant cannot be opened |
| Data from another environment | It arrives at the core as `from/<sender>/<set>/…`, written by the receiving side's republisher, and is read through a pod like any other key |

Today's pod template (`zagros-moon-pod/host/zenoh-router.json5.tmpl`) admits reads only on the pod's own slot and one inbound
prefix, `INBOUND_NAMESPACE`. Phase 2 replaces that prefix with the rendered read rules; until then a device behind a pod reads
only those two prefixes.

### Moving devices that are already behind pods

| Device today | Move |
|---|---|
| Holds its own slot and certificate, connects through the pod's slot | Route-only update: the bundle gains the pod as a router hint. No new certificate, no quota cost |
| Holds no identity (a legacy stream into the pod) | Enrolled as a machine, or as a keyless source behind an ingress adapter (§3) |
| Publishes under the interim key `<pod-slot>/routed/<source-slot>/…` | Keeps flowing. Once the source is carried under its own slot, subscribers move to the source's key and the pod's operator retires the interim prefix |

## 3. Provenance at entry

- The signature rides in the sample's attachment and is added once, where data enters goat. Pods forward it unchanged.
- Grades: **device-signed** (the source's own key), **adapter-attested** (an ingress adapter signs on a declared keyless
  source's behalf, naming the source slot, interface, port and sender address), **pod-vouched only** (no signature;
  transitional).
- A keyless source is enrolled like a machine, with its own slot and a custodian, but holds no key.
- Nothing is blocked by grade today. Subscribers can filter by grade; a topic can later require a minimum grade.
- The first adapter is for ASTERIX over UDP, building on the legacy-feed carriage demonstration (per-frame digest); CoT next.
  Signing format and key handling are settled in the build, reusing goat's grant-crypto key handling.

## 4. A cut-off pod and its local authority

- A cut-off pod enforces its last signed rule generation and reports its age in health.
- The pod's signed rules name its local authority in advance. That person signs in with the goat certificate or card they
  already hold; the pod checks it against cached CA roots.
- Local authority may add, extend, revoke or reactivate rules **for that pod only**, each with an end date, each signed locally
  and kept in an append-only ledger. It cannot change another pod or the core, cannot grant without an end date, and cannot
  mint identities (new devices met in the field use offline enrollment).
- Each local change re-renders the pod's ACL and restarts its router, with the last-known-good rollback
  `zagros-moon-pod/host/first-boot.sh` already uses.
- On reconnection the pod reports its ledger. Data sent under a local rule arrives marked "local rule, not yet ratified". An org
  lead ratifies each rule or lets it lapse. Goat's revocations win.

## 5. Tooling

The pod's local API and these `goat pod` verbs are primary. Fieldkit Gatepost offers the same six actions to operators who are
not developers ([`fieldkit-and-bleating-edge.md`](fieldkit-and-bleating-edge.md)).

| Command | Does |
|---|---|
| `goat pod status` | Connected or cut off, rule age, sources connected, carriage health |
| `goat pod rules ls` | Effective rules, central and local, with end dates and origin |
| `goat pod rules allow <source> --until <time> --reason "…"` | Local rule letting a source use this pod |
| `goat pod rules extend <rule> --until <time>` | Extend a rule |
| `goat pod rules revoke <source>` | Cut a source off this pod at once |
| `goat pod rules reactivate <rule>` | Bring back an archived rule |
| `goat pod pending` | Local changes waiting for ratification |

Status: the `goat pod` verbs are built (`crates/goat-cli/src/cmd/pod.rs`) and tested against a mock of the §7.4 API. `allow`,
`extend` and `reactivate` send an end date and a reason (`--until` takes an RFC 3339 time or a length such as `4h`, `24h`,
`72h`); `revoke` takes effect at once and sends only a reason. The pod is named by `--pod <url>` (`GOAT_POD_URL`, or
`pod_endpoint` in the context's `profile.toml`) and `--pod-slot <slot>` (`GOAT_POD_SLOT`, or `pod_slot`). The CLI signs in
with the context's certificate and key. The pod serves its own goat leaf, which names its slot rather than the address
dialled, so the CLI checks that the leaf chains to the context's CA roots and that its CN is the pod's slot (`<slot>` or
`<slot>.g<N>`) instead of checking a hostname (`crates/panoscope-core/src/pinned_tls.rs`). A count the pod reports as
`null` (sources connected, not measured) is shown as unknown, never as 0. Every list and status verb takes `--json`. Refusals, from the portal or the pod, map by the server's `error.code` to seven catalog
codes (`ops/errors/catalog.yaml`): `E_CARRIAGE_EXPIRED` (`carriage_rule_expired`, `carriage_rule_archived`,
`local_rule_expired`), `E_CARRIAGE_WINDOW_PASSED` (`carriage_beyond_reactivation_window`), `E_CARRIAGE_TOO_LONG`
(`carriage_until_required`, `carriage_over_max_length`), `E_CARRIAGE_END_INVALID` (`carriage_until_past`,
`carriage_org_rule_no_end`, `carriage_exclusion_no_end`), `E_CARRIAGE_NOT_AUTHORIZED` (`carriage_not_authorized` or any 403),
`E_CARRIAGE_STATE_CONFLICT` (`carriage_not_archived`, `carriage_already_reactivated`, `carriage_not_proposed`,
`local_rule_not_pending`, `carriage_pod_org_mismatch`, `carriage_source_revoked`, `ledger_conflict`) and
`E_CARRIAGE_UNKNOWN` (`not_found`, `pod_not_registered`, `source_not_live`, `pod_slot_not_live`). A portal `config`
(503) is `E_PORTAL_SERVER_ERROR`; the bulk enrollment refusals `invalid_keyless` and `invalid_pods` are shown per entry.

## 6. Edges and local choices

| Edge | Choice |
|---|---|
| Many live paths at once | Any allowed path is valid. Measured: a client-mode session given two pods uses one at a time and every publication arrived once; a peer-mode session linked to two routers delivered every publication twice. Any remedy is session-side (options below) |
| Pod behind a pod | Not supported; a pod forwards only sources it authenticated |
| Across sovereigns | A rule never crosses a trust root ([ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md), [ADR 1020](../adr/1020-zenoh-federated-peering-trust-model.md)) |
| Silent denial | A pod refuses an uncovered source at connect time and publishes per-source carriage health |
| Scale | ACL size grows with covered slots per pod. zenohd 1.9 applies an ACL change only by restarting the router (F-156), so rendering follows the slot fragment's pattern (`ops/ansible/roles/goat-zenoh/files/slot-fragment-reload.py`): removals at once, additions batched |

**Measured** 2026-11-19, zenohd 1.9.0 (`eclipse/zenoh:1.9.0` by digest), with
`N=1000 RUNS=3 bash tests/duplicate-delivery-local.sh` in USAREUR-AF/zagros-moon-pod: a core router and two pod routers
rendered from the pod template, each pod on its own Docker network and admitting one device certificate as the writer of
its slot; a subscriber on the core counts arrivals of 1,000 sequence-numbered publications, three runs per case, on a
GitHub-hosted runner (Intel Xeon Platinum 8370C, 4 vCPU). The same script with the routers run as zenoh 1.9.0 Python
router sessions on an Apple M3 gave the same counts.

| Publisher session | Connected to | Arrivals per publication (3 × 1,000) |
|---|---|---|
| client, one endpoint (control) | pod-a | 1 for all 3,000 |
| client, endpoints pod-a and pod-b | pod-a only (a client holds one router link) | 1 for all 3,000 |
| peer, endpoints pod-a and core | pod-a and core | 2 for all 3,000 |
| peer, endpoints pod-a and pod-b | pod-a and pod-b | 2 for all 3,000 |

Standard pattern: a device publishes in client mode and lists the pods it may use as router hints, so it holds one router link at a time. Peer mode to two routers delivers every publication twice and is not used for publishing through pods.

A further session-side remedy, not built: the
subscriber drops a sample whose `prov` (signer, `ts`, `sha256`) it has already accepted, which covers signed samples only.
Whether zenoh's advanced publisher and subscriber sequence numbers drop such duplicates is unmeasured.

**Measured** 2026-11-19, signature overhead per sample of the §7.2 attachment, with
`cargo run --release -- 5000` in `ops/zenoh/prov-bench` (p256 0.14 and sha2 0.10, the crates goat-cli uses) on an Apple
M3 laptop (8 cores) under a load average of 60 to 140 from other work. Sign = SHA-256 of the payload, canonical JSON,
ECDSA P-256 sign, base64, attachment JSON; verify = parse, canonical JSON, ECDSA verify, SHA-256 compare. Certificate fetch
and chain check are cached and not included. Medians of four runs of 5,000 samples each:

| Payload | Sign, median (µs) | Verify, median (µs) | SHA-256 alone (µs) |
|---|---|---|---|
| 64 B | 149 to 154 | 508 to 697 | 0.7 to 0.8 |
| 1 KiB | 154 to 159 | 513 to 714 | 5.3 to 6.2 |
| 64 KiB | 547 to 602 | 1,096 to 2,056 | 377 |

The attachment adds 413 bytes for `device-signed` and 476 bytes for `adapter-attested` (the DER signature base64-encodes to
96 characters at every length P-256 produces). For reference, `openssl speed ecdsap256` on the same machine at the same time:
25,247 signs and 8,490 verifies per second.

## 7. Interfaces

These are the contracts the build modules share. Each is versioned; a change that breaks one takes a new version.

### 7.1 Rule generation (`goat.carriage.generation/v1`)

The portal renders one generation per pod. The pod pulls it, verifies it, renders its ACL and restarts its router.

```json
{
  "schema": "goat.carriage.generation/v1",
  "pod_slot": "<32 hex>",
  "generation": 42,
  "issued_at": "2026-11-19T10:00:00Z",
  "local_authority": [{"identity": "<pki>:<identifier>", "cert_cn": "<slot>.g<N>"}],
  "read": ["*/**"],
  "sources": [
    {
      "slot": "<32 hex>",
      "rule_id": "cr_<id>",
      "kind": "org | named | temporary",
      "until": "2027-02-17T00:00:00Z",
      "principals": ["<slot>.g<N>"],
      "write": ["<slot>/**", "volatile/<slot>/**", "<slot>/**/@adv/**"]
    }
  ],
  "signer": "<portal certificate CN>",
  "signature": "<base64 ECDSA P-256 DER>"
}
```

- `principals` lists the certificate CNs the pod admits as writers of that slot: the device's own leaf, or for a keyless
  source the ingress adapters declared for it. `until` is null only for an org rule.
- `read` is the goatnet's read set for carried sources, a portal setting. It never contains a bare `**`.
- The signature covers the document without `signature`, serialized as JSON with sorted keys and no insignificant
  whitespace, prefixed by the bytes `goat-carriage-v1\n`. The pod verifies it against the portal signer certificate in its
  bundle and refuses a generation lower than the one it holds.
- Pull: `GET /api/carriage/pods/{pod_slot}/generation`, authenticated by the pod's own certificate.

### 7.2 Provenance attachment (`prov` v1)

The per-sample zenoh attachment is a JSON object. Provenance sits under the key `prov`; other components may add their own
keys and must keep `prov` unchanged. Pods and routers forward the attachment as it is.

```json
{"prov": {
  "v": 1,
  "grade": "device-signed | adapter-attested",
  "src": "<source slot>",
  "signer": "<signer certificate CN>",
  "ts": "2026-11-19T10:00:00.123456789Z",
  "key": "<key expression the sample was published on>",
  "sha256": "<hex digest of the payload>",
  "via": {"iface": "udp", "port": 10009, "from": "192.0.2.10:5000"},
  "sig": "<base64 ECDSA P-256 DER>"
}}
```

- `via` is present only for `adapter-attested`. A sample without `prov` is graded **pod-vouched only** when it arrived
  through a pod and **unsigned** otherwise.
- The signature covers `prov` without `sig`, serialized like the generation, prefixed by `goat-prov-v1\n`. The signer uses
  the private key of its own goat leaf certificate.
- Each signer publishes its leaf certificate (PEM) once on `<signer anchor>/_goat/prov/cert`, a stored key in its own
  namespace. A verifier fetches it, checks that it chains to the goatnet CA and that its CN equals `signer`, caches it, and
  then checks the signature and the digest.

### 7.3 Keyless sources and ingress adapters

- A keyless source is enrolled like a machine with `keyless: true`: it gets a slot and a custodian, and no key or
  certificate is minted. Its record names the adapters that may publish for it (`adapters: [<adapter slot>]`) and how it is
  received (`interface`, `port`, `sender`).
- An ingress adapter is a machine with its own slot and certificate. It accepts only the declared sender on the declared
  port, publishes each frame under the keyless source's slot, and signs `adapter-attested` provenance.
- The core admits the adapter's writes to the keyless source's slot through a carriage rule (phase 4) or, at a pod, through
  `principals` in the generation.

### 7.4 Pod local API and ledger

- The pod serves an HTTPS API on the mesh and on localhost, with client-certificate sign-in. Only identities named in
  `local_authority` may change rules; anyone the pod's CA trusts may read status.
- Routes: `GET /v1/status`, `GET /v1/rules`, `POST /v1/rules` (allow), `POST /v1/rules/{id}/extend`,
  `POST /v1/sources/{slot}/revoke`, `POST /v1/rules/{id}/reactivate`, `GET /v1/pending`. Every change requires `until` no
  later than the longest rule length, and a `reason`.
- Each change is appended to a ledger (one JSON object per line) signed by the pod's key, naming the authority's identity.
  On reconnection the pod posts the ledger to `POST /api/carriage/pods/{pod_slot}/ledger`; the portal lists the local
  rules for an org lead to ratify or let lapse.
- `goat pod` calls this API; Fieldkit Gatepost is a page served by the same API.

### 7.5 Portal API and CLI

| Route | Who | CLI |
|---|---|---|
| `POST /api/carriage/pods` `{pod_slot, org}` | operator | `goat carriage pod add <pod-slot> --org <org>` |
| `GET /api/carriage/pods` | signed in | `goat carriage pod ls` |
| `POST /api/carriage/rules` `{source_slot or source_org, pod_slot or pod_org, kind, until, reason}` | the source's custodian; accepted by the pod's custodian | `goat carriage allow <source> --pod <pod> --until <time> --reason "…"` |
| `GET /api/carriage/rules?pod=&source=&state=active\|archived` | signed in | `goat carriage ls` |
| `POST /api/carriage/rules/{id}/extend` `{until}` | rule owner | `goat carriage extend <rule> --until <time>` |
| `POST /api/carriage/rules/{id}/revoke` | rule owner, either custodian | `goat carriage revoke <rule>` |
| `POST /api/carriage/rules/{id}/reactivate` | rule owner, inside the window | `goat carriage reactivate <rule>` |
| `PUT /api/carriage/sources/{slot}/org-rule` `{opt_out}` | the source's custodian | `goat carriage opt-out <source>` |
| `GET /api/carriage/pods/{pod_slot}/generation` | the pod's own certificate | — |
| `POST /api/carriage/pods/{pod_slot}/ledger` | the pod's own certificate | — |
| `GET /api/carriage/local-rules?state=pending` | org lead | `goat carriage pending` |
| `POST /api/carriage/local-rules/{id}/ratify` and `/lapse` | org lead of the pod's org | `goat carriage ratify <id>` / `lapse <id>` |

The org rule is created when a pod is registered for an org (`POST /api/carriage/pods`) and covers every source whose
custodian org is that org, except sources that opted out. Machine enrollment (single and bulk) accepts `keyless: true`,
`adapters` and `receive` for keyless sources, and `pods` to request named rules at enrollment.

Status: the `goat carriage` verbs in the table are built (`crates/goat-cli/src/cmd/carriage.rs`) and tested against a mock
portal. `<source>` and `--pod` take a 32-hex slot or `org:<org>` (sent as `source_org` / `pod_org`); `allow --kind`
is `named` (default) or `temporary`; a rule granted by someone who is not also the pod's custodian stays `proposed`
until the pod's custodian runs `goat carriage accept <rule>` (`POST /api/carriage/rules/{id}/accept`);
`reactivate --until` is optional; `pending --state` takes `pending` (default), `ratified`, `lapsed` or `all`;
`opt-out --undo` sends `opt_out: false`. `goat carriage ls` lists exclusions (kind `exclusion`, a ratified local
revoke, no end date) with the other rules; `goat carriage ratify` shows the central rule ratification produced. List verbs print `{"pods"|"rules"|
"local_rules": […]}` with `--json`. The bulk enrollment file (`goat machine enroll --bulk`) carries `keyless`, `adapters`
(slots), `receive` (`interface`, `port`, `sender`) and `pods` (slots) to the portal; the CLI refuses a keyless entry
without adapters and adapters or receive on an entry that is not keyless. Single enrollment (`goat machine enroll`) sends the same fields from
`--keyless`, `--adapter <slot>` (repeatable), `--receive-iface` / `--receive-port` / `--sender` (all three or none) and
`--pod <slot>` (repeatable), checked by the same rules.

## 8. Build order

| Phase | What | Status |
|---|---|---|
| 0 | Guidance: no hand-edited pod ACL entries; interim key convention `<pod-slot>/routed/<source-slot>/…`, documented as unverified | Can do now |
| 1 | Measure: duplicate delivery over two live paths, signature overhead per sample (ACL reload is settled: restart, F-156) | Measured 2026-11-19 (§6) |
| 2 | Carriage rules and pod ACL rendering, reads included; bundle router hints; rules in the bulk enrollment file | Designed; `goat carriage` verbs and the bulk-file fields built in the CLI |
| 3 | Provenance: attachment signature, grades, keyless sources, ASTERIX ingress adapter, verification in the CLI | Designed |
| 4 | Core ACL rendering, carriage health, drift check | Designed; after the current security fixes to the core template are disclosed |
| 5 | Local authority: pod API and ledger, `goat pod`, ratification, then Gatepost | Designed; `goat pod` and `goat carriage pending\|ratify\|lapse` built in the CLI |
| 6 | Move existing devices: route-only updates, enrollment of identity-less sources, retirement of interim `routed/` prefixes | Designed; per pod, after phase 2 |
