# Federation package, version 1

| Field | Value |
|---|---|
| Status | Written; conformance vectors written; reference implementation predates this revision |
| Plan + open decisions | `../project/cross-sovereign-federation-plan.md` |
| Conformance vectors | [`../../ops/federation/vectors/`](../../ops/federation/vectors/) — **normative** |
| Reference implementation | [`../../ops/federation/reference/`](../../ops/federation/reference/) |
| Transport design | `cross-sovereign-peering-transport.md` |

The contract between two organisations setting up a data link. This document is the
authority; our tool is one implementation of it. **A partner must be able to implement
this from scratch, in any language, without running our software.** If any statement here
is only true when both sides run the same binary, this specification is broken.

Deliberately small: a competent implementer should finish in an afternoon.

---

## 1. What a package is

A **package** is one party's half of a federation, as one JSON object, signed by that party.

Two packages compose into one **agreement**. Both parties compose independently and compare
the agreement's digest. Equal digests prove both sides hold the identical agreement —
that is the entire point of the format.

A package never contains a secret. Public keys, public names, agreed limits. It is
reviewable by people who are not engineers.

## 2. The crux: an agreement has no "us" and "them"

The obvious way to build this is wrong. If each side composed a document with `our_*` and
`their_*` fields, the two sides would produce mirror images — different bytes, different
digests, no comparison possible.

So the composed agreement has **no perspective**. The two halves are sorted by `sovereign_id`
into `parties[0]` and `parties[1]` — by UTF-16 code unit, per §4 — and every derived field is expressed
in terms of those positions. Both sides then compute byte-identical output.

> Everything else in this specification is bookkeeping. This is the idea.

## 3. Package structure

```json
{
  "spec": "federation-package/v1",
  "sovereign_id": "xaa",
  "issued": "2026-09-05",
  "expires": "2026-12-05",

  "identity": {
    "trust_domain":   "spiffe://xaa.example",
    "ca_fingerprint": "sha256:9f2c..."
  },

  "capabilities": {
    "transports":        ["ipsec-fips", "wireguard"],
    "fabric_fips_image": true,
    "pqc_capable":       true,
    "fabric_version":    "1.9.0"
  },

  "transport_offer": {
    "endpoint":             "_fed._udp.xaa.example",
    "listener":             true,
    "public_key":           "...",
    "proposed_link_subnet": "169.254.10.0/31",
    "measured_mtu":         1380
  },

  "data_offer": {
    "release":                  ["release/weather/**"],
    "accept_hz":                100,
    "accept_bytes_per_message": 1048576,
    "accept_distinct_keys":     5000
  },

  "cut": { "contact": "..." },

  "signature": "..."
}
```

### Presence is the declaration

There is no tier field and no tier ladder. **A package carrying `transport_offer` is willing
to connect. A package carrying `data_offer` is willing to exchange.** A party willing to do
neither publishes `identity` and `capabilities` and stops there. What a *pair* can do is
derived from what both packages carry (§7), so nothing needs to be claimed.

| Always required | Optional as a block; complete once present |
|---|---|
| `spec`, `sovereign_id`, `issued`, `expires`, `identity`, `capabilities`, `signature` | `transport_offer`, `data_offer`, `cut` |

A block that is present but incomplete is **refused, never defaulted** — silent defaults are
how a link ends up with no rate limit.

#### `data_offer` has two independent halves

`data_offer` is one block carrying two separable declarations, and **each is complete once
present, independently of the other**:

| Half | Members | Says |
|---|---|---|
| Send | `release` | "I will publish these key expressions across the link" |
| Receive | `accept_hz`, `accept_bytes_per_message`, `accept_distinct_keys` | "I will accept traffic up to these limits" |

A `data_offer` must carry at least one half. `release` and no `accept_*` is a **send-only**
party; all three `accept_*` and no `release` is a **receive-only** one; both is both. A
*partial* receive half — one or two of the three `accept_*` values — is refused, because a
limit that is not stated is a limit that does not exist. That is §7's rule that a flow is never
emitted with a limit it does not have, seen from the other end.

**One-way federation is a real case, not a degenerate one.** A partner who will only receive
has nothing to release. Were all four members required together, that partner would have to
declare a release set it will never publish under — and an empty one is not allowed, so it
would have to invent a plausible-looking key expression. A package containing a fiction written
to satisfy a validator is worse than one honest half, because that fiction is later read as an
intention.

**An empty array is not a declaration.** `"release": []` is a malformed `data_offer` and the
package is refused; it is not a send half that happens to carry nothing. The same holds for
`capabilities.transports`. So "carries `release`" in §7 means carries a non-empty one — no
other kind reaches composition.

`cut` is required alongside `data_offer` whichever halves it carries: a peering with no agreed
cut procedure gets severed badly, at speed, by someone guessing.

Validation is therefore internal consistency of whatever blocks are present. A claimed level
was a second place for a package to be wrong — it could claim `exchanging` and omit
`data_offer`, or carry `data_offer` and claim `recognised` — and those two mistakes read as
different errors while being the same mistake. Removing the claim removes the disagreement
between a package and itself.

### `sovereign_id` — where the value comes from

2–16 characters, lowercase `a-z0-9-`. It fixes `from/<id>/**` and must be unique between the
two parties.

**A sovereign is an organisation, not one of its environments.** A single sovereign may run
several environments — a sandbox, a production fabric, an experimental one — and they all
share its one `sovereign_id`. Putting an environment name in this field is a modelling
error, and produces peering names that cannot survive that environment being retired.

The value is not freely chosen:

| Kind of party | Value | Example |
|---|---|---|
| A nation | Its **ISO 3166-1 alpha-3** code, lowercased | `usa`, `isl`, `can`, `aus` |
| A multinational body | Its established short name, lowercased | `nato` |
| A sub-entity of either | The parent, a hyphen, then a local suffix | `nato-hqx` |

Taking these from a published international standard means **two parties who have never
communicated cannot collide**, and no registry is needed to prevent it. That matters: a
central allocator would be exactly the authority this design exists without.

Uppercase forms of the same codes appear in identity attributes elsewhere; this field is the
lowercase form of the same code, because it appears in key expressions and certificate names
where case sensitivity would otherwise be a source of silent mismatch.

**Examples and test fixtures use the standard's user-assigned range** — `xaa`, `xab`, and
so on through `xzz` — which is reserved precisely so that private use cannot collide with a
real assignment. Two consequences, both wanted: an example can never be mistaken for a real
relationship, and a fixture copied into production cannot collide with an actual party.

**Never use a word with a meaning in matching syntax.** An earlier draft of this document
used `any` as an example party, which produced the rule pattern `fed.any.*`. That reads as
*"federation, any issuer, anything"* — the exact over-grant the naming scheme exists to
prevent — when it in fact names one specific party. A name that an operator can misread as
a wildcard in an access rule is a defect in the name, not in the operator.

