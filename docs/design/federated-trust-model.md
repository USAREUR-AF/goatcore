# Federated trust model

How goatnet recognizes and trusts identities from **other** trust domains (peer goatnets, and
moon pods in the learning setup, [ADR 1239](../adr/1239-a-moon-pod-is-a-learning-tool-and-not-a-sovereignty-guarantee.md)) — and how that recognition is rotated, assured, validated, revoked, graduated, and
differentiated per partner. Companion to the SPIRE/SPIFFE design (ADR 0412),
the federated-peering data-plane trust ([ADR 1020](../adr/1020-zenoh-federated-peering-trust-model.md)),
revocation ([ADR 1031](../adr/1031-credential-revocation-mesh-first-offboarding.md)) and rotation
([ADR 1044](../adr/1044-credential-rotation-in-place-reissue.md)). Proven end-to-end on the live
ZAGROS fabric — see `hello-galaxy-spire-spiffe-patterns` (`docs/live-on-zagros.md`).

## The foundation — trust is five independent questions, not one dial

The mistake that wrecks federation designs is treating trust as a single thing. It decomposes:

1. **Authentication** — *who/what is this?* (the SVID / x509 / WireGuard key / OIDC token)
2. **Trust-anchoring** — *do I accept the authority that vouches for it?* ← **federation lives here**
3. **Authorization** — *what may it do?* (the Zenoh ACL / namespace / release-set)
4. **Admission** — *can it even reach me?* (NetBird). Between two sovereigns this is its own
   design problem — neither mesh may absorb the other: `cross-sovereign-federation.md`.
5. **Assurance** — *how strongly do I believe the identity is genuine + uncompromised?*

**Federation = recognition, not authorization.** Accepting partner B's CA means "I can now
*verify* the identities B issues" — it grants them nothing. Authorization is a separate, later
step. Conflating the two ("we federated, so they're trusted") is how partners become
over-privileged. In the live proof: federation = the pod root in the router's trust bundle;
authorization = the CN-gated ACL. Two different edits, deliberately.

> **Where that separation actually holds — read this before federating anything.** The
> invariant is real in the **SPIFFE plane**, where the trust-domain prefix travels inside the
> SVID and per-domain policy can be keyed on it. It is **structurally unavailable in the x.509
> fabric plane**: Zenoh 1.9.0's ACL matches on CN string, link protocol and ZID, and has no
> issuer matcher — so a second root in the router's (already multi-root) trust concat lets that
> issuer mint any CN the subject table recognises, and the first edit silently performs the
> second. That is F-424.
> The live proof is unaffected — it federated *our own* SPIRE issuer, where the collapse is
> harmless — but it does not license adding a sovereign partner's root to a core router.
> Cross-trust-root peering terminates at the edge instead — a Tier-2 sovereign peer via its
> own forwarding `zenohd`, a Tier-3 third party via ADR 1020's re-publishing gateway:
> [ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md) +
> `cross-sovereign-federation.md`.

The **trust domain is the unit** of anchoring, policy, revocation, and blast-radius isolation —
one per partner.

## How federation works (mechanics)

- Each domain runs its **own CA + own attestation authority** (SPIRE). They start trusting nothing
  of each other.
- Federation = exchange of **public trust bundles only** (roots/JWKS): a one-time out-of-band
  bootstrap, then auto-refresh via the SPIFFE Federation API bundle endpoint (`https_spiffe`).
  Private keys never cross — this is what makes it sovereignty-clean and scalable.
- A verifier in A validates B's leaf by chaining to B's root, and reads the **trust-domain prefix**
  of the SPIFFE ID (`spiffe://B/...`) to apply per-domain policy.
- Federation is **pairwise and NON-transitive** (A↔B and B↔C does *not* make A trust C — this
  prevents trust-laundering) and may be **asymmetric** (inbound-only vs mutual).

## Rotation — two clocks, make-before-break

- **Leaf/SVID rotation** (short TTL, auto): invisible across the boundary — same CA signs the new
  leaf, the peer already trusts the CA.
- **CA/bundle rotation** (long TTL, `ca_ttl=2160h`): the one federation must survive. The peer's
  trust bundle must carry the new root *before* the old retires (**bundle overlap**), and the
  Federation API / a refresh loop must update peers within the overlap window. This is the F-202
  lesson (F-202) applied
  across the boundary: refresh cadence must be shorter than the overlap, which is why a partitioned
  (denied-edge) peer is the hard case. **Make-before-break, overlap sized to worst-case partition.**

