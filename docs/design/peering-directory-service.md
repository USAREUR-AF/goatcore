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
and `goatbuilder federate` implement (until the move under ADR 1225, `docs/adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md`, is complete, released `goat federate` runs the same code), and it works with a partner who is entirely offline.

The directory sits *above* that as a convenience. Adding it is not a security decision, which
is precisely why it can be added later without reopening anything:

| Rendezvous | Precedent | When |
|---|---|---|
| Offline exchange, digest compared by voice | SSH host keys, PGP signing parties | **The default.** Zero infrastructure; works with an air-gapped partner |
| A well-known endpoint serving your own signed package | OIDC discovery, SAML metadata, Matrix `.well-known` | Convenience, once a partner can reach one. Never the thing establishing trust |
| A neutral directory holding pointers | PeeringDB | This service. It also hosts each party's package and CA, and the driver's phase 5 exchanges through it |

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
| **Write** | `PUT` / `DELETE` of an entry signed by the party, under trust-on-first-use: the first write claims the name, every later write must verify under the key bound by the first. A different key is refused (HTTP 403) unless the request is signed by the key it replaces: that is a rotation, and the directory then binds the new key. Phase 5's step signs that first write with the seed in `FED_PREVIOUS_SIGNING_KEY`, and without it publishes nothing and prints GB-285 ([federation-key-rotation.md](federation-key-rotation.md), K8) |
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
key read back on the call, and composes. The directory is not an input to that check.

**A party behind NAT needs no host.** The directory keeps up to three files beside an entry,
each uploaded under the entry's bound key and served byte for byte, with the upload's own key
and request signature (`X-Saltlick-Key`, `X-Saltlick-Signature`, `X-Saltlick-Resource`):

| File | Path | Checked on upload | Checked by the reader |
|---|---|---|---|
| The signed package | `/v1/parties/<id>/package` | `federation-package/v1`, `sovereign_id` is the path's, canonical bytes, signature under the bound key | digest against `package_sha256`; signatures against the key read back on the call |
| The CA certificate | `/v1/parties/<id>/ca` | PEM certificates, each an X.509 CA | SHA-256 of the DER against the package's `identity.ca_fingerprint` |
| The SPIFFE trust bundle, when SPIRE federation is on | `/v1/parties/<id>/spiffe-bundle` | a SPIFFE bundle (JWKS: `x509-svid` keys with an `x5c`, `jwt-svid` keys) | the upload's request signature under the key read back on the call |

The driver uploads all three, so none of them travels by mail. The only act outside the
machine is reading the verifying key and the CA fingerprint aloud (§6.1).

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
   places it where phase 6 reads it when nothing is there yet (a package signature that does
   not verify under the key in the keyring places nothing); fetches their CA, checks it
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

Without `FED_DIRECTORY_PER_COUNTERPARTY` the step is the single-package flow as it was before
§6.3: the same files written, the same requests, the same output (a test runs it beside an
unmodified copy of the earlier script and compares all three). The stricter checks of §6.3
(a pointer-path package signature that fails under the key in the keyring is a FAIL and places
nothing; a package on disk that fails under that key is moved aside with the CA certificate and
SPIFFE bundle beside it; the mark file and the later `ours.pointer.json`) apply only with the
flag. The one difference is how the variable itself is read: `true` and `yes` now turn it on,
and a value that is none of `1`, `true`, `yes`, `0`, `false`, `no` or empty is refused.

### 6.3 A party with several peerings: one package per counterparty

Decision: ADR 1225 §6, "Directory: several peerings per party" (`docs/adr/1225-federation-belongs-to-goatbuilder-one-command-one-decision.md`).

Status: WIP — gate: two peerings of one party exchanged through one directory on the test
environment, each counterparty composing the package meant for it, and both agreements' digests
matching on both sides.

A package carries one `transport_offer`: one endpoint, one WireGuard key, one proposed link
subnet. Each peering has its own interface, port and /31 (the per-peering allocation in
`goatbuilder federate`), so a party with two peerings signs two packages. With one
`/v1/parties/<id>/package` slot per party, publishing the second replaced the first.

saltlick therefore also keeps one package per counterparty:

| File | Path | Checked on upload | Checked by the reader |
|---|---|---|---|
| The package signed for one counterparty | `/v1/parties/<id>/packages/<counterparty>` | as the single package; `<counterparty>` has the form of a `sovereign_id` and is not `<id>`; an upload that was withdrawn is refused | the upload's request signature, whose resource line is `<id>/packages/<counterparty>`, under the key read back on the call (before the call, under the key the directory has bound, which the call then confirms); then the package signature |