## 3.1 Canonical peering names

The name inside the certificate is what the far side's access rules match on, and in the
underlying fabric **those rules match the name and cannot see which authority issued it**.
A weak name therefore turns a collision into an authorisation failure. Names are not
cosmetic here.

Every peering identity is named:

```
fed.<issuer>.<counterparty>.<ordinal>
```

`fed.usa.xaa.01` is issued by `usa`, for its peering with `xaa`, first instance.
Its opposite number is `fed.xaa.usa.01`.

| Part | Carries | Why it must be present |
|---|---|---|
| `fed.` | This is a federation edge identity | An access rule can state that **nothing matching `fed.*` may ever touch operational namespaces**, as a class. It also makes a peering identity impossible to confuse with an internal one in a log line. |
| `<issuer>` | Which sovereign minted it | The name asserts its own origin, so the rule granting access to `fed.xaa.*` cannot be satisfied by anything our own authority issued, and vice versa. This is the mitigation for the access layer's blindness to the issuer. |
| `<counterparty>` | Which peering this identity is for | Per-peering isolation. Revoking `fed.usa.xaa.01` does not disturb `fed.usa.xab.01`. One partner's compromise is one partner's problem. |
| `<ordinal>` | Which instance | Zero-padded, two digits. For a concurrent pair, not for rotation — see below. |

**The separator is a full stop, and this is load-bearing.** A `sovereign_id` may itself
contain a hyphen (a multinational body such as `nato-hqx`), so a hyphen-separated name is
ambiguous: in `fed-nato-hqx-xaa-01`, is the issuer `nato` or `nato-hqx`? A full stop cannot occur inside
a `sovereign_id`, so the name parses unambiguously by splitting on it. Maximum length is 40
characters, comfortably inside the 64-character limit.

**A rotated certificate keeps its name.** The access rules match the name, so changing it on
rotation would sever the link. The ordinal increments only to run a *second, concurrent*
identity — a high-availability pair — never to replace one.

### These names are derived, not exchanged

Both names follow entirely from the two `sovereign_id` values, which are already in the
packages. So the composer **computes** them, and **a package does not declare them at all**.
The derived name appears in the agreement, which is where it is used.

This removes an entire class of failure. The most dangerous step in a hand-run bootstrap is
agreeing these names by hand, because a mistyped one matches no rule and is then refused in
silence, at both ends, with no error — which looks exactly like a broken link and costs days.
A name that is never written cannot be mistyped.

An earlier draft of this document also carried the derived name inside the package and
refused any package where the two disagreed. That declaration carried no information the
derivation did not already have, and it added a failure mode — a package correct in substance
and refused on a restatement of itself. A value that can only be derived cannot be disagreed
about, so there is nothing to check.

The composer derives ordinal `01`. A second, concurrent identity — the high-availability pair
in the table above — is named by the same rule with the next ordinal, and arranging one is
outside this version.

The certificate should also carry the issuing sovereign in its subject-alternative name,
using the anchor already reserved for this purpose in the identity grammar.

### What the peering name must not carry

The name is **the authorisation key** — it is the string the access rules match, and it is
matched as a string. Everything in it is therefore load-bearing for authorisation, and
nothing else belongs.

Specifically, it does **not** encode organisational hierarchy, command echelon, unit
lineage, or human authority chains. Four reasons, in descending order of how much they hurt:

1. **Those facts are mutable and the name is not.** Units reorganise, task forces form and
   dissolve, commands are re-subordinated. A hierarchy baked into the name forces a
   certificate re-issue on every reorganisation — and during the re-issue window the name no
   longer matches its rule, which fails **silently at both ends**. The system would generate
   its own outages on a routine administrative event.
2. **A hierarchical-looking name implies inheritance the enforcement layer does not have.**
   Matching is on the whole string. `usa-cmda-unitb` does not inherit anything from
   `usa-cmda` unless somebody writes a prefix rule against identity — and prefix rules
   against identity are precisely how a grant becomes wider than anyone intended. A name
   shaped like a tree but matched like a string is a trap for the next reader.
3. **It duplicates a field that already carries this.** The certificate's subject-alternative
   name already carries affiliation and echelon as a deliberately frozen origin stamp, and
   the registry carries the current, mutable version. Mutable organisational state that
   drives authorisation would couple access to the org chart.
4. **Length.** Two hierarchy chains and an ordinal will not fit in 64 characters.

### When a name may have more than one part

A party name such as `nato-hqx` does contain a hierarchy, so the boundary needs a rule
sharper than taste. It is this:

> **A name segment exists if and only if that entity operates its own certificate authority.**

The name tracks *cryptographic* independence, never organisational structure. A subordinate
command that runs its own authority, signs its own packages and peers in its own right is a
party, and its name says so. A subordinate command that does not is not a party — it appears
in attribution and in the registry, and never in a peering name.

The test is verifiable rather than negotiable: an entity either holds a distinct trust
domain or it does not.

## 3.2 The publishing prefix is one symmetric rule

**Every party publishes across the link under `from/<its own sovereign_id>/`.**

That is the entire rule, and it is the same rule on both sides. `xaa` publishes under
`from/xaa/`; `usa` publishes under `from/usa/`. Each side's edge grants the counterparty
write access under `from/<counterparty>/` and nothing else — again, one rule, applied by
both.

Describing it as *"we publish under ours, they publish under theirs"* makes one rule look
like two conventions, and is the same perspective error §2 exists to prevent. There is no
our-form and their-form.

Like the peering name, the publishing prefix is therefore **derived** — it is always
`from/<sovereign_id>/**` — and **a package does not declare it**. It appears in the
agreement, computed from `sovereign_id`, which is where it is used. The reasoning is the one
given for the peering name in §3.1: a restatement of a derivation adds a way to be refused
and no way to be right.

**This prefix is not the same thing as a release set**, and conflating them causes real
confusion:

| | What it is | Where it lives |
|---|---|---|
| `from/<sov>/**` | The **wire name**. Where a party's data appears once it has crossed. The only origin claim that is enforced rather than merely asserted, because the access rules bind it to that party's peering identity. | On the link, and on the receiver |
| a release set | The **local staging name** for data a party has agreed to export, e.g. `release/weather/**` | Inside the publishing party, before anything crosses |

Moving data from the second into the first is a small republishing process that each party
runs **on its own side, over its own data**. It is deliberately not a router feature: the
underlying fabric cannot rename key expressions in transit, and building that capability
would mean forking it.

## 3.3 Coalitions, alliances and frameworks

**No coalition is named anywhere in this specification, and no package declares its own
standing.** Both are deliberate.

### Three things that are easy to confuse

