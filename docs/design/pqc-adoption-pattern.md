# Bringing any endpoint into post-quantum key exchange — the generalized pattern

| Field | Value |
|---|---|
| Status | Design of record ([ADR 1086](../adr/1086-pqc-adoption-pattern.md)) |
| Date | 2026-07-17 |
| Scope | Every TLS 1.3 endpoint on the fabric — routers, Rust/Go/Python/C clients, browsers, bridges — regardless of language or runtime |
| Related | [`zenoh-pqc-tls.md`](zenoh-pqc-tls.md) (the fabric instance + FIPS variant, ADR 1079), `fips-pqc-path.md`, [`zenoh-rest-pub-bridge.md`](zenoh-rest-pub-bridge.md) (the wrap-it escape hatch), F-247 |

This document exists so that post-quantum adoption is a **repeatable
per-endpoint recipe with a built-in proof**, not a pile of custom per-service
work. If you are here to make one more thing speak PQ, read §2 (the contract),
find your stack in §4, and verify with §5. Everything else is the *why*.

## 1. The one idea

**Post-quantum key exchange on a TLS link is a property of the crypto library
at each endpoint — nothing else.** A TLS 1.3 handshake negotiates the hybrid
group **X25519MLKEM768** if and only if *both* ends offer it. So "bring X into
PQC" always reduces to the same two-part question:

1. Does X's TLS stack **offer** X25519MLKEM768 in its ClientHello / accept it
   in its ServerHello? (Per-stack recipe, §4.)
2. Is the **peer** it talks to already offering it? (For the fabric, the
   router — done: the `zenoh-pqc` / `zenoh-pqc-fips` images, ADR 1079.)

There is no protocol change, no schema change, no coordination flag-day, and
no fabric-specific magic. This is the whole reason it scales.

## 2. The universal contract (the checklist)

To bring **any** endpoint into PQC, in any language:

1. **Identify** the endpoint's TLS library and version (§3 — this is the only
   thing that varies).
2. **Apply** the per-stack recipe (§4) so the endpoint offers
   X25519MLKEM768 — or, if the stack can't, **front it with a PQ-capable
   bridge** (§6).
3. **Rebuild + redeploy** the endpoint (its normal image/release pipeline —
   PQC adds no new deploy shape).
4. **Verify on the wire** (§5): the negotiated group is X25519MLKEM768. Never
   assert adoption from config; read it from the handshake.
5. **Coverage is measured, not claimed** (§5): the `crypto-posture` surface
   (ADR 1079) reports the fraction of sessions actually negotiating PQ. That
   number climbing *is* the deliverable.

That is the entire process. The rest of this doc is the per-stack detail and
the invariants that make step 4 safe to do incrementally.

## 3. Two invariants that make this safe to do one endpoint at a time

- **Negotiation is order-free and mixed-safe.** TLS 1.3 group negotiation
  means a PQ endpoint and a classical endpoint always interoperate: they
  down-negotiate to the best shared classical group. So endpoints can be
  flipped in **any order, independently, with no flag-day** — a half-migrated
  fabric is fully functional. This is why there is no "big bang" and no
  rollback coupling. (Wire-proven both directions: M-081.)
- **FIPS is orthogonal to PQ, and lives at the endpoint that needs the
  claim.** Offering X25519MLKEM768 makes a link *post-quantum*. Whether that
  key exchange runs *inside a CMVP-validated module* is a separate property of
  *which crypto provider* the endpoint uses (a FIPS module vs a stock one).
  The **router** carries the FIPS module because the claim
  package is about router-terminated
  sessions; ordinary clients use the stock PQ provider and that is correct —
  the claim never depended on client-side FIPS. So "make it PQ" and "make it
  FIPS-validated" are two independent decisions; do not conflate them.

## 4. Per-stack recipes

Each recipe makes the endpoint **offer X25519MLKEM768**. Versions are the
minimum that ships the group; **verify the actual toolchain version at
adoption** (evidence discipline) rather than trusting a floor here.

### Rust — rustls (the fabric's native stack)
- Deps: `rustls` ≥ 0.23.22 with features `["aws-lc-rs", "prefer-post-quantum"]`.
- Code: install the provider **before any TLS init** —
  `rustls::crypto::aws_lc_rs::default_provider().install_default()` at the top
  of `main()` (or, for a library, once at first-use behind a `OnceLock`).
