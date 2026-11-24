# Pod carriage and provenance

| Field | Value |
|---|---|
| Status | Partly built ([ADR 1251](../adr/1251-every-source-keeps-its-identity-through-a-pod-carriage-rules-provenance-at-entry-and-pod-local-authority.md), Accepted): phases 3 and 4 and most of phases 2 and 5 are built; the CLI verbs, bundle router hints and phase 6 are not (§8); build tracked in goatcore#1025 |
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

- Consent once per rule: the source's custodian grants, the pod's custodian accepts; for the org rule both are the org. A rule
  granted by someone who is not also the pod's custodian is `proposed` until the pod's custodian accepts it
  (`POST /api/carriage/rules/{id}/accept`); only `active` rules are rendered.
- A device can opt out of the org rule.
- Expired rules are archived and kept. The owner can reactivate one in a single step within the reactivation window (six
  months by default, a goatnet setting); after that it is approved like a new rule. A named or temporary rule runs at most 90
  days by default, also a goatnet setting.
- Rendering: a generator produces each pod's ACL (each covered source's own certificate under its own slot; only the pod's
  certificate on the pod's slot) and the core router's ACL (a pod's connection may write the slots its rules cover). Pods pull
  a signed generation. A drift check compares rendered ACLs with the rules.
- Core rendering (built, phase 4): the portal's slot fragment gains one subject per pool slot, `carry-<anchor>`, naming the
  leaf CNs of every registered pod whose generation lists that slot and, for a keyless source, of its ingress adapters; the
  unassigned CN when nobody carries it. The router template binds `carry-<anchor>` to that slot's own write rule
  `slot-w-<anchor>` and nothing else (`zenoh_carriage_core_enabled`, off by default; off, the render is byte-identical to the
  template before it). So a carrier writes exactly the source's own write set, a topic-budget narrowing included, and gains
  no read. The pod's generation and the core's carriers come from one snapshot of the registry, so they agree. Every rule
  change (pod registration, grant, accept, revoke, reactivate, opt-out, ratification) re-renders the fragment, and the
  portal re-renders it every minute for expiry; the router host's reload watcher loads it: a carrier removed is a
  revocation (prompt), a carrier added is batched. A fragment rendered before carriage declares no `carry-` subject: the
  template then binds each carriage policy to an inert `carry-<anchor>-none` until one role run after the portal upgrade;
  after that, rule changes need no role run.
- Drift check (built): `GET /api/carriage/drift` (operator) and `goat-zenoh-portal carriage-drift --db-dsn … --acl-config …`
  (exit 1 on drift) compare the `carry-` subjects in the live router config and in the reload watcher's loaded record with
  the carriers the rules give, and name every difference: a carrier missing, a carrier with no rule, a slot with no `carry-`
  subject, a policy still bound to the inert `-none` subject, the template flag off while rules name carriers.
- Carriage health (built): `GET /api/carriage/pods` carries per pod `health`: `state` (`never_pulled`, `current`,
  `behind` when the rules changed since the pod's last pull), the last generation served and its age, the last ledger
  upload and its age, and the number of pending local rules. Operator page:
  [`pod-carriage.md`](../operations/pod-carriage.md).
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
  Signing format and key handling are in §7.2; the signer's goat leaf key is P-256, read with goat's grant-crypto key
  handling.

### What is built

| Piece | Where | Status |
|---|---|---|
| Format, canonical JSON, sign and verify, certificate publication and fetch, chain check, cache | `ops/provenance/goat-provenance` | Built |
| `goat pub --sign` (device-signed), `goat sub --provenance` (grade and verify each sample) | `crates/goat-cli` | CLI verification: built, held until crates/Cargo.lock can change |
| ASTERIX ingress adapter | `ops/provenance/goat-ingress`, operator page [`ingress-adapter.md`](../operations/ingress-adapter.md) | Built |
| Keyless-source enrollment (`keyless: true`, `adapters`, `receive`) | portal, §7.3, §7.5 | Built |
| CoT adapter | — | Not built |

When it lands, `goat sub --provenance` prints each sample as `<key> [<grade>] <payload>`, where the grade is `device-signed by <cn>`,
`adapter-attested by <cn> (<iface> port <port> from <sender>)`, `pod-vouched only`, `unsigned`, or `INVALID: <reason>`.
A subscriber cannot see which router forwarded a sample, so without `prov` it grades **pod-vouched only** for an interim
`<pod-slot>/routed/<source-slot>/…` key and **unsigned** for any other key. Each signer certificate is fetched once and
cached for ten minutes; a missing one is remembered for thirty seconds, so a stream from one signer costs one query, not
one per sample ([ADR 1123](../adr/1123-client-demand-must-not-amplify.md)).

### ASTERIX key layout

The adapter publishes each received frame under the keyless source's slot, one key per ASTERIX category:

```text
<source-slot>/stream/asterix/<cat>/raw/v1            default
volatile/<source-slot>/stream/asterix/<cat>/raw/v1   with volatile = true (routed, never stored; ADR 1032)
```

`<cat>` is three digits (`034`, `048`). The payload is the received bytes, unchanged. A datagram that carries one category
is one sample; a datagram that mixes categories becomes one sample per category, with that category's blocks in their
received order. A datagram from any address other than the declared sender, a datagram whose block framing does not hold,
and a datagram carrying a category the source did not declare are dropped and counted, never published. The legacy-feed
demonstration's sibling digest key (`raw-digest/v1`) is not used: the digest is `prov.sha256` in the attachment.

To run the adapter, see [`ingress-adapter.md`](../operations/ingress-adapter.md): one TOML file names the adapter's goat
context and each keyless source (slot, listen address, interface, sender, categories), then
`goat-ingress --config <file>`.

## 4. A cut-off pod and its local authority

- A cut-off pod enforces its last signed rule generation and reports its age in health.
- The pod's signed rules name its local authority in advance. That person signs in with the goat certificate or card they
  already hold; the pod checks it against cached CA roots.
- Local authority may add, extend, revoke or reactivate rules **for that pod only**, each with an end date, each signed locally
  and kept in an append-only ledger. It cannot change another pod or the core, cannot grant without an end date, and cannot
  mint identities (new devices met in the field use offline enrollment).
- Each local change re-renders the pod's ACL and restarts its router. When the router rejects the result, an addition rolls
  back to the last known good configuration; a removal never does (§6, "When the pod router rejects a change").
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
| `goat pod rules revoke <source>` | Cut a source off this pod now, by restarting its router (timing in §6) |
| `goat pod rules reactivate <rule>` | Bring back an archived rule |
| `goat pod pending` | Local changes waiting for ratification |

Status: the `goat pod` verbs are built (`crates/goat-cli/src/cmd/pod.rs`) and tested against a mock of the §7.4 API. `allow`,
`extend` and `reactivate` send an end date and a reason (`--until` takes an RFC 3339 time or a length such as `4h`, `24h`,
`72h`); `revoke` takes effect with the pod router's restart (§6, "When a change takes effect") and sends only a reason. The pod is named by `--pod <url>` (`GOAT_POD_URL`, or
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
`E_CARRIAGE_UNKNOWN` (`not_found`, `pod_not_registered`, `source_not_live`, `pod_slot_not_live`). The pod's own refusals map the same way, plus `conflict` (`E_CARRIAGE_STATE_CONFLICT`),
`not_signed_in` (`E_POD_NOT_SIGNED_IN`) and any other refusal, such as `invalid_request` (`E_POD_REFUSED`, with the pod's
message and fix); a pod that cannot be reached or whose certificate is refused is `E_POD_UNREACHABLE`, and a missing pod
URL or slot `E_POD_NOT_NAMED`. A portal `config` (503) is `E_PORTAL_SERVER_ERROR`; the bulk enrollment refusals `invalid_keyless` and `invalid_pods` are shown per entry.

## 6. Edges and local choices

| Edge | Choice |
|---|---|
| Many live paths at once | Any allowed path is valid. Measured: a client-mode session given two pods uses one at a time and every publication arrived once; a peer-mode session linked to two routers delivered every publication twice. Any remedy is session-side (options below) |
| Pod behind a pod | Not supported; a pod forwards only sources it authenticated |
| Across sovereigns | A rule never crosses a trust root ([ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md), [ADR 1020](../adr/1020-zenoh-federated-peering-trust-model.md)) |
| Silent denial | An uncovered source's TLS session to the pod opens, because its certificate chains to the goatnet CA; the pod router then denies every message from it by default (measured 2026-11-23 on zenohd 1.9.0: puts, subscriptions and queries; the refusal is in the router log and the source sees no error). The pod publishes per-source carriage health. At the core, the drift check names every carrier the router does not admit, and the pod's `health.state` says whether it holds the current rules |
| When a change takes effect | zenohd loads an ACL only at start, so every change waits for a router restart; until then the old rules apply. **At the pod**: a local change starts the restart when it is made; a new generation when the pod pulls it (every 60 s by default); an end date when it passes, connected or not (built: a timer on the earliest end date). Measured 2026-11-24 on the version before that timer and the short stop grace: a local revoke 10.1 s until the new router process started and 16.3 s until it was healthy; an expiry 55 s (the next 60 s tick, then the restart). zenohd 1.9 runs as PID 1 and installs no SIGTERM handler, so docker waited out its 10 s stop grace on every restart (5 of 5); the grace is now 1 s. The timing with the timer and the 1 s grace is unmeasured. **At the core**: router restart 12 s after a central revoke and 56 s after an expiry (one measurement each); additions are batched |
| When the pod router rejects a change | Built, tested with a stub router, not run on a pod. **Addition** (no grant is taken away): the previous configuration is restored and the generation is not tried again. **Removal** (an expiry, a revoke, a lapse, or a generation that takes any grant away): the previous configuration is never restored. The router falls back to the pod's base configuration, which carries no source and keeps the pod's own identity working; the desired configuration is tried again at each tick, each try restarting the router; when the base fails too the router is stopped. The generation stays adopted, the base becomes the last known good configuration, and `/v1/status` shows `suspended: true`, `last_apply_error` and every source `refused`. A change is a removal when any (certificate, direction, message, key) the live configuration grants is missing from the new one |
| Scale | ACL size grows with covered slots per pod. At the core: one `carry-<anchor>` subject and one policy per pool slot, and one CN per (carrier, carried slot) pair. zenohd 1.9 applies an ACL change only by restarting the router (F-156), so rendering follows the slot fragment's pattern (`ops/ansible/roles/goat-zenoh/files/slot-fragment-reload.py`): a removal restarts the router without batching, additions are batched |

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
  "local_decisions": [{"seq": 3, "local_rule_id": "lr_<id>", "state": "ratified | lapsed"}],
  "signer": "<portal certificate CN>",
  "signature": "<base64 ECDSA P-256 DER>"
}
```

- `principals` lists the certificate CNs the pod admits as writers of that slot: the device's own leaf, or for a keyless
  source the ingress adapters declared for it. `until` is null only for an org rule.
- `read` is the goatnet's read set for carried sources, a portal setting. It never contains a bare `**`.
- The signature covers the document without `signature`, serialized as JSON with sorted keys and no insignificant
  whitespace, prefixed by the bytes `goat-carriage-v1\n`. Exactly: numbers as written, strings as UTF-8 with `<`, `>`
  and `&` not escaped, no trailing newline (Go `encoding/json` with `SetEscapeHTML(false)`; Python
  `json.dumps(doc, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`). The signature is ASN.1 DER ECDSA
  P-256 over the SHA-256 of those bytes, base64. The pod verifies it against the portal signer certificate in its
  bundle and refuses a generation lower than the one it holds.
- Pull: `GET /api/carriage/pods/{pod_slot}/generation`, authenticated by the pod's own principal: its enrollment API key.
- `local_decisions` lists every line of the pod's ledger the portal has decided, by `seq`, including lines lapsed on
  arrival. A ratified grant also appears in `sources` as a central named rule with the end date the pod gave it; the pod
  then drops its local copy of each decided line. It is `[]` when nothing is decided.
- The portal issues a new generation (number plus one) whenever the document's content changes, which includes every new
  decision in `local_decisions`.

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
  `POST /v1/sources/{slot}/revoke`, `POST /v1/rules/{id}/reactivate`, `GET /v1/pending`. Every change requires a `reason`;
  every change except a revoke requires `until` no later than the longest rule length.
- Each change is appended to a ledger (one JSON object per line) signed by the pod's key, naming the authority's identity.
  On reconnection the pod posts the ledger to `POST /api/carriage/pods/{pod_slot}/ledger`; the portal lists the local
  rules for an org lead to ratify or let lapse.
- Ledger line (v1): `{"v":1, "seq":<1, 2, …>, "ts":"<RFC 3339 UTC>", "pod_slot", "action":"allow|extend|revoke|reactivate",
  "rule_id", "source_slot", "until":"<RFC 3339 UTC>", "reason", "authority":{"identity","cert_cn"}, "sig"}`. `until` is
  empty for a `revoke`, which is not held to the longest rule length; every other action needs it. `sig` is
  ECDSA P-256 with the pod's leaf key over the line without `sig`, serialized like the generation, prefixed by
  `goat-ledger-v1\n`. The upload body is `{"pod_cert":"<PEM of the pod's leaf>", "entries":[<line>, …]}`; the portal checks
  that `pod_cert` chains to the goatnet CA and carries the pod slot's current leaf CN, and that every line verifies.
  Re-sending a line is a no-op; a different line at a used `seq` is refused with 409 `ledger_conflict`. The response is
  `{"accepted", "duplicate", "pending", "lapsed", "last_seq", "entries":[{"seq","local_rule_id","state","duplicate","note"}]}`;
  `last_seq`, the highest line the portal holds for the pod, is the acknowledgment.
- On arrival a line is `lapsed`, with the reason, when its authority is not named in the pod's `local_authority`, its `until`
  is later than `ts` plus the longest rule length, or goat revoked that source from that pod (the source's slot is revoked,
  or the latest central event covering the source on that pod is a revoke). Otherwise it is `pending`. Revoking a central
  rule also lapses the pending lines it covered. Ratifying a grant creates a central named rule for that source on that pod
  with the same end date (an `extend` naming an active central named rule moves that rule's end date instead); ratifying a
  `revoke` archives the named rules for that source on that pod and records an exclusion (§7.5).
- `goat pod` calls this API; Fieldkit Gatepost is a page served by the same API.

Built in goatcore `ops/goat-pod-api` (pod side; operator page
[`goat-pod-api.md`](../operations/goat-pod-api.md)). The details the routes and the ledger settle:

| Route | Body | Answer |
|---|---|---|
| `GET /v1/status` | — | `state` (`connected`, `cut off`, `no portal`), `connected`, `cut_off_since`, `generation`, `generation_issued_at`, `generation_age_seconds`, `sources_carried` (what the running router admits; `null` when a restart failed and that is not known), `sources_wanted` (what the rules carry now), `change_pending` (the running router does not enforce the current rules yet), `suspended`, `sources` (per source: `slot`, `carried` by the running router, `change` adding, removing or changing while it differs from the rules, `carriage` ok, expiring (or being removed) or refused, `state`, `origin`, `rule_id`, `until`, `principals`), `pending`, `lapsed`, `ledger_unsent`, `local_authority`, `you` (`identity`, `cert_cn`, `may_change`), `max_rule_days`, `max_rule_hours`, `reactivation_days`, last pull, apply and upload errors. `sources_connected` and `sources[].connected` are `null`: which carried devices hold a session now is not measured |
| `GET /v1/rules` | — | `rules`: every rule the pod knows (central from the held generation, central rules from earlier generations as `archived`, local rules), each with `id`, `source` and `source_slot`, `kind`, `origin`, `state` (active, expired, revoked, archived), `until`, `local_until`, `ended_at`, `principals`, `write`, `authority`, `ratification` (pending, ratified, lapsed, or null for a central rule no local change touched); `revocations`: local revokes in force |
| `POST /v1/rules` | `{source, until, reason, principals?}` | `201 {rule, entry}`. `source` (or `source_slot`) is a slot, or a certificate name `<slot>.g<N>` that becomes the principal; a bare slot is accepted only for a source the pod has carried before (its principals are reused) |
| `POST /v1/rules/{id}/extend` | `{until, reason}` | `200 {rule, entry}`; active named, temporary or local rules only (an org rule has no end date) |
| `POST /v1/rules/{id}/reactivate` | `{until, reason}` | `200 {rule, entry}`; an archived, expired or revoked rule that ended inside the reactivation window and that the portal has not decided |
| `POST /v1/sources/{slot}/revoke` | `{reason}` | `200 {rule: null, entry}`; the source is cut off when the pod router has restarted, which the answer waits for (timing in §6), and stays cut off until a later local allow or reactivate, or until the portal decides the revoke, which frees central rules only (below). A revoke takes no `until` |
| `GET /v1/pending` | — | `{pending: [...]}`: every local change the portal has not ratified, each ledger entry with `uploaded`, `portal_state` (from the upload answer) and `ratification` (pending or lapsed) |

- Errors are `{"error": {code, message, what, why, fix}}`. Codes: `carriage_until_required`, `carriage_until_past`,
  `carriage_over_max_length`, `carriage_reason_required`, `invalid_request` (400); `not_signed_in` (401);
  `carriage_not_authorized` (403); `not_found` (404); `carriage_rule_expired`, `carriage_rule_archived`,
  `carriage_beyond_reactivation_window`, `carriage_not_archived`, `conflict` (409). A change the router refuses is still
  recorded: `202 {rule, entry, warning}`, and the `warning` says what
  runs instead (§6, "When the pod router rejects a change").
- `until` must be in the future and no later than the longest rule length (90 days by default, the pod setting
  `max_rule_days`); `reason` is required, at most 500 characters. A pod holding no generation names no local authority
  and refuses every change.
- The pod's local API server certificate is the pod's goat leaf (CN = pod slot), chaining to the goatnet CA; its names do
  not match a local or mesh address, so a client checks the chain and the CN.
- Ledger entry, as uploaded: `{v: 1, seq, ts, pod_slot, action (allow|extend|revoke|reactivate), rule_id ("lr_<16 hex>"
  for an allow, the rule changed for extend and reactivate, empty for revoke), source_slot, until (RFC 3339 UTC, empty
  for revoke), reason, authority {identity, cert_cn}, sig}` and nothing else. `sig` is ECDSA P-256 DER (base64) with the
  pod's leaf key over `goat-ledger-v1\n` + the entry without `sig`, serialized as in §7.1. `seq` runs from 1 without a
  gap and is never rewritten. The pod stores each entry with pod-only detail (the principals and write set of an
  allow) that is not uploaded, and refuses to start on a ledger whose signatures or sequence do not
  verify, or that is shorter than what the portal acknowledged.
- Upload: `POST /api/carriage/pods/{pod_slot}/ledger`, `Authorization: Bearer <pod API key>` (the portal key issued at
  the pod's machine enrollment, the goat profile's `portal.token`; the generation pull uses the same key), body
  `{"pod_cert": "<PEM of the pod leaf>", "entries": [...]}` with the entries after the last acknowledged one. The portal
  answers `{accepted, duplicate, pending, lapsed, last_seq, entries: [{seq, local_rule_id, state, duplicate, note}]}`;
  resending a stored entry is a no-op, a different entry at a stored `seq` is `409 ledger_conflict`.
- Ratification reaches the pod in the generation's `local_decisions: [{seq, local_rule_id, state: ratified|lapsed}]`. A
  decided entry stops acting locally: a ratified allow arrives as a central source entry, a lapsed one is dropped, a
  ratified revoke arrives as the source missing from `sources`, a lapsed revoke lifts the block from central rules. The portal bumps the
  pod's generation on every ratify and lapse.
- A decided revoke (built, tested, not run on a pod): a decision on a local revoke, ratified or lapsed, lifts its block from
  central rules only. A local allow, extend or reactivate of that source with a lower `seq` stays cut off until a later local
  allow or reactivate. A central rule applies again as soon as the portal names the source.
- What a locally allowed source may read: a pod has one `read` set, and the rendering gives it to every carried source,
  central or local. It equals what the core would admit (ADR 1251 decision 3) only where every source has the same read
  grant. Where grants differ per source (scoped read), a carried source, a local-only one included, can read through the pod
  more than its own core grant. Closing that needs per-source read keys in the generation, for local-only sources included
  (not designed). Measured 2026-11-24: a source admitted by a local allow alone read stored and live keys of two other slots
  through the pod, and none before the allow or after the revoke; what the core would have admitted that source itself was
  not measured.
- One local rule shows two ids: the pod's `rule_id` and the portal's `local_rule_id`. The portal keeps the pod's as
  `pod_rule_id`; decisions match on `seq`. A display matter for `goat pod` and the console.
- Carriage health: when the pod setting `health_publish` is on, the pod publishes the JSON form of `EnvHealthV1`
  (`domain` `pod`, `scope` the pod slot, `detail.pod` the status above) on `ops/pod/health/<pod-slot>` each pull interval
  with `goat pub`, and its rendering admits that one key from the pod's own identity toward the fabric. The core admits it
  with phase 4. Off by default.

### 7.5 Portal API and CLI

| Route | Who | CLI |
|---|---|---|
| `POST /api/carriage/pods` `{pod_slot, org}` | operator | `goat carriage pod add <pod-slot> --org <org>` |
| `GET /api/carriage/pods` | signed in | `goat carriage pod ls` |
| `POST /api/carriage/rules` `{source_slot or source_org, pod_slot or pod_org, kind, until, reason}` | the source's custodian; active at once when the caller is also the pod's custodian, otherwise `proposed` | `goat carriage allow <source> --pod <pod> --until <time> --reason "…"` |
| `GET /api/carriage/rules?pod=&source=&state=proposed\|active\|archived` | signed in | `goat carriage ls` |
| `POST /api/carriage/rules/{id}/extend` `{until}` | rule owner | `goat carriage extend <rule> --until <time>` |
| `POST /api/carriage/rules/{id}/revoke` | rule owner, either custodian | `goat carriage revoke <rule>` |
| `POST /api/carriage/rules/{id}/reactivate` | rule owner, inside the window | `goat carriage reactivate <rule>` |
| `PUT /api/carriage/sources/{slot}/org-rule` `{opt_out}` | the source's custodian | `goat carriage opt-out <source>` |
| `POST /api/carriage/rules/{id}/accept` | the pod's custodian (a `proposed` rule becomes `active`) | `goat carriage accept <rule>` |
| `GET /api/carriage/pods/{pod_slot}/generation` | the pod's own principal: its enrollment API key (operators refused) | — |
| `POST /api/carriage/pods/{pod_slot}/ledger` | the pod's own principal: its enrollment API key (operators refused) | — |
| `GET /api/carriage/local-rules?state=pending` | org lead | `goat carriage pending` |
| `POST /api/carriage/local-rules/{id}/ratify` and `/lapse` | org lead of the pod's org | `goat carriage ratify <id>` / `lapse <id>` |

"The pod's own key" is the portal API key issued to the pod's slot at its machine enrollment, sent as a Bearer token: the
portal authenticates by key, not by client certificate. Operators are refused on these two routes.

An **exclusion** (rule kind `exclusion`, `source_slot` and `pod_slot`, no end date) takes one source off one pod's org rule;
named and temporary rules for that pair still apply. It is recorded when an org lead ratifies a pod's local `revoke`, is
listed by `GET /api/carriage/rules` like any rule, cannot be extended or reactivated (`carriage_exclusion_no_end`), and is
lifted by `POST /api/carriage/rules/{id}/revoke` by the source's custodian only.

`POST /api/carriage/pods` also takes `local_authority: [{identity, cert_cn | slot}]`; a `slot` is rendered as that slot's
current leaf CN. The goatnet settings are `PORTAL_CARRIAGE_MAX_RULE_DAYS` (90), `PORTAL_CARRIAGE_REACTIVATION_DAYS` (183),
`PORTAL_CARRIAGE_READ` (`*/**`), and `PORTAL_CARRIAGE_SIGNER_KEY` / `PORTAL_CARRIAGE_SIGNER_CERT` (PEM files; the
generation route answers 503 without them). The ledger route needs the CA root the portal already bundles
(`PORTAL_CA_ROOT_CERT`).

The org rule is created when the first pod is registered for an org (`POST /api/carriage/pods`) and covers every source whose
custodian org is that org, except sources that opted out. A source's orgs are the data roster's orgs of its own slot and of
its custodian (the definition governance uses); without a roster the org rule covers nothing. When a source is covered by
several rules, the generation lists it once, under the org rule if one covers it, otherwise under the rule that ends last.
A source with no certificate that could write for it (no current leaf; a keyless source with no live adapter) is left out.

Machine enrollment (single and bulk) accepts `keyless: true` with `adapters: [<adapter slot>]` (enrolled machines) and
`receive: {interface, port, sender}` (`sender` an IP address or `IP:port`) for keyless sources: a slot and the custody edge
are recorded, the source counts against its custodian's quota, and no leaf, API key or mesh key is minted. `pods: [<pod
slot>]` requests a named rule per pod at enrollment, running the longest rule length, under the same consent rule as
`POST /api/carriage/rules`; the response lists each request's rule and state, and a refused request does not undo the
enrollment.

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
| 2 | Carriage rules and pod ACL rendering, reads included; bundle router hints; rules in the bulk enrollment file | Portal built: rule storage (registry migration 3), consent, expiry, reactivation, opt-out, the signed generation, keyless sources and `keyless`/`adapters`/`receive`/`pods` in single and bulk machine enrollment. Pod rendering built (`ops/goat-pod-api`: pull, verify, render, apply with rollback; tested against zenohd 1.9.0 in zagros-moon-pod `tests/pod-carriage-local.sh`). `goat carriage` verbs and the enrollment fields (single and bulk) built in the CLI. Bundle router hints not built |
| 3 | Provenance: attachment signature, grades, keyless sources, ASTERIX ingress adapter, verification in the CLI | Built: signature, grades, ASTERIX adapter (§3 "What is built"). CLI verification: built, held until crates/Cargo.lock can change. Keyless-source enrollment built in the portal (phase 2) |
| 4 | Core ACL rendering, carriage health, drift check | Built: `carry-<anchor>` subjects in the slot fragment bound to `slot-w-<anchor>` (`zenoh_carriage_core_enabled`, off by default), drift check, pod carriage health in the portal. Proved by `tests/carriage-core-check.py` (the router's decision rules, no live zenohd); not yet proved against a live zenohd |
| 5 | Local authority: pod API and ledger, `goat pod`, ratification, then Gatepost | Portal built: ledger ingest, pending list, ratify and lapse, revocations win. Fieldkit Gatepost (against a mock pod) and the portal ratification page built ([`fieldkit-and-bleating-edge.md`](fieldkit-and-bleating-edge.md)). Pod API and ledger built (`ops/goat-pod-api`; serves Gatepost at `/`). `goat pod` and `goat carriage pending\|ratify\|lapse` built in the CLI |
| 6 | Move existing devices: route-only updates, enrollment of identity-less sources, retirement of interim `routed/` prefixes | Designed; per pod, after phase 2 |