| | What it is | Does it peer? | Where it is named |
|---|---|---|---|
| **Party** | An organisation holding its own certificate authority | **Yes** — parties are what peer | `sovereign_id`, in this specification |
| **Environment** | One fabric a party operates. A party may run several — a sandbox, a production one, an experimental one | No | Nowhere in this specification |
| **Framework** | A named set of parties operating under shared terms — an alliance, a coalition | No — it is the space peering happens *in* | A framework registry, outside this specification |

The distinction is not academic. An environment built before any federation existed will
often carry a name that later becomes the name of the **federation itself** — the first
system built gets to name the space, and everyone else joins it. If that name had been used
as the party name, the party would eventually collide with the federation containing it.

This is why a party is named for the **organisation**, never for one of its environments,
and why party names come from a published international standard rather than from whatever
the first system happened to be called.

**A framework name and a party name may be the same string** when one body both defines a
membership and operates its own fabric — a multinational headquarters is a plausible example
of both. They are separate namespaces, resolved in separate registries, and are never
compared to one another. Reading a framework name as a party, or the reverse, is an error in
the reader, not an ambiguity in the format.

### The format is coalition-neutral

Naming a coalition in the format makes every party outside it a second-class citizen of the
format, and makes the naming coalition an authority over it. Neither is acceptable for a
document intended to be adopted, and eventually stewarded, by bodies that did not write it.

### One version lineage, globally

The `spec` value is the same for every party in every coalition. **Composition refuses across
versions** (§6), so a coalition-specific version string would make cross-coalition federation
structurally impossible — precisely inverting the purpose. A successor steward inherits this
lineage; it does not fork it.

A coalition that needs stricter rules **profiles, it does not fork**: it requires its members
to set particular values — a validated cryptographic build, a minimum packet size, a maximum
expiry window — and each member enforces that locally before signing. Policy sits on top of
the format. The format does not move.

### Standing is resolved by the receiver, never asserted by the sender

Membership is not a property of a party. It is a property of a party **relative to a
framework**, and the active framework belongs to the environment doing the evaluating, not to
the party being evaluated. Two peers may operate under different frameworks, so a bare
`coalition: member` carries no meaning without naming the framework — and even then, it is a
party asserting its own standing.

So standing does not travel. **The receiver resolves it from its own framework registry, keyed
on the counterparty's `sovereign_id`**, which is already in the package. Concretely: a registry
lists each framework's members with the date each joined; the receiver looks up the
counterparty and gets a current answer it can defend.

A party asserting its own membership, and a receiver believing it, would be recognition
becoming authorisation — the failure this whole design is arranged to prevent. The same
reasoning already excludes standing from certificates, where it is excluded for the weaker
reason that it is mutable; here it is excluded because it would be self-asserted.

### Reserved for a later version

`transport_offer.via` is reserved. If a future version adds it, it records that
`transport_offer.endpoint` is a peering forwarder and who operates it, as
`{"kind": "forwarder", "operator": "<sovereign-id>"}`. It is informational: it changes no
negotiated field and no rule, and its presence puts the forwarder into the digest. In v1 a party
behind a forwarder offers the forwarder's `host:port` as `endpoint` with `listener: true`, and
declares the forwarder in its partner declaration and interconnection agreement
([`peering-forwarder.md`](peering-forwarder.md) §6).

`frameworks` is reserved as a member name. If a future version adds it, it means *"the
frameworks this party claims to belong to"* — an assertion, for display and for compatibility
hints only, and **never an input to an access decision**. It is not part of v1 because nothing
in v1 needs it: release sets are explicit lists, so no lane abstraction crosses the link.

---

## 4. Canonical form

Serialisation follows **RFC 8785 (JSON Canonicalization Scheme)**: keys sorted by UTF-16
code unit, no insignificant whitespace, defined number formatting, UTF-8 output.

Do not invent an alternative. Homegrown canonicalisation is where formats like this die,
and every disagreement in serialisation becomes a false digest mismatch — and the first
false mismatch destroys confidence in the mechanism permanently.

Seven additional constraints, because JCS alone does not give determinism here:

- **No floating-point numbers anywhere.** Integers only. Rates, sizes and counts are exact.
- **A number carrying a fractional part or an exponent is refused, not coerced.** `1380.0`,
  `1.38e3` and `1380.` are each malformed, exactly integral though they are. The test is on the
  **token as written in the document**, not on the parsed value: reject any number whose JSON
  text contains `.`, `e` or `E`. Coercion was the other option and is worse — it makes the set
  of accepted documents depend on how each language's parser happens to widen numbers, which is
  the disagreement this section exists to remove.

  Three more number tokens are refused for the same reason, and none of them is exotic enough
  to leave unstated: **a leading zero** (`01`), **a leading plus** (`+1`), and **negative zero**
  (`-0`). The first two are not JSON at all, and a lenient parser that accepts them hands on a
  value whose canonical form is a different token. `-0` *is* valid JSON, and JCS emits it as
  `0` — so a package carrying it is not in canonical form, and an implementation that passes
  the token through produces different bytes for the same value.

  **In JavaScript this needs care, and the care belongs on the reading side.** JavaScript has
  one number type, so by the time a value is a `Number` the distinction between `1380` and
  `1380.0` is gone and no parser there can recover it. A JavaScript *producer* is nonetheless
  conforming by default: `JSON.stringify(1380)` emits `1380`, with no fractional part and no
  exponent. A JavaScript *consumer* must apply this rule to the document **text** — a parser
  that exposes the source token, or a scan of the raw bytes — and never to the parsed value,
  which cannot carry the answer.
- **Every integer is in the closed range −9007199254740991 … 9007199254740991**, that is
  ±(2^53 − 1). A value outside it is refused, in a package and in an agreement alike.
  This is the range every mainstream runtime represents exactly. JavaScript holds all
  numbers as IEEE-754 doubles, so a larger integer silently loses precision there and
  re-serialises to different bytes — a digest mismatch with no wrong value anywhere to find.
  Bounding the range costs nothing: no rate, size or count this format carries approaches it.
- **A member name occurs at most once in an object.** A parser handed the same name twice
  **must refuse, never resolve**. Go's decoder keeps the last occurrence and several other
  parsers keep the first, so a package carrying two `expires` members composes into two
  different agreements and each party believes its own.

  RFC 8785 is cited above for **output**, and says nothing about what a parser does with a
  malformed input. This is the gap: an implementation can canonicalise perfectly and still
  disagree with its counterpart before canonicalisation begins.

  This is not a forgery — it produces a *visible* mismatch, not a false match, so nobody is
  deceived into accepting a wrong agreement. It is the other failure this section names: two
  conforming implementations, two digests, and no wrong value anywhere to find. The signature
  verifies on both sides, because it covers the bytes, and the bytes are the same bytes.

