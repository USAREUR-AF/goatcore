# Pod carriage and provenance

| Field | Value |
|---|---|
| Status | Designed, not built ([ADR 1251](../adr/1251-every-source-keeps-its-identity-through-a-pod-carriage-rules-provenance-at-entry-and-pod-local-authority.md), Proposed) |
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
- Expired rules are archived and kept. The owner can reactivate one in a single step within the reactivation window (proposed
  six months); after that it is approved like a new rule. The maximum length of a named or temporary rule is proposed at 90 days.
- Rendering: a generator produces each pod's ACL (each covered source's own certificate under its own slot; only the pod's
  certificate on the pod's slot) and the core router's ACL (a pod's connection may write the slots its rules cover). Pods pull
  a signed generation. A drift check compares rendered ACLs with the rules.
- A pod's read access is rendered from the read grants of the sources it may carry, never `**`
  ([ADR 1123](../adr/1123-client-demand-must-not-amplify.md)).

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

## 6. Edges and local choices

| Edge | Choice |
|---|---|
| Many live paths at once | Any allowed path is valid. Duplicate delivery over two paths is measured first; any remedy is session-side |
| Pod behind a pod | Not supported; a pod forwards only sources it authenticated |
| Across sovereigns | A rule never crosses a trust root ([ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md), [ADR 1020](../adr/1020-zenoh-federated-peering-trust-model.md)) |
| Silent denial | A pod refuses an uncovered source at connect time and publishes per-source carriage health |
| Scale | ACL size grows with covered slots per pod; whether an ACL change needs a router restart is measured first |

## 7. Build order

| Phase | What | Status |
|---|---|---|
| 0 | Guidance: no hand-edited pod ACL entries; interim key convention `<pod-slot>/routed/<source-slot>/…`, documented as unverified | Can do now |
| 1 | Measure: duplicate delivery, ACL reload, signature overhead | First |
| 2 | Carriage rules and pod ACL rendering; bundle router hints; rules in the bulk enrollment file | Designed |
| 3 | Provenance: attachment signature, grades, keyless sources, ASTERIX ingress adapter, verification in the CLI | Designed |
| 4 | Core ACL rendering, carriage health, drift check | Designed; after the current security fixes to the core template are disclosed |
| 5 | Local authority: pod API and ledger, `goat pod`, ratification, then Gatepost | Designed |