A slot is removed with the entry, or alone by a `DELETE` of its path signed by the bound key
over the sha256 of the package being withdrawn, when the peering ends (`saltlick sign -method
DELETE -id <id> -upload packages/<counterparty> -file <that package>` prints the header; the
driver step does not withdraw slots). A withdrawal is final: saltlick keeps the withdrawn
upload's signature and refuses it if replayed, so the slot returns only with a newly signed
package (saltlick `SPEC.md`, "One package per counterparty").

The package format does not change (spec §11.1). **The agreement digest does not always catch a
misdelivered package.** The agreement carries neither the offer's `endpoint` nor its
`public_key`, and takes `link_subnet` only from the party that sorts first, so a party's two
packages that differ only there compose with the same counterparty package to the same digest.
The slot's upload signature is then the only check that the package was meant for us: its
resource line names both parties. A reader taking a package from a slot must verify that line
before using it, and the driver step does.

The driver step does this when `FED_DIRECTORY_PER_COUNTERPARTY` is `1`, `true` or `yes` (or
`directory.py --per-counterparty`); `0`, `false`, `no`, empty or unset is off, and any other value
is refused with nothing done. **Built.** `directory.py` alone is off by default, and so is
`peering-standup.sh`; `goatbuilder federate` passes `--per-counterparty` unless the operator sets
the variable (gate row B6; [goatbuilder.md](goatbuilder.md), stage 11.5). With it set, the step:

1. uploads our package to `/v1/parties/<us>/packages/<FED_COUNTERPARTY_ID>`, and reads it back
   byte for byte. With no counterparty id it publishes no slot and says so, and step 2 still
   applies, so the run cannot replace another peering's single slot;
2. writes the pointer and the single `/package` slot only when they are this peering's: the
   single slot is empty, holds the bytes this peering last wrote (recorded in
   `$FED_WORK/directory-single-slot.sha256`, which a run with the flag writes when it uploads
   the single slot; the work directory is per peering), holds the package
   named by this peering's last published pointer (`$FED_WORK/ours.pointer.json`, which
   releases before the mark file also wrote, and which the step, with the flag, writes only once the
   directory holds the pointer and, with no `FED_PACKAGE_URL`, serves our package back byte for
   byte, so a run that fails partway leaves it as it was), or holds a
   package with our `transport_offer`. With
   `FED_PACKAGE_URL` set, the pointer is this peering's when there is none or it names our
   `FED_PACKAGE_URL`. Otherwise the step leaves both alone, so they keep serving the first
   peering to a counterparty on an older release. Only that first peering's runs refresh the
   entry's `expires` or rotate its key;
3. looks the counterparty up at `/v1/parties/<them>/packages/<us>` first, and places that
   package only when its upload signature verifies over the resource `<them>/packages/<us>`:
   under the key in the keyring, or, with none yet, under the key the directory has bound
   (printed for the call, where it must match the key read back), and the package's own
   signature verifies under that same key, so a directory that signs the slot under a key of
   its own cannot carry a genuine package. A slot served with no signature, either signature
   failing, or a keyring key that is not a key, places nothing new. A file already on disk is
   moved to `<file>.refused-<UTC time>` (an earlier refused copy is kept) only on its own
   evidence: when the keyring holds their key and
   the on-disk package's signature does not verify under it. The CA certificate and SPIFFE
   bundle that came with it go aside with it. A refused upload signature alone moves nothing,
   so a directory that lies cannot push a correct file aside;
   then the CA certificate and SPIFFE bundle: before their key is in the keyring, the step
   saves them as `$FED_WORK/theirs-ca.pem.unconfirmed` and
   `$FED_WORK/theirs.spiffe-bundle.unconfirmed`, never in place, and says to run phase 5 again
   once the key is recorded. With the key recorded, it places the CA certificate the checked
   package names and the bundle whose upload signature verifies under the recorded key, but
   only where no file is there yet, and removes the unconfirmed copies. It never replaces an
   on-disk CA certificate or bundle, even with one that verifies: a signature carries no date,
   so a directory can replay any bundle or package the partner ever uploaded, including one
   with a retired SPIFFE CA, which phase 13 would import. An on-disk file that differs is a
   WARN and is left; it goes aside only with a package refused on its own evidence (above),
   so `theirs-ca.pem` and the package cannot drift apart after a partner rotates. A partner
   rotation therefore reaches us when the operator, having confirmed it on a call, removes the
   old files and runs phase 5 again. Without the flag, the single-package flow places both as
   before (the federation end-to-end test reads them in place before it records the keys);