- **Dates are `YYYY-MM-DD`.** No times, no zones, no clock skew between the parties. The one
  date not carried in a package is the evaluation date `now`, and §6 fixes both its source and
  its timezone for the same reason.
- **Arrays that represent sets** — `capabilities.transports`, `data_offer.release` — are
  de-duplicated and sorted ascending, **comparing the elements' UTF-16 code units one at a
  time, the shorter string first where one is a prefix of the other**. That is the same
  comparison RFC 8785 applies to member names, and this document uses no other:
  "lexicographic" on its own names no collation and is not a rule. See *One ordering* below.

### Sorting: the signer is obliged, the composer normalises anyway

A signer emits a package whose set-arrays are already sorted and de-duplicated — that is an
obligation on the party producing the package. A **composer sorts and de-duplicates every
set-array it reads**, whatever order it arrived in, on the way into the agreement.

These are not alternatives and neither is optional. The signature covers the bytes as signed,
so a composer handed an unsorted package cannot repair it and must not try: the package is what
it is. But the *agreement* is composed output, and normalising there is what keeps two
composers byte-identical even on an input that ignored the obligation. Stated the other way:
the agreement never inherits an ordering from a package.

### One ordering, everywhere

Everything this document orders is ordered the same way: **by UTF-16 code unit, compared one
code unit at a time, with the shorter string first where one is a prefix of the other.**

| What is ordered | Fixed by |
|---|---|
| Object member names | RFC 8785, which specifies this comparison |
| Set-array elements | This document (§4) |
| The two parties, by `sovereign_id` | This document (§6 step 5) |

One collation rather than two is deliberate. An implementer must build this comparison anyway
to canonicalise member names; reusing it for arrays means there is no second comparator that
can differ from the first in a way nothing tests.

**It is not the ordering most languages give you by default**, and the difference is invisible
on ASCII:

| Language | Default | Correct here? |
|---|---|---|
| JavaScript | `Array.prototype.sort()` compares UTF-16 code units | **Yes** — the default is right. `localeCompare` and `Intl.Collator` are not; they are locale-dependent |
| Go | `sort.Strings` compares bytes | No — encode each element with `utf16.Encode([]rune(s))` and compare the code units |
| Python | `sorted()` compares code points | No — `sorted(xs, key=lambda s: s.encode("utf-16-be"))` gives the right order, since every code unit is two bytes |

Byte-wise UTF-8 ordering and code-point ordering are the *same* ordering, so Go and Python agree
with each other and both differ from this rule. They differ from it on exactly one thing: a
character above the basic multilingual plane is a surrogate pair beginning `D800`–`DBFF`, so
UTF-16 places it **before** `U+E000`–`U+FFFF`, where a code-point comparison places it after.

That is why this is written down rather than left to the phrase "sorted lexicographically".
Three implementers reading that phrase in three languages produce two orderings and each
believes they followed the rule. `compose-collation-utf16` in the conformance vectors is the
fixture that separates them.

### No member without a value, and no null anywhere

**A member is never emitted without a value.** There is no empty token: no `"mtu":,` and no
`"mtu":null` standing in for a value that could not be derived. A member is either present
with a value or absent. A canonicaliser handed a member with no value **must return an
error** and must not produce output — output that parses nowhere, carrying a clean digest and
a nil error, is the worst failure this format can have, because both sides can agree on it.
Where a value cannot be derived, the containing member is omitted by the rules of §8, or the
composition refuses.

**Text is Unicode, and an escape that denotes no character is an error.** A `\uXXXX` escape
naming an unpaired surrogate — `\ud800` with no low surrogate following, or a low surrogate
alone — is valid JSON *syntax* and denotes no Unicode scalar value, so it has no UTF-8 encoding
at all. Such a package is refused. So is one whose bytes are not valid UTF-8.

**Never substitute `U+FFFD`.** That is what most decoders do by default, and it is the worst
available behaviour here: it is silent, it changes the bytes, and it therefore changes the
digest, with nothing reported on either side and no field that differs. A decoder that replaces
invalid input has composed a document that was never signed.

**A null is an error at any depth, on input as much as on output.** The paragraph above governs
emission; this governs what a package may contain. `"cut": null`, `"measured_mtu": null`, and a
null *element* inside `release` or `transports` are each a malformed package and each refused —
not an absent member, and not a member to be normalised away.

**This is a rule about what a package and an agreement may contain, not a rule about what the
canonicaliser may write.** The distinction matters and has been read the wrong way round: the
canonicaliser **serialises a null faithfully, as `null`**, and it is structural validation that
refuses the package.

Two reasons, and the second is the one that bites:

- The canonicaliser is how a receiver reproduces bytes it did not produce. One that refused a
  null could not render a package containing one at all — so it could not show an operator what
  was rejected, which is the moment a faithful rendering is most wanted.
- **The findings are different, and only one of them is right.**
  "Null at `transport_offer.measured_mtu`" names the rule and the member. "Canonicalisation failed" names neither, and
  sends the reader to the serialiser, which is working correctly. An implementation that
  enforces this rule inside its canonicaliser cannot produce the first finding.

Contrast the paragraph above: a member with **no value at all** — a nil with nothing behind it,
the `"mtu":,` case — *is* a serialisation error, because there is nothing to write. A null is a
value, and writing it is the canonicaliser's job. The two rules sit next to each other and are
enforced in different places.

It must be a refusal rather than a repair, because **the signature covers the null**. A receiver
that quietly dropped `"cut": null` would verify one document and compose a different one, and
the two parties would then each be normalising on their own judgement — which is precisely how
one pair of files yields two digests. The rule holds in an agreement too: nothing in §8.1 may be
null, and an implementation that emits one has produced output no other implementation will
reproduce.

## 5. Signature

Ed25519 over the canonical form of the package with the `signature` member removed. Detached,
placed at the top-level member `signature`, encoded **base64 with the standard alphabet of
RFC 4648 §4** — `A–Z a–z 0–9 + /`, padded with `=`. Not the URL-safe alphabet of RFC 4648 §5;
a value containing `-` or `_` is malformed, not something to be decoded a second way.

Decoding is **strict**. Reject any character outside the alphabet, whitespace and line breaks
included; reject padding that is missing, doubled, or anywhere but the end; reject a final
quantum whose unused bits are not zero. An Ed25519 signature is **exactly 64 bytes**, which is
**exactly 88 base64 characters ending in `==`**. Any other length is refused before verification
is attempted, so a wrong-shaped value fails as a malformed package rather than as a bad
signature — two different findings that would otherwise look alike.

Two padding characters, because 64 bytes is 21 whole three-byte groups with **one** byte over,
and a final quantum of one byte pads with two `=`. The verifying key of §5.1 is 32 bytes, which
is 10 groups with **two** over, so it pads with one. **The two values do not have the same
padding, and an implementer who assumes they do gets one of them wrong** — the numbers are given
in both places for that reason.

