# ADR 1157 — Cross-sovereign federation: SPIRE-federated trust, one edge router per partner, identical from both sides

- **Status:** **Proposed.** Supersedes the 2026-09-02 draft of this ADR in full. Nothing in a live goatnet changes on merge.
- **Date:** 2026-09-02
- **Deciders:** operator
- **Evidence:** F-424 · `docs/project/cross-sovereign-federation-implementation-plan.md` · `docs/rfc/annex-d-pki-obstruction-register.md`
- **Amends:** [ADR 1020](1020-zenoh-federated-peering-trust-model.md) — corrects its founding premise (origin identity *does* cross a hop; it is simply unauthenticated) and moves its re-publishing gateway out of the cross-sovereign decision entirely.
- **Related:** ADR 0412 · [ADR 0844](0844-zenoh-deployment-topology-and-acl-v1.md) · [ADR 1092](1092-moon-pod-peer-classes.md) · [ADR 1106](1106-grant-crypto-sdk-and-endpoint-forward-secrecy.md) · [ADR 1129](1129-warrant-scoped-expiring-network-authorizations.md) · [ADR 1138](1138-pre-cut-partner-slot-blocks.md) · `docs/design/cross-sovereign-federation.md`

## TL;DR (for a layperson)

Two countries each run their own copy of this system, each with its own certificate authority
and its own private network, and they want to share data without either joining the other's
network or handing over control of who gets in.

Each side puts one small extra data router at its edge, dedicated to that one partner. It trusts
that partner's certificate authority and nothing else, it accepts connections rather than making
them, and it passes messages inward without rewriting them. The two edge routers are joined by a
single encrypted tunnel belonging to neither country's internal network. Ending the relationship
is one command on your own machine that needs no cooperation from the other side.

The tempting sophistication — encoding *how much* a partner is trusted inside the certificates
themselves — is the part we are not doing. That mechanism is standardised and twenty-five years
old, and it cannot work on this software: the field that would carry it is discarded before any
permission is decided, and the standards body now advises against issuing it at all.

What we do instead is simpler and already built. Certificates still decide permissions — they
just do it through the **name** on them, which the issuing authority controls. That only works
while one name can come from only one authority, so each partner gets a dedicated router that
trusts their authority and nobody else's.

## Context

Each nation runs its **own** sovereign environment. A nation's nodes may sit at locations in
another nation's territory and remain in their own nation's environment; they never become part of
the host nation's environment, and the host nation's nodes never join theirs. Nations share by
federating their environments, and that process must exist and be proven before two sovereigns
exchange data.

This decision was reached by a three-team, 74-agent investigation across ten lines of pursuit,
then a second round settling twelve contested claims at source. Findings below are cited to the
vendored crate or to the repo; the implementation plan
carries the sequencing and the obstruction register
carries what we owe upstream.

**A prior draft of this ADR was wrong in three measured ways** and is superseded, not amended:
it claimed Tier 2 had no rate chokepoint (`downsampling` and `low_pass_filter` are stock
router-mode interceptors); it claimed attribution would come from a sovereign-id namespace prefix
(`config.namespace` applies only at `Runtime::new_primitives`, never to forwarded traffic); and it
justified a separate process by "`transport.link.tls` is global" (per-endpoint trust override
works — the conclusion survives for a different reason, below).

## Decision

### 1. Each side runs the identical four-step process against the other.

Nothing about the arrangement is asymmetric.

| # | Step | Mechanism | State |
|---|---|---|---|
| 1 | **Reachability** — one dedicated tunnel between the two edge hosts, on neither NetBird management plane; FIPS is carried by the fabric session inside it | WireGuard, or IPsec where the partner requires L3 | yes |
| 2 | **Trust material** — each side federates the other's SPIRE trust domain, refreshed automatically | `spire_federates_with` (`server.conf.j2:26-45`) + `spire-trust-sync`, driver phase 13 | **built, measured between two hermetic servers, not run with a real partner** |
| 3 | **Identity** — the partner's peering router presents an X.509 SVID whose Subject CN is agreed between us | SPIRE `dns_name` → Subject CN | built, live on a production goatnet |
| 4 | **Permission** — one dedicated `zenohd` per partner, trusting only that partner's bundle | stock zenoh, second config profile | config only |