- This is the F-247 seam: the *first* installed process-default provider wins,
  and zenoh's own `ring` install is `.ok()`-tolerant. Proven in `zenohd`
  (14-line patch), the bench, `fips-probe`, and `crypto-posture`.
- **Chokepoint rule:** if several binaries share a TLS-init crate (the fabric's
  `panoscope-core` is shared by panoscope / stethoscope / flashlight), do the
  install **once in that crate** — one change, N binaries.

### Go — crypto/tls
- **Go 1.24+** offers X25519MLKEM768 **by default** (Go 1.23 shipped the
  draft `X25519Kyber768Draft00`). No code: build with the Go 1.24+ toolchain
  and the client negotiates it automatically against a PQ server.
- If a service pins `GODEBUG=tlskyber=0` or an explicit `CurvePreferences`
  that omits the group, remove that pin. Otherwise the only action is a
  toolchain bump in the image's builder stage.
- Verify: build reproducibly on Go ≥ 1.24; confirm on the wire (§5).

### Python — ssl / OpenSSL
- CPython's `ssl` uses the system/bundled OpenSSL. **OpenSSL 3.5+** ships
  ML-KEM and the `X25519MLKEM768` group; on 3.5 it is offered by default in
  TLS 1.3. Below 3.5: not available.
- Action: build the image on a base with OpenSSL ≥ 3.5 (or link a PQ-capable
  OpenSSL); no application code change if the default group list includes it
  — otherwise `SSLContext.set_groups(["X25519MLKEM768", ...])`.
- If the runtime can't be moved to OpenSSL 3.5: use the bridge (§6).

### C / C++ — OpenSSL / BoringSSL / AWS-LC
- OpenSSL ≥ 3.5, BoringSSL (current), or AWS-LC all expose X25519MLKEM768.
- Set the group list to include it (`SSL_CTX_set1_groups_list`); or link
  AWS-LC (the same module the router uses) for a matched posture.

### Browser / JS (wasm zenoh clients)
- Modern Chrome and Firefox negotiate X25519MLKEM768 natively; a browser
  client inherits it from the browser's TLS stack with no app change. A
  wasm/Node client using its own TLS (rustls-wasm, Node ≥ a version bundling
  OpenSSL 3.5) follows the Rust / OpenSSL recipe respectively.

### The general rule when your stack isn't listed
Find what TLS library the runtime uses, confirm it has an ML-KEM hybrid group
(everything current does as of 2025-2026), enable/verify the group. If it
genuinely cannot: §6.

## 5. Verification + coverage — the evidence loop

Adoption is only real when the **wire** shows it. Two tools, already built:

- **Per-endpoint proof:** the `crypto-posture` publisher (ADR 1079) performs a
  live raw-TLS handshake and records the *negotiated* group; a wire capture
  (tcpdump on the `key_share` extension) is the manual equivalent. Either way
  the evidence is the handshake, never the config.
- **Fleet coverage:** `crypto-posture` publishes `ops/fabric/crypto/<site>`;
  `goat health --domain crypto` renders it. As endpoints are flipped, the
  fraction of sessions negotiating X25519MLKEM768 climbs — that surface *is*
  the migration dashboard. An endpoint that hasn't been flipped shows
  classical; a stale probe shows grey. No green without evidence.

**Governance:** a PQC adoption is "done" for an endpoint when its handshake is
observed at X25519MLKEM768, not when its PR merges.

## 6. The escape hatch — wrap anything in a PQ-capable bridge

Some endpoints will never speak PQ natively (a legacy runtime, a vendor
appliance, a language with no ML-KEM TLS). The pattern still covers them
**without custom crypto work**: front them with a PQ-capable Rust bridge.

The fabric already has this shape — the `rest-pub-bridge` (ADR 1023) gives
non-Rust publishers a native Rust mTLS identity: the client posts plaintext to
the bridge *inside the mesh* (already WG-encrypted), and the **bridge's**
connection to the router is PQ by the Rust recipe (§4). So the PQ boundary
moves to the bridge, and the wrapped endpoint needs zero change. Generalize
this: **any endpoint that can't do PQ at its own TLS layer inherits PQ by
terminating through a PQ-capable proxy one hop closer to the router.**

## 7. Transport parity — TLS and QUIC are the same recipe

