# Peering directory service — how two parties find each other before they federate

| Field | Value |
|---|---|
| Status | **Built.** Service: `USAREUR-AF/saltlick`, image `ghcr.io/usareur-af/saltlick:v0.1.0` (public). Deployed on the internet outside every goatnet (§5.1), from the deploying team's private overlay. The pointer entry of §6 is designed; the driver step of §6.2 is built behind `FED_DIRECTORY_URL`. |
| Audience | Internal — author / reviewer / whoever stands the service up |
| Companions | [`federation-package-spec.md`](federation-package-spec.md) (the artifact this directory serves), `cross-sovereign-peering-transport.md` (the eight-step bootstrap), `cross-sovereign-federation.md`, `partner-peering-portal.md` (Block 84 — composes with this, does not compete) |
| Repo scope | **A new repository, not this monorepo.** See §4. |

## 1. What it is, in one paragraph

A place where a party records where it can be reached and the fingerprint of the package it has
signed, and where a counterparty can look that up without either side operating a service the
other has to trust. Two organisations use it to find each other's package and check
compatibility *before* they build anything. It is modelled on PeeringDB, which internet network
operators use to look each other up before turning up a BGP session, and it steals exactly one
property from it: a public listing that grants nothing.

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

The mechanism reachable from both sides is **the specification**: each side computes the composed
agreement independently, with its own implementation of the same `spec` version checked against
the normative conformance vectors, and the two compare one digest. Neither side needs the other's
tool, release or commit. No server, no
third party, no infrastructure. That is what [`federation-package-spec.md`](federation-package-spec.md)
and `goat federate` already implement, and it works with a partner who is entirely offline.

The directory sits *above* that as a convenience. Adding it is not a security decision, which
is precisely why it can be added later without reopening anything:

| Rendezvous | Precedent | When |
|---|---|---|
| Offline exchange, digest compared by voice | SSH host keys, PGP signing parties | **The default.** Zero infrastructure; works with an air-gapped partner |
| A well-known endpoint serving your own signed package | OIDC discovery, SAML metadata, Matrix `.well-known` | Convenience, once a partner can reach one. Never the thing establishing trust |
| A neutral directory holding pointers | PeeringDB | This service. The first exchange is out of band until a directory is reachable by both sides |

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
(`saltlick`) and the container package (`ghcr.io/usareur-af/saltlick`). Mountain goats travel real distances to mineral
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

## 5.1 It sits outside every goatnet, on the internet

**D-14: the directory is a neutral meeting place outside every party's mesh, reachable by any
party over the internet with TLS.** No party needs to be on another party's network, or on the
directory operator's, to read or write. A directory instance is never a mesh peer of any goatnet.

Being on the internet is safe because of §2 and §3: the directory grants nothing, an entry is
signed by the party it describes, and a reader checks every signature against a key it bound out
of band. What a reader fetches through the directory is checked the same way whatever path it
took.

| | |
|---|---|
| **Read** | `GET /v1/parties` and `GET /v1/parties/{sovereign_id}`, open to anyone. The entry is returned byte for byte as submitted |
| **Write** | `PUT` / `DELETE` of an entry signed by the party, under trust-on-first-use: the first write claims the name, every later write must verify under the key bound by the first. A different key is refused (HTTP 403) |
| **Protected by** | Public TLS; a rate limit per source address, stricter for writes; a 64 KiB request body cap and server timeouts; HTTP 422 for an entry carrying `transport_offer`, `data_offer` or `cut`. There is no account and no sign-in, because nothing the directory serves is trusted |
| **Never holds** | Offers, reachability details, release sets, accepted limits, contacts, private keys, accounts, or any statement that a party is who it claims |

Federation still works with the directory switched off, or never reached. A partner who cannot
reach it exchanges the package by any channel that carries a file, and the receiver's check is
unchanged. No code path may make a missing directory harder to work around than "exchange the
package another way".

## 6. What gets published

An entry is a **pointer, not a package**: where the party's signed package can be fetched, and
the digest of that package, until a date, signed by the party itself.

| Member | Holds |
|---|---|
| `spec` | `saltlick-pointer/v1` |
| `sovereign_id` | The party, 2–16 characters of `a-z0-9-`, as in the package |
| `package_url` | An `https://` address where the party's signed package is served. Either the directory's own `/v1/parties/<id>/package`, or a public host the party runs, outside any mesh |
| `package_sha256` | `sha256:` and the hex digest of the package's canonical bytes |
| `expires` | The last day the pointer is valid. The directory serves an expired entry and flags it; it never refuses one |
| `signature` | Ed25519 over the canonical form without `signature`, by the key that signs the party's package |

No key material, no identities, no namespaces. The directory's write control binds a key to the
name on first use, and serves that key in a response header; that header is a cache of a
binding, never its source.

A counterparty looks the party up, fetches the package from `package_url`, checks that its
sha256 matches `package_sha256`, checks the pointer's and the package's signatures against the
key it bound out of band, and composes. The directory is not an input to that check.

**A party behind NAT needs no host.** The directory keeps two files beside an entry, each
uploaded under the entry's bound key and served byte for byte:

| File | Path | Checked on upload | Checked by the reader |
|---|---|---|---|
| The signed package | `/v1/parties/<id>/package` | `federation-package/v1`, `sovereign_id` is the path's, canonical bytes, signature under the bound key | digest against `package_sha256`; signatures against the key bound out of band |
| The CA certificate | `/v1/parties/<id>/ca` | PEM certificates, each an X.509 CA | SHA-256 of the DER against the package's `identity.ca_fingerprint` |