**No new software is written.** Steps 2 and 3 are the moon-pod federation process already in the
repo; step 4 is a second profile for a router we already run.

### 2. SPIRE carries both the trust material and an enforceable identity.

An earlier draft concluded SPIRE could not drive fabric authorization, because SPIFFE identity
lives in a certificate's URI SAN and Zenoh's ACL cannot read it. **That was wrong, and the repo
disproves it.** `roles/spire/defaults/main.yml:99-100`: *"The CRITICAL field is `dns_name` — SPIRE
sets the X509-SVID Subject CN to the FIRST dns_name, which is how the Zenoh ACL matches it."*
A production goatnet runs this today — `fabric-health` takes its Zenoh identity from SPIRE and the router
authorizes it as `cert_common_names: ["fabric-health-trial-peer"]` (`config.json5.j2:1205`).

So certificates **do** decide authorization here. The ACL matches a name; the issuing CA controls
that name. What the ACL cannot do is ask *which* CA issued it — and that only matters once a
single trust store holds two CAs, because then either can mint the same name. Hence decision 3.

Federation moves **public bundles only**; no private key leaves a domain. `spire-trust-sync`
re-pulls each tick, so a partner rotating their CA does not break the link — which is the half a
hand-managed PEM file gets wrong.

### 3. One dedicated `zenohd` per partner, because a name is only meaningful under one CA.

Two ways exist to keep name→issuer unambiguous. **`nameConstraints`** is the standard one and
fails here: constraints on the subject DN, where CN lives, are unimplemented — `directoryName`
subtrees are reject-all (`subject_name/mod.rs:140-158`). **One CA per trust store** needs nothing
from the library and is the design.

Per-endpoint TLS trust override does work (`zenoh-transport-1.9.0/src/unicast/manager.rs:394-407`)
and does not help: `AclConfigSubjects` has no listener, port or issuer axis
(`zenoh-config-1.9.0/src/lib.rs:196-205`), so two listeners still share one flat CN namespace.
**The process is the only available unit of separation.**

The edge is a second, **empty-by-construction** profile — never the core profile with edits —
because the ACL takes the first Allow across matched subjects and breaks
(`access_control.rs:983-1021`), so a narrower subject can never subtract from a broader one.

### 4. The trust level is declared data in the repo, not a certificate extension.

The Federal Bridge CA's *principle* transfers: the level is agreed by people and recorded outside
the certificate. Its *ceremony* — a signed memorandum, an annual audit letter — does not, and
importing it would be wrong for a platform that changes at the speed of a merge. Here the level
lives in a file, changed by pull request, reviewed like any other configuration, and compiled into
router config at deploy.

### 5. Three levels of access, each naming what stops a peer going further.

| | WireGuard | TLS | Zenoh ACL |
|---|---|---|---|
| **Recognised** | — | partner bundle held on the edge host only | none — a CI guard keeps the bundle off any core host |
| **Connected** | tunnel up | the edge trusts their bundle and no other | no ACL subject matches their CN, and the default is deny |
| **Exchanging** | tunnel up | their SVID carries the agreed Subject CN | a pre-cut ACL block for that CN: ingress `put`/`delete` on `from/<sov>/**` only; read and egress only on the declared release prefixes |

A level nothing enforces is decoration and is not in the table. "May send but nothing lands" was
cut: data cannot arrive without landing somewhere, so it is Exchanging under another prefix.