## Assurance — authorize on it, not just on identity

A cryptographically-valid identity is not automatically trustworthy. Assurance derives from:
**attestation strength** (join-token bearer < hardware/TPM/x509pop), **key custody** (HSM/enclave
vs on-disk), **CA governance** (offline ceremony vs online automated), and **freshness** (short
TTL = smaller leak window). SPIFFE carries no assurance level natively — it is encoded (per-domain
metadata or an LOA claim), mirroring NIST 800-63 AAL/IAL. **High-impact actions require a minimum
assurance level.** *(Not yet formalized — the largest open piece; see Open work.)*

## Validation (every connection — zero trust, every hop)

Chain-to-trusted-root → not expired → extract SPIFFE ID → is this domain federated + allowed? →
does its policy permit *this* action/resource? Fail-closed, denial observable (the F-177
expected-vs-actual lesson). No CRL/OCSP check — validation is positive (chain + expiry + ACL);
revocation is handled out-of-band.

## Revocation — layered, mesh-first (no CRL/OCSP by design)

1. **Network admission** — drop the peer → dark-by-default makes the cert inert. **The dominant,
   instant cut, and the only one of these four that is immediate.** For an intra-mesh peer that is
   dropping the NetBird peer; for a cross-sovereign peering there is no NetBird peer, and the
   equivalent is removing the far side from the tunnel on your own edge host
   (`cross-sovereign-federation.md` §7).
2. **Authorization** — remove the ACL grant → the cert still handshakes, authorized for nothing.
   **Not instant: Zenoh's ACL is static.** The hot-reload function exists and is tested but is
   unreachable behind two one-line filters (`dispatcher/tables.rs:525-547`, `api/config.rs:270-278`,
   `adminspace.rs:242-244`) and no SIGHUP handler exists. So this costs a config re-render
   and a router restart. Wiring that channel up is the cheapest of our upstream asks
   (obstruction register, O9).
3. **Federation kill-switch** — `spire-server bundle delete` the partner's trust domain.
   **This alone does NOT stop the router trusting them, and an earlier version of this document
   implied it did.** `spire-trust-sync` appends into the router's trust file and never prunes
   (`spire-trust-sync.sh.j2`), and the deploy task appends too (`goat-zenoh/tasks/main.yml:167-196`,
   which does rewrite from pristine first). So the real sequence is: `bundle delete` **first** — or
   the 15-minute timer re-adds the root — then drop the CA from config, re-run the role so the
   trust file is rewritten, then restart. It is a real primitive, and it is several steps and a
   restart, not a switch.
4. **Short TTLs** — residual-window backstop; a missed cut self-heals as leaves expire. Note the
   fabric's own peering leaves are minted at **365 days** today
   (`ops/goat-zenoh-portal/internal/certmint/certmint.go:161`), so this backstop is currently
   theoretical for them; shortening that is P4 of the
   federation plan.

**There is no CRL and no OCSP**, by design and by implementation: `WebPkiClientVerifier` is built
without `.with_crls()` and `zenoh-link-tls` carries no revocation code at all. Single-leaf
revocation does not exist. Cutting admission is the control that actually works; everything else
is cleanup.

## Graduated trust — a spectrum on orthogonal axes

Never one dial. Graduate independently along: **namespace scope** (narrow → wide), **quarantine vs
canonical** (inbound from a fresh/third-party domain lands origin-attributed + non-canonical until
promoted — [ADR 1020](../adr/1020-zenoh-federated-peering-trust-model.md)), **direction**
(inbound-only vs mutual), **classification/releasability** (release-sets,
[ADR 1027](../adr/1027-partner-release-set-governance.md)), **assurance gate**, **rate/QoS**
(throughput envelope, [ADR 1045](../adr/1045-qos-throughput-envelopes.md)), and **probation**
(short bundle TTLs + tight scope at first, widening with confidence).

## Different trust per partner — the trust domain is the differentiator

