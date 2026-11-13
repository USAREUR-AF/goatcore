# Federation package and agreement v2 — proposed normative companion

Status: **designed, not built; owner decision required**.

This companion defines `federation-package/v2`, `federation-agreement/v2`, and dual-version
dispatch. The v1 document, readers, emission, composition entry point, and normative vector bytes
remain unchanged.

## 1. Identifiers and explicit selection

| Artifact | Exact identifier |
|---|---|
| Package | `federation-package/v2` |
| Agreement | `federation-agreement/v2` |

V2 is selected per peering by reviewed configuration. It is never selected from field shape,
missing endpoint, counterparty input, or parser fallback. V1 remains the default. A party that
cannot truthfully emit required v1 fields refuses under v1 and names coordinated v2 adoption as
the remedy.

V2 packages compose only with v2 packages and emit only agreement/v2. A v1/v2 pair refuses. A
reader never retries v1 after a v2 failure, translates a signed package, invents an endpoint, or
silently downgrades.

## 2. V1-pinned conformance versus general dispatch

These are distinct public operations:

| Operation | Purpose | Version behavior |
|---|---|---|
| `load_v1` / `compose_v1` | Prove the unchanged v1 implementation against the existing corpus | Applies v1 structure/signature rules, then the literal v1 spec pin at v1 composition step 1. Any non-v1 spec, including two v2 strings, yields the existing v1 version refusal. |
| `dispatch_compose` | Compose supported production versions | Applies the complete routing-safety scan, then obtains routing hints. Two routing-eligible `federation-package/v1` hints invoke the complete frozen v1 parse, signature, and composition path on the original bytes. Other supported routes authenticate the common envelope, compare authenticated versions, then invoke exactly one body parser/composer. |
| `dispatch_lint` | Structural lint of one package, without a bound key | Applies the complete routing-safety scan, then invokes the selected version's structural parser. It never authenticates and never describes the hint as an authenticated claim. |

The first `spec` read is an **untrusted routing hint**. It can select a parser or cause an
unsupported-version refusal; it is not accepted as a party's claim until the package signature
has verified under that party's externally bound key. No field from an unsupported version is
interpreted and no signature algorithm is guessed.

The production dispatcher and version-pinned v1 conformance path deliberately have different
boundaries. The frozen v1 entry points do not acquire the new pre-routing scan. Production
dispatch does, so an input with invalid UTF-8, JSON/token form, duplicate decoded members, null,
an invalid number token/range, or an unpaired surrogate can be refused before it reaches the v1
parser and can receive a different `json.*` or `dispatch.*` diagnostic. This proposed diagnostic
change requires owner approval. It does not change any v1 normative vector byte or expected
version-pinned v1 result.

### 2.1 Complete routing-eligibility algorithm

Routing never calls a general-purpose JSON decoder and asks what it happened to accept. A
version-independent byte scanner performs these operations in order on the entire input:

1. Validate UTF-8 without replacement. A bad byte is `json.invalid_utf8`; scanning stops at its
   byte offset, retaining a path only if the token stack already establishes one.
2. Tokenize exactly one RFC 8259 JSON value with no BOM, trailing non-whitespace, trailing comma,
   or missing value. Missing values and other JSON grammar failures are `json.syntax`. A leading
   plus or forbidden leading-zero numeric form is recognized as `json.number_token` for a stable
   diagnostic; recognizing it does not accept, repair, or route the input. The top-level value
   must be an object; a valid non-object root is `dispatch.root_wrong_type` at step 1.
3. Decode every string escape itself. Reject an unpaired surrogate as
   `json.unpaired_surrogate`; never accept a library's U+FFFD substitution.
4. Compare object member names **after escape decoding** and reject a duplicate at any depth.
   Thus `"spec"` and `"sp\u0065c"` are the same member and together yield
   `json.duplicate_member`. One escaped spelling that decodes exactly to `spec` is eligible.
5. Refuse `null` at any depth. Check every number token before numeric conversion: integers only,
   no fraction/exponent/leading plus/leading zero/negative zero, and the exact ±(2^53−1) range.
6. After the complete safety scan succeeds, require exactly one decoded top-level member named
   `spec`. Its decoded value must be a string. Compare that scalar exactly with the supported
   identifiers. Escapes in the value are decoded before comparison; there is no case folding or
   normalization.

Steps 1–5 produce routing-safety findings even if a textual `spec` occurs earlier in the bytes.
They do not validate version-specific required, unknown, or typed body members. An input is
**routing-eligible** only after all six operations succeed. Unsupported spec is a step-1 refusal,
not an authenticated claim. `spec:null` is step-0 `json.null`; a valid boolean/number/object/array
is step-1 `dispatch.spec_wrong_type`; an invalid number token fails at step 0 first.

For a pair, candidate keys are computed from raw bytes before the scan. Step-0/1 package findings
use §3's total order. No pair route exists until both candidates are eligible. Then v1/v1 invokes
the frozen v1 path, v2/v2 proceeds through v2 authentication/body validation, and v1/v2 proceeds
through common-envelope authentication before `dispatch.version_mismatch`.

For a v1/v1 hint pair, dispatch immediately hands the original byte strings and bound keys to
the complete v1 entry point. V1 structural validation still precedes v1 signature verification;
v1 composition still uses its existing signature/spec/party/expiry/transport precedence and
existing identifiers. None of §3's new cross-version or v2 finding selection changes that path.

For structural lint, the one package first must be routing-eligible. A v1 hint invokes the frozen
v1 structural parser and a v2 hint invokes the v2 package-only rules at steps 4–8. Authentication
step 2 and every pair rule at steps 3, 8, and 9 are inapplicable. Intra-package step-8 rules still
run. V2 lint checks `sovereign_id` and signature **syntax** at step 6; it neither finds a key nor
verifies the signature. Lint has no evaluation date: it checks `issued`/`expires` syntax but never
decides whether a package is expired. Compose alone applies its explicitly supplied UTC `now`.