**Mandatory precondition.** `router-peer` today binds `trial-peer-r-all` + `trial-peer-deliver-all`
— read and delivery on `**` (`config.json5.j2:1375`, `:643-650`, `:755-761`). Until those are
deleted, Exchanging is a label and nothing more. It is the same code path a sovereign peering
would use.

**Degrees cost a restart; members must cost nothing.** Zenoh's ACL is static — the code to
hot-reload exists and is tested but is unreachable behind two one-line filters
(`dispatcher/tables.rs:525-547`, `api/config.rs:270-278`, `adminspace.rs:242-244`). So pre-cut
`peer-01..NN` subject blocks, ungated from `connect_endpoints`, in the ADR 1138 shape.

### 6. Provenance across the boundary is asserted, never verified.

`ext_sinfo` (origin ZID) and `ext_respid` **do** cross a forwarding hop verbatim — `route_data`
rewrites only `wire_expr` and `ext_nodeid`. But nothing anywhere compares them to the transport
peer, and they are `None` at all fourteen construction sites. So ADR 1020's premise is wrong in
letter and right in effect: origin crosses, and is worthless.

**The ingress prefix is `from/<sov>/`, naming the PUBLISHER'S OWN sovereign code on both sides**
(renamed from `recv/` on 2026-09-04, before anything was built on it). The ACL rule is then the
same sentence on both fabrics — *a peer may write only under `from/<its own code>/`* — which is
what "the identical four steps" requires; a receiver-centric name cannot express it. It also
applies to us: because no rename happens in transit, what we send a partner must be
published under `from/<ours>/` too. Rationale and rejected alternatives in the design doc §5.6.

**Attribution is the ingress prefix `from/<sov>/**` and the peering CN, and nothing else.** No
surface may render a foreign `src_zid` as verified.

### 7. FIPS is carried by the fabric TLS session, not by the tunnel.

Cross-organisational traffic is FIPS-relevant (`fips-pqc-path.md`
row 3), and WireGuard cannot carry the claim — its primitives are not CMVP-validated and upstream
declined a FIPS suite (§2 of that doc). **It does not follow that the link must be IPsec.**

We already discharge the obligation one layer up, and it is built. The standing claim
(`zenoh-fips-pqc-claim.md`) is that Zenoh TLS 1.3
sessions on the `zenoh-pqc-fips` image perform their cryptography — including the ML-KEM-768 half
of the X25519MLKEM768 hybrid — **inside AWS-LC FIPS 3.1.0, CMVP #5298 / #5314, in approved mode,
fail-closed**. So:

- **Both edge routers run the FIPS fabric image.** The partner-facing session is then
  FIPS-approved-mode at the layer that carries the data. This is a **peering precondition** and
  belongs in the agreement — the claim covers routers running the image, so a partner who does not
  run one yields a one-sided claim, which is a decision to record rather than assume.
- **The tunnel underneath is reachability plus a second independent layer**; it carries no FIPS
  claim of its own. The claim package itself frames the composition: *"an adversary must
  break both."*
- **The x86 IPsec boundary node** (`fips-pqc-path.md` §3, **designed, not built**) remains the
  answer where the partner cannot run our fabric image, where an accreditor wants the managed
  interface at layer 3 as a device, or where the partner's own accreditation mandates IPsec. It is
  an alternative for the outer layer, not a prerequisite.

An earlier draft of this ADR asserted the boundary node as mandatory. That over-corrected: it
made an unbuilt component a prerequisite for a control we already satisfy with a deployed one.

Cutting a peering remains local, immediate and unilateral — drop the partner from the tunnel on
your own edge host. `spire-server bundle delete` must still precede dropping the CA, or the
refresh timer re-adds it.

Network boundary, firewall placement and the full RMF control mapping:
`cross-sovereign-federation.md` §5.

### 8. Refuse lateral transit; permit hierarchical transit; seal anything crossing more than one link.

Bilateral-only is 1,984 two-party accreditation documents at N=32 and contradicts how the
alliance actually shares. Two-level regions give a link-state domain of V=N rather than
N+N(N−1) — but regions are **link-state containment only, never a trust boundary**, because
assignment is on self-asserted ZIDs.

