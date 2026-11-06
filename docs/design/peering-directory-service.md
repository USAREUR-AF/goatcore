# Peering directory service — how two parties find each other before they federate

| Field | Value |
|---|---|
| Status | **Decided 2026-09-05. Not built — no repository exists.** The decisions were recorded as D-4, D-14, D-15 and G-10 in `../project/cross-sovereign-federation-plan.md`; what was missing until now is a design doc, which is what this is. |
| Audience | Internal — author / reviewer / whoever stands the service up |
| Companions | [`federation-package-spec.md`](federation-package-spec.md) (the artifact this directory serves), `cross-sovereign-peering-transport.md` (the eight-step bootstrap), `cross-sovereign-federation.md`, `partner-peering-portal.md` (Block 84 — composes with this, does not compete) |
| Repo scope | **A new repository, not this monorepo.** See §4. |

## 1. What it is, in one paragraph

A directory where a party publishes its own interconnection capability — who it is, what
transports it can speak, what limits it will accept — so that two organisations can check
compatibility *before* they build anything. It is modelled on PeeringDB, which internet
network operators use to look each other up before turning up a BGP session, and it steals
exactly one property from it.

## 2. The invariant: advisory, never authoritative

**Being listed in the directory grants access to nothing.** The directory's entire job is to
stop two competent parties from misconfiguring each other. It is not a trust root, not an
authorisation source, and not a party to any agreement.

This is the property that makes the whole idea safe to build, and it is the property that will
be under constant pressure to erode — every future request to "just have the directory check
X" is a request to make it load-bearing. It must be refused.

The reason is that recognition and authorisation are separate concerns throughout this design
(see [`federated-trust-model.md`](federated-trust-model.md)). A directory that grants anything
would collapse that separation at exactly the point where two sovereigns meet.

## 3. What actually establishes trust — and it is not this

The mechanism reachable from both sides is **the specification plus the same versioned tool**:
each side computes the composed agreement independently and compares one digest. No server, no
third party, no infrastructure. That is what [`federation-package-spec.md`](federation-package-spec.md)
and `goat federate` already implement, and it works with a partner who is entirely offline.

The directory sits *above* that as a convenience. Adding it is not a security decision, which
is precisely why it can be added later without reopening anything:

| Rendezvous | Precedent | When |
|---|---|---|
| Offline exchange, digest compared by voice | SSH host keys, PGP signing parties | **The default.** Zero infrastructure; works with an air-gapped partner |
| A well-known endpoint serving your own signed package | OIDC discovery, SAML metadata, Matrix `.well-known` | Convenience, once a partner can reach one. Never the thing establishing trust |
| A neutral directory | PeeringDB, eduGAIN, InCommon | This service |

A party must always be able to federate without touching the directory at all. If that ever
stops being true, the directory has become authoritative and the invariant in §2 is broken.

## 4. Why it cannot live in this monorepo

**Self-hostability is a hard requirement, and it forces the split.** A coalition body cannot be
handed a directory that lives inside our monorepo, and the intent is to hand it over. The
codebase is also expected to be open-sourced.

So the work splits in two:

- **Main peering work — stays here.** `docs/design/`, `ops/federation/`, `crates/goat-cli/`.
- **The directory service — its own repository**, including whatever cloud infrastructure it
  needs, so that another organisation can stand up their own instance from it.

Name: **`saltlick`**, settled 2026-09-06 and carried through the repository
(`saltlick`), the container package (`ghcr.io/usareur-af/saltlick`), the Ansible role
and the mesh hostname (`saltlick.<site>.<zone>`). Mountain goats travel real distances to mineral
licks and congregate there — a known fixed location that **nobody owns, and being there grants
you nothing**, which is the invariant restated as a name. `goat-col` was the alternative (a col
is the pass between two peaks) and is rejected: it names the link rather than the meeting place,
and the link is the thing this service deliberately does not create.

## 5. Governance — who runs it

