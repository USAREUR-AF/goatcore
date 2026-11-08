# ADR 1094 — Grant-keyed payload encryption: the M2 machine-read mechanism

| Field | Value |
|---|---|
| Status | Proposed (built: the grant-crypto SDK and `grant-keyd` in `ops/grant-crypto/`; grant, rotate, revoke and restart proven on a live goatnet) |
| Date | 2026-07-10 |
| Repo scope | goat core — machine-plane read enforcement |
| Supersedes | — (fills in the "M2" mechanism ADR 1076 committed to) |
| Superseded by | — |
| Related | [ADR 1076](1076-access-language-and-machine-plane-parity.md) (M2 committed as a roadmap item), [ADR 1027](1027-partner-release-set-governance.md) (release-sets = the audience unit), ADR 0412 (SPIFFE workload identity = the wrap target), [ADR 1074](1074-data-access-role-architecture.md)/[1075](1075-producer-data-labels-and-access-request-loop.md) (grant rows = policy input), [ADR 0216](0216-goat-net-federation-and-site-planes.md) (Spock replication), design: `grant-keyed-encryption.md` |

## TL;DR (for a layperson)

Machines reading data on the fabric are controlled today only by "are you enrolled at all" — any enrolled machine can read almost everything. To fix that without rebuilding the message router (which can't decide who-may-read-whom), we stop asking the router to decide: the data is encrypted, and we control who holds the key. The data is encrypted once with a single key; that key is then locked to each allowed machine's own certificate and handed out over the fabric. Only allowed machines can unlock it; taking access away means changing the key. The list of who's allowed is the *same* list that governs what people see in their browsers.

## Context

ADR 1076 committed to closing the one open cell in the plane×operation matrix — machine **reads** — via "grant-keyed encryption," but left the mechanism unspecified. The router (stock Zenoh, deliberately not forked) has a static, restart-required, exact-CN ACL: it can enforce per-identity **writes** (each cert writes only its own namespace, pre-cut at enrollment) but not **reads**, because read grants are relational (consumer×producer) and cannot be pre-cut. A per-request read check inside the router is impossible without a fork; a central key/policy server in the read path is the broker the fabric rejects (island-mode liability). The grant rows (ADR 1074/1075) and SPIFFE identities (ADR 0412) exist and are the natural inputs.

## Decision

**Enforce machine-plane per-stream reads by envelope-encrypting payloads to a release-set and distributing the wrapped data key over the fabric, governed by the same grant rows as the human plane. Enforcement transport = key possession; the router keeps routing ciphertext, unforked.**

1. **Envelope encryption.** One symmetric data key (AES-256-GCM) per release-set per generation encrypts every message once (audience-size-independent). That data key is wrapped per authorized member to its x509/SPIFFE public key (ADR 0412). Standard hybrid encryption; the enrollment cert is the root that avoids any key-bootstrap regress.
2. **Keys ride Zenoh.** The wrapped-key bundle is published on `keys/<releaseset>/<gen>` — one key topic per audience, generation as a field. Broadcast-safe (non-members receive only ciphertext they cannot open); inherits mesh P2P caching, replay, island-survival. No key server in the read path.
3. **A per-site key-distribution service off the data path** owns the calculus: it watches the grant rows, resolves the audience via the data-roles resolver (ADR 1074 item 1) to member cert keys, generates/rotates the data key, wraps to each member, and publishes. The **publisher only** sets rules (the ADR 1075 grant loop) and encrypts with the data key it is handed. The service's unavailability blocks only new rotations, never ongoing reads — so it is not a broker.
4. **Revocation = rotation.** Remove a member → new generation excluding it. Each message carries a generation tag (a Zenoh attachment) so recipients switch lazily, no flag-day; the instant hard cut remains cert revocation. History keys are retained as long as their ciphertext is.
5. **Fail-closed grant merge under scale-out.** When the registry replicates via Spock (multi-node), the grant table merges revoke-wins / monotonic-absorbing, not timestamp-wins — a partition must never resurrect a revoked grant. Banked now (single-node today), built at multi-node.
6. **Honest boundary.** This reaches **topic (release-set) granularity, not field/row** — the machine plane commits access at publish time, so it cannot match the human plane's read-time field/row predicates without a plaintext mediator (a per-stream CDS proxy, out of scope). "Same policy source, coarser machine projection" — never a claim of identical read enforcement.
7. **Scale.** Flat wrapping within the scope envelope (ADR 0215, audiences tens–low-thousands); a key tree (MLS/TreeKEM) is the named upgrade past low-thousands or under heavy churn, riding the same key topic.

## Alternatives considered

- **Per-request read checks in the router.** Impossible without a fork; read grants are relational and un-pre-cuttable. Rejected — the standing router-boundary decision (ADR 0858).
- **Central key/policy server in the read path.** The broker the brokerless fabric rejects; dies in island mode. Rejected — the key service here is off the data path and per-site-replicated.
- **Attribute-based encryption (encrypt-to-policy).** Elegant (access follows attributes, no per-member wrapping) but revocation is unsolved and maturity/performance don't fit the bar. Rejected; envelope-over-the-bus is the boring correct choice.
- **Per-data-topic keys instead of per-release-set.** Finer but multiplies keys; deferred to the design's open-decisions list (release-set is the ADR 1027 unit).
- **Fine-grained (field/row) machine access.** Only achievable with a plaintext mediator per stream — available as an explicit fine-but-mediated option for a specific stream, at the cost of the broker for it; not the default.

## Consequences

- **+** The last open cell (machine reads) has a concrete, brokerless, island-surviving mechanism that reuses live pieces (grant rows #1522, resolver #1515, SPIFFE ADR 0412, Zenoh) and keeps the router unforked.
- **+** One policy source across planes is preserved (the key service reads the same grant rows; `no_third_shim` holds).
- **−** New moving parts: a per-site key-distribution service, publisher-side envelope encoding, and a key path for every reader of stored *content* (decoded views, forensic tools, replay). Designed, staged, not free.
- **−** Enforcement is topic-grained and revocation is rotation-latency-bound — materially weaker than the human plane's read-time granularity/immediacy. Must be stated, never oversold.
- **−** Production go-live is a standard control, not conditional on who shares an environment ([ADR 1208](1208-sovereign-environments-release-namespaces-and-grant-encryption.md)).

## Build sequencing

Design doc §8: M2.0 envelope proof → M2.1 key-distribution service (single site) → M2.2 rotation/revocation → M2.3 inspection-surface + storage integration → M2.4 scale hardening + fail-closed merge → M2.5 go-live + tripwire lift. Gate: the ADR 1075/1076 grant-loop live-fire lands first.