The existing `refuse-both-v2` and `refuse-version-mismatch` bytes stay in the v1 corpus with their
original expected v1 refusal. Dispatcher tests may reuse those immutable bytes under a separate
expectation file:

| Fixture | V1-pinned expectation | Dispatcher expectation |
|---|---|---|
| `refuse-version-mismatch` | v1 spec pin/version mismatch | `dispatch.version_mismatch` after both supported envelopes authenticate; body fields are not interpreted |
| `refuse-both-v2` | v1 spec pin/version mismatch | after both authenticate, apply §3 ordering to the two step-4 forbidden endpoints; xab (SHA-256 `31774fc83524f465b40f66da3c7a0ec52488cb4689f1ca278413629c2a68ddb3`, 815 bytes) sorts before xaa (SHA-256 `32dc4970785e40a3caadfb0258da9b58f7d3de813bcae452a334139e79ccf2c5`, 828 bytes) |

No dispatcher expectation edits a fixture or replaces its v1 normative expectation.

### 2.2 Existing malformed single-package dispatcher expectations

These exported v1 fixtures remain unchanged. V1-pinned conformance retains every original
identifier. General structural dispatch gives a separate routing expectation:

| Fixture | General dispatcher result |
|---|---|
| `invalid-utf8-byte.json` | does not route: `json.invalid_utf8`, `cut.contact` |
| `empty-number-token.json` | does not route: `json.syntax`, `transport_offer.measured_mtu` |
| `leading-zero-number.json` | does not route: `json.number_token`, `transport_offer.measured_mtu` |
| `negative-zero.json` | does not route: `json.number_token`, `data_offer.accept_hz` |
| `non-integer-number.json` | does not route: `json.number_token`, `transport_offer.measured_mtu` |
| `integer-above-range.json` | does not route: `json.integer_range`, `data_offer.accept_distinct_keys` |
| `null-member.json` | does not route: `json.null`, `transport_offer.measured_mtu` |
| `unpaired-surrogate.json` | does not route: `json.unpaired_surrogate`, `cut.contact` |
| `duplicate-member.json` | does not route: `json.duplicate_member`, second `expires` |
| `empty-release.json` | eligible v1 lint route; frozen v1 `M_EMPTY_SET_ARRAY` |
| `no-signature.json` | eligible v1 lint route; frozen v1 `M_MISSING_MEMBER` at `signature` |

These are dispatcher expectations over immutable fixture bytes. They do not replace the fixtures' frozen v1 conformance expectations.

## 3. Total validation and refusal order

This section governs cross-version routing and the v2 path. Once a v1/v1 pair is routed to the
frozen v1 entry point, v1's own complete precedence governs instead.

### 3.1 Step-major order

The v2 dispatcher uses this step order. **Step is the first sort key across both packages.** All
applicable candidates are checked for step N before any finding at step N+1 can win.

| Step | Category | Rule |
|---:|---|---|
| 0 | JSON safety | Refuse invalid UTF-8, invalid JSON/token form, duplicate object member, null at any depth, invalid integer token/range, unpaired surrogate. |
| 1 | Routing hint | `spec` must exist and be a string. Unsupported identifiers refuse here. A supported value is still untrusted. |
| 2 | Authenticated common envelope | Except v1/v1 and lint: validate the common `sovereign_id` and signature encoding, find the externally bound key, and verify the signature over the exact canonical input. The supported version is now an authenticated claim. |
| 3 | Pair dispatch | Two authenticated supported claims must name the same package version. Otherwise refuse `dispatch.version_mismatch` before body interpretation. |
| 4 | Version object membership | Invoke exactly one parser. For v2, forbidden- and unknown-member findings are selected under §3.2's total order; category rank in §12 is only the final tie-breaker. |
| 5 | Required/applicable members | Missing required member or member forbidden by `listener`/path/mode. |
| 6 | Type and scalar syntax | Wrong JSON type, boolean/integer bounds, address/DNS/reason syntax. |
| 7 | Array semantics | Empty, duplicate, or incorrectly ordered v2 address array. |
| 8 | Cross-field and pair semantics | Address-kind disagreement, path/policy mismatch, party equality, expiry, neither-listener, and unsupported negotiated transport. |
| 9 | Agreement derivation | Listener choice and deterministic agreement emission. |
| 10 | Local applicability | Before render/apply: supported profile, address applicability, port/interface ownership, listener-side translation absence, verified post-NAT source policy, policy equality, and enforcement prerequisites. |

### 3.2 Package, pair, and path order

Before parsing, each raw input receives a stable candidate key:

1. SHA-256 of the exact input bytes, compared as 32 unsigned bytes;
2. input length, numeric ascending;
3. exact input bytes, unsigned-byte lexicographic;
4. original ordinal only for byte-identical duplicates, whose findings are necessarily identical.

This key—not `sovereign_id`, signature validity, CLI argument order, or canonical output—orders
packages at every dispatcher/v2 step. After authentication the report may display the sovereign
ID, but finding selection retains the raw candidate key. Malformed or missing identity fields
therefore cannot change package order.

Within a candidate and step, paths compare segment by segment. Object-member segments sort by
UTF-16 code unit. Array-index segments sort as non-negative integers, so `[2]` precedes `[10]`.
A member segment sorts before an index segment if unlike kinds meet; a path prefix sorts before
its child. Step 0 findings without a recoverable path use the root path, then numeric input byte
offset. Finally, the category rank in §12 breaks a same-step, same-package, same-path tie.

Package findings have scope rank 0. Pair findings have scope rank 1, so a package finding wins
over a pair finding at the same step. A pair key is the two candidate keys sorted ascending; it
does not use either identity. Pair paths live below the synthetic root `@pair`, for example
`@pair.version`, `@pair.sovereign_id`, `@pair.trust_domain`, `@pair.expires`,
`@pair.transport`, and `@pair.listener`. They use the same typed-path comparison. A pair finding
records both candidate keys; it never borrows one candidate's key or path.

The total selection tuple is therefore:

```text
(step, scope-rank, candidate-key-or-sorted-pair-key, typed-path,
 byte-offset-when-applicable, category-rank)
```

