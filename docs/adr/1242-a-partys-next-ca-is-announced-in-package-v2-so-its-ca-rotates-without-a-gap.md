# ADR 1242 — A party's next CA is announced in package v2, so its CA rotates without a gap

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-15 |
| Deciders | operator |
| Repo scope | goat core (`crates/goat-federation`, `crates/goatbuilder/src/federate`, `ops/federation/vectors`, `docs/design/federation-package-spec.md`, `docs/design/federation-key-rotation.md`) |
| Supersedes | — (supersedes no ADR; for package v2 it replaces the single CA fingerprint per party in [`federation-package-spec.md`](../design/federation-package-spec.md) §5.1 and §8.1) |
| Superseded by | — |
| Related | [ADR 1170](1170-federation-growth-by-corroborated-party-roster.md) (a party's bound values are a durable record) · [ADR 1225](1225-federation-belongs-to-goatbuilder-one-command-one-decision.md) (one command, two confirmations) · [ADR 1229](1229-a-partners-new-package-is-confirmed-within-confirmation-1.md) (a partner's new package is confirmed in confirmation 1) · ADR 1230 (package v2, proposed in an open pull request) · [`federation-key-rotation.md`](../design/federation-key-rotation.md) §3 and §4 · [`federation-certificate-lifecycle.md`](../design/federation-certificate-lifecycle.md) · [`internal-tls-ca.md`](../design/internal-tls-ca.md) |

## TL;DR (for a layperson)

Two organisations that link their networks each trust the other's certificate authority, the
"CA" that vouches for its machines. Today each side trusts exactly one CA of the other's, so
when one side replaces its CA the link stops until the other side catches up. With this
decision, a side that is about to replace its CA first tells its partner the new one, and the
two operators confirm it on a call as they already confirm the first one. For a while the
partner trusts both, the side switches its machines over, and then the old one is dropped, so
data keeps arriving throughout. The announcement goes into the next version of the package
format, because version 1 is frozen and a partner on version 1 must be refused clearly rather
than silently cut off. A CA that may have been stolen is never kept on during such a period: it
is dropped at once, and the link pauses until both sides have moved.

## Context

The package specification binds two values per party over an authenticated channel: the
signing key and `identity.ca_fingerprint` (`federation-package-spec.md` §5.1). The agreement
copies that one fingerprint into each party (§8.1), and every check that uses it accepts
exactly one CA: phase 5 confirms one fingerprint (`crates/goatbuilder/src/federate/confirm.rs`),
admission requires `partner-ca.pem` to hold exactly one certificate
(`crates/goatbuilder/src/federate/admit.rs`, "must hold exactly one certificate"), and the
edge role checks the same ([`federation-certificate-lifecycle.md`](../design/federation-certificate-lifecycle.md),
"the partner CA file holds exactly one certificate").

Rotating our CA (key K2 in [`federation-key-rotation.md`](../design/federation-key-rotation.md)
§2) therefore cannot keep data flowing: the moment our leaves are issued under the new CA, the
partner's edge refuses them, and the moment the partner trusts only the new CA, our old leaves
fail. Gate row B3 of the goatbuilder 1.0 gate requires a scenario that rotates each key in turn
while data keeps arriving ([`goatbuilder-v1-gate.md`](../design/goatbuilder-v1-gate.md)). The
key-rotation design §4 sets out an announce, switch and retire overlap and says it needs an ADR
that supersedes §5.1, and that the ADR must settle version skew, the normal case between two
sovereigns.

Two facts constrain the answer. Version 1 is pinned literally: composition refuses any package
whose `spec` is not exactly `federation-package/v1` (§6 step 1), "the format does not move"
within a version (§3.3), and the conformance vectors are normative and written by hand from the
text (§10), so a partner can implement v1 from the document alone. Second, our site roots are
made with a path length of zero (`ops/enrollment/x509ca/root.go:89`), so no intermediate CA can
sit under one, and cross-signing a new CA with the old root does not validate.

## Decision

1. **A party's CA binding is one or two fingerprints.** It is the CA that issues the party's
   leaves now and, only during a planned rotation, the CA that will issue them next. Each is a
   §5.1 value: confirmed over a channel authenticated independently of the relationship, read
   aloud by the same rule (sixteen groups of four hex characters, then read back), and recorded
   in the party's corroboration record (ADR 1170 decision 1). A next CA learned from a package,
   a directory or a SPIRE bundle without that confirmation is not bound.