**Why a bare detached signature rather than a signing envelope.** An envelope format such as
DSSE or COSE introduces a *second* canonical form — DSSE signs a pre-authentication encoding,
COSE signs CBOR — so an implementation would have to agree with its counterpart on two
serialisations instead of one. That doubles the surface for the exact disagreement the digest
comparison exists to eliminate. Those formats earn their complexity through algorithm agility
and payload-type binding across many artifact types; this specification has one payload type
and one algorithm.

The property an envelope would give that matters is **domain separation** — signing *"this is
a federation package"* rather than raw bytes. That is already satisfied, because the canonical
form includes the `spec` member, so the signed bytes describe what they are.

Algorithm agility is not lost. **The `spec` version is the migration path**: a future version
changes the algorithm, and composition refuses any package whose `spec` is not exactly this
version (§6 step 1).

State the protection precisely, because the loose form of it is false. A downgrade attack does
not have to move one side. It can hand *both* parties packages declaring a version neither
implementation has — packages that are perfectly consistent with each other, and so pass any
check that merely compares the two. What the literal pin gives is the property that actually
holds:

> An implementation of this version verifies signatures under **this** version's algorithm, and
> only over packages that declare **this** version. A package written for an older or a newer
> lineage is refused unread, in either direction.

What it does not give, and no format rule can, is protection for a party still running a version
whose algorithm has since been broken. Withdrawing a version is a decision for its operators.
The pin is what makes that decision effective rather than advisory.

The verifying key is the one **bound to that party's `sovereign_id` over the channel of
§5.1**. It is never taken from the package: a package carrying its own key authenticates
nothing.

A party rotating its key does so through the same human-approved path as the first binding —
a key change is exactly as significant as a first appearance, and is treated as one. Where a
directory is in use it may carry the rotation, and an update published that way must also be
signed by the key it replaces; that makes a directory compromise detectable, and does not
make the directory the source of the binding. See §5.1.

### 5.1 The two values that need an authenticated channel

Two values must reach the counterparty over a channel that is itself trustworthy:

| Value | What depends on it |
|---|---|
| The party's **Ed25519 signing public key** | Every signature check, and therefore composition step 0 (§6) |
| **`identity.ca_fingerprint`** | The trust anchor for that party's peering certificates |

They are established in the same act, once per party, and again only if that party changes
them. Everything else in this specification is machine-checkable **because** these two were
established some other way.

The requirement is not "out of band" in the loose sense of *some other channel*. It is
specific:

> Both values must be confirmed over a channel **authenticated independently of the
> relationship being established**.

Two properties, and both are necessary:

- **Independent of this link.** You cannot authenticate the material that secures a channel
  over that same channel. The circularity is total, not merely inelegant.
- **Authenticated by something that already exists.** A confirmation is worth exactly as much
  as your ability to know who you are talking to. A telephone call to a person whose voice you
  recognise qualifies. A meeting qualifies. A signed instrument through an established
  diplomatic channel qualifies. **A call to a number the counterparty supplied over the
  unauthenticated channel does not** — that authenticates nothing, and merely feels like it
  does.

#### How these two values are written down

The channel above is a telephone call, a meeting, or a courier. None of them carries a file
extension, so neither value's encoding can be left to be inferred — and the plausible guesses
for a public key (raw hex, base64, PEM `SubjectPublicKeyInfo`, OpenSSH `ssh-ed25519 AAAA…`)
either decode to different bytes or do not decode at all.

> The verifying key is the **32-byte raw Ed25519 public key**, encoded base64 with the same
> alphabet as a signature (§5) and padded: **exactly 44 characters, ending in one `=`.**
> One `=`, not two — a signature is 88 characters ending `==`.
>
> `identity.ca_fingerprint` is `sha256:` followed by **exactly 64 lowercase hex characters**.

**That asymmetry is intentional, and it follows from the payloads.** Base64 encodes three bytes
to four characters and pads the remainder: 32 bytes is ten whole groups with **two** over, which
pads with one `=`; 64 bytes is twenty-one groups with **one** over, which pads with two. The two
values are different lengths, so they take different padding, and neither count is a typo for the
other. Both are given with their arithmetic so a reader can confirm rather than trust — a
cross-reference saying the key is encoded "like a signature" is the kind of half-truth nobody
re-derives, and it invites someone to "correct" whichever one they saw second.

Not PEM, not DER, not OpenSSH — and this matters because **PEM is what most libraries hand
you**. An Ed25519 public key exported by OpenSSL, or by Python's `cryptography` at its default
encoding, is a `SubjectPublicKeyInfo`: the same 32 bytes behind a fixed 12-byte DER prefix,
`302a300506032b6570032100`. Its base64 is 60 characters and always begins `MCowBQYDK2Vw`, which
is a reliable tell that the wrong thing is being transcribed. Every Ed25519 library has a raw
public-key entry point; that is the one to use.

**Reading a value aloud.** Read it in **groups of four characters, stating the number of groups
first** — eleven groups for a key, sixteen for the hex of a fingerprint — spelling each
character with an alphabet that distinguishes upper from lower case. The receiving party then
reads the whole value back. Both halves are needed and neither is redundant: the group count
catches a dropped or doubled character, which a readback alone can slide past, and the readback
catches a transposition or a substitution, which the count cannot see at all.

This is the irreducible human step, and it is the same problem as accepting a server's key the
first time you connect to it: it cannot be solved cryptographically from a standing start.
Two parties with genuinely no prior relationship solve it institutionally — a meeting, a
courier, an exchange witnessed by a body both already recognise — not with a better protocol.

**A directory is a cache of this binding, never its source.** A shared directory makes the
binding convenient to look up and lets a party publish a rotation once rather than once per
counterparty. It establishes nothing: a key learned only from a directory is a key learned
from whoever runs the directory. Two parties who share no directory federate exactly as well
as two who share one — that is the test of whether this specification depends on one, and it
must not, because a directory both parties are obliged to trust is precisely the central
authority this design exists without.

## 6. Composition

### Where `now` comes from

**`now` is the current date in UTC**, formatted `YYYY-MM-DD`. Never the local date of whoever
happens to be composing.

§4 took clock skew out of the packages by carrying dates and no times. Leaving `now` unstated
puts it straight back, one layer down, in the evaluator. Two parties in different countries
composing the same pair either side of local midnight would otherwise reach a refusal on one
side and an agreement on the other, on the day an `expires` falls due — a digest comparison that
cannot even be run, and no field to point at.

It is **invisible in development**, which is why it is stated here rather than discovered: the
conformance vectors supply `now` from `now.txt`, so a wrong source is never exercised. The one
fixture whose verdict turns on this rule is `compose-expiry-boundary`, where an `expires` equals
`now` exactly; every other expiry in the set is months away.

