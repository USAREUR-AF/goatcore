# ADR 1169 — A federated partner may hold a release-set audience slot, validated by the existing SPIRE federation

- **Status:** **Accepted, and decisions 2 and 3 are built** (2026-09-07). Still nothing changes in a live goatnet on merge: the key scope is rendered only when an operator names an audience, and unsealed exchange remains the default. Proven end to end on a two-region peering — `grant-keyd` rotated with a federated member (`spiffe://xaa.fed.lab/member/partner`) and that partner fetched `keys/weather/1` across the edge; with no audience named the same fetch is refused. Three defects in the shipped rule had to be fixed first (F-499).
- **Not yet usable between two real environments.** `grant-keyd` reads a federated member's certificate from a file staged by hand on the sealing side, and the SVID key it wraps to changes hourly. [ADR 1223](1223-federated-recipient-key-crosses-the-boundary-signed-and-long-lived.md) (Proposed) replaces decision 2's key source with a signed, long-lived recipient key the partner publishes.
- **Date:** 2026-09-06
- **Deciders:** operator
- **Evidence:** F-481 — the partner edge grants no read on the key topic, so a federated partner cannot obtain a data key and can neither read sealed data nor seal its own.
- **Related:** [ADR 1094](1094-grant-keyed-payload-encryption.md) · [ADR 1106](1106-grant-crypto-sdk-and-endpoint-forward-secrecy.md) · [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) · F-424 · `grant-keyed-encryption.md` · [`federation-package-spec.md`](../design/federation-package-spec.md) §5.1

## TL;DR (for a layperson)

Inside one organisation's own network, data can be locked so that only a named list of readers
can open it. A partner in another country, reached across a federation link, could not be on
that list — so everything they sent or received had to travel unlocked.

They can be on the list now. **We** decide who is on the list; the partner decides nothing and
cannot add anyone, including themselves. The key their lock is cut to is checked against a
credential we already verify for this partner — the same one that lets them through the door at
all — which is kept current automatically, so their routine housekeeping does not silently lock
them out.

## Context

Payload confidentiality uses one symmetric data key per (release-set, generation). A key service
wraps that data key separately to each member of the audience under that member's public key,
signs the bundle, and publishes it on `keys/<releaseset>/<generation>`. Every member — readers
and the producer alike — fetches the bundle and unwraps only its own slot
(`grant-keyed-encryption.md`).

The producer is deliberately a member of its own release-set, decided 2026-07-29, so it acquires
its data key by the reader's path with no second secret channel. The property that decision
buys is that **"add a producer" and "add a reader" are the same operation** — an audience
membership — which keeps the data owner's control surface single.

A federated partner had no way into that. The partner edge renders exactly two rules and the key
topic appears in neither, so the partner could ship bytes into its landing prefix and receive
release data, and nothing more. Every grant-keyed capability an ally holds inside a goatnet
stopped at the sovereign boundary.

## Decision

**1. A release-set audience may contain an identity belonging to a federated partner.**

**2. The public key its slot is wrapped to is taken from an SVID validated against that
partner's federated SPIRE trust bundle** — the bundle already exchanged out of band at step 3 of
the bootstrap and refreshed automatically thereafter (`spire_federates_with` +
`spire-trust-sync`). It is never taken from an unvalidated certificate the partner simply hands
over.

**3. The partner's read of the key topic is a separate, read-only access rule, scoped to the
release-sets it belongs to.** `put` and `delete` are absent and must remain absent; the scope is
`keys/<releaseset>/**` and never `keys/**`.

## Why the existing federation, and not a separately pinned key

An earlier draft of this ADR required the audience key to be **pinned** — bound by voice as a
third value alongside the two in [`federation-package-spec.md`](../design/federation-package-spec.md)
§5.1. That was wrong, on a false choice between pinning and reading a key from whatever
certificate the partner presents.

The built mechanism is neither. Each side federates the other's SPIRE trust domain: the bundle
crosses out of band once, at the step that already exists for it, and then refreshes
automatically inside the tunnel. An SVID validated against that bundle is cryptographically
checked material, not an unverified assertion.

**Pinning is also actively worse here**, and that is the decisive argument. The design
deliberately made trust bundles self-refreshing so that *"a partner CA rotation is survivable
rather than an outage."* A hand-pinned audience key does not survive rotation: the partner
reissues, the wrap goes to a key nobody holds, and nothing reports it — reintroducing at the
confidentiality layer exactly the silent breakage automatic refresh was built to remove.

## What is retained, and what is relied upon

**Retained — governance, entirely.** We decide audience membership. We configure the exact common
name the edge rule matches, and matching is exact string equality, so an identity minted under
any other name matches no rule and receives nothing. The partner cannot add a member, cannot
widen its own scope, and cannot publish a key bundle — the rule permitting its read omits the
write verbs precisely so it can never publish one wrapping a data key of its own choosing.

**Relied upon — the same thing already relied upon to authenticate the link.** Their SPIRE
decides which workload holds the agreed identity. That is not a new dependency: it is the
identical reliance accepted the moment their trust bundle authenticates the peering. Extending
it from "may connect" to "may open sealed data it is already permitted to receive" adds no new
party to the trust relationship.

## Residual risks, accepted