**Conflicting-error vectors.** First, candidate 0 (lower candidate key) is missing
`transport_offer.dial_target` at step 5 while candidate 1 carries reserved
`transport_offer.endpoint` at step 4. Candidate 1's `package.v2.forbidden_member` wins because
step precedes scope/package. Second, candidate 0 has step-8
`package.v2.address_kind_mismatch` while the pair also has step-8
`compose.same_sovereign_id` at `@pair.sovereign_id`. Candidate 0's package finding wins because
package scope precedes pair scope within the step. Reversing CLI arguments changes neither result.

## 4. Package/v2 structure

V2 package and agreement objects are closed. The tables below are exhaustive; a member not
listed for that exact object refuses. V1 scalar rules for dates, sovereign IDs, fingerprints,
signatures, integers, key expressions, subnets, MTU, and capability values are copied into v2
unless this companion gives a stricter rule.

| Object | Exact allowed members | Requiredness and empty-string rule |
|---|---|---|
| package top level | `spec`, `sovereign_id`, `issued`, `expires`, `identity`, `capabilities`, `transport_offer`, `data_offer`, `cut`, `signature` | Required: `spec`, `sovereign_id`, `issued`, `expires`, `identity`, `capabilities`, `signature`. Optional: `transport_offer`, `data_offer`, `cut`; when present each is complete under its row. Required strings follow their named scalar grammar and are never empty. |
| `identity` | `trust_domain`, `ca_fingerprint` | Both required strings; empty refuses. |
| `capabilities` | `transports`, `fabric_fips_image`, `pqc_capable`, `fabric_version` | All required. `transports` is non-empty; `fabric_version` is a non-empty string. |
| `transport_offer` | `listener`, `public_key`, `proposed_link_subnet`, `measured_mtu`, `listener_path`, `dial_target`, `ingress_policy` | First four always required. Last three follow the listener table below. String values are non-empty. |
| `dial_target` | `address_kind`, `address`, `port` | All required and non-empty where strings. |
| `ingress_policy` | `partner`, `carrier_sources` | `partner` required; `carrier_sources` follows path applicability. |
| `partner` | `mode`, `addresses`, `reason` | `mode` required; other members follow mode applicability. |
| `data_offer` | `release`, `accept_hz`, `accept_bytes_per_message`, `accept_distinct_keys` | At least one complete send/receive half. `release` is non-empty if present; receive values are all three or none. |
| `cut` | `contact` | If `data_offer` is present, `cut` is required. Whenever `cut` is present, `contact` is its sole required member: a string of 1–512 UTF-8 bytes, no C0/C1 control, and no leading/trailing ASCII space. Empty refuses. |

No inherited member is grammar-by-implication. Its exact v2 rule is:

| Path | Exact type/value rule |
|---|---|
| `spec` | string exactly `federation-package/v2` |
| `sovereign_id` | string, 2–16 ASCII bytes, only lower-case `a-z`, `0-9`, `-` |
| `issued`, `expires` | strings exactly `YYYY-MM-DD` and real Gregorian dates |
| `identity.trust_domain` | non-empty string; v2 intentionally adds no URI normalization beyond the existing comparison-by-exact-string contract |
| `identity.ca_fingerprint` | `sha256:` plus exactly 64 lower-case hex digits |
| `capabilities.transports` | non-empty array of strings; the v1 signer sorting/deduplication obligation remains; transport negotiation recognizes only its fixed preference names |
| `capabilities.fabric_fips_image`, `.pqc_capable` | booleans |
| `capabilities.fabric_version` | non-empty string; informational and compared/emitted exactly, with no SemVer normalization |
| `transport_offer.listener` | boolean |
| `transport_offer.public_key` | non-empty string at protocol parse; the initial WireGuard applicability check requires strict standard-base64 decoding to exactly 32 bytes |
| `transport_offer.proposed_link_subnet` | non-empty string at protocol parse; v2 agreement derivation retains the v1 exact-string/CIDR contract and initial local applicability requires a usable two-address IPv4 link subnet |
| `transport_offer.measured_mtu` | integer in the global exact-integer range; no additional v2 protocol range is invented |
| `data_offer.release` | non-empty array of strings when present; v1 key-expression and composer-normalization rules remain exact |
| three `data_offer.accept_*` members | integers in the global exact-integer range, all three or none; no additional v2 range is invented |
| `signature` | strict RFC 4648 standard base64, exactly 88 characters ending `==`, decoding to 64 Ed25519 bytes; signature input is JCS of the exact object without this member |

Where this table deliberately preserves a broad v1 scalar rule, it says so explicitly; a local
applicability refusal is not smuggled into protocol parsing.

`endpoint` is a reserved v1 transport name and is specifically forbidden inside v2
`transport_offer`. It is not an extension point. At any other path it is simply an unknown
member. No other v1 name is implicitly reserved; every unlisted member receives the generic v2
unknown-member refusal.

The transport offer always has:

| Member | Type | Rule |
|---|---|---|
| `listener` | boolean | Chooses whether listener-only members apply |
| `public_key` | non-empty string | Existing WireGuard-shaped v1 rule while WireGuard is the only supported v2 transport |
| `proposed_link_subnet` | non-empty string | Existing v1 rule |
| `measured_mtu` | integer | Existing v1 rule |

Listener-only members:

| `listener` | `listener_path` | `dial_target` | `ingress_policy` |
|---|---|---|---|
| `true` | required: `direct` or `forwarder` | required | required and compatible with path |
| `false` | must be absent | must be absent | must be absent |

V1 `transport_offer.endpoint` is forbidden in every v2 package, not ignored as an extension.

### 4.1 Dial target

```json
{"address":"198.51.100.20","address_kind":"ipv4","port":51820}
```

The object is closed and all three members are required.

| Member | Type | Rule |
|---|---|---|
| `address_kind` | string | exactly `dns`, `ipv4`, or `ipv6` |
| `address` | string | canonical valid value for the declared kind |
| `port` | integer | 1–65535; strings/fractions/exponents refuse |

### 4.2 Ingress policy

Direct listener:

```json
{
  "partner": {
    "addresses": ["198.51.100.44"],
    "mode": "addresses"
  }
}
```

Forwarder listener:

```json
{
  "carrier_sources": ["192.0.2.81"],
  "partner": {
    "addresses": ["198.51.100.44"],
    "mode": "addresses"
  }
}
```

`partner` is the source policy at the first partner-facing public interface: the direct edge for
`direct`, the forwarder public interface for `forwarder`.

| Partner mode | Required | Forbidden | Meaning |
|---|---|---|---|
| `addresses` | non-empty `addresses` | `reason` | only the exact listed source addresses may reach this peering UDP port |
| `unrestricted` | valid `reason` | `addresses` | any source may reach this peering UDP port, subject to separate local authorization |

For `direct`, `carrier_sources` is forbidden. For `forwarder`, it is required, non-empty, and
names exact sources as observed by the edge after forwarder source translation. The carrier is
never unrestricted.

## 5. Deterministic scalar grammar

### 5.1 Address table

Address parsing is protocol validation, not a claim that this build can deploy the address. It
uses the numeric rules below, never a library's `is_global`, `is_private`, or equivalent, whose
results differ by language and release.

**Textual canonical form.** IPv4 is exactly four base-10 octets separated by dots; an octet is
`0` or begins `1`–`9`, and is 0–255. IPv6 is parsed into eight 16-bit words and re-rendered in
lower-case RFC 5952 form: no leading zero in a word; the longest run of at least two zero words is
compressed; the first run wins a tie; a single zero word is not compressed; and no dotted-decimal
suffix is emitted. Input must equal that rendering. CIDR, brackets, zone, port, integer, hex,
octal, URI, and alternate spellings refuse.

**Numeric protocol classes.** These predicates are exact:

| Context/family | Protocol-valid numeric values | Always refused |
|---|---|---|
| IPv4 `dial_target` or `partner.addresses` | any value from `1.0.0.0` through `223.255.255.255` except the refused ranges; this deliberately includes RFC 1918, `100.64.0.0/10`, and documentation ranges | `0.0.0.0/8`, `127.0.0.0/8`, `169.254.0.0/16`, `224.0.0.0/4`, `240.0.0.0/4` |
| IPv4 `carrier_sources` | the same, plus `169.254.0.0/16` | `0.0.0.0/8`, `127.0.0.0/8`, `224.0.0.0/4`, `240.0.0.0/4` |
| IPv6 `dial_target` or `partner.addresses` | `2000::/3` or `fc00::/7`, minus every refused transition prefix below | unspecified, loopback, `fe80::/10`, `ff00::/8`, and every transition prefix below |
| IPv6 `carrier_sources` | `2000::/3`, `fc00::/7`, or `fe80::/10`, minus every refused transition prefix below | unspecified, loopback, `ff00::/8`, and every transition prefix below |

The refused transition/embedded prefixes are `::/96` (IPv4-compatible), `::ffff:0:0/96`
(IPv4-mapped), `::ffff:0:0:0/96` (IPv4-translated), `64:ff9b::/96` and
`64:ff9b:1::/48` (NAT64), `2001::/32` (Teredo), and `2002::/16` (6to4). Both dotted forms such as
`::ffff:198.51.100.44` and canonical-hex forms such as `::ffff:c633:642c` refuse. This decision is
about unambiguous source enforcement, not a general judgement that those mechanisms are invalid.

The protocol cannot identify an IPv4 directed-broadcast address without a prefix, so it does not
guess. Local applicability checks the selected listener interface prefix and refuses a target
that is that prefix's network or broadcast address.

**Documentation ranges.** `192.0.2.0/24`, `198.51.100.0/24`, `203.0.113.0/24`, and
`2001:db8::/32` are protocol-valid specifically so normative examples and offline vectors remain
synthetic. Production local applicability refuses them as target/source values. A test harness
may mark an evaluation synthetic, but that mode cannot render, load a firewall, or apply.

Examples:

| Value | Verdict/reason |
|---|---|
| `198.51.100.44` | protocol-valid synthetic IPv4 source; production apply refuses documentation space |
| `198.051.100.44` | invalid leading zero |
| `2001:db8:44::1` | protocol-valid synthetic RFC 5952 IPv6; profile remains unsupported |
| `2001:0DB8:44:0:0:0:0:1` | invalid uppercase/non-shortest wire spelling |
| `::ffff:198.51.100.44` | invalid IPv4-embedded form |
| `::ffff:c633:642c` | invalid mapped form even though the text is canonical IPv6 |
| `64:ff9b::c633:642c` | invalid NAT64 embedded form |
| `fe80::1%eth0` | invalid zone; `fe80::1` is allowed only as a carrier source |

The recommended initial apply profile still supports IPv4 only. A protocol-valid IPv6 or DNS
value parses and signs but receives its stable unsupported-profile refusal before render/apply.

### 5.2 DNS and IDNA

The wire carries an already encoded lower-case ASCII A-label hostname. Readers do no Unicode
mapping, case folding, NFC, IDNA conversion, DNS lookup, or trailing-dot removal.

- total length 1–253 bytes;
- two or more dot-separated labels, each 1–63 bytes;
- label characters `a-z`, `0-9`, and interior `-`; first/last character alphanumeric;
- no empty label, `_`, wildcard, URI scheme/path, embedded port, or trailing dot;
- the final label must contain at least one `a-z`; therefore an all-numeric final label refuses,
  whether or not the entire name resembles an IPv4 address;
- an `xn--` label is checked only by this same ASCII grammar; implementations do not decode it,
  avoiding library-dependent IDNA behavior.

Examples: `edge.xab.example`, `123.edge.example`, and `xn--bcher-kva.example` are valid wire
syntax. `edge.123`, `198.51.100.20`, `Edge.xab.example`, `edge_xab.example`, `bücher.example`,
and `edge.example.` are invalid.

### 5.3 Unrestricted reason

The exact signed string:

- is 1–512 bytes in UTF-8;
- contains no code point U+0000–U+001F or U+007F–U+009F;
- does not begin or end with ASCII space U+0020;
- is not trimmed, normalized, wrapped, or case-folded by a reader.

