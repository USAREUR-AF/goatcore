# Zenoh data fabric

> **60-second summary:** goat's data fabric is Zenoh: one protocol that carries pub/sub, query/reply and storage, with routers that run the stock upstream image and peers that identify themselves with certificates from the goatnet's offline CA. It rides the NetBird/WireGuard mesh, so a peer that cannot reach the mesh cannot reach the fabric. Topics are organised under a small set of reserved top-level prefixes; the prefix fixes a topic's quality-of-service class; access is default-deny with a per-prefix allowlist; and no single subscription may name the whole keyspace. Hierarchical routing is configuration (`gateway.south`), not code, and regions route — they never authorize.

> **The one property that explains the rest:** Zenoh is **brokerless** — no central component every message passes through. That is the source of the scale win *and* of several recurring design problems (QoS, "where is my DLQ", static ACLs, silent denial). Read [`zenoh-brokerless-consequences.md`](zenoh-brokerless-consequences.md) first: it maps the brokerless root to each consequence and how goat engineers around it.

---

## Scope — Zenoh as the universal peer-interaction substrate

Zenoh's native pattern — key expressions as declarative capability addresses, pub/sub by semantic topic, no endpoint configuration — is the peer-interaction pattern phoenix-native peers need across the whole system, not only for data flows ([`phoenix-architecture.md`](phoenix-architecture.md) §2A).

Phoenix-architecture §2A defines four properties of a phoenix-native peer: persistent cryptographic identity, declarative role announcement, capability-based references (not hard-coded endpoints), and supply-chain-delivered code. Zenoh provides properties 2 and 3 for every peer-level interaction. Property 1 is the offline CA with hardware-held keys; property 4 is the image supply chain.

Seven interaction patterns, all on one substrate:

| Interaction | On the fabric |
|---|---|
| **Data publish/subscribe** | All telemetry, sensor data, tracks and status events across every peer |
| **Service discovery** | Services register under `services/<class>/<instance>/`; consumers query by pattern. Replaces the service-registry role of Consul / Kubernetes Service / Eureka |
| **Control / command** | Zenoh's native request-response primitive: a peer sends a command to a key expression and the recipient replies. Replaces bespoke HTTP / gRPC command channels |
| **Configuration distribution** | Config published under `config/<scope>/<key>`; peers subscribe to their scope. Replaces ConfigMap / etcd / Consul KV patterns |
| **Identity attestation** | Upstream Zenoh's authentication and access control, anchored to the offline-CA-issued peer identity. goat defines no capability-token format or issuer service of its own. Upstream `access_control` is static (below), so dynamic authorization is enforced outside the router |
| **Health / observability** | Peers publish health to `health/<class>/<instance>` and `ops/<domain>/health/<scope>`; aggregators subscribe by pattern. Prometheus scrape is kept as a complementary path |
| **Partner federation** | A bridge translating to a partner's protocol, for partners who do not speak Zenoh, built with the shim pattern: one shim per vendor class, not per deployment |

**Architectural claim:** every new peer integration either publishes/subscribes to Zenoh natively (if goat owns the code) or goes through a shim (if the peer is a vendor appliance that keeps its own protocol). No bespoke per-peer integration paths and no per-peer configuration on the management plane.

### What the fabric replaces

| Tool / pattern | What it does | What Zenoh does instead |
|---|---|---|
| Apache NiFi | ETL, data routing via GUI processor chains | Subscribers and shims written as ordinary tested services |
| Apache Kafka | Streaming, persistent topics, consumer groups | Native pub/sub with router storage for persistence and replay |
| Elasticsearch | Queryable data store | Queryable storage via get/reply |
| Legacy DDS | Real-time pub/sub, type system, QoS | Native pub/sub; `zenoh-bridge-dds` for interop |
| Custom glue scripts | Move data between all of the above | One protocol, so no inter-tool translation |
| Consul / Kubernetes Service / Eureka | Service discovery and registration | Key expressions under `services/<class>/<instance>/` |
| etcd / Consul KV / ConfigMap | Configuration distribution | Config under `config/<scope>/<key>` |
| Bespoke HTTP/gRPC command channels | Control plane between services | Native request-response |
| Per-integration sidecars | Mediate between services that do not share a protocol | The shim pattern — one shim per vendor class |

### What the fabric does not replace

| System | Why it stays |
|---|---|
| A cross-domain solution | A hardware-enforced boundary. The fabric conditions data on either side of it; it does not replace it |
| Application databases (PostgreSQL, etc.) | Application state belongs in a database. Zenoh storage is for fabric caching and replay |
| The NetBird mesh | The transport. Zenoh rides on top of it |
| Identity issuance | The offline CA issues identity. Zenoh transports attestation; it does not issue it |
| Disk encryption / key custody | LUKS + TPM, orthogonal to the fabric |

### Authorization — Zenoh is the static outlier

Zenoh wins the "one protocol" comparison on pub/sub, storage, query and discovery. It **loses** on one axis that matters for a multi-tenant fabric: **dynamic per-tenant authorization.**

Zenoh's `access_control` is **static, file-based and restart-required by design**, in every current version. The [access-control manual](https://zenoh.io/docs/manual/access-control/) states ACL config "cannot be updated at runtime, and requires a restart"; subjects are limited to three static identity types (network interface / TLS cert CN / username), with no pluggable matcher, callback, token or attribute support. Attribute/token-based authorization is an unimplemented upstream request ([zenoh#1432](https://github.com/eclipse-zenoh/zenoh/issues/1432)). In practice, under per-tenant write enforcement, a tenant added after the router started is denied all writes until the next router restart.

Comparable pub/sub systems solve this without a restart, with one of two patterns:

| System | Dynamic per-tenant authz? | Mechanism | Pattern |
|---|---|---|---|
| **Zenoh** (all versions) | **No** | Static `access_control` file; subjects = interface / cert-CN / username; reload = process restart | — |
| [NATS](https://docs.nats.io/running-a-nats-service/configuration/securing_nats/auth_intro/jwt) | Yes | Permissions in **signed JWTs** (account + user), resolved at connect | **Authz-in-identity** |
| [DDS Security](https://cyclonedds.io/docs/cyclonedds/0.10.5/security.html) | Yes | **Signed Permissions document** per participant, bound to its identity cert | **Authz-in-identity** |
| [Mosquitto](https://mosquitto.org/documentation/dynamic-security/) (MQTT) | Yes | Dynamic-security plugin: role/ACL changes while the broker runs | **Runtime authz hook** |
| [EMQX](https://www.emqx.com/en/blog/authorization-in-mqtt-using-acls-to-control-access-to-mqtt-messaging) (MQTT) | Yes | External HTTP auth callback per operation, or DB-backed ACL | **Runtime authz hook** |

- **Authz-in-identity** (NATS/DDS): the credential states its permissions and the router verifies the signature. Adding a tenant is issuing a credential — no central rule list, no restart, and each router validates locally against the shared trust root. This is the cleaner direction for goat, but it needs custom enforcement, because Zenoh's built-in ACL matches the cert CN as an opaque string.
- **Runtime authz hook** (MQTT brokers): the broker consults a policy on each operation. On Zenoh that would be an inline proxy in the write path — a throughput chokepoint and a new single point of failure. The fallback if authz-in-identity is infeasible.

Two further notes: NATS JetStream, the documented fallback substrate (below), has the dynamic JWT authz Zenoh lacks. And gossip does not close the gap: Zenoh gossip is peer discovery, not ACL propagation, and federated traffic loses origin identity across a hop ([ADR 1020](../adr/1020-zenoh-federated-peering-trust-model.md)).

### Substrate boundary and fallback

The conventions goat builds on the fabric — agent-class peers, schema declaration and enforcement, queryable-by-default, citation and lineage, partner release namespaces — must stay expressible against an interface NATS JetStream could also satisfy, so that falling back is a substrate swap rather than a rewrite of every convention. The documented fallback for the tactical data plane is NATS JetStream with Go consumers.

### Image posture — upstream `eclipse/zenoh` + bind-mounted rocksdb backend

The `zenohd` container is upstream `eclipse/zenoh:1.9.0`, pinned by content digest ([ADR 0858](../adr/0858-zenoh-image-upstream-pivot.md), superseding the build-from-source posture of [ADR 0708](../adr/0708-build-zenoh-from-source.md)). The version is `=1.9.0` everywhere ([`zenoh-version-pinning.md`](zenoh-version-pinning.md)). There is no in-tree zenoh build and no in-process plugin sharing an ABI with `zenohd`.

The upstream image ships `zenohd`, `libzenoh_plugin_rest.so` (admin REST + Prometheus metrics) and `libzenoh_plugin_storage_manager.so` (storage abstraction with the built-in memory backend). It does not ship the rocksdb backend, so the role:

1. downloads `libzenoh_backend_rocksdb.so` from the matching upstream `eclipse-zenoh/zenoh-backend-rocksdb` 1.9.0 release (per-arch `.zip`, SHA256-pinned);
2. drops it into the host plugin directory (`/opt/<zenoh_stack>/plugins`);
3. bind-mounts that directory into the container read-only;
4. renders `config.json5` with `storage_manager.backend_search_dirs` pointing at it and `volumes: { rocksdb: {} }`.

Router and backend come from the same upstream release tag, so the plugin ABI is aligned by construction.

`zenoh-bridge-dds` is not part of the router image. A DDS-translation consumer deploys it as a sibling container from upstream `eclipse/zenoh-bridge-dds:<tag>`.

**Content processing runs outside the router.** Inspection, transformation, conditioning and audit are subscribers and shims — ordinary services with their own tests and images — not in-router plugins. The audit subscriber is `ops/ansible/roles/audit-subscriber/`. The Rust services that link `zenoh` pin `zenoh = "=1.9.0"` exactly, CI-gated by `.github/workflows/rust-audit.yml`.

### Which role deploys a goatnet's router

Two Ansible roles can start a `zenohd`. Only one deploys a goatnet's router:

| Role | Container | Transport / authz | Deployed by | Scope |
|---|---|---|---|---|
| `roles/goat-zenoh` | `{{ zenoh_stack }}-router` | mTLS, offline-CA root, slot-pool ACL, mesh-IP bind | `zenoh-deploy.yml -e target=<site>_hosts` (standup phase 9) | **every goatnet** |
| `roles/zenoh` | `zenoh-router` | plain TCP on `0.0.0.0:7447`, no mTLS, no ACL | lab bootstrap plays only | a lab, never a goatnet |

**Standing rule for a new goatnet:** the router comes from `zenoh-deploy.yml` after the phase-5 certificates exist. Do not wire `roles/zenoh` into any goatnet playbook, and do not reintroduce `zenoh_enabled`: on a goatnet host the two roles collide on `:7447`. `site-bootstrap.yml` fails closed if a site still carries `zenoh_enabled`, and points at `zenoh-deploy.yml`.

**Redeploy hazard.** Re-running `zenoh-deploy.yml` re-renders `config.json5`. On a goatnet whose live ACL is hybrid (an Ansible seed plus runtime rules appended by the portal), a full re-render would drop the runtime rules; the role's pre-flight fails closed on that case (`roles/goat-zenoh/tasks/acl-migrate.yml`), and an image-only bump is the surgical path.

### Repository disposition — substrate in core, payload out of tree

The fabric substrate lives in goatcore ([ADR 1005](../adr/1005-monorepo-separation-map.md)): it is glued to mesh identity, the management-host stack, the Ansible role tree and the same failure domain.

**In core:** the `zenohd` deployment (role, container, federation topology); the namespace conventions in this document; the schema-evolution discipline; reference services that publish on the fabric; the shim conventions; shims where goat owns the vendor relationship.

**Out of tree by default:** mission workloads (fusion pipelines, sensor-specific integrations, effector logic); per-vendor shims where the relationship lives elsewhere; partner-deployed routers and their consumers; hierarchical router tiers tied to a specific deployment.

Out-of-tree services join by speaking standard upstream Zenoh, presenting an offline-CA-issued identity, and following the namespace conventions below. **There is no goat profile of Zenoh.** A service that runs against stock `zenohd` can join.

### Top-level key-expression prefixes

Reserved top-level prefixes, to prevent prefix-squatting between in-tree and out-of-tree publishers. A new top-level prefix is added by editing this list in a PR.

| Prefix | Purpose | Example |
|---|---|---|
| `data/` | Mission, sensor and telemetry payloads in canonical schemas | `data/sensor/detection/<site>/<id>` |
| `services/` | Service self-announcement and discovery | `services/shims/<format>/<instance>/announce` |
| `health/` | Liveness, readiness, structured health | `health/router/<site>/<instance>` |
| `config/` | Declarative configuration distribution (consumers subscribe to their scope) | `config/<service>/<instance>` |
| `cmd/` | Request-response control / command (native get/reply), including queryable-by-default | `cmd/router/<site>/restart-storage`, `cmd/<peer>/query`, `cmd/lineage/forward/<topic>` |
| `attest/` | Identity, time and supply-chain attestation; schema declarations and the queryable catalog ([`data-dictionary.md`](data-dictionary.md)) | `attest/ntp/<peer-id>/<sample>`, `attest/schema/<topic-pattern>`, `attest/schema-declaration/<topic-pattern>`, `attest/queryable/<peer>` |
| `ops/` | Operational events emitted by any service on the fabric — role-shaped, not source-tree-shaped ([ADR 0844](../adr/0844-zenoh-deployment-topology-and-acl-v1.md), [ADR 1035](../adr/1035-operator-namespace-ops-canonical.md)) | `ops/<service>/status/<event-id>`, `ops/fabric/health/<scope>` |
| `agents/` | Agent-class peer output — derived data with full citation arrays, kept apart from raw `data/` for audit clarity ([ADR 0512](../adr/0512-rapid-adaptation-with-ai.md)) | `agents/<agent>/anomaly/<track-id>`, `agents/<agent>/<request-id>/answer` |
| `release/` | Data deliberately released to a named partner — default-deny, policy-gated ([`mission-partner-sharing.md`](mission-partner-sharing.md), [ADR 0821](../adr/0821-mission-partner-sharing-pattern.md)) | `release/<partner>/sensor/track/<id>` |
| `task/` | Mid-level effector tasking, distinct from low-level `cmd/`; site-scoped (no cross-site authority) | `task/<site>/<task-id>` |
| `ack/` | Command and task replies, keyed by the originating cmd-id | `ack/<cmd-id>` |
| `telemetry/` | Wire-faithful surfaces for non-canonical wire formats. Format-shaped, not source-shaped; canonical schemas live under `data/` | `telemetry/cot/<site>/<id>`, `telemetry/<format>/<site>/<id>` |
| `volatile/` | **Routed live, never persisted** — high-throughput telemetry and bounded large-file transfer, with router storage and audit storage both off ([`zenoh-volatile-class.md`](zenoh-volatile-class.md), [ADR 1032](../adr/1032-zenoh-volatile-non-persisted-namespace-class.md)). Two kinds: `.../stream/**` (loss-tolerant firehose) and `.../blob/**` (chunked ≤100 MiB transfer). No `storages` entry ever matches `volatile/**`; the audit subscriber excludes it | `volatile/<peer>/stream/sensor/<id>`, `volatile/<peer>/blob/<artifact-id>/chunk/<NNNNNNNN>` |

Subtree structure below the first segment is the publisher's choice.

**QoS class is bound to the prefix.** The top-level prefix fixes a topic's quality-of-service class — control (`cmd/`, `task/`, `ack/`, `ops/fabric/health/**`), mission (`data/`, `release/`), situational (`agents/`, `telemetry/`, `health/`), and bulk/scavenger (`volatile/**`, dropped first). Because the class rides the ACL-governed, authenticated address rather than a sender-set priority byte, a producer cannot promote its own traffic above its grant. See [`zenoh-qos-and-prioritization.md`](zenoh-qos-and-prioritization.md) ([ADR 1033](../adr/1033-zenoh-qos-prioritization-reserved-mission-slice.md), designed, not built).

### Data-class structure under `data/`

Shape: `data/<class>/<site>/<id>`. The site segment is the **target** site (where the entity, sensor product, effector or alert resides); the issuing peer's identity comes from its certificate.

| Class | Shape | Federation behaviour |
|---|---|---|
| Bulk sensor product | `data/sensor/raw/<site>/<id>` | **Site-local only** (bandwidth; cross-site bulk fetch goes through a separate storage layer) |
| Sensor detection | `data/sensor/detection/<site>/<id>` | Federated read |
| Persistent track | `data/entity/track/<site>/<id>` | Federated read |
| Asset (long-lived owned thing) | `data/entity/asset/<site>/<id>` | Federated read |
| Snapshot observation | `data/entity/observation/<site>/<id>` | Federated read |
| Point-in-time event | `data/event/<site>/<id>` | Federated read |
| Alert | `data/alert/<site>/<id>` | Federated read |

### Conventions for entity-tracking workloads

Three additive conventions on top of `data/<class>/<site>/<id>`. None changes the key shape; `<site>/<id>` stays the entity-identity anchor. Organisational hierarchy and vendor names stay out of keys: they break under reorganisation and embed one buyer's assumptions in the wire format.

#### Geospatial boundary lifecycle events under `data/event/`

When a tracked entity (`data/entity/track/<site>/<id>`) crosses a geospatial cell boundary (MGRS, geohash, S2 or any discrete partition), the publisher MAY emit a paired event under `data/event/<site>/<crossing-id>` with a `cell_crossing` payload naming the entity, the cell, the direction (`departed` / `arrived`) and a timestamp. The entity's canonical topic keeps carrying position updates uninterrupted; subscribers wanting the lifecycle stream consume `data/event/<site>/**` and filter by type. Cell shape and resolution are payload concerns, not key concerns. Lifecycle state is never encoded in the entity key, which would change the key on every crossing and break subscriber continuity.

#### Queryable-resource vocabulary on `get` selectors

A three-mode vocabulary on the `get` selector (Zenoh's `?key=value` form):

- `?mode=last_known` (default) — the most recent value held by storage.
- `?mode=snapshot&at=<rfc3339>` — a point-in-time aggregate.
- `?mode=history&from=<rfc3339>&to=<rfc3339>` — a time-windowed series.

The mode is a selector parameter, not a key segment. A storage backend that does not implement snapshot or history MUST reply with a typed `unsupported_mode` error rather than degrading to `last_known`. Schema declarations under `attest/schema/<topic-pattern>` MAY declare which modes a topic supports.

#### Geospatial sibling-topic index pattern

For "all entities currently in this cell", the publisher MAY emit a sibling topic under `data/index/geo/<grid-system>/<site>/<cell>/<id>` (`<grid-system>` is `mgrs`, `geohash`, `s2`, …). Its payload is small — the entity ID, a reference to its canonical `data/entity/<class>/<site>/<id>` topic and a position timestamp — so a subscriber can do `data/index/geo/mgrs/<site>/4QFJ12**/**` and then `get` the entities it cares about. On a crossing the sibling key is re-emitted with the new cell and the old one SHOULD be tombstoned (empty payload + retention TTL). The canonical `<id>`-keyed topic is unaffected: the index churns, the entity does not. The pattern is optional.

### Deployment topology + ACL contract

**Zenoh inside NetBird.** Peers on the same site LAN ride the same substrate: they join the NetBird mesh and reach the routers through it, with the same identity, encryption and ACL stack. Zenoh is the primary substrate by design, not the only one: peers may use direct TCP / HTTP / gRPC / vendor-native protocols over the same mesh when a case justifies it.

**One logical Zenoh mesh** across a goatnet's routers. Routers live on management hosts, and each router binds **one interface — the host's NetBird mesh IP**. Router-to-router federation and all publisher/subscriber traffic ride the mesh. Two planes stay separate: the **control plane on wg-cp0** (management API, signal federation, replication, peer-key attestation — [ADR 0313](../adr/0313-silent-control-plane-outer-wg-wrap.md)), and the **data plane on the NetBird mesh** (Zenoh and peer-to-peer application traffic).

**Authority model.** Only command-echelon identities publish on `task/…` and `cmd/effector/…`; sensors are deny-by-default there. **No cross-site command authority:** command at site A publishes only `task/A/…` and `cmd/effector/A/…`.

**ACL contract.** Default-deny plus a per-prefix allowlist. Peer identity is an offline-CA-rooted X.509 certificate ([ADR 0407](../adr/0407-internal-x509-ca-for-service-tls.md)) whose URI-SAN encodes site, role and echelon: `urn:goat:peer:site=<site>,role=<role>,echelon=<echelon>`. Two layers: NetBird ACL groups gate IP-layer reachability; the Zenoh ACL gates topic-level read and write.

**Latency, partition, ack — defaults, with per-effector schema override:**

- Latency: bounded best-effort, sub-second target. Publish + ack, no bus-side retry, caller-side timeout.
- Partition: commands queued during a partition **expire**; they do not auto-replay.
- Ack-timeout: observational only (logged to audit and wide-event telemetry); no auto-alarm, auto-retry or auto-failover.

**Schema discipline.** Every publisher declares its schema ([`data-dictionary.md`](data-dictionary.md), ADR 0512). Enforcement starts in audit mode (counters and stamps, no rejections) and moves to enforce mode per goatnet.

**Schema neutrality.** Canonical schemas under `data/` are informed by, but not identical to, any single vendor or open standard. Translation lives at the boundary, in adapter components. Wire-faithful vendor surfaces ride `telemetry/<format>/`; canonical data lives under `data/`; equivalence tables in the data dictionary map fields.

Decision of record: [ADR 0844](../adr/0844-zenoh-deployment-topology-and-acl-v1.md).

### No one subscription may match the whole keyspace

**The rule.** A peer may filter with wildcards. It may not name the ENTIRE keyspace in one declaration. `**/<org>/**` (one org across tenants) and `**/airtracks/v1/*` (one topic across tenants) are filters and work for every peer with no grant. The literal `**` is a grant, bound to named certificates: the infrastructure identities that hold their own subject (operator, audit subscriber, the `infra-*` publishers) plus a declared list, `zenoh_fabric_wide_read`, each row with an owner, a reason and an `until:` date.

**Why the shape matters.** One subscription is one socket and one queue the router must keep fed. When that subscription is the entire fabric, a consumer that stops draining can block the router's receive path, and delivery stalls fabric-wide while the process and its healthcheck look healthy. A filter is a subset, and the router can shed it.

**The list is a transition list.** It names the peers that held a whole-fabric subscription when enforcement was turned on, so turning it on breaks nobody; the rule is aimed at the next one. At each row's `until:` date the router's subscriber census is read again: rows whose peer no longer declares `**` are deleted, and rows whose peer still does are argued, not renewed. A list that only grows has become the policy it replaced.

**The mechanism.** Zenoh matches a rule to a declaration by INCLUSION, not intersection (`zenoh-keyexpr` 1.9.0 `key_expr/include.rs`): the rule's key expression must cover the declared one, and a rule chunk that is not `**` never includes a declared `**` chunk. Three consequences:

* `*/**` covers every declaration that does not begin with `**` — every anchor, every root, every concrete key — and refuses a declared `**`. It is the first key expression of the peer read rule.
* `**`-leading FILTERS cannot be covered by a pattern. The rule one reaches for, `**/*/**`, **is not a key expression**: zenoh canonises `**/*` to `*/**`, so `key_exprs` (which deserialise as `OwnedKeyExpr`) refuse it and the router does not start. Proven against the crate in `crates/goat-cli/src/cmd/flow.rs::everything_except_star_star_is_not_a_key_expression` and re-derived by `acl-evaluate.py --self-test`. So each filter shape is ENUMERATED in `zenoh_peer_cross_tenant_read`, exactly as its subscriber declares it.
* A DENY cannot carve `**` out either. Matching is by inclusion in both the deny and allow trees (`authorization.rs` `policy_decision_point`), and the only rule key that includes a declared `**` is `**` itself — which includes everything else too. **Not granting `**` is the only mechanism there is.**

**The equivalent-spelling gap.** The ACL refuses the literal `**`. It does not refuse an equivalent spelling: `*/**`, `*/*/**`, or any expression made only of `*` chunks remains declarable, and `*/**` matches every key with at least one chunk — everything really published. The ACL cannot express the difference (the rule that would, `**/*/**`, is not a key expression); closing it properly means matching on the set a declaration covers inside the router's interceptor, which is a fork of zenoh and is not taken.

**The mitigation is detection plus a per-peer decision.** The health probe reads `@/*/router/subscriber/**` each run and counts declarations whose chunks are ALL wildcards (one literal chunk makes it a filter). It exports `goat_zenoh_fabric_wide_subscribers` and `goat_zenoh_fabric_wide_shape_present{shape}`, and two alerts fire on them: `ZenohFabricWideWorkaroundShape` when a spelling other than `**` appears, and `ZenohFabricWideSubscriberUnexpected` when the session count exceeds `zenoh_health_fabric_wide_max_sessions` (a measured session count plus headroom — not the length of the grant list, because one grant can carry several sessions). When one fires, a person decides for that peer: block it, or add it to the list with an owner and a reason.

What the ACL buys is the ACCIDENT — a subscriber written as `**` because that was the example in a README — and a named, countable population for the deliberate case.

**What renders.** With `zenoh_peer_read_scoped: true` the universal read/deliver pair becomes `peer-r-scoped` / `peer-deliver-scoped` on `*/**` + `**/@adv/**` (the second because `@`-prefixed chunks are verbatim and `**` never reaches them — [ADR 1040](../adr/1040-zenoh-advanced-pubsub-liveliness-adoption.md)) plus the declared filter shapes, and one rule `fabric-wide-r` grants `**` to one subject, `fabric-wide-read`. `declare_queryable` on `**` is NOT part of the named grant: a named reader may ask the fabric anything, and may answer only for its own namespaces. Offering to answer for everything is governed separately by the edge-queryable lockdown.

**The cost of enumerating filters.** A NEW filter shape that nobody has listed is **denied silently**: the subscriber declares without error and then receives nothing, which reads to its owner as "that topic is quiet".

> **How to add one:** append the shape to `zenoh_peer_cross_tenant_read` in that goatnet's `group_vars`, exactly as the subscriber declares it; re-render; deploy the role; restart the router (SIGHUP does not reload the ACL).

**Fail-closed otherwise.** An empty named list means nobody may subscribe fabric-wide. The pre-flight is a measurement: read the router's own subscriber census and ask the ACL what it would refuse.

```
curl -s "http://127.0.0.1:8000/@/*/router/subscriber/**" > /tmp/subs.json
<zenoh_stack>-acl-evaluate impact /etc/<zenoh_stack>/config.json5 trial-peer-read /tmp/subs.json
```

`acl-evaluate.py` (`roles/goat-zenoh/files/`, installed on every router host) ports zenoh 1.9.0's inclusion, canon-form and rule-matching semantics; `--self-test` carries the upstream vectors and is what a zenoh bump must re-prove. `roles/goat-zenoh/tests/scoped-read-check.py` asserts the decisions this section states, including the boundary (`**` refused, `**/<org>/**` allowed) and the `*/**` gap.

Rollback is the flag: `zenoh_peer_read_scoped: false` re-renders the previous ACL exactly, effective on the router restart.

### Hierarchical broker topologies are configuration, not code

Zenoh supports peer / router / client modes and arbitrary federation topologies. A regional / partner / mission-tier router hierarchy is a `zenohd` config change, not new code and not a new design. The primitive is `gateway.south`, which every router already runs on its `Auto` default — see §"Hierarchical routing — `gateway.south` and regions".

### Routing at scale, and the tactical/enterprise boundary

Zenoh routing is **interest-scoped** (a router forwards only what a downstream subscriber has declared interest in) and **loop-free by construction**, so a many-node federation does not flood or loop by default. goat relies on these as protocol properties; validating them at large fan-out is part of the scaling programme ([`zenoh-federated-peering-trust.md`](zenoh-federated-peering-trust.md)).

Two shapes of data movement stay distinct. The tactical fabric is a **live, mission-scoped sharing bus** (loss-tolerant `volatile/` firehoses, decouple-and-drop — [ADR 1032](../adr/1032-zenoh-volatile-non-persisted-namespace-class.md), [ADR 1012](../adr/1012-load-testing-discipline.md)). Aggregation across hundreds of organisations belongs in a **high-resource tier** (a cloud data lake, an enterprise analytics platform) that **subscribes to and ingests from** the fabric — a one-directional egress, never a routing peer that would turn the tactical mesh into a global sync engine. Two guardrails: the egress-consumer boundary (the edge exposes; the resourced tier reaches down and lifts), and the per-class movement contract (a publisher declares firehose vs persisted by which class it writes). Video rides its own transport for the same reason.

### Per-entity mode selection + peer-mode edge continuity

**The topology as deployed.** "One logical Zenoh mesh" is a *federated-router* mesh, not direct client-to-client Zenoh. Two layers stack, and only the lower one is peer-to-peer:

- **Network layer — NetBird/WireGuard — is peer-to-peer.** Every peer. This is what "denying a path denies data" refers to, and what survives a management outage.
- **Pub/sub layer — Zenoh — is router-mediated by choice.** Per-site `zenohd` routers federate to each other (`mode: router`); application publishers and subscribers connect to their site router as `mode: client`. A message travels **pub → site router → (federation) → site router → sub**. Routers, not a pure peer mesh, because the router holds storage, the static ACL and the audit tap.

**Disconnected sites keep working because routers are edge-local.** The router sits with the peers at each site, so a site cut off from every other site keeps its local pub/sub; only cross-site federation pauses.

**`mode` is per session, and hybrid is native.** Each entity sets `client` / `peer` / `router` independently, and all three interoperate. Peer mode is a superset of client mode: a peer still uses the router when reachable and can also form direct sessions with other peers. Adoption is selective — most entities stay `client`; only edge-critical entities opt into `peer`; never a blanket flip.

**Router-less peer continuity is designed, not built** ([ADR 1095](../adr/1095-zenoh-per-entity-mode-and-peer-edge-continuity.md)). For two endpoints to keep exchanging pub/sub when no router is reachable, three things are needed:

1. **Discovery.** Peer discovery is multicast scouting, which does not traverse the routed NetBird L3 overlay; normally peers learn each other through the router's gossip, which dies with the router. Router-less peers need pre-configured or cached `connect` endpoints.
2. **Enforcement and audit.** Direct peer traffic bypasses the router — no ACL, no audit, no storage — unless the endpoints do it themselves. Payload-level grant-keyed encryption ([ADR 1094](../adr/1094-grant-keyed-payload-encryption.md)) still holds because it is content-bound; the namespace ACL does not. Encrypt any stream that might run router-less.
3. **Bring-up.** Default peer mode drops publishes through the router in goat's configuration, so applications run `mode: client`; peer mode needs its own bring-up, not a flag flip.

When there is no IP path at all (RF or a fully denied edge), the bearer is Reticulum ([ADR 1050](../adr/1050-reticulum-denied-edge-bridge.md)), not Zenoh peer mode.

### Scaling lever — shard-per-core via process model

`zenohd` 1.9 uses userspace TLS (`tokio-rustls` with ring) on the TLS link and `quinn` on the QUIC link; it has no kTLS or `io_uring` path. If load ever makes a router CPU-bound, the cheapest lever is **shard-per-core via several `zenohd` processes** pinned to cores and joined by the existing federation primitive — no upstream patch, no port. Intrusive levers (kTLS through tokio-rustls feature flags, a `tokio-uring` port) wait for a measurement that justifies them. The QUIC link is already available as a parallel transport. CPU and per-connection counters on the data-plane TLS terminators are part of the wide-event observability, so profiling is on before it is needed.

### GOAT-Net

External material calls this substrate **GOAT-Net**. Vendors integrate with GOAT-Net and partners federate with it; inside the project it is "the fabric". The scaling design is [ADR 0216](../adr/0216-goat-net-federation-and-site-planes.md).

---

## Zenoh's three primitives

Everything the fabric does is a composition of three operations.

### 1. Put / Subscribe (pub/sub)

A publisher puts data to a key expression. Subscribers receive data matching their declared key expression.

```
Publisher                                           Subscriber
   │                                                    │
   │  put("data/entity/track/site-a/T-4401", payload)   │
   │ ──────────────────────────────────────────────────► │
   │                                                    │
   │  Subscriber declared:                              │
   │  subscribe("data/entity/track/site-a/**")          │
   │                                                    │
   │  Router forwards because the key matches           │
```

**Key expressions** are hierarchical with wildcards:

| Pattern | Matches |
|---|---|
| `data/entity/track/**` | every track at every site |
| `data/entity/track/site-a/*` | tracks at one site |
| `data/sensor/detection/*/<id>` | one sensor's detections, whatever site they target |
| `ops/*/health/**` | every operational health topic |

Subscribers receive only what matches their declared interest, and routers forward only matching data. Filtering is structural, at the protocol level.

### 2. Get / Reply (query)

A consumer sends a query with a key expression and optional selector parameters. Responders (storage or applications) reply with matching data.

```
Querier                                              Queryable
   │                                                    │
   │  get("data/entity/track/site-a/**?mode=last_known")│
   │ ──────────────────────────────────────────────────► │
   │                                                    │
   │                        reply(matching_samples)     │
   │ ◄────────────────────────────────────────────────── │
```

Queries propagate through the network, and any node with matching data can reply: distributed query, not a central index.

### 3. Storage

Routers run the storage manager, which persists published data and answers queries against it.

```json5
{
  plugins: {
    storage_manager: {
      volumes: { rocksdb: {} },
      storages: {
        track_history: {
          key_expr: "data/entity/track/**",
          volume: { id: "rocksdb", dir: "/var/zenoh/tracks" }
        }
      }
    }
  }
}
```

A consumer that missed data queries the storage; a node that comes online queries for recent history. Storage is per live slot ([ADR 1101](../adr/1101-zenoh-lazy-per-slot-storage.md)), and no storage ever matches `volatile/**`.

---

## Terminology mapping

### NiFi → Zenoh

| NiFi | Zenoh | What changes |
|---|---|---|
| Processor | Subscriber / shim | GUI-configured Java class → a service with unit tests |
| FlowFile | Sample | Opaque blob with attributes → key expression + typed payload + metadata |
| Connection | Session | GUI-drawn link → protocol-managed multiplexed transport |
| Controller Service | Router config | GUI settings page → JSON5 file in version control |
| Provenance | Audit subscriber | Internal database → structured records to your stack |
| Back pressure | Congestion control | Per-connection GUI threshold → protocol-level per-publisher policy |

### Kafka → Zenoh

| Kafka | Zenoh | What changes |
|---|---|---|
| Topic | Key expression | Flat string → hierarchical path with wildcards |
| Partition | — | Zenoh routes by key expression; no rebalancing |
| Consumer group | Subscriber | Broker-managed offsets → protocol-level delivery |
| Schema Registry | `attest/schema/**` declarations | Separate service → declarations on the fabric |
| Broker | Router | Central broker with a persistent log → mesh of routers, no single point of failure |
| ZooKeeper/KRaft | — | No coordination service |
| Topic retention | Storage | Broker-managed log segments → router storage with a configurable backend |

### Elasticsearch → Zenoh

| Elasticsearch | Zenoh | What changes |
|---|---|---|
| Index | Storage | Separate cluster → router storage |
| Document | Sample | JSON document → key expression + payload |
| Query DSL | Selector + parameters | JSON query language → key expression with selector parameters |
| Cluster | Router mesh | Separate cluster → storage distributed across routers |

### Legacy DDS → Zenoh

| Legacy DDS | Zenoh | What changes |
|---|---|---|
| Topic | Key expression | IDL-defined type → hierarchical path, any payload encoding |
| DataWriter/DataReader | Publisher/Subscriber | Generated code → a direct API call |
| QoS policy | Congestion control / priority | Many parameters → a small set of defaults; goat binds class to prefix |
| IDL | Protobuf / JSON / CBOR | Mandatory code generation → the encoding of your choice |
| Domain | Scope prefix | Numeric domain ID → key-expression prefix |
| DDS Security | TLS + ACL | Plugin security model → mTLS + access control |
| Discovery | Scouting / gossip | Multicast-based → router gossip on routed networks |

`zenoh-bridge-dds` subscribes to DDS topics on a local domain and publishes them as Zenoh key expressions (and the reverse), configured rather than coded. See [zenoh-plugin-dds](https://github.com/eclipse-zenoh/zenoh-plugin-dds).

---

## Access control

Three layers, all config-as-code:

1. **Key-expression routing.** Subscribers declare what they need; routers forward only matching data.
2. **The router ACL.** Default-deny rules per key expression and per subject (the certificate CN), rendered by `roles/goat-zenoh` from the slot-pool model and the declared grants. A denied write or subscription is silent at both ends ([`zenoh-brokerless-consequences.md`](zenoh-brokerless-consequences.md)), so ACL changes are tested by asking the ACL (`acl-evaluate.py`, `goat acl test`), not by watching for errors.
3. **Payload-level control.** Grant-keyed encryption ([ADR 1094](../adr/1094-grant-keyed-payload-encryption.md)) protects content for named readers, independent of the path it takes.

---

## Router configuration

A router is configured with one JSON5 file — version-controlled, diffable, rendered and deployed by `roles/goat-zenoh` (template `templates/config.json5.j2`). The shape:

```json5
{
  mode: "router",
  listen: { endpoints: ["tls/<mesh-ip>:7447"] },

  transport: {
    unicast: {
      tls: {
        root_ca_certificate: "<tls dir>/ca-root-cert.pem",
        listen_certificate:  "<tls dir>/router-cert.pem",
        listen_private_key:  "<tls dir>/router-key.pem",
        enable_mtls: true
      }
    }
  },

  plugins: {
    rest: { http_port: "<rest port>" },
    storage_manager: {
      backend_search_dirs: ["<plugin dir>"],
      volumes: { rocksdb: {} },
      storages: { /* one per live slot; never volatile/** */ }
    }
  },

  access_control: {
    enabled: true,
    default_permission: "deny",
    rules: [ /* rendered from the slot pool and the declared grants */ ],
    subjects: [ /* certificate common names */ ],
    policies: [ /* rule ↔ subject bindings */ ]
  }
}
```

The ACL is read at start only: a change takes effect on router restart, not on SIGHUP.

---

## Hierarchical routing — `gateway.south` and regions

zenoh 1.9.0 assigns every session a **region**, and the assignment decides how much routing state that session costs. The primitive runs on every router on its `Auto` default, which no goat config overrides. It is not a federation feature and not an access control.

The census driver is [`../../ops/scripts/zenoh-region-census.sh`](../../ops/scripts/zenoh-region-census.sh).

### What a region is

A region is a routing domain. Each session a router holds sits in exactly one (`zenoh-protocol-1.9.0/src/core/region.rs:82-89`):

| Region | Rendered as | Meaning |
|---|---|---|
| `North` | `north` | The main region. Its members share a link-state graph and an all-pairs route table. |
| `South { id, mode }` | `south:<id>:<mode>` | An indexed subregion below this router. Members are terminated here, not routed through. |
| `Local` | `local` | Same-process sessions. |

Every node in the **north** region appears in a link-state graph flooded to every other node, and every router in it holds a next-hop entry for every ordered pair of nodes. A **south** subregion's members are the router's downstream: they do not enter the north graph and cost the routing layer nothing beyond their own transport session.

`gateway.south` is the config key that decides which of a router's sessions go south (`zenoh-config-1.9.0/src/gateway.rs:24-25`). There is no `gateway.north`; north is whatever is left.

### The `Auto` preset

`gateway.south` is `Preset(Auto)` or `Custom([...])` (`gateway.rs:30-32`), defaulting to `Preset(Auto)` (`gateway.rs:36-38`). No goat role sets `gateway`. The consumer reads `config.gateway.south.clone().unwrap_or_default()` (`zenoh-1.9.0/src/net/runtime/region.rs:122`), so absence of the key is `Auto`.

Under `Auto` the region is a pure function of the two sessions' modes (`region.rs:145-159`) — no address, interface, certificate or name:

| This node | Remote | Region assigned to the session | Remote's view of us |
|---|---|---|---|
| router | client | `south:0:client` | north |
| router | peer | `south:0:peer` | north |
| peer | client | `south:0:client` | north |
| router | router | `north` | north |
| peer | peer | `north` | north |
| peer / client | router | `north` | south |
| client | peer | `north` | south |
| client | client | — | **session refused**: "North-north client-client configuration (invalid)" |

So with per-site routers and applications attached as clients, the `Auto` outcome is: **site routers share one flat north region; every application is terminated in `south:0:client` and is invisible to routing.** Under `Auto` the subregion index is always `0`.

### The arithmetic that motivates the primitive

Each router in a north region of `V` nodes holds the full all-pairs next-hop matrix — `V(V−1)` entries. The growth term is the count of nodes **in the north region**, and the primitive exists to keep that count small. Terminating entities as clients keeps them out of it entirely: client sessions contribute no rows to any link-state graph or route table.

With `Custom([...])` a router can also put *other routers* south — the multi-level shape regions exist for. In a controlled five-router rig with one gateway declaring one south subregion, north `V` went 5 → 3 on every router, and the route table went 20 → 6 at each member and 20 → 12 at the gateway. The general form is **Σ V(V−1) summed over the regions a router holds**; north-only `V(V−1)` is the single-region case.

### `Custom([...])` semantics

Read from source; this is the shape, not operational guidance.

`Custom` is an ordered list of subregions. A peer is placed in the **first** one whose filter list matches, and the region id **is that list index** (`region.rs:124-136`); a peer matching none goes north (`region.rs:138`). Reordering the list renumbers regions.

Each subregion holds `filters: Option<Vec<GatewayFiltersConf>>` (`gateway.rs:50-57`). A subject matches the list if it matches **any** filter; `None` matches everything, and an **empty list matches nothing**.

Each filter matches on five fields (`gateway.rs:65-71`); a subject matches the filter only if it matches **all** of them, and a field left `None` matches (`region.rs:163-196`):

| Field | Matches when | Asserted by |
|---|---|---|
| `modes` | the peer's `whatami` matches | the transport |
| `interfaces` | **every** interface of **every** link is in the list | the local host |
| `zids` | the peer's ZID is in the list | **the peer** |
| `region_names` | the peer asserts a region name in the list | **the peer** |
| `negated` | inverts that filter's result | config |

- A router cannot be put in a south subregion by a non-router (`region.rs:129-131`, hard error).
- **A disagreement between the two ends refuses the session outright.** South-facing on both sides (`:81`), a north-north pairing of unequal modes (`:75`), or one end's `Custom` contradicting what the other end's `Auto` computes (`:58`, `:68`) each `bail!` during Open (`region.rs:47-86`). A region misconfiguration does not degrade routing — it drops the peering.

### Practice

- **The partition is mutual.** Members of a declared south region and routers left north of it become invisible to each other at the link-state layer — each sees itself, its own region's members, and the gateway. Plan a split as a partition of the fabric's routing knowledge.
- **The gateway pays for every region it fronts.** A gateway terminating two regions of three holds 6 + 6 = 12 where a member holds 6. Size gateways accordingly; do not put a gateway on a site's smallest host.
- **The region id is the list index.** Treat the `Custom` list as append-only unless a renumber is intended.
- **Configuring one end only is the supported shape.** `Custom` on the gateway and `Auto` on its members is handled at `region.rs:50` and `:51-62`, not by an error path.
- **What none of this licenses.** No sizing rule at fleet scale, and no claim about the data plane: routing-state counts say nothing about throughput, latency, or how interests and queries behave across a region boundary.

### Regions route. They do not authorize.

`gateway.south` is a list of filters over peer attributes with an allow/deny shape, which makes it look like an access control. It is not one: two of its five axes — `zids` and `region_names` — are **values the peer asserts about itself**, and a ZID is a label, not a credential ([ADR 1041](../adr/1041-zenoh-router-zid-pinning-and-source-provenance.md), citing [ADR 0844](../adr/0844-zenoh-deployment-topology-and-acl-v1.md)).

A peer that lies about its region gets its traffic **routed** differently. It gains no access, because access is the TLS trust store plus the Zenoh ACL, and neither has any notion of regions.

**Do not scope anything by region.** Scope by certificate identity, per §"Access control".

---

## Deployment

- **Router:** `ops/ansible/zenoh-deploy.yml -e target=<site>_hosts`, role `ops/ansible/roles/goat-zenoh/` (standup phase 9). Runbook: [`../operations/zenoh-runbook.md`](../operations/zenoh-runbook.md).
- **Audit subscriber:** `ops/ansible/audit-subscriber-deploy.yml`, role `ops/ansible/roles/audit-subscriber/`.
- **Load generator:** `ops/zenoh/loadgen/`, for soak and load tests under [ADR 1012](../adr/1012-load-testing-discipline.md). Its container pins Go 1.26.10 and Alpine 3.24 and disables automatic toolchain downloads.
- **PQ hybrid TLS images:** [`zenoh-pqc-tls.md`](zenoh-pqc-tls.md).

---

## References

- [Zenoh documentation](https://zenoh.io/docs)
- [zenoh-plugin-dds](https://github.com/eclipse-zenoh/zenoh-plugin-dds) — DDS bridge
- [Zenoh Rust API](https://docs.rs/zenoh/latest/zenoh/)
- [`zenoh-brokerless-consequences.md`](zenoh-brokerless-consequences.md) · [`zenoh-version-pinning.md`](zenoh-version-pinning.md) · [`zenoh-qos-and-prioritization.md`](zenoh-qos-and-prioritization.md) · [`zenoh-volatile-class.md`](zenoh-volatile-class.md) · [`zenoh-federated-peering-trust.md`](zenoh-federated-peering-trust.md) · [ADR 0844](../adr/0844-zenoh-deployment-topology-and-acl-v1.md)
