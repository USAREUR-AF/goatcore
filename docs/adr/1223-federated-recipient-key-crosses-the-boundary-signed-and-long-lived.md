# ADR 1223 — A federated partner's recipient key crosses the boundary as a signed, long-lived key the partner publishes

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-08 |
| Deciders | operator |
| Repo scope | goat core (grant-crypto, goat-zenoh, federation) |
| Amends | [ADR 1169](1169-partner-audience-membership-via-spire-federation.md) (decision 2: where a federated member's recipient key comes from) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1094](1094-grant-keyed-payload-encryption.md) · [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) · [ADR 1208](1208-sovereign-environments-release-namespaces-and-grant-encryption.md) · [`WIRE-FORMAT.md`](../../ops/grant-crypto/WIRE-FORMAT.md) · [`sovereign-federation-runbook.md`](../operations/sovereign-federation-runbook.md) Step 6 |

## TL;DR

When one nation seals data for another, the sealing side needs the reader's public key. Today
nothing carries that key across the boundary, and the only key on offer, the reader's SPIRE
certificate key, changes every hour. Under this decision, the reading nation creates a
long-lived recipient key in its own environment, keeps the private half there, and publishes
the public half across the edge. The public half is signed with the reader's SPIFFE identity,
so the sealing side can check it against the trust bundle it already holds. No shared service
and no new trust root are involved.

## Context

Grant encryption wraps one symmetric data key per (release set, generation) to each audience
member's P-256 public key, and publishes the signed bundle on `keys/<set>/<generation>`
([`WIRE-FORMAT.md`](../../ops/grant-crypto/WIRE-FORMAT.md), suite `v1-ecdh-p256`). Inside one
environment the portal's recipient-cert store (`GET /api/access/recipient-cert`) serves each
member's current public key to `grant-keyd`, so wrapping needs no hand-staging.

Across a boundary, ADR 1169 lets a federated identity join an audience and takes its key from an
SVID validated against the partner's federated SPIRE trust bundle. Two things stop that from
working between two real environments:

- **Nothing delivers the key.** `grant-keyd` reads a federated member's certificate from
  `<cert_dir>/federated/<domain>/<path>.pem` (`ops/grant-crypto/keyd/src/recipient.rs`),
  which an operator stages by hand on the sealing side.
- **The key does not last.** SVIDs live one hour (`spire_default_svid_ttl`,
  `ops/ansible/roles/spire/defaults/main.yml`) and SPIRE issues a new key pair at each renewal.
  A bundle wrapped to one hour's key can be opened only by whoever kept that hour's private key,
  and `svid.rs` refuses an expired leaf, so a wrap made after the staged leaf expires fails the
  whole bundle.

The two-region test behind ADR 1169 ran both sides in one operator's hands within one hour and
checked that the partner fetched a bundle naming it.

## Decision

**1. A federated member is sealed to a recipient key, not to its SVID key.** The reading
environment generates a P-256 recipient key pair for each member that will read sealed data.
The private key never leaves that environment. Its lifetime is the reading environment's choice.

**2. The public half crosses as a signed recipient-key statement.** The statement carries the
member's SPIFFE ID, the recipient public key, an issue time, an expiry and a sequence number. It
is signed with the member's current SVID key, and the SVID leaf travels with it. The format is
added to `WIRE-FORMAT.md` beside the bundle format, using the same length-prefixed transcript
and ECDSA P-256 signature.

**3. The partner publishes it under its own landing prefix.** The key is
`from/<its code>/keys/recipients/<member path>`. The partner edge already admits only
`from/<its own code>/` ([ADR 1157](1157-cross-sovereign-federation-edge-terminated.md)), so a
partner can publish only its own members' statements. The partner republishes before expiry
and on rotation. The receiving core stores the prefix for at least the longest statement
lifetime.

**4. `grant-keyd` gains a federated recipient source.** It reads statements from
`from/*/keys/recipients/**` and checks four things. The SVID chains to the partner's federated
trust bundle at the statement's issue time, the same as-issued check `svid.rs` already applies to
its own stored bundles. The SPIFFE ID in the statement matches the SVID and the audience entry
exactly. The statement has not expired. Its sequence number is not lower than one already
accepted. It then wraps to that recipient key. A member with no valid statement fails the
bundle, which is the existing fail-closed rule.

**5. Rotation is the partner's act.** Publishing a statement with a higher sequence number
changes the key that the next generation is wrapped to. The partner keeps older private keys for
as long as it wants to open retained generations. Withdrawal is unchanged: a new generation that
excludes the member, plus withdrawing its `keys/<set>/**` read (ADR 1169).

**6. Governance stays with the sealing side.** A statement makes a key available. It does not
make anyone a member. Audience membership, the admitted trust domain and the edge read rule are
still set only by the sealing environment's operators.

**7. The mechanism is symmetric.** Each environment runs it in both directions with its own key
service. No service is shared between sovereigns.

## Plan

Each step is separate work and lands in order. Status: designed, not built.

| # | Work | Where |
|---|---|---|
| 1 | Recipient-key statement format and test vectors | `ops/grant-crypto/WIRE-FORMAT.md`, `ops/grant-crypto/envelope/` |
| 2 | Reading side: generate and hold recipient keys, sign statements with the member SVID, publish and republish before expiry | `ops/grant-crypto/keyd/` (publisher mode) |
| 3 | Reading side: the SDK opens with the recipient key, keeping a keyring of earlier keys | `ops/grant-crypto/sdk/`, `goat sub --open` |
| 4 | Sealing side: federated recipient source in `grant-keyd`, with the four checks in decision 4 | `ops/grant-crypto/keyd/src/recipient.rs`, `svid.rs` |
| 5 | Sealing side: core storage for `from/*/keys/recipients/**`, retention at least the longest statement lifetime | `ops/ansible/roles/goat-zenoh/` |
| 6 | Runbook Step 6 rewritten: no hand-staged certificate | `docs/operations/sovereign-federation-runbook.md` |
| 7 | Acceptance between two environments run by different operators: seal and open; partner rotates, the next generation opens with the new key and a retained one with the old; an expired or wrongly signed statement is refused; withdrawal closes the next generation | a recorded run, kept by each environment |

## Alternatives rejected

- **Wrap to the SVID key directly** (ADR 1169 as built): the key changes hourly and must be
  staged by hand.
- **Pin the recipient key out of band in the peering agreement:** fails silently on rotation,
  which ADR 1169 already rejected for the same reason.
- **Trust the partner's portal CA in `grant-keyd`:** adds a second foreign root to exchange and
  refresh, when the SPIFFE bundle already crosses through the peering directory and refreshes
  automatically.
- **A key service shared between sovereigns:** adds a party that both sides must trust, against
  ADR 1208's model that each environment runs its own services.

## Consequences

- Sealed exchange between two real environments becomes possible without either side holding
  the other's private keys or running a joint service.
- A partner that seals data for us runs the same publisher for its own audience members, in its
  own environment.
- Until steps 1 to 7 are done, grant encryption across a boundary is not usable between two
  real environments. Unsealed exchange over the federated edge, the default, is unaffected.