1. **Their SPIRE decides which workload holds the agreed identity.** If their issuance is
   careless, the entity behind that name may not be the one intended. Bounded by the ACL, which
   matches one exact name we configure, and identical in kind to the reliance already accepted
   for link authentication.
2. **Removal is two acts, and only the second reaches retained history.** Removing a member mints
   a new generation whose key is not wrapped to them, which closes everything sealed from that
   point. It does not close the past: the key store deliberately retains every bundle at audit
   tier "since keys must outlive the ciphertext they cover", and sealed ciphertext sits in router
   storage and object storage for its retention window. A removed party whose **access rule is
   not also withdrawn** can continue to fetch retained generations it never previously pulled.
   For a federated partner that second act is withdrawal of the `keys/<releaseset>/**` scope on
   the edge. Decision 3 is now implemented — `goat federate apply --partner-audience <set>` renders
   it and omitting the flag renders nothing — so the scope exists to withdraw, and withdrawing it is
   re-running apply without that set. The core-side read scope it depends on shipped separately
   (F-482, closed 2026-09-07). Exposure is otherwise bounded
   by data retention, not by anything cryptographic.

## What removal actually does today, and what it does not

Removing a member from an audience mints a new generation whose key is not wrapped to them,
and this is **built, deployed, and fired end-to-end on real portal-minted certs** —
M-090 steps 6–7, 2026-07-30:
revoking the grant re-derived the audience, the next sweep published generation 2 without the
partner, the revoked partner was denied on a gen-2 message, the producer still opened it, and the
partner kept the gen-1 message it already held. A federated partner inherits that path unchanged,
because ADR 1094's audience is the same mechanism whatever plane the member's identity came from.

What that closes is everything sealed from the rotation onward. What it does not close is the
past, and the second act meant to close the past does not exist.

| Act | Effect | State |
|---|---|---|
| Mint a new generation excluding them | Everything sealed from that point is unreadable to them | Built and live-fired |
| Withdraw their read of `keys/<releaseset>/**` | They can fetch no further bundle, including retained generations they had not already pulled | **Built** — `apply --partner-audience` renders the scope per set; withdrawing is re-running without it. Core-side scope closed with F-482 |

F-482 is the reason decision 3 above is a
build instruction and not a description. On the live router exactly one ACL rule names the key
topic and it is the write gate; bundle read is all-or-nothing between blanket-`**` infra subjects
and everyone else, with no per-release-set granularity to scope or to remove. Decision 3 is
therefore the first place that scope gets built, and a first federated partner cannot ship before
it — not because the partner could read something it should not, but because **nothing could stop
it re-reading what it once could.** The key store deliberately retains every bundle at audit tier,
since keys must outlive the ciphertext they cover.

**The consequence for a first partner, stated plainly.** Removal closes the forward direction and
today has no lever on the backward one. Anything already sealed to them, and any bundle they
already hold, stays theirs regardless — that part no rotation ever fixes. Scope a first partner to
a release-set they could hold indefinitely without consequence, and treat decision 3 as a
prerequisite of the first live partner rather than a refinement after it.

## Alternatives rejected

**Pin the audience key as a third out-of-band value.** Rejected: it adds ceremony at the slowest
step of the bootstrap, and it breaks rotation survivability — a pinned key fails silently when
the partner reissues, which is the failure mode this system handles worst.

**Take the key from an unvalidated certificate the partner presents.** Rejected: a credential
that vouches for itself establishes nothing. Validation against the federated bundle is the
whole difference.

**Refuse foreign identities in audiences.** Rejected: every cross-sovereign exchange would be
unsealed in both directions — a partner could neither read sealed data nor seal its own — which
is strictly weaker than an ally inside a goatnet and would have to be recorded as a permanent
property of Pattern C.

**A partner-facing directory of consumers' public keys, so the partner wraps for itself.**
Rejected on the design's own precedent: producers never see consumers' keys, and a dedicated
key-handoff path was already refused in 2026-07-29 for adding an online dependency and a second
secret path. A boundary-side key directory is that rejected shape reappearing, and it splits the
data owner's control surface in two.

## Consequences

- A partner joins an audience the same way any member does, so the data owner's surface stays
  single and the "add a producer is add a reader" property survives federation.
- No new out-of-band value. The trust-bundle exchange that already exists carries this too, and
  a partner CA rotation stays survivable rather than becoming a silent confidentiality outage.
- The edge grows a third rule whose narrowness is load-bearing; a review that merges it into the
  receive rule silently grants write on the key topic.
- Unsealed exchange remains the default. With no key scope configured the rule is not rendered
  at all, which is a safe and deliberate configuration rather than a broken one.

## Open

- Whether any release-set warrants a stricter standard for audience membership than for link
  authentication. If one ever does, pinning becomes a per-release-set option — never the
  default, for the rotation-survivability reason above.

**Not open, recorded because it was raised and answered.** A *scheduled* generation-rotation
cadence was proposed for this ADR and is not needed. Rotation here is event-driven — a removal
mints a new generation — and a periodic rotation would re-wrap the new key **to the same member
keys**, including a compromised one, so it contains nothing a removal does not already contain.
The bound on exposure is data retention plus the two acts of removal above.