### Structural validation comes first, and the mechanism forces it

**A package is checked against the structural rules of §3 and §4 before any of the steps below
run.** This is not a preference between two workable orders. The other order is unimplementable.

Verifying a signature means computing the signing input, and the signing input is a canonical
form. **A malformed package may not have one.** A package carrying `1380.0` cannot be
canonicalised at all — §4 permits no such token, so there is no byte string to verify a signature
against, and an implementation that genuinely put verification first would have to fail on a
package it cannot serialise. It would then report *"unverifiable signature"* for a package whose
signature is fine.

Two consequences, both worth stating because each has been read the other way:

- **"Step 0 is first" means first within this section**, not first overall. Structural validation
  is not step −1; it is a different phase, applied to one package, before there is a pair to
  compose.
- **A malformed package never produces a composition finding** — not even when its signature
  would also have failed. The most specific true statement wins: *"null at
  `transport_offer.measured_mtu`"* sends a reader to the member that is wrong, while *"signature
  did not verify"* sends them to a key exchange that is working. An implementation reporting the
  second for a malformed input has the right verdict from the wrong phase, and has cost someone
  an afternoon.

### The steps

Given two well-formed packages, `P` and `Q`, and an evaluation date `now`:

0. **Refuse** unless each package's signature verifies under the key bound to that package's
   `sovereign_id` per §5. Nothing below means anything on an unauthenticated package, so this
   is first, and there is no partial result before it.
1. **Refuse** unless each package's `spec` is **exactly `federation-package/v1`** — a literal
   pin against the version of this document, not a comparison of the two packages with one
   another. Never compose across versions: two implementations producing two digests is a false
   alarm, and false alarms are fatal here.

   **Equality between the two packages is not sufficient, and the gap is an attack.** Two
   packages both declaring `federation-package/v2` are equal to each other, so an implementation
   that only compares them composes them under v1 rules and emits a v1 agreement — verifying
   under Ed25519 a signature that a later version may have specified some other algorithm for.
   Nothing about either package looks irregular, because the attack moves both sides at once.

   The pin refuses the mismatched pair and the both-wrong-version pair alike, and it refuses
   them **here, at step 1**, rather than earlier in structural validation. That placement is
   deliberate: both cases then produce the same named composition refusal, and an implementation
   that reports which rule fired (§10) reports the same one for both.
2. **Refuse** if `sovereign_id` is equal, or if `identity.trust_domain` is equal. Two parties
   must be distinguishable at every layer.
3. **Refuse** if either package's `expires` is **before** `now`. The boundary is **inclusive**:
   a package whose `expires` equals `now` is valid that day and composes. A term runs through
   the end of its `expires` date, not up to its start.
4. **Refuse**, *only when both packages carry `transport_offer`*, if either
   (a) their `capabilities.transports` share no entry **that appears in the preference order of
       §7**, or
   (b) neither declares `listener: true`.
   Both parties have said they want a link and the pair cannot produce one, which is a
   disagreement worth stopping on. When they do not both carry `transport_offer`, neither
   check applies: no link was asked for, and there is nothing to fail.

   Check (a) is stated against the preference order, not against the raw intersection, and the
   difference is load-bearing. **`capabilities.transports` is an open list.** A party may name a
   transport this version has never heard of, and two parties may name the same one — two
   packages each offering `["quic"]` do intersect. A bare intersection test passes them, and
   composition then arrives at §7's selection holding a shared entry that the preference order
   does not name, and so with nothing to choose — a state this document would not otherwise
   define. Testing against the same
   ordered list that selection walks means the pair either refuses here or selects there, and
   never neither.

   Keeping the list open is the point: a later version adds a transport by extending one ordered
   list, and a party may offer one today without anything having to change.
5. Sort the two into `parties[0]`, `parties[1]` by `sovereign_id`, ascending, by UTF-16 code
   unit (§4). A `sovereign_id` is `a-z0-9-`, so every collation agrees here; the rule is stated
   for the same reason the others are, and so that there is one comparison in the document.
6. Derive the negotiated fields by the rules in §7.
7. Emit the agreement object of §8.

**Composition has exactly seven ways to refuse**: bad signature, wrong `spec`, same
`sovereign_id`, same `trust_domain`, expired, no usable transport in common, neither listener.
The conformance vectors carry a fixture for each, and two apiece for the rules where a wrong
implementation passes one and fails the other: the `spec` pin (one mismatched pair, one pair both
on a version that is not this one) and 4(a) (one disjoint pair, one sharing a transport that the
preference order does not name).

A **malformed** package is a different thing and is refused before composition begins, by the
structural rules of §3 and §4: a missing member, a null, a non-integer number, an empty
set-array, a partial `data_offer` half, a signature of the wrong length. Those refusals are not
in this list and are not numbered, because they are properties of one package rather than
verdicts on a pair.

Refusal is an outcome, not an error condition: it is the mechanism working.

## 7. Negotiated fields — every rule is deterministic

| Field | Rule |
|---|---|
| `expires` | The **earlier** of the two packages' `expires`. A composed link inherits the shorter term, so it has one, and neither party's term is silently extended by the other's. |
| `transport` | Emitted **only if both** packages carry `transport_offer`. Otherwise the whole member is absent — one party willing to connect is not a link. |
| `transport.kind` | Walk the fixed preference order `["wireguard", "ipsec-fips"]` and take the first entry present in **both** `capabilities.transports`. Reached only when both packages carry `transport_offer`, and only when §6 step 4(a) has already found such an entry, so the empty case does not arise here. Entries either party names that are not in the preference order are ignored. The order is part of this specification, not a local setting — a local preference would break determinism. **A party that requires IPsec declares `ipsec-fips` and not `wireguard`**; the intersection then has one member and this order is never reached, so the exception costs a declaration rather than a change to this specification. |
| `transport.mtu` | `min(P.measured_mtu, Q.measured_mtu)`. Never an average, never a default. |
| `transport.link_subnet` | The `proposed_link_subnet` of `parties[0]`. Position decides, so both sides agree without negotiating. |
| `transport.listener` | From the two `listener` booleans: exactly one `true` → that party. Both `true` → `parties[0]`. Neither `true` → **refuse: neither party will accept a connection.** The member holds the party that **listens**, and is named for what it holds. |
| `flows[]` | One entry per direction that is fully specified, ordered `parties[0]` first. **Direction X→Y is emitted only if X carries `data_offer.release` and Y carries all three `accept_*` values.** A direction missing either half is not emitted; a pair where neither direction is complete yields `flows: []`. A flow is never emitted with a limit it does not have. |
| `envelope` per direction | The **receiver's** declared `accept_*` values. You cannot be obliged to accept more than you declared, and the sender cannot raise your ceiling. |
| `parties[].fabric_fips_image` | Carried **verbatim, per party**. Never combined into one value. Both `false` and one-of-each are different situations, and a single boolean cannot tell them apart. |
| `parties[].peering_cn` | Derived, never declared: `fed.<this party>.<other party>.01` (§3.1). |
| `flows[].publish_under` | Derived, never negotiated: `from/<sovereign_id>/**` for the originating party (§3.2). |
| `flows[].release` | The originating party's own `data_offer.release`, normalised per §4 — de-duplicated and sorted by UTF-16 code unit, and otherwise unaltered. Never merged with the counterparty's, never widened. There is no wildcard form of a release set. |