`"Provider NAT has no stable egress address."` is valid. Empty, a single space, leading/trailing
space, newline, tab, NUL, or a 513-byte UTF-8 encoding refuses. The reason documents the signed
choice; it is not authorization.

## 6. Arrays, unknown members, and normalization

| Array/member class | Wire rule | Generator/composer behavior |
|---|---|---|
| `partner.addresses`, `carrier_sources` | non-empty; exact strings; strictly increasing UTF-16 code-unit order; duplicates refuse | generator validates values then sorts; duplicate declaration values refuse rather than disappear; composer copies already-valid order |
| v1-derived `capabilities.transports`, `data_offer.release` | unchanged v1 signer obligation and composer normalization | do not strengthen v1 behavior while adding v2 |
| agreement/v2 copied source arrays | exact canonical package order | no local reordering after composition |
| unknown member in v2 object | refuse | never preserve/ignore/reinterpret |
| v1 `endpoint` in v2 transport | specific forbidden-member refusal | never treated as an unknown extension or target |

Sorting happens only before a package is signed. A reader never repairs signed bytes. Array
vectors cover empty, duplicate, reverse order, mixed IPv4/IPv6 order, and a valid sorted list.

## 7. Declaration, signature, and authorization

The selected listener declares target/policy in its local federation declaration. Init validates
and signs the complete v2 package with the unchanged Ed25519/JCS mechanism; the literal v2 spec,
target, policy, and reason are signed. The counterparty cannot select the listener's policy.

Signature authentication means “the listener signed this statement.” It does not mean the
operator was authorized to widen a firewall. Unrestricted remains unsupported in the initial
profile. If owners later approve and implement it, the proposed authorization rule is:

1. `partner.mode: unrestricted` must be in the selected listener's signed package/agreement.
2. The selected listener's reviewed local configuration must carry an explicit object containing
   `enabled: true` and `agreement_digest: sha256:<64 lowercase hex>` for the exact peering. A bare
   boolean, peering-wide default, version-only value, or digest of a different agreement refuses.
3. Apply recomputes the canonical agreement digest, requires exact equality, verifies this party
   is the agreement listener, and records the local authorization source. Recomposition produces
   a new digest and therefore requires a new local authorization; expiry of the agreement ends it.
4. On a forwarder, the forwarder host requires the same peering/agreement-scoped opt-in before an
   unrestricted public partner rule; edge carrier sources remain restricted.

This creates no new approver, ticket, or waiver system. Existing control over reviewed deployment
configuration determines who may set the digest-bound opt-in. The signed reason remains evidence,
not authorization. Owners must approve or replace this policy before unrestricted is implemented;
until then the refusal is `apply.v2.unrestricted_unsupported`, not an authorization error.

## 8. Forwarder and NAT observation points

For direct reachability, listener and dialer checks are different.

**Selected listener.** The initial profile observes the packet on the declared ingress interface
at prerouting/mangle. It refuses every listener-side translation between the signed target and
the WireGuard socket: local or declared upstream DNAT, redirect, port-forward, reverse proxy,
destination address/port translation, or source translation before the policy hook. The advertised
IPv4 address must be assigned to that interface and the advertised port must equal the local
WireGuard port. An undeclared upstream translation cannot be proved locally and remains an
evidence gap, never a conformance claim.

**Dialer.** Source NAT on the dialer side is permitted. `partner.addresses` names the exact source
addresses the selected listener observes **after all dialer-side source NAT** and before any
listener-side translation. A restricted initial profile is applicable only when:

1. the signed list is non-empty, canonical IPv4, and complete for every possible egress used by
   this peering;
2. controlled pre-enable observation at the declared listener interface verifies each allowed
   path arrives as a listed address and no tested path arrives outside the list;
3. an inventory of the dialer's applicable NAT/egress policy establishes that the finite set is
   stable for the agreement term; and
4. any source-set change keeps the link down until new packages are signed and recomposed.

CGNAT is not classified by name. A CGNAT or other source-NAT service with a verified stable finite
post-NAT set can satisfy the exact-address policy. A source behavior whose possible observed set
is unknown, incomplete, or changes outside the signed set returns
`apply.v2.source_policy_unverified`. Dynamic egress does not inherently require unrestricted: a
complete supported exact-address set may cover movement within a finite pool. When no such set can
be proved, the initial profile refuses; unrestricted remains separately unsupported.

For a forwarder:

1. the public forwarder sees the partner source before its DNAT and enforces `partner` on the
   advertised public UDP port;
2. it DNATs that public port to the edge's carrier address/local peering port and masquerades, so
   the edge observes the forwarder's carrier address;
3. the edge enforces `carrier_sources` on the locally configured carrier interface and local
   peering listen port.

In a future forwarder profile, the advertised target port and local edge listen port may differ
because the managed forwarder translates them. Agreement/v2 signs the advertised target and apply
must read back the explicit mapping. The initial direct profile permits none of these
listener-side translations.

Current `partner_sources: []` means any at the forwarder. V2 never inherits that default:

- restricted agreement: local `partner_sources` must equal the agreement list;
- unrestricted agreement: empty nft source match is permitted only after the local authorization
  gate; state records mode `unrestricted`, never the ambiguous word `any` alone;
- missing agreement/policy or empty restricted list: refuse.

## 9. Agreement/v2 and enforcement consistency

Listener selection is unchanged: sole true listener; parties[0] when both; refuse when neither.
Agreement/v2 copies the selected listener's path, target, and policy into `transport`.

Agreement/v2 retains the v1 top-level `expires`, `parties`, and `flows` derivation rules. Its
literal `spec` is `federation-agreement/v2`. Every agreement object is closed:

| Object | Exact members and requiredness |
|---|---|
| agreement top level | required `spec`, `expires`, `parties`, `flows`; `transport` only when both packages offer transport |
| `parties[n]` | required `ca_fingerprint`, `fabric_fips_image`, `peering_cn`, `sovereign_id`, `trust_domain` |
| `transport` | all seven members in the table below |
| `transport.dial_target` | required `address_kind`, `address`, `port` |
| `transport.ingress_policy` | required `partner`; path-applicable `carrier_sources` |
| `transport.ingress_policy.partner` | required `mode`; mode-applicable `addresses` or `reason` |
| `flows[n]` | required `envelope`, `from`, `publish_under`, `release`, `to` |
| `flows[n].envelope` | required `bytes_per_message`, `distinct_keys`, `hz` |

No package-only name—including `signature`, `identity`, `capabilities`, `transport_offer`,
`data_offer`, `cut`, or v1 `endpoint`—is valid in an agreement object unless the table names it.
When both packages carry a transport offer, `transport` has:

| Member | Type/source |
|---|---|
| `kind` | selected common transport under the unchanged preference rule |
| `link_subnet` | unchanged v1 derivation from parties[0] |
| `listener` | selected listener's `sovereign_id` |
| `mtu` | unchanged minimum of the two offers |
| `listener_path` | exact selected-listener value, `direct` or `forwarder` |
| `dial_target` | exact selected-listener structured target |
| `ingress_policy` | exact selected-listener policy, including reason and ordered arrays |

If both packages do not carry transport offers, `transport` is absent as in v1. All v2 agreement
objects are closed, null is forbidden, and a reader validates `spec` before interpreting any
other agreement member. A direct restricted example transport is:

```json
{
  "dial_target": {
    "address": "198.51.100.20",
    "address_kind": "ipv4",
    "port": 51820
  },
  "ingress_policy": {
    "partner": {
      "addresses": ["198.51.100.44"],
      "mode": "addresses"
    }
  },
  "kind": "wireguard",
  "link_subnet": "169.254.10.0/31",
  "listener": "xaa",
  "listener_path": "direct",
  "mtu": 1380
}
```

Those RFC documentation addresses are valid signed protocol examples and production-inapplicable;
attempting to apply this literal example returns `apply.v2.documentation_address`.

When both packages say listener=true, the non-selected party is the dialer for this agreement.
Its own target/policy remain authenticated package statements but are not copied, enforced, or
used as failover. It renders the selected listener's target and opens no inbound peering rule.

Local source overrides are forbidden. Apply compares the agreement policy with the selected
listener package and local declared values:

- equality: continue;
- local wider or narrower: refuse and name the differing path;
- local decision to disallow the agreement: take/keep link down.

### 9.1 Initial direct enforcement profile

The initial profile is enforceable only when all of these local facts pass before rendering:

- selected transport is WireGuard, path `direct`, IPv4 target, and restricted non-documentation
  IPv4 partner sources;
- local configuration names one existing ingress interface and declares
  `listener_translation: none`;
- the target IPv4 address is assigned to that interface, is not its network/broadcast address,
  and target port equals the WireGuard listen port;
- the signed dialer post-NAT source list has the complete/stable observation evidence required by
  §8; absence, expiry, or mismatch of that evidence refuses before render;
- `(network namespace, UDP, listen port)` is owned only by this peering. Apply checks `wg`, `ss`,
  all peering state, container-published ports, every nft DNAT/redirect/forward rule, forwarding
  sysctls, and routes. If the host can transit unrelated packets at that destination port and the
  managed rule graph cannot prove otherwise, or any owner/state is unreadable, apply refuses;
- the managed host-firewall input carries the exact same source set/interface/port, with no
  override touching that peering rule. An unknown base chain that can intersect this tuple, or a
  known chain whose effective result cannot be shown equal, refuses.

The dedicated `inet` base chain is hook `prerouting`, priority `mangle` (numeric −150), policy
`accept`. This observes original destination address/port and the post-dialer-NAT partner source
before listener-side translation. Because listener-side translation is unsupported here, those
destination values must equal the advertised target and local socket.
Its semantic rule order is:

```text
iifname <ingress> udp dport <owned-port> ip saddr { agreed-v4 } accept
udp dport <owned-port> counter drop
```

The accept is interface-scoped. The final drop intentionally is not: it rejects unlisted IPv4,
all IPv6, and packets arriving through an unintended interface. This is safe only because the
exclusive ownership check proves no other listener, peering, published container, or routed/DNAT
service uses that UDP port in the namespace. If ownership cannot be proved, the profile refuses;
it does not install the drop. Other ports, TCP, output, and established unrelated flows are not
matched. An accept in another nft base chain cannot override this drop; a drop in another chain
can narrow reachability, which is why managed-chain equality and unknown-chain refusal are gates.

The host-firewall baseline must represent the same agreement rule. A deployment override may
reject the entire peering and keep it down, but may not widen or silently narrow its sources,
interface, or port. Unrestricted, forwarder, IPv6, DNS, shared port, unreadable ownership,
unverified post-NAT source policy, listener-side translation, and unmanaged
intersecting firewall rules all refuse before apply.

After loading, apply must read `nft -j`, `wg show ... dump`, `ip -j address/link/route`, `ss`,
container-published-port state, and managed peering state. It compares hook/priority/policy, rule
order, ingress interface, protocol, owned port, exact IPv4 set, absence of an IPv6 accept, final
drop, absence of listener-side translation, local address, WireGuard port/peer, host-firewall
equality, and agreement digest. State also records the
signed dialer post-NAT set and its local evidence reference. State stores expected/observed
normalized values, verdict, and differences. Hashes are
provenance only. Isolated behavior probes must additionally show listed source acceptance and
unlisted source, IPv6, and unintended-interface rejection while unrelated ports/protocols pass.

The original #438 deployment's applicability to this initial profile is **unestablished**. The
available issue evidence does not establish its selected listener's address/interface
binding, exclusive port ownership, absence of listener-side translation, or the complete stable
post-NAT source set observed from its dialer. No conclusion about that deployment follows from the
protocol design alone.

## 10. Supported profile matrix

Parsing a valid v2 shape does not claim this build can enforce it. Local conformance runs before
render/apply and refuses unsupported profiles with a stable reason.

