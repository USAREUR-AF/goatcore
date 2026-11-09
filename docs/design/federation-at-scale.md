# Federation at scale

How goatnets federate when there are more than two of them.

> ## Unreviewed. Do not build against this yet.
>
> **No platform engineer has reviewed this document, and the operator who commissioned it has not
> read it.** It was produced in a single session from repository reading and public sources. It is
> a proposal to be argued with, not a plan to execute.
>
> **Whoever picks this up reviews it before writing any code** — the review is the first task, not
> a formality on the way to P1. Review the judgement, not only the facts: whether the problem is
> framed correctly, whether the three candidates are the right three, and whether the recommended
> one is actually the right call for this substrate.
>
> The known reasons to distrust it are in [§12](#12-what-a-reviewer-should-attack-first), which
> also lists the specific claims to attack first.

| Field | Value |
|---|---|
| Status | **Research spike and design of record. Nothing here is built.** The bilateral mechanism it builds on is written, implemented and fired once between two purpose-built environments (M-113, 2026-09-05); no goatnet has a peering with anything today. |
| Decision | [ADR 1170](../adr/1170-federation-growth-by-corroborated-party-roster.md) |
| Tracking | `../project/federation-at-scale-plan.md` |
| Companions | `cross-sovereign-federation.md` (the pairwise mechanism) · [`federation-package-spec.md`](federation-package-spec.md) (the artifact) · [`federated-trust-model.md`](federated-trust-model.md) (recognition vs authorization) · [`partner-data-exchange.md`](partner-data-exchange.md) · [`data-access-roles.md`](data-access-roles.md) |
| Constraint | F-424 — the fabric's access layer matches a certificate Subject CN by exact string and cannot see the issuer, so a foreign root in a router **is** an authorization grant. |

---

## 1. The problem is two costs in two currencies, and only one of them is forced

Federating two goatnets costs a negotiation, a signed declaration from each side, a tunnel, a
dedicated router process, a dedicated trust store, and five rendered configuration artifacts.
Doing that for every pair is `N(N-1)/2` peerings: **10 at five parties, 190 at twenty.** The
arithmetic is the visible problem. It is not the whole problem, and treating it as one leads
straight to a hub.

The work behind that arithmetic splits into two currencies that scale for different reasons:

- **Machine cost** — processes, trust stores, tunnels, config files. Genuinely `O(N²)` for a
  party that peers with everyone, and **forced by F-424**: one certificate authority per trust
  store is the only separation the fabric offers, so two partners can never share one.
- **Human cost** — negotiating terms, and confirming two values over a channel authenticated
  independently of the link. This looks `O(N²)` because it happens once per peering today. It
  is not `O(N²)`, and the specification already says so.

[`federation-package-spec.md`](federation-package-spec.md) §5.1, line 604, on the Ed25519
verifying key and the CA fingerprint:

> They are established in the same act, **once per party**, and again only if that party
> changes them.

Nothing in the repository represents "a party whose two values I have confirmed" as a durable
object. Packages are exchanged per peering, so corroboration is redone per peering, so a cost
the specification bounds at `N` is paid `N(N-1)/2` times. **That is the gap this document
closes.**

### 1.1 What one peering actually costs

Every row is a real artifact with a real name. `goat federate apply` — the only writing verb
of the six in `crates/goat-federation/src/verbs/mod.rs:83,97,112,128,141,150` — renders rows 4–7.

| # | Artifact | Where | Currency | Forced to be per-pair? |
|---|---|---|---|---|
| 1 | Confirm the counterparty's Ed25519 key and `identity.ca_fingerprint` over an independently authenticated channel | a call, a meeting, a courier | human | **No** — per party, per §5.1 |
| 2 | Negotiate release sets, rate and size limits, expiry | staffing | human | **No** — profilable, per ADR 1143's shape |
| 3 | Compose the agreement, compare one digest | `goat federate compose` / `diff` | machine, seconds | yes, and it is cheap |
| 4 | Partner trust-level declaration | `ops/federation/partners/<id>.yaml` | machine | yes |
| 5 | Edge group_vars | `ops/ansible/group_vars/EDGE_HOST/federation-<id>.yml` | machine | yes |
| 6 | ACL fragment | `ops/federation/acl/<id>-fragment.json5` | machine | yes |
| 7 | Trust anchor placement | manual step named by `crates/goat-federation/src/render.rs:617` | machine | yes |
| 8 | One `zenohd` container | `goat_zenoh_edge_stack` (`roles/goat-zenoh-edge/defaults/main.yml:46`) | machine | **yes — F-424** |
| 9 | One trust store holding exactly that partner's CA | `goat_zenoh_edge_partner_ca_path` (`:26`) | machine | **yes — F-424** |
| 10 | One SPIRE federation relationship and one edge trust-sync entry | `spire_federates_with`, `spire_trust_sync_federated_edges` (`roles/spire/defaults/main.yml:156,90`) | machine | yes |
| 11 | One transport tunnel | `roles/goat-peering-edge/` (IPsec) or WireGuard | machine | yes |

Rows 8–11 are the ones people mean when they say federation does not scale, and they are the
ones that cannot be removed. Rows 1 and 2 are the ones that actually stop a coalition forming,
because they consume weeks of institutional time rather than megabytes of disk, **and they are
the ones that are not forced.**

---

## 2. Boundary conditions

### 2.1 What the enforcement layer cannot do

The fabric's link layer parses a peer certificate and keeps one string — the first Subject CN —
discarding issuer, SANs, policies and serial. The access layer matches that string by exact
equality and has no issuer axis, no listener axis and no port axis. So no chain property can
reach an authorization decision, at any configuration, and a second root in one trust store
lets that issuer mint any name the subject table recognises. That is
F-424,
demonstrated at the PKI layer by `ops/federation-smoke/assert-pki.sh` (7/7) and at the
handshake by `assert-handshake.sh` (6/6), both 2026-09-02.

Two consequences govern everything below. **The process is the only unit of separation**, which
is why [ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md) puts one forwarding
`zenohd` per sovereign root at the edge. And **a name is only meaningful while exactly one
authority can issue it**, which is why the peering name carries its own issuer
(`fed.<issuer>.<counterparty>.<ordinal>`, [`federation-package-spec.md`](federation-package-spec.md) §3.1).

### 2.2 Shapes that are wrong by construction

Each of these is a plausible answer to many-party federation, and each fails against the
boundary conditions rather than against taste. They are recorded so they are not re-proposed.

| Shape | Why it is wrong here |
|---|---|
| **A coalition CA every member's core router trusts** | The definition of F-424. Any member could mint `goat-zenoh-router` and inherit `router-peer`, which binds read and delivery on `**`. |
| **A shared fabric all parties join** | A broker. The fabric is brokerless by root decision ([`zenoh-brokerless-consequences.md`](zenoh-brokerless-consequences.md)); a shared router is a central QoS, ACL and failure point re-added. |
| **Transitive trust — A↔B and B↔C makes A trust C** | SPIFFE federation is explicitly one-way and non-transitive, and [`federated-trust-model.md`](federated-trust-model.md) names non-transitivity as what prevents trust-laundering. Under F-424, a transitive path is not a weaker grant than a direct one — it is the identical grant, obtained without the grantor's act. |
| **OpenID Federation-style trust chains to a common anchor** | Metadata policy flowing down a chain from a top-level authority is a central trust anchor with extra steps. Noted below for what it does well. |
| **An IXP route-server analogue for the fabric** | A route server is control-plane only and forwards nothing. There is no equivalent split here: the thing that would be centralised is authorization, and authorization is the thing that must not be. |
| **A directory that vouches for members** | [`peering-directory-service.md`](peering-directory-service.md) holds the invariant that being listed grants nothing. A directory that vouches has become the source of the §5.1 binding, and the specification's own words are that a key learned only from a directory is a key learned from whoever runs the directory. |

---

## 3. How other systems solved many-party federation

Each row answers the same four questions. Sources are linked at the end of the section.

| System | Trust root | What scales | What breaks at N parties | What was given up |
|---|---|---|---|---|
| **IXP route servers** (RFC 7947) | None — each participant's own policy | BGP sessions drop from `N-1` per router to 2 | Path hiding: the server selects one best path for all clients, so a client can lose a policy-compliant path it would have had bilaterally | Path diversity, and visibility of the other side's policy |
| **PeeringDB** | None — advisory | Discovery of who is reachable where, before configuring | Nothing, because it is authoritative for nothing | Any guarantee at all; it is a phone book |
| **RPKI** | Five RIR trust anchors | Route-origin validation across the whole routing table | Every unavailable repository in a validation path is a single point of failure for its entire sub-tree; a coerced RIR can mis-issue and relying parties cannot detect it | Local autonomy — recovered only partially, by SLURM local overrides |
| **SPIFFE/SPIRE federation** | Each trust domain's own CA; bundles exchanged directly | Non-transitive one-way relationships; bundle auto-refresh survives CA rotation | Full-mesh bundle replication (5 servers × 5 servers = 25 shared bundles each way), and adding or removing a trust domain needs a config change and restart on every participating deployment | Dynamism — federation topology is deployment-time, not runtime |
| **Matrix** | Each homeserver's own signing key | Rooms replicate over full-mesh federation with per-event signatures and a deterministic state-resolution algorithm | Full-mesh event push cost grows with participating servers; state resolution has to be order-independent to converge at all | Central moderation and any global view; consistency is eventual |
| **eduGAIN / InCommon** | One aggregate signed by the interfederation operator | ~79 countries' federations interoperate through one layer instead of bilateral pairs | The aggregate signature becomes the thing all trust rests on; bilateral attribute-release negotiation had to be replaced by entity categories to stop being `O(N²)` again | A neutral aggregate is a central authority for metadata, and is accepted as one |
| **eIDAS trusted lists** | Commission-signed List of Trusted Lists pointing at member-state-signed lists | Each state signs its own list; the LOTL only points | Two-level hierarchy: the LOTL signature is a single anchor for the whole scheme | The same trade as eduGAIN, made explicit in regulation |
| **OpenID Federation 1.0** | One or more trust anchors | On-demand trust-chain resolution instead of batch metadata files; policy constrained down the chain | The anchor is the authority, and metadata policy is inherited rather than chosen | Receiver autonomy over policy, in exchange for freshness |
| **Sigstore** | A TUF root established by ceremony | Keyless signing for the whole ecosystem; no component failure is systemic — a rogue Fulcio is caught by the CT log, a compromised Rekor by witnesses | The root ceremony and the OIDC issuers are concentrated trust; private deployments need their own root | A single global root, in exchange for detectability |
| **DNSSEC** | One root KSK | RFC 5011 makes rollover automatic for the great majority of resolvers | One root is one coercion target; islands of trust needed DLV, which was withdrawn in 2017 as redundant | Any alternative to the root, deliberately |
| **Tor directory consensus** | Nine hard-coded authorities, majority threshold | A relay is listed only if a majority of votes include it, so no single authority controls the consensus | The authority set itself is hard-coded and changes rarely | Openness of the authority set, in exchange for no single point of control |

**Three findings carry into the design.**

**Every system that scaled did so by centralising *distribution*, and the ones that stayed
safe kept *decision* local.** PeeringDB distributes and decides nothing. RPKI distributes and
also decides, and the consequences are the sub-tree availability failure and the coercion
problem. eduGAIN distributes and, in practice, decides — service providers accept identity
providers because they are in the aggregate — and its participants accept that trade in
writing.

**The bilateral bootstrap is never solved cryptographically.** SPIFFE's `https_spiffe` profile
needs the counterparty's bundle before it can fetch the counterparty's bundle. SSH host keys,
PGP signing parties and Sigstore's root ceremony all resolve the same circularity
institutionally. `federation-package-spec.md` §5.1 states this in the same terms and reaches
the same conclusion: two parties with no prior relationship solve it with a meeting, a courier,
or an exchange witnessed by a body both already recognise.

**Threshold signing is how a directory stops being one owner's directory.** Tor's consensus is
valid on a majority of nine, so no single authority can insert a relay. That is the only
mechanism in the survey that makes a shared list survivable without making its operator an
authority.

Sources: [RFC 7947](https://www.rfc-editor.org/rfc/rfc7947.html) ·
[IXP Manager route servers](https://docs.ixpmanager.org/6.4/features/route-servers/) ·
[Keep Your Friends Close, but Your Routeservers Closer (RPKI at IXPs)](https://arxiv.org/pdf/2303.11772) ·
[Models of Trust for the RPKI, RIPE Labs](https://labs.ripe.net/author/gih/models-of-trust-for-the-rpki/) ·
[RPKI and Trust Anchors, APNIC](https://blog.apnic.net/2020/04/21/rpki-and-trust-anchors/) ·
[SPIFFE Federation specification](https://spiffe.io/docs/latest/spiffe-specs/spiffe_federation/) ·
[SPIRE issue 2130, operational improvements to federation](https://github.com/spiffe/spire/issues/2130) ·
[Matrix Server-Server API](https://spec.matrix.org/unstable/server-server-api/) ·
[eduGAIN Metadata Aggregation Practice Statement](https://wiki.geant.org/display/eduGAIN/Metadata+Aggregation+Practice+Statement) ·
[InCommon interfederation and eduGAIN](https://spaces.at.internet2.edu/x/ywgOCg) ·
[eSignature List of Trusted Lists](https://ec.europa.eu/digital-building-blocks/sites/spaces/DIGITAL/pages/467109149/eSignature+List+of+Trusted+Lists) ·
[OpenID Federation 1.0](https://openid.net/specs/openid-federation-1_0.html) ·
[Sigstore threat model](https://docs.sigstore.dev/about/threat-model/) ·
[RFC 5011](https://www.rfc-editor.org/rfc/rfc5011) ·
[Tor consensus formats](https://spec.torproject.org/dir-spec/consensus-formats.html) ·
[Analyzing inconsistencies in the Tor consensus](https://dl.acm.org/doi/fullHtml/10.1145/3487664.3487793).

### 3.1 The one thing the survey could not establish

**The Cross-Community Information Sharing shape attributed to FMN Spiral 6 is not obtainable
from public sources.** What is public is that FMN develops in numbered spirals, that Spiral 6
is the current increment, and that Spiral 5 and 6 material exists as working-group drafts,
NCIA technical interchange notes and CWIX outcomes rather than as published specifications
([Federated Mission Networking, Wikipedia](https://en.wikipedia.org/wiki/Federated_Mission_Networking) ·
[FMN spiral roadmap beyond Spiral 4](https://corvusintell.com/blog/interoperability/fmn-spiral-roadmap-future/)).
No CCIS specification was reachable. Nothing in this design rests on it. What would settle it
is access to the Spiral 6 specification set through an FMN participant.

---

## 4. Candidate 1 — a roster of corroborated parties, per framework

**A framework publishes a signed list of its members' identities, so the confirmation each
party must make about every other party happens once per party instead of once per pair.**
Peerings stay bilateral and stay `O(N²)` in processes; what stops being `O(N²)` is the phone
call and the staffing cycle.

The roster carries exactly the two values of §5.1 — the Ed25519 verifying key and
`identity.ca_fingerprint` — plus `sovereign_id`, trust domain, bundle endpoint, and the date
and method by which each entry was corroborated. It carries no `transport_offer`, no
`data_offer` and no `cut`, for the same reason the peering directory excludes them: those are
sovereign operational detail.

Three properties keep it from becoming an authority.

**Corroboration is structural, not advisory.** A party may not `apply` a peering with a
counterparty whose two values it has never confirmed over an independently authenticated
channel. The roster is where the confirmed value is *kept and reused*; it is not where the
value is *learned*. This is the specification's own rule — a directory is a cache of the
binding, never its source — turned into a refusal in the tool rather than a sentence in a
document. Concretely: a per-party record under `ops/federation/parties/<sovereign_id>.yaml`
holds the two values and the act that established them, and `goat federate apply` refuses when
the counterparty's package disagrees with that record or no record exists.

**The roster is valid on a threshold of member signatures, not on one operator's.** Tor's
shape. A framework of `n` members accepts a roster carrying at least `k` member signatures, so
the body that hosts the file cannot add a party by itself.

**A local file outranks the roster, always.** The shape RPKI reached for with SLURM (RFC 8416). Each party keeps its own
overrides — refuse this member, pin this fingerprint regardless — and they are applied after
the roster, never before.

### 4.1 Join, leave, rotate, revoke

**Join.** A new party corroborates its two values with `k` existing members over independently
authenticated channels, and those members co-sign a roster revision adding it. Every other
member's next roster sync obtains the new party's fingerprint through a channel it has already
authenticated. A peering with any specific member is then a machine act: exchange packages,
compose, compare the digest, `apply`. **Corroborations to join a framework of 20: `k`, not 19.**

**Leave.** The framework issues a roster revision without the departing party. Each member's
convergence uses machinery that already exists: move the peering's entry from
`spire_trust_sync_federated_edges` to `spire_trust_sync_federated_edges_retired`
(`roles/spire/defaults/main.yml:90,110`), which makes the edge trust file reconcile to our own
bundle alone on the next tick and restarts the edge router. `ops/ci/check-federated-trust-retirement.sh`
already refuses the wrong edit, which is the natural one: *deleting* the entry stops the
authority being re-added and leaves the copy already in the trust file untouched.

**Withdrawal is not instant and the roster does not make it so.** It is a config change and a
router restart per member, because the fabric's ACL is static
(F-156 — SIGHUP
is a no-op for ACL reload, and the hot-reload path exists but is unreachable). The immediate cut remains the one
[`federated-trust-model.md`](federated-trust-model.md) names as the only immediate one:
removing the far side from the tunnel on your own edge host.

**Rotate.** Unchanged and already automatic. A partner CA rotation propagates through
`spire_federates_with` plus `spire-trust-sync`, which is why [ADR 1169](../adr/1169-partner-audience-membership-via-spire-federation.md) refuses a
hand-pinned audience key. A **signing-key** rotation is a roster revision, which is exactly the
case §5.1 reserves — the two values are established again only when a party changes them.

**Revoke.** A compromised party is removed from the roster by threshold signature, and every
member converges as in *Leave*. What that does not do is close retained history, and that
limit is not a property of this candidate — see §10.

### 4.2 Compromise, cost, and abandonment

**When a party is compromised:** it can present its agreed CN at each edge it is peered to and
exercise exactly the ingress and egress that peering granted. It cannot reach a member it never
peered with, because there is no tunnel and no edge process. It cannot mint another member's
name, because each edge trust store holds one CA. Blast radius is the set of peerings it holds,
which is the property `ops/ci/check-per-partner-trust-isolation.sh` exists to preserve.

**Cost per party:** `k` corroborations to join; then one edge container, one trust store, one
tunnel and five files per peering actually built. A party that peers with everyone in a
20-member framework runs 19 edge containers.

**Cost to abandon:** low. Delete the roster and every party still holds its own corroboration
records and its own agreements; federation continues exactly as it does today. Nothing in the
package format changes, so no package or digest in circulation is invalidated.

**Invariants preserved:** no central trust anchor, no transitive path, no broker, recognition
separate from authorization, one CA per trust store, grant-keyed audiences unchanged.
**Traded:** a party that never exercises its local override has, in practice, delegated its
recognition decision to whoever assembles the roster. §9 is about this.

---

## 5. Candidate 2 — fewer parties, by making the CA the unit

**Reduce `N` rather than reduce the cost per pair.** A party is an entity that operates its own
certificate authority — the rule [`federation-package-spec.md`](federation-package-spec.md) §3.1
already states: *a name segment exists if and only if that entity operates its own certificate
authority.* Adopt it deliberately, and thirty organisations sharing five authorities are five
parties and ten peerings, not 435.

The RIRs made the same move at internet scale: five roots, not sixty thousand.

**Join.** An organisation joins a group by enrolling under that group's authority. No peering,
no package, no corroboration — it is an internal enrolment, and the group's existing peerings
cover it immediately.

**Leave.** Revoke the leaver's certificates within the group. There is nothing to withdraw at
any peering, because the leaver never had one.

**Rotate.** The group's CA rotation is one rotation for every peering the group holds.

**Revoke.** Here the candidate's cost lands. Inside a group, F-424's collapse is total by
construction: any member can mint any other member's names, including infrastructure names.
There is no revocation of one member's *authority* short of re-issuing the group's CA, because
the group's CA is what vouches for them all.

**When a party is compromised:** the whole group. A single compromised member is
indistinguishable, at every peering the group holds, from every other member and from the
group's own routers.

**Cost per party:** near zero to add an organisation to an existing group; the full bilateral
cost to add a group. **Cost to abandon:** high and slow — splitting a group means standing up a
new authority and re-issuing every leaf under it, which
`../operations/zagros-ca-separation.md` plans for one
environment and describes as delicate under active load.

**Invariants preserved:** no central anchor across groups, no broker, no transitive path
between groups. **Traded:** sovereignty inside a group, completely and knowingly. This is a
legitimate lever for organisations that already share a risk owner — a national task force, a
single ministry's environments — and it is the wrong answer between sovereigns, which is the
case this design exists for.

---

## 6. Candidate 3 — sealed objects over any path

**Stop making the link the access boundary.** A payload sealed to a release-set audience is
readable only by identities holding a slot in that audience's key bundle, so it is safe on any
path. Under this candidate a party may forward a sealed object to a party it has no peering
with; the recipient opens it only if the producer granted it a slot. The number of edges
becomes the number of paths bytes must physically travel — a connectivity question — rather
than the number of trust relationships.

This is not a transitive trust path. The recipient's ability to open the object comes from a
direct grant by the producer, with no chain and no intermediate vouching, so the forwarder is a
courier that cannot read what it carries. The grant-keyed constraint survives unchanged and is
in fact the whole mechanism: **the producer is a member of its own release-set, so "add a
producer" and "add a reader" remain the same operation.** [ADR 1169](../adr/1169-partner-audience-membership-via-spire-federation.md) already decides
that a federated partner's identity may hold an audience slot, with the key taken from an SVID
validated against that partner's federated SPIRE bundle.

**Join.** Grant the joining party an audience slot. One act by the data owner, no peering, no
tunnel — the shape ADR 1143 already chose for release: the owner publishes, the recipient
accedes, and the tenth recipient costs what the first cost.

**Rotate.** Unchanged; the SPIRE bundle refresh carries CA rotation, and generation rotation
carries key rotation.

**Leave and revoke — and this is where the candidate fails.** Removing a member mints a new
generation whose key is not wrapped to them, which is built and was fired end-to-end on real
portal-minted certificates. That closes everything sealed from
that point forward. It does not close the past, and the act meant to close the past is
withdrawal of a per-release-set read scope on the key topic.

**That scope now exists, and it does not rescue this candidate.** F-482 measured it
absent on 2026-09-06 — one ACL rule named `keys/`, and it was the write gate — and #790 built and
deployed it the next day: per-set member lanes on the router, no `put` or `delete` on the read side.
Removing a member from a release set now removes its key lane with it. Note the shipped scoping is
by release **label**, not per-producer release set, so the lane is the containment unit in
practice.

**The lane is withdrawable only where a lane exists.** Under candidates 1 and 2 it does, and
admission stands behind it: dropping the far side from the tunnel is immediate and is the control
that actually works. **Under candidate 3 there is neither** — the objects have already travelled,
by design, to parties that were never peered, so there is no ACL lane on their path and no tunnel
to drop. Every copy in flight or at rest is beyond recall.

The project has since decided what to do about the backward window rather than close it:
`grant-keyed-encryption.md` §4b, 2026-09-07, **accepts** it, bounded
at seven days by the router's own data TTL with the audit tier out of scope. That decision is
available to candidates 1 and 2 because their exposure is bounded by a retention they control. It
is not available to candidate 3, whose objects sit in stores belonging to parties it never
peered with.

Provenance weakens for the same reason. The `from/<sovereign>/` prefix is trustworthy because
an ACL rule enforces it at the boundary the data crossed. An object forwarded over a hop it did
not itself make carries no such enforcement, and `ext_sinfo` is unverified at every one of its
fourteen construction sites — a partner can assert any origin, including ours.

**When a party is compromised:** everything sealed to any audience it belongs to, for every
generation it ever held, permanently.

**Cost per party:** an audience slot and a validated bundle relationship — `O(N)`, and the
cheapest of the three. **Cost to abandon:** moderate. The sealing and the grant machinery are
already built for the in-goatnet case and would keep working; what would be abandoned is the
forwarding, which is the new part.

**Invariants preserved:** no central anchor, no broker, no transitive *trust*, grant-keyed
semantics intact and load-bearing. **Traded:** admission as a control, and provenance across a
forwarded hop.

---

## 7. Comparison

| | 1 — corroborated roster | 2 — fewer parties | 3 — sealed objects |
|---|---|---|---|
| Attacks | the authenticated channel and the instrument | `N` itself | the number of links |
| Corroborations to join a 20-party framework | `k` (threshold), not 19 | 0 within a group | 1 per audience-granting owner |
| Edge processes for a fully-peered member | 19 | one per *group* peered | only where bytes must flow |
| A new party joining an existing group | roster revision plus per-pair machine steps | internal enrolment | one grant |
| Withdrawing one party | roster revision; retirement path exists; per-member restart | re-issue the group CA | **no lever on retained history** |
| Compromise blast radius | that party's peerings | the whole group | every audience it ever joined, permanently |
| Cost to abandon | low | high | moderate |
| Grant-keyed encryption | unchanged | unchanged | is the mechanism |
| Depends on something not built | the roster, the party record, the `apply` refusal | nothing | a withdrawal lever that does not exist at any layer |

---

## 8. Recommendation

**Adopt candidate 1.** Build a per-party corroboration record and a threshold-signed framework
roster; keep peerings bilateral, keep one edge process and one trust store per partner, and
refuse `apply` against a counterparty that has never been corroborated.

Three reasons, in order of weight.

**It reduces the cost that is actually blocking growth, and only that one.** Rows 8–11 of §1.1
are forced by F-424 and no design removes them. Rows 1 and 2 are weeks of institutional time
per pair, and the specification already bounds row 1 at once per party. Candidate 1 collects a
bound that has been available since the specification was written and never taken.

**It changes no invariant.** No new trust anchor, no chain, no broker, no shared trust store.
The roster is a cache of a binding each party established for itself, and the tool refuses to
treat it as anything else. Recognition and authorization stay two separate acts, which is the
property [`federated-trust-model.md`](federated-trust-model.md) is built on.

**It is cheap to abandon and cheap to be wrong about.** Delete the roster and federation
continues unchanged, because every party still holds its own records and its own agreements.
No package format changes, so nothing in circulation is invalidated — which matters, because
D-1 in `../project/cross-sovereign-federation-plan.md`
records that changing the signature envelope invalidates every package and digest in
circulation.

Candidate 2 stays available as a **per-group** choice for organisations that already share a
risk owner, and is never the default between sovereigns. Candidate 3 is not rejected on its
mechanism, which is sound. It is rejected on withdrawal: the per-release-set key lane shipped in
#790 is the lever every other candidate uses, and candidate 3 is the one shape that puts objects
where no lane and no tunnel sit on their path.

---

## 9. The strongest argument against the recommendation

**A threshold-signed roster is a central trust anchor wearing a quorum costume.**

The argument runs: once every party derives "party X's CA fingerprint" from the roster, the
roster decides who X is. That is RPKI's coercion problem, where a trusted operator induced to
mis-issue cannot be detected by relying parties, and eIDAS's LOTL, where one Commission
signature anchors a whole scheme. The threshold and the local-override file are mitigations
rather than refutations: a party that never exercises its override has delegated its
recognition decision in practice, whatever the document says. And the stakes here are worse
than PeeringDB's, whose worst case is a misconfigured BGP session. This roster's worst case is
a foreign CA admitted to an edge, which under F-424 **is** an authorization grant.

The counter is one design property, and the recommendation is only sound if that property is
built rather than written: **the roster may not be the source of a first-contact binding.**
`goat federate apply` refuses a counterparty with no local corroboration record, so a party
that has never been confirmed over an independently authenticated channel cannot be peered with
no matter what the roster says. The roster then does what PeeringDB does and what eduGAIN's
aggregate does not — it distributes, and it decides nothing.

**That property is a refusal in a tool, and a refusal in a tool can be removed by a pull
request.** The pressure to remove it will be real and will arrive framed as convenience: a
member that cannot get a call scheduled, an exercise with a deadline, a body that offers to
corroborate on everyone's behalf. [`peering-directory-service.md`](peering-directory-service.md) names the same
pressure on the directory's own invariant and says it must be refused. The honest statement of
residual risk is that this design's safety rests on a refusal holding under operational
pressure, and nothing cryptographic enforces it.

The second-strongest argument is narrower and worth recording: **`k`-of-`n` presumes members
who can independently verify what they are signing.** A member co-signing a roster revision
because four others already did adds a signature, not a check. Whether that degenerates is an
operating question no mechanism settles, and it is the reason the first framework should be
small.

---

## 10. What this does not solve

- **Transitive reachability.** A party reaches only parties it has peered with, except for data
  its owner marks as a general release ([ADR 1224](../adr/1224-transit-is-one-hop-unless-the-owner-marks-a-general-release.md), Proposed), which travels to every
  federated environment. Candidate 3 is the only shape that would give data — never trust — a transitive path,
  and it is refused on withdrawal rather than on mechanism.
- **Withdrawal from retained history.** Removing a member closes the forward direction, which is
  built and fired, and #790 now also removes the member's `keys/<set>/**` lane so it cannot follow
  the stream onward (F-482). What remains open is the backward window — bundles a
  removed party could already have pulled — and `grant-keyed-encryption.md` §4b
  accepts it at a seven-day bound rather than closing it. A first framework is sized against that
  bound, not against an assumption that withdrawal reaches the past.
- **Instant withdrawal.** The fabric's ACL is static, so every authorization change is a
  restart. Pre-cut subject blocks ([ADR 1138](../adr/1138-pre-cut-partner-slot-blocks.md))
  bound how often that is needed; they do not remove it.
- **Assurance levels.** [`federated-trust-model.md`](federated-trust-model.md) records the
  assurance scheme as the largest open piece. A roster could carry an assurance claim per
  member; deciding what one means is not this document's work.
- **Who runs the first roster.** Same answer as the peering directory: we run the first one and
  hand it over when a second member wants to run one. Each framework runs its own; there is no
  global roster and no attempt to build one.

---

## 11. Status

| Piece | State |
|---|---|
| The package format, canonical form, signature and digest rules | **Written** — [`federation-package-spec.md`](federation-package-spec.md); the conformance vectors in `ops/federation/vectors/` are normative |
| The six `goat federate` verbs, `apply` the only writer | **Built** — `crates/goat-federation/src/verbs/mod.rs:83,97,112,128,141,150` |
| Per-partner edge role and its four CI isolation guards | **Built, deployed once, inward link failed** — F-474 |
| Peering fired both directions between two purpose-built environments | **Measured** — M-113, 2026-09-05; both environments destroyed after the run |
| Any goatnet peering with anything | **Not built** |
| Per-party corroboration record | **Designed, not built** |
| Framework roster, threshold signing, local override | **Designed, not built** |
| `apply` refusal on an uncorroborated counterparty | **Designed, not built** |
| Per-release-set key-read scope | **Built and deployed** — #790, scoped by release label (F-482, closed) |
| M2.2 rotation and revocation | **Built, deployed and live-fired** — M-090 steps 6–7, 2026-07-30. Its remaining acceptance half was rescoped 2026-09-07, not completed |
| Candidate 3 forwarding | **Not built.** Not recommended — see §6 |

---

## 12. What a reviewer should attack first

This section exists because the document has not been reviewed and should not be treated as
settled. These are the load-bearing claims — if one of them is wrong, the recommendation moves.

**Two errors were already found in this document after it merged**, which is the concrete reason
for the banner rather than a general caution. Both were status claims about other people's
subsystems that a platform engineer would likely have caught on sight: M2.2 was recorded as
designed-not-built when it had been live-fired since 2026-07-30, and the key-read scope was
recorded as absent one day after it was deployed. Both are corrected here. Neither was found by
review; they were found by re-checking during close-out.

| # | Claim | Why it might be wrong |
|---|---|---|
| 1 | The out-of-band cost is per party, not per pair, so `N` corroborations replace `N(N-1)/2` | Rests on one sentence of [`federation-package-spec.md`](federation-package-spec.md) §5.1 read literally. Whether a real coalition would accept a corroboration performed by someone else, on their behalf, at a different time, is an institutional question this document does not test. |
| 2 | The machine cost is irreducible | Derived from F-424 and ADR 1157 rather than re-argued. If there is a way to get per-issuer separation that does not cost a process, rows 8–11 of §1.1 collapse and the whole framing changes. |
| 3 | A threshold-signed roster is not a central trust anchor | §9 states the strongest argument that it is. The counter rests entirely on a refusal in a tool. A reviewer who thinks that refusal will not hold should say so — it is the decision's load-bearing half. |
| 4 | The three candidates are the right three | They attack the channel, `N`, and the number of links. Whether that is the natural decomposition of this problem, or an artifact of how it was approached, is exactly the kind of judgement this document cannot make about itself. |
| 5 | Candidate 3 is declined on withdrawal | Reargued after F-482 closed. The new reason may be weaker than the old one looked, and the section was rewritten rather than re-derived from scratch. |
| 6 | The survey's lessons transfer | Eleven systems, none of them a brokerless identity-matched fabric. The analogies are argued, not demonstrated. |

**One thing the survey could not establish at all** is in §3.1: the Cross-Community Information
Sharing shape attributed to FMN Spiral 6 is not obtainable from public sources. Nothing here rests
on it, but a reviewer with FMN access should check whether it changes the picture.

---

*See `../adr/GLOSSARY.md` for terms and acronyms.*