### Policy differences are rendered locally, never composed

An earlier draft combined `fabric_fips_image` into one top-level boolean and appended an
English sentence to a `decisions` array when the two parties disagreed. Both are gone.

**Free text inside a digest guarantees that two conforming implementations disagree.** The
digest is the one thing in this format that must be byte-identical on both sides; a sentence
composed by an implementation is the one thing that will not be. Two correct implementations
would have produced two digests and a false mismatch, which §9 says destroys confidence in
the mechanism permanently.

The facts go in the agreement, per party. **What to do about a difference is a local
decision**: each party reads `parties[n].fabric_fips_image`, applies its own policy — a
coalition profile, an accreditation, a waiver — and renders the outcome in its own tooling.
Two parties may legitimately reach different conclusions from the same agreement, and the
agreement stays identical on both sides while they do.

## 8. The agreement

### 8.1 Complete schema

Every member the agreement may carry, its type, and when it is present. **No other member is
defined**, and nothing here may be emitted without a value (§4).

**Top level**

| Member | Type | Present |
|---|---|---|
| `spec` | string, exactly `federation-agreement/v1` | Always |
| `expires` | string, `YYYY-MM-DD` | Always. The earlier of the two packages' `expires` |
| `parties` | array of exactly two party objects, ascending by `sovereign_id` | Always |
| `transport` | object | **Only if both packages carry `transport_offer`.** Otherwise absent — not null, not empty |
| `flows` | array of zero, one or two flow objects | Always. May be `[]` |

**`parties[n]`** — both entries carry all five members.

| Member | Type | Source |
|---|---|---|
| `ca_fingerprint` | string, `sha256:`-prefixed | Verbatim from that party's `identity` |
| `fabric_fips_image` | boolean | Verbatim from that party's `capabilities`. Never combined |
| `peering_cn` | string | Derived: `fed.<this party>.<other party>.01` |
| `sovereign_id` | string | Verbatim |
| `trust_domain` | string | Verbatim from that party's `identity` |

**`transport`** — when the member is present, all four are present.

| Member | Type | Source |
|---|---|---|
| `kind` | string, one of `wireguard`, `ipsec-fips` | Intersection, then the fixed preference order (§7) |
| `link_subnet` | string, CIDR | `parties[0].transport_offer.proposed_link_subnet` |
| `listener` | string, a `sovereign_id` | The party that listens (§7) |
| `mtu` | integer | `min` of the two `measured_mtu` |

**`flows[n]`** — every entry carries all five members, and every `envelope` all three.

| Member | Type | Source |
|---|---|---|
| `envelope.bytes_per_message` | integer | Receiver's `accept_bytes_per_message` |
| `envelope.distinct_keys` | integer | Receiver's `accept_distinct_keys` |
| `envelope.hz` | integer | Receiver's `accept_hz` |
| `from` | string, a `sovereign_id` | The originating party |
| `publish_under` | string | Derived: `from/<from>/**` |
| `release` | array of strings, non-empty, de-duplicated, sorted by UTF-16 code unit per §4 | The originating party's `data_offer.release`, normalised per §4. Not merged, not widened |
| `to` | string, a `sovereign_id` | The receiving party |

**Ordering.** `flows` holds at most two entries: the flow originating from `parties[0]`
first, then the one originating from `parties[1]`. When only one direction is emitted it is
the only entry, and the rule still fixes which one it is. Members within every object are
ordered by the canonical form (§4), not by the order of these tables.

**Two shapes that are valid and are not yet a working link.** Neither is an error, and
neither should be read as one:

| Shape | What it means |
|---|---|
| No `transport` member | The parties have agreed data terms and no path. Nothing can cross yet |
| `flows: []` | The parties have agreed a path that carries nothing |

### 8.2 Example

This is the `compose-full` conformance vector, pretty-printed. Its byte-exact canonical form
and digest are in [`../../ops/federation/vectors/`](../../ops/federation/vectors/).

```json
{
  "expires": "2026-11-20",
  "flows": [
    {
      "envelope": {
        "bytes_per_message": 262144,
        "distinct_keys": 2000,
        "hz": 50
      },
      "from": "xaa",
      "publish_under": "from/xaa/**",
      "release": [
        "release/weather/**"
      ],
      "to": "xab"
    },
    {
      "envelope": {
        "bytes_per_message": 1048576,
        "distinct_keys": 5000,
        "hz": 100
      },
      "from": "xab",
      "publish_under": "from/xab/**",
      "release": [
        "release/imagery/**"
      ],
      "to": "xaa"
    }
  ],
  "parties": [
    {
      "ca_fingerprint": "sha256:d49211cac3357142f3cf9c2ee216cdd452793ce89428d5ea856f9f62e71efe55",
      "fabric_fips_image": true,
      "peering_cn": "fed.xaa.xab.01",
      "sovereign_id": "xaa",
      "trust_domain": "spiffe://xaa.example"
    },
    {
      "ca_fingerprint": "sha256:94ff6f82cb3cab319ad7fe073f808dfd58af2a7ce43e32419c3565f0044e97b7",
      "fabric_fips_image": true,
      "peering_cn": "fed.xab.xaa.01",
      "sovereign_id": "xab",
      "trust_domain": "spiffe://xab.example"
    }
  ],
  "spec": "federation-agreement/v1",
  "transport": {
    "kind": "ipsec-fips",
    "link_subnet": "169.254.10.0/31",
    "listener": "xaa",
    "mtu": 1380
  }
}
```

### 8.3 Nothing that changes the bytes is left to the vectors

The example above is pretty-printed, and the vectors hold its byte-exact form. That is a
convenience, and it must not become the only place a rule lives. **The vectors are normative
(§10), and an external partner may not have them** — they sit in a repository; this document
travels by email, in a meeting pack, as an attachment to an interconnection agreement.

So: **every rule that changes the bytes, or changes whether an agreement exists at all, is
stated in prose in this document.** The vectors confirm an implementation. They are not where it
learns the rules, and an implementer who works only from this document must be able to produce
byte-identical output.