We run the first instance, and hand it over when **a second coalition member wants to run one**
(D-15) — the trigger exists in that form because until someone asks, there is nobody to hand it
to. **Each coalition or alliance runs their own instance**; there is no single global directory
and no attempt to build one. This follows from §2 — a directory that grants nothing does not
need to be universal, and one that tried to be universal would attract exactly the pressure to
become authoritative.

## 5.1 It is mesh-internal, and the consequence is accepted in writing

**D-14: the directory is mesh-internal.** A partner therefore cannot reach it, and the plan
records the consequence rather than working around it: *"A partner cannot reach a mesh-internal
service and offline exchange proves too slow. The design tolerates this — packages are
self-verifying."* The trigger to revisit is **the first partner who cannot fetch**.

This is only survivable because of §3. A package carries its own signature, so a party can hand
it over by any means at all and the receiver's check is unchanged. If the directory were the
thing establishing trust, being unreachable would be a failure; because it is advisory, being
unreachable is an inconvenience with a documented workaround.

It also constrains the build: nothing may assume internet reachability, public DNS, or
anonymous clients, and no code path may make a missing directory harder to work around than
"exchange the package another way."

## 6. What gets published

The directory carries **only the always-required blocks of a federation package** — and the
spec is explicit that there is no tier field and no tier ladder: *presence is the declaration*
([`federation-package-spec.md`](federation-package-spec.md) §3). A package carrying
`transport_offer` is willing to connect; one carrying `data_offer` is willing to exchange.

So a party publishing to the directory publishes:

- `spec`, `sovereign_id`, `issued`, `expires`, `signature`
- `identity` — trust domain, CA fingerprint
- `capabilities` — `transports`, `fabric_fips_image`, `pqc_capable`, `fabric_version`

and stops there. **`transport_offer`, `data_offer` and `cut` never enter the directory.** Those
move directly between the two parties. A directory holding reachability, release sets or
accepted limits would be publishing sovereign operational detail to everyone who can read it.

**A forwarder's address never enters the directory**, for the same reason: it is
`transport_offer.endpoint`. What two parties do need to learn before building is whether either
of them can accept inbound traffic, because when neither can, one runs a peering forwarder
([`peering-forwarder.md`](peering-forwarder.md), ADR 1218) before its package can be issued.
**Designed, not built:** a capability `inbound: "direct" | "forwarder" | "none"`, published
with the other capabilities, which states that fact without saying where the address is.

This is not a special directory-only shape. It is an ordinary, complete package from a party
that is not, in the directory, offering to connect or exchange — which is exactly what a
capability listing should be.

## 7. Relationship to the Block 84 peering portal

They compose; neither replaces the other.

| | Concern |
|---|---|
| This directory | Two parties discover each other and check compatibility before building |
| `goat federate` | Both sides prove they hold the *identical* agreement — independently computed, one digest compared |
| Block 84 portal | Where **our** side approves and applies an agreement, mints the capability, emits the ACL, and records the audit trail |

The portal is a one-sided approval workflow and has no concept of the partner having produced a
verifiable artifact. That is what the package provides, and why the two fit together rather
than overlapping.

## 8. Open questions

- **One instance per environment, or one shared?** Asked and not settled. The question is
  whether dev, gateway and prod each get their own instance.
- **Deployment.** The decision was to build the repository *with* its cloud infrastructure and
  actually deploy it, rather than leave it as a specification.

## 9. Status

| Piece | State |
|---|---|
| The decision, and its invariant | **Decided** — 2026-09-05, recorded here 2026-09-06 |
| Repository | **Not created** |
| Service | **Not built, not deployed** |
| The trust mechanism it sits above — package spec, `goat federate`, conformance vectors | **Built and merged** — see [`federation-package-spec.md`](federation-package-spec.md) |

Federation works today without this service. That is by design, and it is the reason its
absence has not blocked anything — but the decision to build it stands, and the absence of a
written record is what caused it to be lost once already.