2. **The next CA is a separate member.** `identity.ca_fingerprint` keeps its v1 meaning and
   format: the CA issuing the party's leaves now. A new optional member,
   `identity.ca_fingerprint_next`, has the same format (`sha256:` and 64 lowercase hex
   characters). When present it must differ from `ca_fingerprint`; an equal value is a malformed
   package, not a rotation. The agreement copies it verbatim into that party as
   `parties[n].ca_fingerprint_next` when present, and leaves the member out otherwise (never
   null, never empty, §4). Because it is in the agreement, it is in the digest: two sides whose
   digests match trust the same set.

3. **It is not added to v1.** v1's grammar, agreement and vectors stay as they are. The member
   belongs to the next version of the lineage, `federation-package/v2` and
   `federation-agreement/v2`. ADR 1230 proposes that same v2 for the listener fields; the
   lineage gets one new version carrying both, not two. This member depends on none of ADR
   1230's other decisions: if ADR 1230 is withdrawn, v2 is defined with this member alone; if it
   is accepted first, this member joins its v2 grammar before any release emits v2. Once a
   release has emitted v2, a change to it needs v3.

4. **Every check accepts exactly the set the confirmed agreement names for that party.** Phase
   5 shows each fingerprint of the partner's set and confirms each (`--partner-ca-fingerprint`
   takes one value per fingerprint, and each must match). Admission and the edge role require
   `partner-ca.pem` to hold one certificate per fingerprint in the partner's set, each a CA, no
   more and no fewer; order in the file does not matter. Our edge's `our-ca.pem` follows our own
   set the same way. A set that differs from the agreement is refused, as one fingerprint is
   refused today.