One trust domain per partner → independently grantable/scopable/revocable. **Per-domain policy**
keyed on the SPIFFE-ID prefix (`spiffe://partner-a/* → write release/coalition-x/**`). Asymmetric +
bilateral per pair (non-transitive, so coalitions don't bleed). Independent lifecycle — one
partner's compromise is contained to its domain. Maps onto the identity-grammar **affiliation**
dimension (ADR 1043).

## Implementation — the `spire` role (Phase C, recognition layer only)

Phase C automates **layer 2 (recognition)** so a declared partner is trusted by the production
Zenoh router without the hand-applied trust edit + restart the first live-fire required. It does
**not** touch authorization (the ACL/namespace stays the separate per-partner authz layer).

Role surface (`ops/ansible/roles/spire/`), default **off** (no-op until a partner is declared):

```yaml
spire_federation_enabled: false          # gates the whole federation block
spire_federates_with: []                 # one entry per partner trust domain:
  # - trust_domain: partner-a.pod
  #   bundle_endpoint_url: https://<partner-a bundle endpoint>
  #   endpoint_spiffe_id: spiffe://partner-a.pod/spire/server
  #   bundle_b64: "<base64 of the partner's public bundle>"   # one-time bootstrap (optional)
spire_federation_bundle_endpoint_port: 8443
```

Three mechanisms:
1. **`server.conf` federation block** — renders `federation { bundle_endpoint {…}
   federates_with "<td>" {…} }` from `spire_federates_with`, configuring SPIRE to serve our public
   bundle (port 8443) and to pull each partner's for **auto-refresh** — the durable rotation path.
   (Auto-refresh additionally needs the bundle-endpoint port reachable between domains, i.e. 8443
   published in the compose overlay + a serving profile; verify at deploy. The end-to-end path
   exercised so far is the bootstrap + trust-sync below, which does not depend on it.)
2. **Bootstrap** — a one-time `spire-server bundle set` per partner that provides `bundle_b64`
   (the out-of-band first exchange `https_spiffe` needs before it can self-refresh). For a
   cross-sovereign peering, goat-peering-spire's phase 13 sets it only when the server holds no
   bundle for the partner, or the exchanged file's `spiffe_sequence` is newer, so a rerun never
   replaces a bundle the refresh already advanced (`roles/goat-peering-spire/tasks/bundle-import.yml`).
3. **Edge trust + rotation survival** — `spire-trust-sync` writes, per peering declared in
   `spire_trust_sync_federated_edges`, **own (`bundle show`) ⊕ that one partner
   (`bundle list -id <trust domain>`)** into that peering's EDGE router trust file, every tick;
   the existing per-cert idempotent append + restart-on-change then re-converges when the partner
   CA rotates. The CORE router trust file receives the own bundle alone and never a federated one.
   This is the single change that buys federation *and* rotation-survival, reusing the F-202
   machinery.

   **One partner per edge file, not all of them.** The first implementation folded
   `bundle list` — every federated authority — into every edge trust file. With one partner that
   is indistinguishable from correct; with two, the edge facing B also trusts A's authority, and
   because the ACL matches a Common Name and cannot see the issuer, A could mint B's agreed
   peering CN and be admitted at B's edge. Per-peering tunnels and firewall rules still stand in
   front of that, so it was defence in depth that had failed rather than an open door — and it was
   latent, since `spire_federation_enabled` is false everywhere. Guarded by
   `ops/ci/check-per-partner-trust-isolation.sh` and
   `ops/ansible/roles/spire/tests/render-check.py`.

4. **Retirement — the direction append could not go.** See below; it is the mechanism that makes
   *ending* a peering take effect on a host rather than only in `group_vars`.

## Retiring a peering

Until this was built, **nothing removed a certificate authority from a trust file.** `spire-trust-sync`'s
`append_certs` adds what is missing and never removes what is present — deliberately, because that
is what makes a 15-minute timer safe against a live trust file: a transient failure to read a bundle
can never empty one. The cost was that a retired peering's partner authority stayed trusted on that
edge indefinitely, and a host that had run the pre-isolation all-partners fold kept every partner's
authority with no path back. Per F-424 the Zenoh ACL matches a Common Name and cannot see the issuer,
so a stale authority on an edge is a live ability to mint that edge's names.

**Retirement is a declared change, not a deletion.** Move the peering's entry from
`spire_trust_sync_federated_edges` to `spire_trust_sync_federated_edges_retired`, and remove the
trust domain from `spire_federates_with`. *Deleting* the entry instead is the wrong edit and the
natural one: it stops the authority being **added** and leaves the copy already in the file, because
nothing visits a file no declaration names. `ops/ci/check-federated-trust-retirement.sh` refuses that
edit by comparing the branch against its merge base.

**What converges, and when.** On the next trust-sync tick (≤15 min, or at boot):

| Declared as | Trust file converges on | On a failed partner-bundle fetch |
|---|---|---|
| active edge | our own bundle ⊕ that one partner's | append-only — nothing is removed |
| retired edge | our own bundle **alone** | n/a — no partner fetch is involved |
| core router | our own bundle, appended, never reconciled | append-only |

Reconciliation writes the file **from** the declared set, so anything else in it — a retired
partner, or a foreign CA an older version of this script appended — is removed and the edge router
restarted so it re-reads the file. It runs only when the whole declared set is in hand: for an active
edge that means the partner's bundle was actually fetched this tick, and a fetch that fails falls
back to `append_certs` instead. **A transient failure can never reduce trust.** A retired edge's
declared set is our own bundle alone, which the script has already proven non-empty, so retirement
converges on every tick rather than only when a fetch happens to succeed.

A file named in either list is **wholly owned** by trust-sync: never point one at a trust file that
also carries a hand-placed or separately-managed CA. The core router trust file is deliberately
never reconciled for exactly that reason.

**How an operator confirms it took effect.** Before the deploy, on the host:

```bash
/usr/local/sbin/spire-trust-sync.sh --dry-run   # names every cert it would REMOVE, and from where
```

After it, weakest evidence to strongest:

```bash
journalctl -u spire-trust-sync --since -1h | grep -E 'REMOVE trust|reconciled'   # it said it did
docker inspect -f '{{.State.StartedAt}}' <edge router container>                 # it re-read the file
openssl crl2pkcs7 -nocrl -certfile <edge trust file> \
  | openssl pkcs7 -print_certs -noout                                            # the retired subject is GONE
```

The trust file itself is the authority: a partner subject still present there is still trusted,
whatever the declaration says. The `spire-server bundle delete` the role runs for a retired domain is
defence in depth behind that — it defaults to `mode=restrict` and **refuses** while any registration
entry still federates with the domain, which the deploy reports rather than treats as a failure.

**Escape hatch, and its cost.** `spire_trust_sync_prune_enabled: false` restores the historical
append-only behaviour. With a retirement declared it means the partner authority stays on that edge;
the deploy warns and the script logs it every tick, naming the edge.

Acceptance test: declare a partner in `spire_federates_with`, deploy, and re-run the
`hello-galaxy-spire-spiffe-patterns` pod-side over NetBird — it succeeds with **no** hand edits to
the router.

> **Caution before declaring a partner that is not ours.** Mechanism 3 writes a foreign root only
> to an EDGE router trust file, and only that partner's — the CORE router trust file receives our
> own bundle alone. Both halves of that are the F-424 path being closed, and both are guarded
> (`ops/ci/check-spire-federated-trust-files.sh` for core/edge,
> `ops/ci/check-per-partner-trust-isolation.sh` for edge/edge,
> `ops/ci/check-federated-trust-retirement.sh` for the removal direction). Phase C is safe for **our own**
> issuers (its live proof and its acceptance test both are). For a sovereign peer, the peering is
> edge-terminated and the foreign root stays on the edge component
> ([ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md) decision 6).
> `spire_federation_enabled` defaults to false, so nothing is exposed until a partner is declared.

## Open work (the parts that are decisions, not mechanics)

- **Assurance-level scheme** (LOA) + assurance-conditioned authorization — not designed.
- **Graduated-trust promotion** workflow (quarantine → canonical) — designed in ADR 1020, not built.
- **Per-partner authorization as declared config** (the ACL/namespace/release-set half) — the layer
  Phase C deliberately leaves separate. **Answered in design** by
  [ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md), which moves it to the
  peering gateway — the layer that can hold per-issuer policy at all — and gives it a group_vars
  surface. Not built.
- **Cross-sovereign admission + data plane** — two environments with no shared CA and no shared
  management plane: `cross-sovereign-federation.md`. Designed,
  not built; the live proof is blocked on standing up a genuinely foreign second environment.