That is a checkable claim, so here is the whole list of what it takes, and where each is:

| Rule | Where |
|---|---|
| Serialisation is RFC 8785 JCS; member names ordered by UTF-16 code unit | §4 |
| Set-array elements de-duplicated and ordered by UTF-16 code unit — one collation, everywhere | §4 |
| The signer is obliged to sort; the composer normalises regardless | §4 |
| Integers only, bounded to ±(2^53 − 1); a fractional part or exponent in the token is refused | §4 |
| A null is an error at any depth, in a package and in an agreement alike | §4 |
| A member name occurs at most once; a duplicate is refused, never resolved | §4 |
| A leading zero, a leading plus and `-0` are refused number tokens | §4 |
| An unpaired surrogate, and any input that is not valid UTF-8, is refused — never replaced with `U+FFFD` | §4 |
| No member is ever emitted without a value | §4 |
| Dates are `YYYY-MM-DD` | §4 |
| Signature: Ed25519, exactly 64 bytes, base64 standard alphabet, padded, strictly decoded | §5 |
| The verifying key: 32 raw bytes, base64, exactly 44 characters | §5.1 |
| `now` is the current UTC date | §6 |
| `spec` is pinned literally, not compared between the two packages | §6 step 1 |
| `expires == now` composes — the boundary is inclusive | §6 step 3 |
| Transport compatibility is tested against §7's preference order, not the raw intersection | §6 step 4 |
| Party order: ascending `sovereign_id`, by UTF-16 code unit | §6 step 5 |
| Every negotiated value, including the transport preference order itself | §7 |
| Every member an agreement may carry, and the condition under which each is present | §8.1 |
| Flow order: the flow originating from `parties[0]` first | §8.1 |
| The digest input: canonical UTF-8 bytes, no trailing newline, no byte-order mark | §9 |

An implementation that satisfies all of these and still disagrees with a vector has found a
wrong vector, and the vector is fixed (§10).

## 9. Digest

**The input to the hash, exactly:** the canonical form of the agreement (§4) encoded UTF-8, and
nothing else — no trailing newline, no byte-order mark, no leading or trailing whitespace of any
kind. `sha256` over those bytes; the digest is `sha256:` followed by the lowercase hex of the
32-byte result.

That is not pedantry, and the failure it prevents is common rather than exotic. A canonical form
is a byte string, not a text file, and the ordinary way to hash one is to write it to a file
first — at which point most editors, `echo`, here-documents and several standard libraries add a
newline on your behalf. The §8.2 agreement digests to
`sha256:b29bb23071a5c668d0d0f2a3aec0a1c85337ed8a9d4cf94feae1f052d34410df`. The same agreement
with one `\n` appended digests to
`sha256:27f0dcf0f79994f7e4e463151a302b058458bb803fe83661412b48cc1e63d5ad`.

**This is the number the two parties compare, and the channel does not need to be secure.**
An adversary who alters a digest in transit causes a *visible mismatch*, which is
investigated. To cause a false *match* they would have to make both sides independently
compute the same wrong agreement, which means forging a signature. So this comparison may
travel by any means at all, including one nobody trusts.

Contrast §5.1: the fingerprint that bootstraps trust has the opposite requirement.

- Equal → both hold the identical agreement. Transcription error is now impossible.
- Different → each side prints its composed agreement and diffs them. The offending field
  names itself. Loud, in the first hour, instead of silent for three weeks.
- **Different, and the two printed agreements are identical** → the disagreement is in the bytes
  hashed, not in the agreement, and the diff above has nothing to show. Compare the two byte
  **lengths** before anything else. A difference of one is a trailing newline on one side; three
  is a byte-order mark; equal lengths with differing digests means the two sides encoded the
  same characters differently, and §4 says UTF-8.

  This third case is worth stating because without it the result reads as impossible — two
  identical documents, two digests — and an impossible result is investigated by re-running
  everything rather than by measuring one number.

  **The length rule assumes both sides hashed a canonical form.** Where that premise fails the
  arithmetic stops meaning anything, and two ways it fails are common enough to name:

  - **An indented rendering was hashed instead of the canonical form.** The lengths then differ
    by an arbitrary amount and the difference says nothing about the cause. A tool implementing
    this diagnostic should check that each side is canonical before reporting a length verdict.
  - **One side is UTF-16 and the document is ASCII.** Such a byte string is *valid UTF-8*, because
    the padding bytes are `U+0000`, so no validity check rejects it and it does not present as an
    encoding error. Re-decoding the unreadable side as UTF-16 and asking whether it is the same
    agreement distinguishes this from a genuinely corrupt input.

## 10. Conformance

Agreeing is not the same as being able to deliver. Before any traffic, each party runs its
own configuration generator against the agreement **without applying it** and asserts the
output would be valid on its own system.

Each then publishes one line: *"conformance ok for `sha256:...`"*, signed with the same key.

At that point, and only then, both properties hold: the two sides provably hold the same
agreement, and each has proved it can implement it.

**The conformance vectors are normative.** They live in
[`../../ops/federation/vectors/`](../../ops/federation/vectors/), in two kinds. **Pairs**: thirteen
composing pairs and a `refuse-*` fixture for every rule in §6, each with byte-exact canonical
forms, signing inputs, signatures and digests. **Single packages**, in `malformed/`: one per
structural rule of §3 and §4 that changes a verdict. The second kind exists because §4's rules
fire on inputs a well-formed pair cannot contain — a pair's packages come from a conforming
serialiser, so no pair can carry an empty number token, a duplicate member name or an unpaired
surrogate, however many pairs are written. Every fixture is built so that only its own rule fires, and a conforming implementation
must report **which** rule fired: a fixture refused for the wrong reason is a passing test hiding
a broken implementation. An implementation that disagrees with a
vector is wrong. A vector that disagrees with this document is wrong, and is fixed there.

They were derived by hand from this document rather than produced by our implementation, which
is what makes them a test of it rather than a description of it.

## 11. What this specification deliberately leaves out

- **Any notion of authority.** Nothing here grants access to anything. A package is a
  statement of intent and capability. Enforcement lives in each party's own configuration,
  derived from the agreement, and is that party's business alone.
- **Transport of the packages.** Email, USB, a web directory, a courier. Because packages
  are signed and agreements are digest-compared, **the channel does not need to be trusted**,
  and adding a new one later is not a security decision.
- **Revocation of an agreement.** Out of scope for v1: each party severs unilaterally per
  its own cut procedure. There is no distributed state to revoke, by design.
- **Whether a policy difference blocks.** The agreement carries the facts, per party. What
  each party does about a difference is that party's policy, applied locally, rendered
  locally, and never inside the digest (§7).
- **A directory.** Convenient, and never required. Two parties who share none federate
  exactly as well (§5.1).