5. **A planned rotation is three runs of the command, each ending with phase 14.**
   - **Announce.** Our package carries the new CA as `ca_fingerprint_next`. It is a new package
     and a new agreement, so confirmation 1 (with ADR 1229 for the partner's side) and the
     agreement digest comparison happen again. Both sides then place both of our CAs. Our leaves
     are unchanged, so data keeps flowing.
   - **Switch.** Refused unless the agreement both sides confirmed names our new CA as
     `ca_fingerprint_next`. Phase 8 re-issues our leaves under the new CA, and phases 10 and 12
     install them. The agreement does not change. The only interruption is the edge and
     republisher restart, as for a leaf rotation (key-rotation design §3, K3).
   - **Retire.** Refused until every leaf our hosts present for this peering is issued by the
     new CA. Our package carries the new CA as `ca_fingerprint` and no `ca_fingerprint_next`. A
     new agreement is confirmed, and each side removes the old CA. Every leaf of the old CA
     stops being accepted at that moment.

   The overlap needs no timer of its own: the old CA is trusted no later than the agreement's
   `expires`, and a renewal composed while a next CA is announced carries the announcement on.

6. **Version skew is refused by name, never absorbed.** A partner that reads only v1 cannot
   compose a v2 package: §6 step 1 refuses it as a wrong `spec`. The overlap is therefore
   available only on a peering where both parties have selected v2 in reviewed configuration
   (ADR 1230 decision 3). On a v1 peering, a CA rotation is a **gap rotation**: one new agreement
   names only the new CA, both sides confirm it, and data stops from whichever side moves first
   until the other has moved. The command names that gap before it re-issues anything and does
   not start until the partner has confirmed the new agreement.

7. **A CA that may be compromised is never overlapped.** Announcing it as still trusted is what
   an attacker holding its key would want. Its retirement is a gap rotation on either version,
   started at once, and the announce step refuses when the operator records the rotation as a
   compromise.

8. **Scope is the federation follow-through.** Making a new site CA and moving every other
   service under it is the site's larger ceremony ([`internal-tls-ca.md`](../design/internal-tls-ca.md));
   this decision starts after the new CA exists. It does not cover the IPsec transport's CA,
   which is per peering and already rotates with an announce and retire of its own
   (key-rotation design §8.3), nor SPIRE's CA, which rotates itself.

9. **What is built, where.** `goat-federation` parses, composes and lints v2 with this member,
   and the v2 conformance vectors include a pair with `ca_fingerprint_next` on one side, a pair
   with it on both, and a malformed package whose next equals its current CA, written by hand
   from the specification as v1's were. `goatbuilder federate` carries the set through
   confirmation, admission and the edge role, and gains the `rotate … ca` stage the key-rotation
   design §5 describes. `federation-package-spec.md` gains the v2 text when this is built, not
   before; v1's text is unchanged.

## Alternatives considered

**Add `ca_fingerprint_next` to v1 as an optional member.** It would arrive sooner, and an older
goatcore that ignores the member would still verify the package. Rejected: v1 is literally
pinned and frozen, and its vectors are normative. A partner that implemented v1 from the
current text would compose a different agreement whenever a next CA is announced, or refuse the
package as malformed if its grammar is closed; either way the disagreement would surface as a
digest mismatch with no named cause. A version change surfaces it as a named refusal at step 1.

**Cross-sign the new CA with the old root.** No format change and one fingerprint throughout:
the partner keeps trusting the old root, and the new leaves chain to it through the cross
certificate until the agreement switches. Rejected: our roots carry a path length of zero
(`ops/enrollment/x509ca/root.go:89`), which forbids the intermediate this needs, and lifting
that means a new root, the very thing being rotated. It would also require every partner
implementation to build chains through an intermediate the specification never mentions.

**One member holding a set of one or two fingerprints.** More general, with no distinction
between the two. Rejected: the switch and retire steps need to know which CA issues leaves now
and which is next, and a set cannot say. Keeping `ca_fingerprint` with its v1 meaning also lets
one code path read the current CA from either version.

**Gap rotation only.** Works today on v1 with no format change: `--partner-ca-fingerprint`
already re-runs phase 5 onward. Kept as the path for v1 peerings and for a compromised CA.
Rejected as the only path because gate row B3 requires data to keep arriving through every
rotation.

**Learn the next CA from the partner's SPIRE bundle or a directory.** No call needed. Rejected:
§5.1 makes a directory a cache of the binding, never its source, and the peering's TLS trust is
not SPIRE's trust domain. A CA trusted without the call is a CA trusted on the word of whoever
served it.

## Consequences

**Positive.** A planned CA rotation keeps data arriving, with the same two confirmations ADR 1225
fixes and no new kind of human decision. Version skew is refused by a rule a partner can read in
the specification, not discovered as an unexplained digest mismatch. The current-CA member and
its checks are unchanged in meaning, so v1 peerings keep working as they are.

**Negative.** The CA part of gate row B3 cannot be built until v2 is: on v1 peerings only the gap
rotation exists, so row B3 cannot fully pass before both v2 and this member are built. A planned
rotation is three runs and two calls with the partner. During the overlap the old CA is still
trusted for the peering, for up to the agreement's term if a rotation is announced and left
unfinished. If ADR 1230's v2 is delayed or narrowed, this member's arrival follows it, or v2 is
defined for this member alone (decision 3).

## Assessor view (SCA-R)

The decision keeps the trust anchors for a partner's peering certificates under the same
out-of-band confirmation as the first binding, which maps to SC-12 (cryptographic key
establishment and management), SC-17 (public key infrastructure certificates), IA-5(2)
(public-key-based authentication) and IA-3 (device identification and authentication). Every
added anchor is confirmed on a call, recorded in the party's corroboration record and bound to a
compared agreement digest, which supports AU-10 (non-repudiation). Residual risk: during an
announced overlap the outgoing CA remains trusted for the peering; the compensating controls are
the bound of the agreement's `expires`, the retire step's check that no leaf of the old CA is
still presented, and the rule that a possibly compromised CA is retired at once and never
overlapped. No waiver is needed.

## Trigger conditions for revisit

- ADR 1230 is withdrawn, or a release emits v2 before this member is in its grammar: decision 3
  then takes the other branch, or the member moves to v3.
- Site roots are regenerated with a path length of one or more: cross-signing becomes possible
  and may make a format change unnecessary for later rotations.
- A partner or coalition asks to announce more than one next CA at a time.
- The site CA rotation ceremony in `internal-tls-ca.md` is built and changes when a new CA exists
  relative to the announce step.
