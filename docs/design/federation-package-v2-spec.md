# Federation package v2

Package v2 is version 1 (`federation-package-spec.md`) with one member added. That member
announces the CA that will issue a party's leaves next, so a party's CA can rotate without a gap
(ADR 1242). Everything this document does not change is v1's text, rule for rule: the grammar,
the canonical form (§4), the signature (§5), the binding over an authenticated channel (§5.1),
composition (§6 to §9), and the vectors' conventions (§10). Version 1's text and its vectors are
unchanged, and v1 remains the version a party emits unless it selects v2.

**Status.**

- Built in `crates/goat-federation`: parsing, routing and composition, held to the vectors in
  `ops/federation/vectors-v2/`; `init --package-version v2` emits v2, `compose --package-version
  v2` composes a v2 pair only, and `apply` and `verify` read an agreement/v2.
- Built in `goatbuilder federate`: a peering selects v2 with `peerings[].package: v2`
  (FED_PACKAGE_VERSION reaches the phases); phase 3 then emits v2 and phase 6 composes it. A v1
  peering's phases are unchanged.
- Not built:
  - a next CA in our package (phase 3 announces one only for the `rotate … ca` stage);
  - carrying the CA set through phase 5, admission and the edge role, which read the current CA
    only;
  - the `rotate … ca` stage;
  - v2 in the Go reference implementation;
  - v2 in the partner bundle.

## 1. Versions

- A v2 package declares `"spec":"federation-package/v2"`.
- A v2 agreement declares `"spec":"federation-agreement/v2"`.
- A pair is composed under one version. Both parties select the same version in reviewed
  configuration before either emits it (ADR 1230 decision 3, ADR 1242 decision 6).

## 2. The new member: `identity.ca_fingerprint_next`

- **Optional.** Present only while the party has announced a planned CA rotation.
- **Format** as `identity.ca_fingerprint`: `sha256:` followed by exactly 64 lowercase hex
  characters. A value in another format is malformed: `M_CA_FINGERPRINT`, at
  `identity.ca_fingerprint_next`.
- **Different from `identity.ca_fingerprint`.** An equal value is malformed (`M_CA_NEXT_SAME`), not
  a rotation.
- **Bound like the current CA.** A next CA is trusted only once it has been read aloud and confirmed
  over the channel §5.1 requires, by the same rule (sixteen groups of four hex characters, then
  read back), and recorded with the party's other bound values. A next CA learned from a package,
  a directory or a bundle without that confirmation is not bound.
- **Its meaning.** `identity.ca_fingerprint` keeps its v1 meaning: the CA issuing the party's
  leaves now. `ca_fingerprint_next` is the CA that will issue them next.
- **Under v1.** A v1 package's grammar has no such member. A v1 package that carries one is read by
  v1's rules, which do not read it.

## 3. The agreement

The v2 agreement is the v1 agreement with three differences:

- `spec` is `federation-agreement/v2`;
- each `parties[n]` copies its party's `ca_fingerprint_next` verbatim when the party's package
  carries one;
- that member is left out otherwise: never null, never empty (§4).

Because the next CA is in the agreement, it is in the digest. Two sides whose digests match trust
the same set of CAs for each party.

## 4. Routing

An implementation that composes both versions routes a pair as follows:

1. **Step 0 first.** It verifies both signatures (§6 step 0) before it acts on either `spec`
   value, since a `spec` value is a claim of an unauthenticated package until then. This is sound
   because `spec` is inside the signed bytes in both versions: the signing input is the canonical
   form with only `signature` removed. A package relabelled to another version after signing fails
   verification (`C_SIGNATURE`) and is never composed under either version
   (`vectors-v2/refuse-downgrade-*`, `refuse-relabel-v1-as-v2`).
2. **Both v1.** The pair is composed by the v1 rules exactly, through the same steps, findings and
   bytes.
3. **Both v2.** The pair is composed by the v1 rules, with step 1's pin set to
   `federation-package/v2` and the agreement of §3.
4. **One of each.** The pair is refused at composition, before any agreement is written:
   `C_SPEC_MIXED`. A mixed pair is never composed under either version, and no agreement or digest
   exists for it. The two parties then agree one version and select it in reviewed configuration:
   either both publish v2, or both stay on v1 and the side that published v2 publishes v1 again.
5. **Any other version.** The pair is refused: `C_SPEC_PIN`.

There is no retry and no downgrade.

An implementation that composes v1 only keeps v1's literal pin. It refuses every v2 package with
`C_SPEC_PIN`, as `../vectors/refuse-both-v2` and `../vectors/refuse-version-mismatch` show.

## 5. Findings

| Finding | Where | When |
|---|---|---|
| `M_CA_FINGERPRINT` | `identity.ca_fingerprint_next` | the next CA is not in the fingerprint format |
| `M_CA_NEXT_SAME` | `identity.ca_fingerprint_next` | the next CA equals the current one |
| `C_SPEC_MIXED` | `spec` | one package is v1 and the other v2. Refused at composition, before any agreement is written, so neither side holds an agreement for the pair. Next: both parties select one version, and either both publish v2 or both stay on v1 |
| `C_SPEC_PIN` | `<party>.spec` | a version this implementation does not compose |

## 6. Vectors

`ops/federation/vectors-v2/` holds v2's vectors; its README lists them and how they were computed.
They follow §10's conventions:

- the evaluation date is its own `now.txt` (the same date as `../vectors/now.txt`), never the wall clock;
- the test keys are `../vectors/testkeys/`;
- canonical files carry no trailing newline;
- a refusal is asserted by the rule that fired, not only that one did.

## 7. What a later v2 adds

ADR 1230 proposes v2 for the listener and ingress fields as well. The lineage gets one new version
carrying both, not two (ADR 1242 decision 3). If those fields are accepted before any release has
emitted v2, they join this grammar and its vectors; once a release has emitted v2, a change to it
needs v3. Nothing here constrains those fields: they are members of `transport_offer` and of the
agreement's `transport`, which this document does not change.
