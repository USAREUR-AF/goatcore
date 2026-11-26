# Post-quantum hybrid TLS on the Zenoh fabric (X25519MLKEM768)

| Field | Value |
|---|---|
| Status | Built + wire-proven; **`zenoh-pqc` is now the router role DEFAULT (2026-07-17)** — see §7.1. Adopted as the standard image because it carries the F-253 accept-throttle fix (bare upstream cannot be patched); PQC preferred, classical clients down-negotiate. FIPS goatnets override to `zenoh-pqc-fips`, built on the AWS-LC FIPS 4.0.0 module (CMVP #5596/#5595, [ADR 1265](../adr/1265-fips-router-moves-to-the-aws-lc-fips-4-module.md)). ([ADR 1079](../adr/1079-zenoh-pqc-hybrid-tls.md)) |
| Date | 2026-07-12 |
| Related | `fips-pqc-path.md`, ADRs [0601](../adr/0601-fips-pqc-boundary-node.md)/[0602](../adr/0602-rosenpass-hybrid-rekey.md)/[0603](../adr/0603-ml-kem-at-boundary.md)/0854 (the parallel PQ paths), [ADR 0858](../adr/0858-zenoh-image-upstream-pivot.md) (image), [`zenoh-version-pinning.md`](zenoh-version-pinning.md), [`universal-deploy-rollback.md`](universal-deploy-rollback.md) |
| Tracking | `../project/zenoh-pqc-tls-implementation.md` |

## TL;DR

Zenoh's TLS 1.3 links can negotiate the **X25519MLKEM768 hybrid post-quantum
key exchange** today, with a 14-line patch confined to the `zenohd` binary
crate — no zenoh library fork, no protocol change, no config-schema change.
Unmodified ring-provider clients (the entire current fleet) keep connecting
classically to a PQ router; PQ-enabled clients negotiate the hybrid group.
This is real harvest-now-decrypt-later protection at the fabric layer — but
**for pure security-in-transit it is largely redundant with the
Rosenpass-on-WireGuard path (ADR 0602)**: all Zenoh TLS traffic already rides
inside the NetBird WireGuard mesh, and once Rosenpass ships, that outer
tunnel is PQ-protected end-to-end. The distinct, non-redundant values are
(1) PQ protection that survives any future deployment where a Zenoh link
leaves the WG mesh (federated peering, public mTLS crutch tier, relay-hosted
surfaces), and (2) a **path** to a FIPS-in-software claim without boundary-node
hardware — a path that became buildable in June 2026 when CMVP #5298/#5314
issued with ML-KEM in-boundary; the FIPS-variant image now builds on the
AWS-LC FIPS 4.0.0 module (CMVP #5596/#5595, ADR 1265; see §5).

## 1. Verified facts (evidence, not inference — checked 2026-07-12)

1. **zenohd 1.9.0 pins the ring provider.** Workspace `Cargo.toml` line 150:
   `rustls = { version = "0.23.31", default-features = false, features =
   ["logging", "ring", "tls12"] }`; `tls-listener` carries `rustls-ring`.
   ring has no ML-KEM and never will (unmaintained algorithm surface).
2. **The provider is a process-default seam, not a hard pin.**
   `io/zenoh-links/zenoh-link-tls/src/utils.rs:228,360` (and the QUIC
   equivalent `io/zenoh-link-commons/src/quic/utils.rs:195`) call
   `rustls::crypto::ring::default_provider().install_default().ok()` — the
   error when a provider is *already installed* is deliberately ignored, and
   both `ServerConfig`/`ClientConfig` are built via the builder paths that
   consult the process default. **First installer wins.** A binary that
   installs `aws_lc_rs::default_provider()` before zenoh initializes gets
   aws-lc-rs key exchange — including X25519MLKEM768 — with zero zenoh
   source modification. Only certificate-chain *signature verification*
   stays on ring's algorithm table
   (`io/zenoh-link-commons/src/tls.rs:83,97` — hardcoded), which is
   irrelevant to key exchange and continues to work with our P-256 chain.
3. **The client fleet is classical-only today.** `crates/goat-cli`,
   `crates/panoscope-core` (inherited by panoscope, stethoscope,
   flashlight-analytics), and every other in-repo zenoh consumer pin
   `zenoh =1.9.0` with no CryptoProvider install of their own → they offer
   classical groups only. TLS 1.3 group negotiation makes the mixed fleet
   safe in both directions (proven on the wire, §4): PQ router + ring client
   → X25519; PQ client + ring router → X25519; PQ + PQ → X25519MLKEM768.
4. **The FIPS certificate has issued — but a claim still needs the
   validated-module build.** CMVP **#5298** (AWS-LC 3 dynamic, 2026-06-03) and
   **#5314** (static, 2026-06-05) validate **AWS-LC FIPS 3.1.0** with
   **ML-KEM-512/768/1024 KeyGen/Encaps/Decaps approved in-boundary** (read
   from the certificate's security policy, not the vendor blog). The older
   certs (#4631/#4759/#4816) remain classical-only, and this prototype builds
   non-FIPS aws-lc-rs — so no FIPS language attaches to it (§5).
5. **rustls grows the PQ group via the aws-lc-rs provider.** rustls 0.23.22+
   with the `aws-lc-rs` + `prefer-post-quantum` features offers
   X25519MLKEM768 as the first key-exchange group (zenoh 1.9.0 resolves
   rustls 0.23.31 — in range).

## 2. What changes (and what deliberately does not)

```
                     TODAY                            PQ VARIANT
 router image   eclipse/zenoh:1.9.0@digest      ghcr.io/usareur-af/zenoh-pqc@digest
                (ring provider)                 (= upstream image, /zenohd rebuilt
                                                 from same tag + 14-line patch)
 key exchange   X25519 / P-256                  X25519MLKEM768 preferred,
                                                classical accepted (mixed fleet)
 record layer   AES-GCM/CHACHA (ring)           AES-GCM/CHACHA (aws-lc-rs)
 certs / CA     offline-CA P-256 chain          UNCHANGED
 zenoh config   config.json5                    UNCHANGED (no new keys)
 ACL/identity   URI-SAN, groups, slots          UNCHANGED
 clients        ring provider                   UNCHANGED until each binary
                                                opts in (one install_default()
                                                call + two cargo features)
```

- **Patch:** [`ops/zenoh/pqc-image/patches/zenohd-pqc-aws-lc-rs.patch`](../../ops/zenoh/pqc-image/patches/zenohd-pqc-aws-lc-rs.patch)
  — `zenohd/Cargo.toml` (+4 lines: rustls `aws-lc-rs` + `prefer-post-quantum`
  features) and `zenohd/src/main.rs` (+10 lines: install the provider first
  thing in `main()`). No library crate touched → plugin-trait ABI surface
  unchanged relative to a same-toolchain build.
- **Image:** [`ops/zenoh/pqc-image/Dockerfile`](../../ops/zenoh/pqc-image/Dockerfile)
  — `FROM eclipse/zenoh:1.9.0@<digest>` with `/zenohd` replaced by our build
  (same source tag, same `rust-toolchain.toml` 1.93.0, dynamic musl). rest +
  storage_manager plugins remain upstream's own `.so`s; the role's
  bind-mounted rocksdb backend is unchanged. CI smoke-boots the image with
  the full fleet plugin set per-arch and fails on the F-122/F-136 ABI class
  (zenohd refuses mismatched plugins at startup — fail-closed).
  Workflow: [`zenoh-pqc-image.yml`](../../.github/workflows/zenoh-pqc-image.yml)
  → `ghcr.io/usareur-af/zenoh-pqc`, multi-arch LIST digest.
- **Client opt-in (phase 2, per-binary):** add
  `rustls = { version = "0.23", features = ["aws-lc-rs", "prefer-post-quantum"] }`
  and one `install_default()` at binary start. Candidate chokepoint:
  `panoscope-core`'s TLS module (single place most Rust surfaces initialize
  zenoh TLS). Not required for router rollout — down-negotiation carries the
  fleet — so the client floor is an adoption-pace decision, not a gate.

## 3. Honest framing — why this is (mostly) redundant, and what it's actually for

Every Zenoh TLS link in the fleet today runs **inside** the NetBird WireGuard
mesh. WireGuard already encrypts the outer path; ADR 0602 (Rosenpass hybrid
PSK rekey) is the committed plan to make that outer layer PQ-resistant, and
ADR 0854 waives its FIPS status. Once Rosenpass ships, an adversary
harvesting fabric traffic must break the PQ-hardened WG tunnel before ever
seeing a Zenoh TLS record. **For confidentiality-in-transit of today's
topology, PQ-in-Zenoh-TLS is a second lock on the same door.**

It is *not* redundant in exactly three situations:

1. **Zenoh links that leave the mesh.** Federated peering to partner fabrics
   (ADR 1020, designed-not-built), the public mTLS crutch tier (default-off),
   or any future surface where a Zenoh TLS endpoint is reachable without a WG
   tunnel. There, Zenoh TLS *is* the only transport crypto, and hybrid PQ is
   the difference between harvest-now-decrypt-later exposure and not.
2. **Defense-in-depth against a WG/Rosenpass failure** (key-management
   compromise, implementation break, Rosenpass schedule slip — it is a small
   upstream and our Block 19 is not deployed fleet-wide yet). The inner layer
   becoming PQ is insurance priced at one image pin.
3. **The FIPS-in-software aspiration** — a Zenoh TLS stack on a CMVP-validated
   module with ML-KEM in-boundary would carry a software-only FIPS-PQC story
   for fabric traffic, without per-site boundary-node hardware (ADR 0601's
   cost). §5: this is a real path but **not a claim anyone can make today**,
   and even when the certificate issues, the surrounding claim is narrower
   than ADR 0601's (the WG outer layer stays non-FIPS under ADR 0854; the
   validated boundary would be the Zenoh TLS session only).

**Recommendation (on the merits):** ship the image to the sandbox goatnet's
routers as a standing canary (real PQ exposure where it costs nothing,
keeps the seam exercised on every zenoh bump), do **not** schedule
fleet-wide adoption ahead of Rosenpass/Block 19 — that path has the better
security-per-effort for today's topology — and re-open fleet adoption when
either (a) the first off-mesh Zenoh link is scheduled (federated peering /
crutch tier), or (b) the FIPS-variant image (validated AWS-LC FIPS
module, §5) is built and a
FIPS-in-software claim becomes cashable. Rationale in
[ADR 1079](../adr/1079-zenoh-pqc-hybrid-tls.md).

## 4. Wire-proof integration test (evidence)

Executed 2026-07-12: functional matrix first on darwin/arm64 with real zenoh
sessions (`zenoh::open` → put → close over mTLS), then wire capture on
integdev (tcpdump on the router host, ClientHello/ServerHello `key_share`
parsed from the pcap). Zero HelloRetryRequests anywhere:

| client → router | negotiated group | on the wire |
|---|---|---|
| PQ → PQ | **X25519MLKEM768** | CH 1443 B carries shares [MLKEM768 (1216 B), x25519 (32 B)]; SH 1206 B selects MLKEM768 (1120 B) |
| classical (today's fleet) → PQ | X25519 | CH 219 B; SH 118 B — byte-identical shape to stock↔stock |
| PQ → stock | X25519 | same 1443 B CH offered; stock router picks the x25519 share, no HRR |
| classical → stock | X25519 | baseline |

The 1216/1120 B shares are exactly ML-KEM-768's 1184 B encapsulation key and
1088 B ciphertext plus the 32 B X25519 component. Zenoh sessions
(open/publish/close) succeed in every cell. **Mixed-fleet coexistence is
proven at the wire level in both directions, so rollout order is
unconstrained.** Evidence + repro: M-081.

## 5. FIPS status — the certificate has issued; the claim is now buildable, not yet makeable

**Correction (2026-07-12, same day):** the first pass of this section said
AWS-LC-FIPS 3.0 was still CMVP in-process, sourced from the vendor's
`crypto/fipsmodule/FIPS.md` — which lags. The **CMVP database is
authoritative and shows the certificates issued**:

- **CMVP #5298** — AWS-LC 3 Cryptographic Module (dynamic), validated
  **2026-06-03**, sunset 2031-06-02; **#5314** (static), validated
  **2026-06-05**. Validated software version **AWS-LC FIPS 3.1.0**.
- The certificate's security policy (140sp5298.pdf, read directly) lists
  **ML-KEM-512/768/1024 KeyGen + Encapsulation + Decapsulation as approved
  services inside the module boundary** (FIPS 203; CAVP A6180/A6184/A6278/
  A6279). The earlier issued certs (#4631/#4759/#4816) remain classical-only.
- **What still separates "buildable" from "claimable":** (1) the router must
  actually build against the validated module — rustls `fips` →
  aws-lc-rs `fips` → aws-lc-fips-sys pinned to the AWS-LC-FIPS-3.1.0 tag;
  (2) this is more than a cargo flag: the FIPS module build brings the
  cmake/Go/Perl chain and is unlikely to fit the alpine/musl image shape —
  expect a glibc image variant, which re-opens the plugin story as a
  matched-pair build (ADR 0858 addendum); (3) X25519MLKEM768's
  approved-mode standing rides NIST SP 800-56C rev2's allowance for an
  approved KEM combined with an auxiliary shared secret — the same basis
  AWS uses for ML-KEM on KMS FIPS endpoints — verify rustls's fips-mode
  group list in the flip PR, don't assume; (4) the cert's tested
  operational environments are Amazon Linux 2023 on Graviton4 and Xeon —
  our Ubuntu/Tegra hosts ride the standard user-affirmation posture, which
  is compliance-package language, not an engineering blocker.
- **Built (2026-07-12, same track): the FIPS-variant image exists.**
  `ops/zenoh/pqc-image/Dockerfile.fips` → `ghcr.io/usareur-af/zenoh-pqc-fips`:
  zenohd on rustls `fips` → aws-lc-rs `fips` → **aws-lc-fips-sys =0.13.11**,
  verified (crate source inspected) to vendor exactly the AWS-LC-FIPS-3.1.0
  tag the certificates validate. Later 0.13.x versions vendor 3.2.0/3.3.0/
  3.4.0 — still in the CMVP queue — so the pin must not float. **The wrapper
  is pinned too: `aws-lc-rs` and `aws-lc-fips-sys` are a coupled pair
  — each `aws-lc-rs` release floors its `aws-lc-fips-sys` requirement, and
  `aws-lc-rs 1.17.3` raised that floor above the frozen 0.13.11, breaking the
  build. The Dockerfile now pins `aws-lc-rs=1.17.1` (newest that *compiles*
  against 0.13.11 — 1.17.2 resolves but calls a fips-sys API absent from
  0.13.11) DOWN before the module pin, with fail-closed version asserts; bump
  both together only at the next validated-module adoption.** Empirical
  answers to the three unknowns above (fips-probe, run on the pinned module
  against both rustls 0.23.31 — zenoh's resolved version — and 0.23.41):
  `try_fips_mode()` = Ok; the FIPS provider offers **X25519MLKEM768 first,
  fips=true** (SP 800-56C rev2 held), with P-256/P-384 behind it and pure
  X25519 excluded; FIPS↔FIPS negotiates X25519MLKEM768 through the plain
  `builder()` seam zenohd uses; a classical ring client down-negotiates to
  **secp256r1** (one extra HelloRetryRequest — today's clients offer only an
  X25519 key_share first). The glibc image re-opened plugins as a
  matched-pair build (rest + storage_manager + rocksdb backend all built from
  the same tag/toolchain; baked into the image, NOT bind-mounted — see the
  Dockerfile.fips header for the role change adoption requires). The probe is
  a fail-closed gate at image build AND a live `--connect` assert in CI smoke.
- **Module moved to AWS-LC FIPS 4.0.0 (2026-11-26, [ADR 1265](../adr/1265-fips-router-moves-to-the-aws-lc-fips-4-module.md)).**
  CMVP **#5596** (AWS-LC 4 Cryptographic Module, static) and **#5595**
  (dynamic), validated 2026-11-19, sunset 2031-11-18, validate **AWS-LC FIPS
  4.0.0** with ML-KEM-512/768/1024 approved in-boundary. The security policy
  (section 11.1) names the source set as the aws/aws-lc archive of commit
  `1276fc7d9cd0f509000294e9657914c3a23e65d2` (SHA2-256
  `33b5443d…968f049`), built with `-DFIPS=1 -DCMAKE_BUILD_TYPE=RelWithDebInfo`,
  reporting "AWS-LC FIPS 4.0.0". No published aws-lc-fips-sys vendors that
  set (0.14.0 vendors a later 4.x snapshot, 0.14.2+ vendor 4.2.0; both differ
  from it inside `crypto/fipsmodule/`), so `Dockerfile.fips` builds the
  module from the policy's archive, verifies the digest, `bssl isfips` = 1
  and the version string, and links it through aws-lc-fips-sys 0.14.4's
  system-library path (`AWS_LC_FIPS_SYS_SYSTEM_DIR`), with aws-lc-rs 1.19.0.
  rustls and rustls-webpki then follow zenoh's own lock: the rustls ≤ 0.23.43
  / rustls-webpki ≤ 0.103.13 hold the 3.1.0 module forced is gone. The
  3.1.0 pin pair (aws-lc-fips-sys =0.13.11 / aws-lc-rs =1.17.1) described
  above is the previous state.
- **The claim package is written** (2026-07-16):
  `../compliance/zenoh-fips-pqc-claim.md`
  — exact permitted language, the certificate→crate→image→container
  provenance chain, the SP 800-56C rev2 hybrid basis, the user-affirmation
  OE posture (tested OEs are Amazon Linux 2023), what is deliberately NOT
  claimed (no whole-system certification, WG layer stays waived, no CNSA
  2.0), and the invalidation conditions. Use its §1 language verbatim;
  anything stronger is over-claiming.

## 6. Measured overhead — PQ vs classical (integdev, real numbers)

Full method + evidence: M-081
(handshake latency + flight size, clean + impaired links) and
M-082
(post-handshake throughput). Headlines:

- **Handshake latency (clean cross-AZ link, full mTLS handshakes):**
  PQ↔PQ p50 **1.80 ms** vs classical **1.62 ms** — **+180 µs (+11%)**;
  p99 2.5 ms vs 2.4 ms.
- **Handshake flight size:** **+1224 B up / +1088 B down** per full
  handshake (total ~2.0 KB up / 1.9 KB down vs ~0.8 KB each way classical).
- **Impaired link (netem):** at 50 ms delay, p50 identical (52.1 vs
  51.9 ms) — **no extra round trip**. Loss shows up tail-only: at 1% loss
  mean +2.4 ms; at 3% loss mean 67.7 vs 61.1 ms with p50 still identical
  (~one extra MTU packet each way → proportionally more retransmit
  exposure). 0 handshake failures in 1440 impaired iterations.
- **Steady-state throughput:** no regression — a modest measured win. The
  record layer moves from ring to aws-lc-rs AES-GCM: 1 KiB ~227k vs ~210k
  msg/s (**+8%**), 32 KiB ~252 vs ~222 MB/s (**+11–14%**, bounds-only —
  2 samples/cell) with unmodified ring clients through the swapped router.
  x86_64 only; arm64 unmeasured.

## 7. Rollout + rollback

### 7.1 Default flip (2026-07-17) — forced by the F-253 correctness fix

The canary ladder below was the planned *PQC-adoption* path. It was overtaken
by a **correctness** decision: the bare `eclipse/zenoh:1.9.0` accept loop
carries the F-253 admission-seizure defect (a 100 ms throttle on per-connection
handshake failures serializes all admission to ~10/s under an mTLS reconnect
storm). Upstream's image can't be patched, so the only way to make the fix the
standard is to make our from-source rebuild the role default. `zenoh-pqc`
already carried the fix, is wire-proven, and is backward-compatible (classical
clients down-negotiate — M-081), so on 2026-07-17 the `goat-zenoh` role
default flipped from bare upstream to
`ghcr.io/usareur-af/zenoh-pqc@sha256:372de39d…`. This decides the "sandbox → other
goatnets" rung *for the non-FIPS default* — but the driver is the fix, not PQC
per se; classical peers are functionally unaffected. FIPS goatnets still
override to `zenoh-pqc-fips`. Reconciles with ADR 0858 (which had retired the
old `ghcr.io/usareur-af/zenoh` lineage for upstream): the default is a maintained
custom build again, now justified by an un-patchable upstream defect + PQC,
not the retired audit-logger build reason. A regression gate (the F-253
accept-throttle check in the pqc-image smoke) guards against a future zenoh
bump silently dropping the fix.

### 7.2 Canary ladder (the original PQC-adoption path, now largely realized)

Each rung gated on the fabric-health surface (ADR 1034) and
`goat top` staying green, with soak per ADR 1008:

1. **Sandbox goatnet router** (single site router; standing canary per §3).
   **LIVE 2026-07-12:** the nova-a trial-fabric router runs
   `ghcr.io/usareur-af/zenoh-pqc@sha256:e74217f3…` (multi-arch list digest,
   PR #1566 merge). Deployed as a surgical compose image swap (config.json5
   untouched — nova-a's live ACL is hybrid seed + portal-added rules that a
   role re-render would clobber); pin recorded in `group_vars/nova_hosts` +
   the deploy-history ledger. Wire-verified live with a real vendor cert:
   20/20 PQ handshakes = X25519MLKEM768 (p50 931 µs, ~2.2 KB up), 20/20
   classical = X25519 (p50 1007 µs) — down-negotiation carrying the
   resident fleet (portal, panoscope, audit subscriber, ~30 synth vendors)
   with zero client changes; plugins loaded from upstream `.so`s (ABI held).
   **FIPS variant LIVE on ZAGROS 2026-07-13:** the ZAGROS sandbox router runs
   `ghcr.io/usareur-af/zenoh-pqc-fips@sha256:097f9bb9…` — the first runtime on
   the CMVP #5298-validated module. Deployed surgically (image swap +
   `backend_search_dirs` → `/opt/zenoh/plugins-baked` + musl bind-mount
   dropped; the role's `goat_zenoh_fips_variant` toggle renders the
   same shape from ansible). Verified: FIPS boot marker asserted, all 1003
   slot-pool storages spawned from the baked glibc rocksdb backend, zero
   plugin errors, resident fleet (panoscope, portal, health publishers,
   synth slots) carried through the bounce. Classical clients on ZAGROS now
   negotiate P-256 (+1 HRR) instead of X25519 — the FIPS provider excludes
   pure X25519 (§5).
2. **One production site's router** — mixed fleet by construction; clients
   unchanged.
3. **Fleet routers** via the normal image-pin bump.
4. **Clients opt in per-binary** afterwards, at release pace (no floor bump
   required; a PQ client to a not-yet-PQ router down-negotiates).

Mechanics: the `zenoh` role's `zenoh_image` pin moves from the upstream
digest to the `ghcr.io/usareur-af/zenoh-pqc` **multi-arch list digest**;
the role's rollback-ledger meta-dependency records every deploy
(`artifact_type: image`); **rollback = `rollback.sh zenoh` → previous pin →
`docker compose up -d --force-recreate`** — sessions re-establish classically
within seconds (clients reconnect; negotiation handles the rest). No config,
cert, ACL, or client change is coupled to the router swap in either
direction — that's the property that makes the canary cheap.

### 7c. Enabling QUIC (transport parity — PQ is free)

QUIC uses the same TLS 1.3 handshake, so on the PQ / PQ-FIPS image a `quic/`
listener inherits X25519MLKEM768 from the same crypto provider that gives the
`tls/` listener PQ. Enabling QUIC is a
transport-availability change, not PQC work:

- **Role toggle** (`goat-zenoh`): `goat_zenoh_quic_enabled`
  (default false = nova-a byte-identical) adds a `quic/0.0.0.0:{{ quic_port }}`
  endpoint alongside `tls/`, and the compose port-map publishes that port
  `/udp`. The transport's `link/tls` block (server cert, `enable_mtls`,
  offline-CA root) is shared — **mTLS + the URI-SAN ACL apply to QUIC
  unchanged**.
- **Enable per goatnet** in group_vars (ZAGROS: `goat_zenoh_quic_enabled:
  true`, UDP 7448). QUIC rides the same NetBird WireGuard tunnel (UDP) as
  everything else.
- **Verify:** a QUIC client (`quic/<router>:7448`) connects, is admitted by
  the same ACL, and negotiates X25519MLKEM768 on the PQ image — same evidence
  path as `tls/` (crypto-posture / wire capture).
- **Rollback:** set the toggle false + redeploy — the endpoint and UDP
  port-map drop; existing `tls/` sessions are untouched.

## 7b. Evidence-based crypto-posture visibility (`crypto-posture` publisher)

The posture must be **surfaced from evidence, never asserted by a hardcoded
badge**. Two facts about a router matter, and they have different natures:

1. **"PQ is negotiating right now."** Only a TLS endpoint can know this —
   zenoh does not expose per-session key-exchange groups in its admin space.
   The evidence-true mechanism is a **live probe**: a PQ-capable rustls client
   handshakes the router and reads the *actually negotiated group*
   (`negotiated_key_exchange_group()`). This is strong, reproducible,
   remotely-observable evidence — a config flag cannot fake a wire result.
2. **"The crypto is the validated FIPS module."** This is *not* remotely
   observable by design (FIPS-mode entry is a host-internal property). It is
   host-side self-attestation: the patched zenohd prints `goat FIPS: …`
   **only if** the module passed its self-tests, and the deploy records that
   plus the running image digest. Honestly labelled self-attested, and
   freshness-graded so a stale attestation degrades, never green-washes.

`ops/zenoh-trial-peers/crypto-posture/` implements both. It is a normal mTLS
Zenoh peer that, each interval: (a) opens a raw-TLS handshake to the router
with an aws-lc-rs + `prefer-post-quantum` client and records the negotiated
group + its `fips` flag; (b) reads the host FIPS attestation file
(`/etc/<stack>/fips-attest.json`, written by the `goat-zenoh` role right
after its boot-marker assert on a fips-variant deploy); (c) reads the router's
self-reported version from REST admin; and publishes a freshness-stamped
`CryptoPostureV1` on `ops/fabric/crypto/<site>`. A reader colors it honestly:
`fips-pq` (PQ negotiated live **and** fresh FIPS attestation), `pq` (PQ
negotiated, no FIPS attestation — the non-FIPS canary case), `classical`, or
`unknown` (probe failed / evidence stale → grey). Surfaced via
`goat health --domain crypto`, rolling up the same way as the fabric/time/mesh
domains. Schema: `crypto-posture/proto/crypto_posture_v1.proto`. The probe
identity reuses `CN=fabric-health-trial-peer`, so the existing
`ops/fabric/**` ACL grant admits it with no ACL change.

## 8. Fork maintenance cost (stated plainly)

This *is* a custom router build again — the thing ADR 0858 retired — but the
delta is materially smaller than the 0708-era build: 14 patch lines against
the binary crate, upstream image reused as the runtime layer, upstream
plugins reused as-is, and the ABI risk converted into a fail-closed CI smoke
test + fail-closed zenohd startup check. Per zenoh version bump: re-apply
patch (loud failure if upstream moves `main()`), bump two ARGs, let CI prove
plugin ABI. If upstream ever ships a provider-selection feature or an
aws-lc-rs default, the patch deletes and the image collapses back to the pin.

## 9. What this doc does NOT decide

- It does not adopt PQ TLS fleet-wide (ADR 1079 records the decision when
  taken; §3 recommends the trigger conditions).
- It does not change the Rosenpass/Block 19 plan (ADR 0602) or the boundary
  node program (ADR 0601) — different layers, different claims.
- It does not make any FIPS claim (§5).
- It does not touch QUIC links operationally (the same seam covers them —
  quinn consults the same process default — but only TLS links were
  measured). **QUIC enablement + the per-endpoint client rollout are now
  worked under the generalized pattern** — [`pqc-adoption-pattern.md`](pqc-adoption-pattern.md)
  (ADR 1086): §7 there covers QUIC parity (PQ is free from the same provider;
  enabling QUIC is a transport-listen decision), and the per-stack recipes
  cover the client fleet.