X25519MLKEM768 is a TLS 1.3 key-exchange group, and **QUIC uses TLS 1.3 for
its handshake**. In rustls/zenoh the QUIC path (`quinn`) reads the *same*
process-default crypto provider as the TLS path (F-247 covers both —
`io/zenoh-link-commons/src/quic/utils.rs`). So:

- A router that offers `quic/` endpoints inherits the hybrid group from the
  same provider install that gives its `tls/` endpoints PQ — **no extra
  crypto work to make QUIC post-quantum.**
- Every §4 client recipe applies unchanged to a QUIC connection.
- Enabling QUIC is therefore a **transport-availability** decision (add a
  `quic/` listen endpoint, open the UDP port), not a PQC decision. The PQC
  property comes along for free. See [`zenoh-pqc-tls.md`](zenoh-pqc-tls.md) §7
  for the fabric's QUIC-enable step.

## 8. What this pattern deliberately does not require

- **No fleet-wide coordination.** Per §3, endpoints flip independently.
- **No new deploy machinery.** Each endpoint uses its existing image/release
  pipeline; PQC is a dependency/toolchain change, not a new artifact type.
- **No client-side FIPS.** Per §2, the FIPS claim lives at the router; clients
  use the stock PQ provider.
- **No custom per-service crypto.** If you find yourself writing bespoke
  handshake code for one service, stop — you want a §4 recipe or the §6
  bridge. Bespoke crypto per endpoint is exactly the anti-pattern this
  document exists to prevent.

## 9. Adoption ledger (instances of this pattern)

**First, classify the endpoint's link to the router — this determines whether
the recipe even applies:**

- **Native mTLS Zenoh peer** — opens a `tls/`(or `quic/`) session to the
  router with a client cert. *These are the only endpoints with a TLS
  handshake to upgrade*; apply the §4 recipe for their stack.
- **REST-plugin / HTTP consumer** — reaches the router over the plaintext
  REST admin plugin on the docker/mesh network (e.g. `goat-zenoh-portal` →
  `http://<router>:8000`). There is **no zenoh-TLS handshake**; confidentiality
  is the WireGuard mesh layer, not zenoh TLS. **PQC-at-fabric-TLS does not
  apply** — do not chase a recipe here. (If such a service needs its *own*
  HTTPS surface hardened, that's a separate TLS endpoint with its own §4 line.)
- **Sidecar / bridge-fronted** — the service talks to a co-located zenoh
  sidecar or the rest-pub-bridge; the *sidecar* is the mTLS peer, so PQ is a
  property of the sidecar's stack (§6), and the wrapped service is unchanged.

Getting this wrong is the classic waste: flipping a REST consumer's toolchain
"for PQC" does nothing, because it never speaks zenoh TLS. Check the link type
before touching the stack.

| Endpoint | Link to router | Recipe | State |
|---|---|---|---|
| `zenohd` router (nova-a) | — (server) | §4 Rust, non-FIPS | live (ADR 1079) |
| `zenohd` router (ZAGROS) | — (server) | §4 Rust, FIPS provider | live (ADR 1079) |
| Router QUIC endpoint (ZAGROS) | — (server) | §7 (free from the provider) | live — quic/7448, mTLS+ACL wire-proven |
| `crypto-posture` probe | native mTLS | §4 Rust (raw probe) | live |
| panoscope / stethoscope / flashlight | native mTLS | §4 Rust — `panoscope-core` chokepoint | code merged; images redeploying |
| `audit-subscriber`, `fabric-health`, mesh/presence/time/net-health, `ntp-health-pub` | native mTLS | §4 Rust (same 2-line recipe each) | remaining incremental flips |
| `goat-zenoh-portal` | REST/HTTP (plaintext, mesh-protected) | **N/A** — not a zenoh-TLS peer | no action (correct) |
| `airdefenders` | zenoh sidecar | §6 — PQ is the sidecar's property | via sidecar |
| `zenoh-synth-vendors` | Python native or bridge | §4 Python (OpenSSL 3.5+) or §6 bridge | classical today; bridge is the clean path |

Keep this table current: a new fabric endpoint is "onboarded to PQC" when it
appears here with an observed-on-the-wire state — and a REST/HTTP endpoint is
"onboarded" by being correctly marked N/A, not by a no-op change.