The driver uploads both, so the package and the CA never travel by mail. The only out-of-band
act is reading the verifying key and the CA fingerprint aloud (§6.1).

**`transport_offer`, `data_offer` and `cut` never enter a directory entry**, which is what the
listing reads. They sit in the package at the address the pointer names, whether the directory
hosts it or the party does, or move directly between the two parties.

**A forwarder's address never enters a directory entry**, for the same reason: it is
`transport_offer.endpoint`. What two parties do need to learn before building is whether either
of them can accept inbound traffic, because when neither can, one runs a peering forwarder
([`peering-forwarder.md`](peering-forwarder.md), ADR 1218) before its package can be issued.
**Designed, not built:** a capability `inbound: "direct" | "forwarder" | "none"`, published
with the other capabilities, which states that fact without saying where the address is.

The package at `package_url` is the half the party would send. That half carries the edge
endpoint and the cut contact, so a public `package_url`, the directory's included, publishes them;
a party that will not publish them serves its recognition-only package (the always-required
blocks) there and sends the full half directly.

saltlick also accepts the earlier entry form, a recognition-only `federation-package/v1`
package, and refuses one that carries an offer.

### 6.1 What a partner is never asked by message

Every fact two parties need before they build is either in the package, derived from the
agreement, or the one check that has to be made by voice. A question list sent to a partner means
one of these was missed.

| Fact | Where it comes from |
|---|---|
| `sovereign_id` | The package |
| Trust domain (the SPIFFE fact) and CA fingerprint | The package, `identity` |
| Transports, and which one the link uses | The package, `capabilities.transports`; the agreement selects by the fixed preference order |
| FIPS fabric image | The package, `capabilities.fabric_fips_image` |
| Post-quantum capability | The package, `capabilities.pqc_capable` |
| Fabric version and spec version | The package, `capabilities.fabric_version` and `spec` |
| Spec version both sides compose under | The package's `spec`; a mismatch is the only version fact that stops composition |
| Tool version | Not required to match. Reserved `capabilities.tool_version` (designed, not built) is a diagnostic hint only |
| Whether an edge accepts inbound traffic | The designed `inbound` capability; until it is built, the listener appears in `transport_offer.listener` |
| Peering names, certificate names, ingress prefix, republish target | **Derived from the agreement**, never asked for and never chosen by hand |

The one act that stays out of band is **reading the verifying key and the CA fingerprint, and
after composition the agreement digest, aloud to the counterparty** — group count first, read
back. The driver prints both in groups of four. The directory replaces none of them.

### 6.2 The step in the peering driver

`ops/scripts/peering-standup.sh` phase 5 and `goatbuilder federate` phase 5 run the directory
step when `FED_DIRECTORY_URL` is set (`ops/federation/standup/directory.py`). Before the phase
stops for the key check, it:

1. publishes our pointer, signed with `FED_SIGNING_KEY`. With `FED_PACKAGE_URL` unset it first
   claims the entry, then uploads our package to `/v1/parties/<us>/package` and names that; set,
   it confirms our own host serves the package byte for byte and names it;
2. uploads our CA certificate to `/v1/parties/<us>/ca`: from `FED_CA_CERT` (the CA phase 8 signs
   the peering leaves with), else the profile's
   `ca-roots.pem`, only the certificate whose SHA-256 is our `identity.ca_fingerprint`;
3. looks up `FED_COUNTERPARTY_ID`, fetches their package, checks digest and signatures, and
   places it where phase 6 reads it when nothing is there yet; fetches their CA, checks it
   against their `identity.ca_fingerprint`, and places it at `$FED_WORK/theirs-ca.pem` when
   nothing is there yet;
4. prints their verifying key and CA fingerprint in groups of four for the voice call, and a
   compatibility report from the two packages, one row per fact in §6.1, each PASS, WARN, FAIL or
   UNKNOWN with its fix.

| A party sets | Own public host | No public host |
|---|---|---|
| `FED_DIRECTORY_URL` | yes | yes |
| `FED_COUNTERPARTY_ID` | yes | yes |
| `FED_SIGNING_KEY` | yes | yes |
| `FED_PACKAGE_URL` | its host's package address | unset |
| `FED_CA_CERT` | our own CA certificate, as phase 8 already reads it; unset, the profile's `ca-roots.pem` | same |

Every directory failure is a warning. Unset, phase 5 is unchanged.

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

- **One instance per environment, or one shared?** One instance serves every goatnet of a
  coalition, because it sits outside all of them; whether a coalition ever wants a second is
  open.
- **The CA certificate and the SPIFFE bundle** still move directly between the two parties. A
  pointer could name them too; not designed.

## 9. Status

| Piece | State |
|---|---|
| The decision, and its invariant | **Decided** — 2026-09-05 |
| Repository | **Public** — `USAREUR-AF/saltlick` |
| Service | **Built** — image `ghcr.io/usareur-af/saltlick:v0.1.0`, multi-arch, pulls anonymously |
| Internet-reachable deployment (§5.1) | **Built** — one instance outside every goatnet, declared in the deploying team's private overlay, never a mesh service on a goatnet host; per-source rate limits in the service designed |
| Pointer entry (§6) | **Designed, not built** in saltlick |
| Driver step (§6.2) | **Built** behind `FED_DIRECTORY_URL` |
| The trust mechanism it sits above — package spec, `goat federate`, conformance vectors | **Built and merged** — see [`federation-package-spec.md`](federation-package-spec.md) |

Federation works without this service. That is by design: the directory is advisory, and a
peering never waits on it.