| Profile | Recommended first implementation | Before support exists |
|---|---|---|
| WireGuard, direct, IPv4 target, restricted IPv4 partner sources, agreement/v2 flow-capable republisher | supported after focused evidence | — |
| WireGuard, direct, unrestricted | future owner decision plus implementation/evidence | `apply.v2.unrestricted_unsupported` |
| WireGuard, DNS target | expanded scope | `apply.v2.dns_target_unsupported` |
| WireGuard with any IPv6 target/source | expanded scope | `apply.v2.ipv6_unsupported` |
| WireGuard through forwarder | expanded scope requiring both observation points | `apply.v2.forwarder_unsupported` |
| `ipsec-fips` | no v2 contract yet | `compose.v2.transport_unsupported` before emitting an agreement, plus defensive apply refusal |
| any other transport | not in v2 | refuse during negotiation |

The smallest useful #438 increment is the first row and includes republisher agreement/v2 support
sufficient to enforce unchanged v2 `flows`; a tunnel that cannot carry declared flows is not
called useful. If owners decline that consumer work, they must explicitly approve a
**transport-only** milestone whose apply path refuses any non-empty `flows`; this proposal does
not silently substitute that smaller claim. Existing v1 behavior remains unchanged. Supporting
another row requires its planned vectors, real task evaluation, and isolated firewall behavior
evidence; merely parsing its fields is insufficient.

## 11. Rollout and rollback

1. Ship immutable v1 entry points plus dispatcher/v2 readers while all emission remains v1.
2. Prove the full v1 corpus through the v1 path and dispatcher-specific vectors separately.
3. Prove the selected v2 apply profile end to end.
4. Both parties explicitly opt one peering into v2 and issue new signatures.
5. Mixed versions refuse; there is no negotiated fallback.
6. Compare agreement/v2 digest, pass local conformance, then apply.

An endpointless peer cannot roll back to a truthful v1 transport package. Rollback can require
withdrawing the transport offer and taking the link down until both parties resume v2. V1 remains
the default for #438 until both parties opt in.

## 12. Stable refusal mapping

This is the authoritative v2 table. Rank is the final tie-breaker in §3; it is not prose order.
V1 identifiers and precedence remain unchanged on the v1 path.