4. on a 404 there, reads their pointer as §6.2 does, but does **not** place that package: their
   pointer may name another of their peerings' packages, signed by the same key, and the digest
   may not tell (above). It saves it at `$FED_WORK/theirs.pointer.json` with a warning. After
   confirming on the call that it is this peering's package, the operator re-runs with
   `FED_DIRECTORY_PLACE_POINTER_PACKAGE=1` (`--place-pointer-package`; `1`/`true`/`yes` on,
   `0`/`false`/`no`/empty off, any other value refused) and the step places it. While the
   package is held, their CA certificate and SPIFFE bundle are not fetched or placed either;
   the confirming run fetches them. Any failure other than a 404 is a warning and nothing is
   placed;
5. against a directory that has no such route (a saltlick release before it), warns and keeps
   the single-package flow;
6. on a 409 for our slot upload, says plainly that the directory refuses this package because
   identical bytes were uploaded there before and then superseded or withdrawn (saltlick keeps
   those signatures, and an identical package signs identically), that the counterparty still
   reads what the slot holds now, and names the two remedies: withdraw the slot, which stops the
   counterparty reading the superseded package but does not make this one uploadable, or
   reissue the package with a changed field. This is what a party meets going A, then B, then
   back to A on one day: the package carries day-granular dates, so A reissued the same day is
   byte-identical to the first A.

Follow-up, **not built**: a per-request nonce in the slot upload statement would let a party
re-upload identical package bytes after superseding them, without changing the package. It
needs a new request-statement version in saltlick and a matching reader change, so it waits for
evidence that same-day reissues happen.

The CA certificate and the SPIFFE bundle stay one per party: they name the party, not a peering.

**What the directory learns.** The slot name is the counterparty's `sovereign_id`, so anyone who
asks for it learns that the two parties hold a peering. saltlick does not list slots in its
catalog or on its home page, does not interpret the name, and still computes nothing about who
should peer with whom. A party that does not want a peering visible exchanges that package
directly, as §6 already allows.

## 7. Relationship to the Block 84 peering portal

They compose; neither replaces the other.

| | Concern |
|---|---|
| This directory | Two parties discover each other and check compatibility before building |
| `goatbuilder federate` (released `goat federate` until the ADR 1225 move completes) | Both sides prove they hold the *identical* agreement — independently computed, one digest compared |
| Block 84 portal | Where **our** side approves and applies an agreement, mints the capability, emits the ACL, and records the audit trail |

The portal is a one-sided approval workflow and has no concept of the partner having produced a
verifiable artifact. That is what the package provides, and why the two fit together rather
than overlapping.

## 8. Open questions

- **One instance per environment, or one shared?** One instance serves every goatnet of a
  coalition, because it sits outside all of them; whether a coalition ever wants a second is
  open.

## 9. Status

| Piece | State |
|---|---|
| The decision, and its invariant | **Decided** — 2026-09-05 |
| Repository | **Public** — `USAREUR-AF/saltlick` |
| Service | **Built** — image `ghcr.io/usareur-af/saltlick:v0.4.0`, multi-arch, pulls anonymously; hosts package, CA and SPIFFE bundle with the upload's signature |
| Internet-reachable deployment (§5.1) | **Built** — one instance outside every goatnet, declared in the deploying team's private overlay, never a mesh service on a goatnet host; per-source rate limits in the service designed |
| Pointer entry (§6) | **Built** in saltlick (`saltlick-pointer/v1`) |
| Package per counterparty (§6.3) | **Built** in saltlick and the driver step; on by default in `goatbuilder federate`, off by default in `directory.py` alone and `peering-standup.sh` (`FED_DIRECTORY_PER_COUNTERPARTY`); tested against a stand-in and a local saltlick, not yet measured on the test environment |
| Driver step (§6.2) | **Built**; phase 5 of the driver, required by preflight unless `FED_NO_DIRECTORY=1`; exercised end to end by `ops/federation-e2e/run.sh` against a saltlick it starts |
| The trust mechanism it sits above — package spec, `goatbuilder federate` (today `goat federate`; ADR 1225), conformance vectors | **Built and merged** — see [`federation-package-spec.md`](federation-package-spec.md) |

Federation works without this service. That is by design: the directory is advisory, and a
peering never waits on it.