Anything crossing more than one link is **sealed to its destination identities at the sender**
(grant-crypto AEAD, [ADR 1106](1106-grant-crypto-sdk-and-endpoint-forward-secrecy.md), sovereign-id in the AAD).
Unsealed data stays bilateral.

## Alternatives

- **A re-publishing gateway at every boundary** — the prior draft's decision. Refused: it is a
  broker on the federation path against an explicit architectural commitment, it becomes the
  `SourceInfo` origin rather than the real publisher, and its provenance field is stamped from a
  CLI string and written before delivery on a `Drop`-by-default put — so the one component built
  to stop provenance lying can lie. **The provenance objection is the decisive one and no
  placement fixes it:** a re-publisher is the publisher of record for everything it emits, so
  partner data would reach our core attributed to us, at exactly the moment attribution matters
  most.

  **Correction, 2026-09-04.** This entry previously also claimed its *"two real buys, namespace
  rewrite and a rate cap, are stock 1.9.0 config (`downsampling`, `low_pass_filter`)"*. **The rate
  cap half is true; the namespace-rewrite half is false.** `downsampling` caps frequency and
  `low_pass_filter` caps size — neither rewrites a key expression, and the one feature that does
  (`namespace`) attaches only to local application sessions, never to a transit face
  (F-446).
  So this alternative was partly dismissed by pointing at configuration that does not do the
  thing. The rejection stands on provenance alone; the reasoning is corrected rather than the
  decision.

  **Retained as a Tier-3 escape hatch.** A partner who genuinely cannot change the namespace they
  publish on is a real case, and refusing to name it does not make it go away. If that arrives:
  the re-publisher runs on that partner's edge, only the translated keys route to the core, and
  the provenance loss is accepted **explicitly and in writing** for that peering — not
  rediscovered later by someone reading a surface that says the data came from us.
- **A zenoh authorization plugin.** Impossible, not merely costly: `access_control` is a
  `pub(crate)` built-in interceptor behind a private `mod net`, plugins receive only `IRuntime`
  with no interceptor hook, a plugin transport handler's `Err` is discarded
  (`runtime/mod.rs:895`), and the plugin ABI demands an exact rustc **commit hash** match.
- **Deleting foreign-CA exchange entirely** (peer identity = one pinned key, as WireGuard already
  provides). Genuinely attractive and it deletes bundle exchange, rotation overlap and the
  append-only trust store. Rejected only because the TLS listener needs *some* anchor and a
  self-signed peer cert used as that link's sole anchor is the same artifact under another name —
  but the *governance* weight around CA exchange is deleted, which is where the cost was.
- **`zids` or `usernames` as the trust axis.** `zids` are self-asserted and unauthenticated
  (ADR 1041, ADR 0844). `usrpwd` is an O(N²) symmetric secret; acceptable as an additive second
  factor, never as primary.

## Consequences

**Nothing new is built at the boundary.** One extra container per partner, one profile, one
tunnel. A foreign operator reads three config blocks.

**The least-privilege repair gates everything.** No graduated design functions while
`router-peer` holds read and delivery on `**`, and the failure is silent — so it must be
live-fire verified, never verified by reading the template.

**We owe upstream work we do not depend on.** Eleven obstructions are recorded, nine of them
upstream PRs against other people's crates, two publications. Our own deployment needs none of
them; what they buy is that the next adopter does not have to route around the same gaps.

## What this ADR does not decide

- **The cross-boundary traffic profile.** No real feed has crossed any boundary. Cardinality,
  not rate, is the risk, and it is unmeasured.
- **The quarantine retention horizon** — the binding sizing number (~0.5 GB/day/peering).
- **Whether the two-level regions topology works.** Inferred from source, never run.

---

*See `GLOSSARY.md` for terms and acronyms.*