| Step.rank | Neutral/Go category | Rust `Finding.rule` | Applies to |
|---|---|---|---|
| 0.01 | `json.invalid_utf8` | `J_INVALID_UTF8` | invalid byte input |
| 0.02 | `json.syntax` | `J_SYNTAX` | invalid token/document form |
| 0.03 | `json.duplicate_member` | `J_DUPLICATE_MEMBER` | repeated object member |
| 0.04 | `json.null` | `J_NULL` | null at exact path |
| 0.05 | `json.number_token` | `J_NUMBER_TOKEN` | fraction, exponent, leading plus/zero, negative zero |
| 0.06 | `json.integer_range` | `J_INTEGER_RANGE` | outside ±(2^53−1) |
| 0.07 | `json.unpaired_surrogate` | `J_UNPAIRED_SURROGATE` | invalid Unicode scalar |
| 1.01 | `dispatch.root_wrong_type` | `D_ROOT_WRONG_TYPE` | routing-safe top-level value is not an object |
| 1.02 | `dispatch.spec_missing` | `D_SPEC_MISSING` | absent routing hint |
| 1.03 | `dispatch.spec_wrong_type` | `D_SPEC_WRONG_TYPE` | non-string routing hint |
| 1.04 | `dispatch.unsupported_spec` | `D_UNSUPPORTED_SPEC` | unsupported routing hint |
| 2.01 | `envelope.missing_member` | `E_MISSING_MEMBER` | missing `sovereign_id` or `signature` needed to authenticate |
| 2.02 | `envelope.wrong_type` | `E_WRONG_TYPE` | wrong common-envelope type |
| 2.03 | `envelope.sovereign_id_invalid` | `E_SOVEREIGN_ID` | unusable key-binding identifier |
| 2.04 | `envelope.signature_encoding` | `E_SIGNATURE_ENCODING` | malformed Ed25519 text/length |
| 2.05 | `compose.key_unbound` | `C_KEY_UNBOUND` | no externally bound key |
| 2.06 | `compose.signature` | `C_SIGNATURE` | signature does not verify |
| 3.01 | `dispatch.version_mismatch` | `D_VERSION_MISMATCH` | pair path `@pair.version`; authenticated supported versions differ |
| 4.01 | `package.v2.forbidden_member` | `M_V2_FORBIDDEN_MEMBER` | reserved v1 `transport_offer.endpoint` |
| 4.02 | `package.v2.unknown_member` | `M_V2_UNKNOWN_MEMBER` | any other unlisted package member |
| 4.03 | `agreement.v2.unknown_member` | `A_V2_UNKNOWN_MEMBER` | unlisted agreement member |
| 5.01 | `package.v2.missing_member` | `M_V2_MISSING_MEMBER` | required member absent |
| 5.02 | `package.v2.member_not_applicable` | `M_V2_NOT_APPLICABLE` | member forbidden by listener/path/mode |
| 5.03 | `package.v2.data_offer_incomplete` | `M_V2_DATA_OFFER_INCOMPLETE` | no half or partial receive half |
| 5.04 | `package.v2.cut_required` | `M_V2_CUT_REQUIRED` | data offer without cut |
| 5.05 | `agreement.v2.missing_member` | `A_V2_MISSING_MEMBER` | required agreement member absent |
| 6.01 | `package.v2.wrong_type` | `M_V2_WRONG_TYPE` | wrong JSON type |
| 6.02 | `package.v2.invalid_value` | `M_V2_INVALID_VALUE` | non-address scalar grammar/range |
| 6.03 | `package.v2.address_invalid` | `M_V2_ADDRESS_INVALID` | address text/class rule |
| 6.04 | `package.v2.dns_invalid` | `M_V2_DNS_INVALID` | DNS/IDNA wire rule |
| 6.05 | `package.v2.text_invalid` | `M_V2_TEXT_INVALID` | reason/contact bytes/control/space rule |
| 6.06 | `package.v2.signature_encoding` | `M_V2_SIGNATURE_ENCODING` | lint-only structural signature syntax; no verification |
| 6.07 | `agreement.v2.invalid_value` | `A_V2_INVALID_VALUE` | agreement type/scalar rule |
| 7.01 | `package.v2.array_empty` | `M_V2_ARRAY_EMPTY` | required non-empty array |
| 7.02 | `package.v2.array_duplicate` | `M_V2_ARRAY_DUPLICATE` | duplicate exact value; indexed path |
| 7.03 | `package.v2.array_order` | `M_V2_ARRAY_ORDER` | not strictly UTF-16 ascending |
| 7.04 | `agreement.v2.array_invalid` | `A_V2_ARRAY_INVALID` | agreement array length/order/content |
| 8.01 | `package.v2.address_kind_mismatch` | `M_V2_ADDRESS_KIND_MISMATCH` | address not of declared kind |
| 8.02 | `package.v2.path_policy_mismatch` | `M_V2_PATH_POLICY_MISMATCH` | listener path and ingress policy disagree |
| 8.03 | `compose.same_sovereign_id` | `C_SAME_SOVEREIGN_ID` | pair path `@pair.sovereign_id`; parties name same ID |
| 8.04 | `compose.same_trust_domain` | `C_SAME_TRUST_DOMAIN` | pair path `@pair.trust_domain`; parties name same trust domain |
| 8.05 | `compose.expired` | `C_EXPIRED` | pair path `@pair.expires`; expiry precedes UTC evaluation date |
| 8.06 | `compose.no_common_transport` | `C_NO_COMMON_TRANSPORT` | pair path `@pair.transport`; no supported common preference |
| 8.07 | `compose.neither_listener` | `C_NEITHER_LISTENER` | pair path `@pair.listener`; both transport offers decline listening |
| 8.08 | `compose.v2.transport_unsupported` | `C_V2_TRANSPORT_UNSUPPORTED` | pair path `@pair.transport`; selected unenforced v2 transport |
| 9.01 | `compose.v2.agreement_invalid` | `C_V2_AGREEMENT_INVALID` | derived output fails the closed agreement/v2 self-check |
| 10.01 | `apply.v2.documentation_address` | `A_V2_DOCUMENTATION_ADDRESS` | synthetic range in production applicability |
| 10.02 | `apply.v2.policy_mismatch` | `A_V2_POLICY_MISMATCH` | signed/agreed/local policy inequality |
| 10.03 | `apply.v2.unrestricted_unsupported` | `A_V2_UNRESTRICTED_UNSUPPORTED` | unrestricted profile not implemented |
| 10.04 | `apply.v2.dns_target_unsupported` | `A_V2_DNS_UNSUPPORTED` | DNS target not implemented |
| 10.05 | `apply.v2.ipv6_unsupported` | `A_V2_IPV6_UNSUPPORTED` | any IPv6 target/source not implemented |
| 10.06 | `apply.v2.forwarder_unsupported` | `A_V2_FORWARDER_UNSUPPORTED` | forwarder profile not implemented |
| 10.07 | `apply.v2.transport_unsupported` | `A_V2_TRANSPORT_UNSUPPORTED` | defensive apply check for non-WireGuard transport |
| 10.08 | `apply.v2.local_binding_invalid` | `A_V2_LOCAL_BINDING` | target address/interface, subnet, port, or WireGuard key is unusable/mismatched |
| 10.09 | `apply.v2.listener_translation_unsupported` | `A_V2_LISTENER_TRANSLATION` | advertised/local listener address or port is translated |
| 10.10 | `apply.v2.source_policy_unverified` | `A_V2_SOURCE_POLICY_UNVERIFIED` | dialer post-NAT exact set is incomplete, unstable, or unverified |
| 10.11 | `apply.v2.port_ownership_unproven` | `A_V2_PORT_OWNERSHIP` | listener/DNAT/published-port ownership not exclusive/readable |
| 10.12 | `apply.v2.firewall_interference` | `A_V2_FIREWALL_INTERFERENCE` | override/unknown chain prevents effective equality proof |
| 10.13 | `apply.v2.data_path_unsupported` | `A_V2_DATA_PATH_UNSUPPORTED` | required v2 flow consumer is unavailable |
| 10.14 | `apply.v2.enforcement_mismatch` | `A_V2_ENFORCEMENT_MISMATCH` | semantic read-back differs after apply |

Every finding carries category, exact typed path, step/rank, and implementation identifier.
Tests assert those fields; message prose is not an identifier. Before unrestricted support exists,
the only applicable unrestricted category is `apply.v2.unrestricted_unsupported`. A future
implemented profile may add a distinct digest-authorization category through an approved spec
revision; this design does not pretend authorization alone makes the profile supported.

## 13. Normative examples and planned vectors

Valid v2 package fragments:

- endpointless dialer: `listener:false`, with no path/target/policy;
- direct restricted listener: IPv4 target and sorted IPv4 partner source list;
- parser-only expanded vectors: DNS target, IPv6 target/sources, forwarder with partner and carrier
  policy, and a syntactically valid unrestricted reason; each apply attempt receives its exact
  unsupported-profile refusal.

Invalid/refusal vectors:

- missing listener target/policy; listener-only member on dialer;
- stray `endpoint`; unknown nested/top-level member;
- address array empty, duplicate, unsorted, wrong type, CIDR, DNS-as-source;
- every invalid IP/DNS/reason example in §§5–6;
- both-listener selection and unused non-selected policy; neither listener;
- mixed supported versions, unsupported version, both-v2 legacy fixture under dispatcher;
- unsupported `ipsec-fips`, forwarder, DNS, IPv6, unrestricted, listener-side translation,
  unverified post-NAT source policy, and missing-data-path profiles;
- local wider/narrower policy, documentation address, shared port, unknown intersecting chain,
  wrong interface/port, missing family drop, and rejected v4/v6 sources;
- the step-major conflicting-error fixture in §3.2, with reversed CLI order;
- immutable SHA-256 manifest of every existing v1 vector byte.
