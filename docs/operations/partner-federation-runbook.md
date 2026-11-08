# Federating with us — the partner side, end to end

**Rehearsing it end to end:** `ops/federation-partnerenv/drill/` stands a peering up between two
cloud regions, runs the checks and tears it down — `./drill.sh all`. This runbook is the real
procedure; the drill is how it gets exercised without a partner.

**Audience: an engineer at another organisation who is going to build the far half of a data
link with us.** You have your own certificate authority, your own data fabric and your own
security policy. You will not be given access to our systems, our source code or our
configuration, and you do not need any of them.

**This document is meant to be sufficient on its own.** Every fact about us that your half needs
is in our signed package, which you fetch from the peering directory (step 3). What still needs a
person is collected under **What still needs a conversation**: the call in step 1, and three
cases that depend on your platform. If you find yourself needing to ask us something else, that
is a defect in this document and we want to hear about it.

Two companion documents you should have alongside this one:

| | What it is | You need it |
|---|---|---|
| **Federation package, version 1** | The wire format and the composition rules. The contract | To build your half |
| **Conformance vectors** | ~30 worked examples with byte-exact expected output | To prove your implementation is right |

Both are public, in the goatcore repository: the specification at
`docs/design/federation-package-spec.md`, the vectors under `ops/federation/vectors/`.

**Status: this procedure has been run in rehearsal between two environments on the same
infrastructure, and never yet against a partner on foreign infrastructure.** Treat it as a
first-run procedure. Four things a same-infrastructure rehearsal cannot tell you — real path
MTU, NAT traversal, real latency and loss, and the behaviour of validated cryptographic builds
on your platform — are exactly the four things your run will establish for the first time.

---

## What you are building, in one picture

Two edge routers, one on each side, joined by one tunnel.

```
your core fabric ── your edge router ══ tunnel ══ our edge router ── our core fabric
   trusts YOUR CA        trusts OUR CA               trusts YOUR CA      trusts OUR CA
```

Three properties, and everything else in this document follows from them.

**Neither side joins the other's network.** The tunnel is a third interface, on neither party's
management plane. We do not enrol your machines and you do not enrol ours. That means you can
cut the link on your own host, immediately, without asking us and without waiting for anything
to converge.

**Your certificate authority stops at your edge.** Our CA is installed in one place on your
side: the trust store of the one process that talks to us. It must never reach a core router.
The access rules in this fabric match on the *name* in a certificate and cannot see which
authority issued it — so a foreign root in a core router is not a smaller risk than a grant,
it *is* a grant.

**The tunnel is reachability, not confidentiality.** The data's cryptography is the TLS 1.3
session running *inside* the tunnel, between the two fabric processes. Two independent layers,
and an adversary must break both.

## Before you start

- [ ] You operate your **own** certificate authority. If you do not, stop — a different pattern
      applies and this one will not work.
- [ ] You can expose **one** UDP endpoint to us, or we can expose one to you. One of the two
      must be able to accept an inbound connection; both cannot be behind NAT with no inbound
      port. Establish which before step 2, not at step 6.
- [ ] You have a host to run the edge router on. It does not need to be powerful. It **must
      not** be a machine that already runs your core fabric.
- [ ] You have someone authorised to decide what data leaves your organisation. Steps 0 and 7
      are their decisions; the rest is mechanical.
- [ ] You can write, or already have, an RFC 8785 JSON canonicalisation and an Ed25519
      signature. See **Step 2**.

## The eight steps

| # | Step | What crosses | You know it worked when |
|---|---|---|---|
| 0 | Decide what you are offering, and choose your name | Nothing | Both sides can state, in one sentence each, what they will send and what they will accept |
| 1 | **Bind two values by voice** | Your signing key and CA fingerprint, spoken | Your counterpart reads both values back to you correctly, and the group counts match |
| 2 | Build and sign your package | Nothing yet | Your own tool verifies your own signature over your own package |
| 3 | Exchange packages, compose, compare digests | Two signed JSON files, then one 64-character number | The two digests are character-for-character equal |
| 4 | Declare conformance | One signed line each | Both sides have asserted they can implement the agreement they hold |
| 5 | Bring the tunnel up, and prove it carries nothing else | Nothing new | The tunnel is established, the fabric port is reachable across it, and nothing else is |
| 6 | Stand up your edge router | Certificate authority material | Your edge holds a TLS session with our edge, and your core has never seen our CA |
| 7 | Prove the refusals, then open the flows | Data | Four deliberate violations are refused, and the agreed traffic arrives |

The order is forced. Each step's output is the next step's input, and every step is a place
where either side can stop without having exposed anything it cannot withdraw.

---

## Step 0 — Decide what you are offering, and choose your name

**Nothing is exchanged here.** This is the decision that everything downstream is scoped by,
and doing it later means building a link nobody agreed to.

### Choose your identifier

You need a short name for your organisation. It is not free-form:

| If you are | Use | Example |
|---|---|---|
| A nation | Your **ISO 3166-1 alpha-3** code, lowercased | `xaa`, `xab`, `xac` |
| A multinational body | Your established short name, lowercased | `nato` |
| A sub-entity of either | The parent, a hyphen, a local suffix | `nato-xyz` |

2 to 16 characters, `a-z`, `0-9` and hyphen only. Three rules that catch real mistakes:

- **Name the organisation, never one of its environments.** You may run a sandbox, a production
  fabric and an experimental one; they all share one identifier. An environment name here
  produces a link name that cannot survive that environment being retired.
- **A sub-entity gets its own name segment only if it runs its own certificate authority.** Not
  if it is merely a separate command, unit or directorate. The test is cryptographic
  independence, and it is verifiable rather than negotiable.
- **Never choose a word that has a meaning in a matching expression** — `any`, `all`, `*`. That
  name will appear in access rules, where a human will read it as a wildcard and it is not one.

### Decide what your half offers

Your package carries three optional blocks, and **presence is the declaration**. There is no
tier, level or ladder to claim.

| Block | Carrying it says | Omitting it says |
|---|---|---|
| `transport_offer` | "I am willing to connect" | You are not building a tunnel |
| `data_offer` — the `release` half | "I will publish these key expressions across the link" | You will send nothing |
| `data_offer` — the three `accept_*` halves | "I will accept traffic up to these limits" | You will receive nothing |

**One-way federation is normal, not degenerate.** If you will only receive, carry the three
`accept_*` values and no `release`. Do not invent a release set to satisfy a validator — an
invented key expression is later read as an intention.

**A block that is present must be complete.** One or two of the three `accept_*` values is
refused rather than defaulted, because a limit that is not stated is a limit that does not
exist.

### If you would rather not disclose your release sets before the tunnel works

You can run this in two passes, and the format supports it with no special case:

1. First package carries `identity`, `capabilities` and `transport_offer` only. Compose. You get
   an agreement with a transport and `flows: []` — a path that carries nothing. Build the tunnel
   against it and prove it (steps 5 and 6).
2. Second package adds `data_offer` and `cut`. Compose again, compare digests again, and open
   the flows.

The cost is one extra exchange and one extra digest comparison. The benefit is that a peering
that fails at step 5 — we are unreachable, the MTU is wrong, a firewall is not what someone
said — costs you nothing but time, having disclosed no namespace and no release set. Take this
option if your policy makes disclosure a decision; skip it otherwise.

Our identifier and our trust domain are in our package (`sovereign_id`,
`identity.trust_domain`), which step 3 fetches from the directory.

---

## Step 1 — Bind two values by voice

> **This is the one step that cannot be automated, cannot be checked by a machine, and cannot
> be repaired later. Everything else in this procedure is machine-checkable *because* this step
> happened. Do not delegate it, and do not skip ahead to it after the files have moved.**

Two values must reach your counterpart, and two must reach you, over a channel that is
**authenticated independently of the link being built**:

| Value | What breaks if it is wrong |
|---|---|
| The **Ed25519 signing public key** | Every signature check on every package that party ever sends |
| The **certificate authority fingerprint** | The trust anchor for that party's peering certificates |

### What "independently authenticated" means, precisely

Two properties, both necessary.

**Independent of this link.** You cannot authenticate the material that secures a channel over
that same channel. The circularity is total, not merely inelegant.

**Authenticated by something that already exists.** A confirmation is worth exactly as much as
your ability to know who you are talking to.

| Qualifies | Does not qualify |
|---|---|
| A telephone call to a person whose **voice you recognise** | A call to a number the counterparty supplied over the unauthenticated channel |
| A meeting, in person | A video call with a person you have not met, at a link sent by email |
| A signed instrument through an established diplomatic channel | An email signed with a key you learned from the same email thread |
| A courier your organisation already uses | A message in the chat channel where the packages are being exchanged |

The second column is the whole point of writing this out. **A call to a supplied number
authenticates nothing while feeling exactly as though it does** — the adversary who supplied the
number answers the phone. If the only contact detail you hold came over the channel you are
trying to secure, you do not yet have an independent channel and you must establish one.

### The exact form of each value

Neither value has a file extension, because neither travels as a file. So the encoding cannot
be left to be inferred, and the plausible guesses do not decode to the same bytes.

> **The verifying key is the 32-byte raw Ed25519 public key, base64-encoded with the standard
> alphabet: exactly 44 characters, ending in one `=`.**
>
> **The CA fingerprint is `sha256:` followed by exactly 64 lowercase hexadecimal characters.**

To produce your key in that form:

```bash
openssl genpkey -algorithm ed25519 -out fed-signing.key          # once, and keep it
openssl pkey -in fed-signing.key -pubout -outform DER | tail -c 32 | openssl base64 -A
```

**The most common wrong answer is the one your library gives you by default.** `openssl pkey
-pubout` without the `tail -c 32` emits a `SubjectPublicKeyInfo`: the same 32 bytes behind a
fixed 12-byte prefix. Its base64 is **60 characters and always begins `MCowBQYDK2Vw`**. If you
see that prefix, you are transcribing the wrong thing. Python's `cryptography` at its default
encoding does the same. Every Ed25519 library has a raw public-key entry point; use it.

To produce your CA fingerprint in that form:

```bash
printf 'sha256:%s\n' \
  "$(openssl x509 -in your-ca.pem -outform DER | openssl dgst -sha256 | awk '{print $NF}')"
```

`openssl x509 -noout -fingerprint -sha256` gives the **same value** in a different shape —
uppercase, colon-separated. Either is fine to read aloud as long as both parties know which
shape they are hearing; the value that goes in the package is the lowercase unseparated one.

### The read-aloud procedure

Read the value in **groups of four characters, stating the number of groups first**, spelling
each character with an alphabet that distinguishes upper case from lower case.

| Value | Groups |
|---|---|
| Verifying key, 44 characters | **eleven** groups of four |
| CA fingerprint, 64 hex characters after `sha256:` | **sixteen** groups of four |

Then the receiving party **reads the whole value back**.

**Both halves are needed and neither is redundant**, and they catch different errors:

| Half | Catches | Misses |
|---|---|---|
| The group count | A dropped or a doubled character — the length is wrong | A transposition; the count is unchanged |
| The readback | A transposition or a substitution | A dropped character, if the listener transcribes what they hear and reads back their own transcription |

So: count first, readback second, and the readback is from the written-down value, not from
memory of what was just said.

### How you know it worked

You do not, cryptographically, and that is the honest answer. This is the same problem as
accepting a host key the first time you connect to a server, and it cannot be solved from a
standing start. What you have afterwards is an institutional record: two named people, on a
channel each organisation already trusts, who each confirmed a value the other read back. Write
down who, when, and over what channel. That record is the trust anchor.

### Rotation

If either party later changes its signing key or its certificate authority, it goes through
**this same step again**. A key change is exactly as significant as a first appearance and is
treated as one.

**The directory moves packages; it never binds a key.** A key you learn only from it is a key you
learned from whoever runs it, so the key you compose with is the one read back on this call.

---

## Step 2 — Build and sign your package

Your package is one JSON object. It contains **no secret** — public keys, public names, agreed
limits — and it is meant to be readable by people in your organisation who are not engineers.

### What you implement

You have two options, and one of them does not exist yet:

| Option | Status |
|---|---|
| Implement the specification in your own language | **This is the path.** It is deliberately small; a competent implementer finishes in an afternoon |
| Run a standalone verifier we supply | **Does not exist.** Do not wait for one |

**The specification is the contract. Our implementation is one implementation of it.** If any behaviour is required for interoperability but is not written
in the specification, the specification is broken and we will fix it — tell us.

### Check yourself against the vectors, not against us

The conformance vectors are **normative**. They were derived by hand from the specification, in
another language, without running any of our code — which is what makes them a test rather than
a description.

> **If your implementation disagrees with a vector, your implementation is wrong.** That is what
> the vectors are for. Do not open a conversation with us about it; fix your code. The only
> exception is a vector that disagrees with the *specification*, in which case tell us and we
> fix the vector — never the specification to match a file.

Each vector directory holds byte-exact files:

| File | What to do with it |
|---|---|
| `package-<id>.signing-input.canonical.json` | Canonicalise the package with `signature` removed; your bytes must equal these, exactly |
| `package-<id>.canonical.json` | Canonicalise the whole signed package; your bytes must equal these |
| `agreement.canonical.json` | Compose the pair; your bytes must equal these |
| `agreement.digest.txt` | Hash those bytes; your value must equal this |
| `NOTES.md` / `REFUSAL.md` | What the vector pins, and why |

**Read them as bytes.** Every canonical file ends `}` with **no trailing newline**. A tool that
appends one produces a different digest over an identical agreement.

Run them at the date in `now.txt` (`2026-09-05`), not at your wall clock. Composition compares
`expires` against `now`, so the composing pairs change verdict as time passes.

For the refusal fixtures, assert **which rule fired**, not merely that it refused. A fixture
that refuses for the wrong reason is a passing test hiding a broken implementation, and it is
the specific way this kind of suite goes quietly wrong.

### Signing

Ed25519 over the canonical form of the package **with the `signature` member removed**. Detached,
base64 with the standard alphabet, padded. Exactly 64 bytes, which is exactly 88 base64
characters ending in one `=`.

```bash
# signing-input.json holds the canonical bytes with "signature" removed, no trailing newline
openssl pkeyutl -sign -inkey fed-signing.key -rawin -in signing-input.json | openssl base64 -A
```

**Use `openssl base64 -A`, not `base64`.** GNU `base64` wraps at 76 characters by default, and
88 is longer than 76 — so you get a value with a newline in the middle of it, which is refused
as malformed. On macOS it happens not to wrap, which is worse: it works on your laptop and
fails in your pipeline.

### How you know it worked

Verify your own signature over your own package before you send it. That catches a canonical
form that differs between your signer and your verifier — which is otherwise discovered by your
counterpart, at step 3, as a refusal you cannot reproduce.

Then compose your package against the vector packages: a package of yours paired with `xaa`'s
from `compose-full` should compose or refuse for reasons you can name.

---

## Step 3 — Exchange packages, compose, and compare digests

### What crosses

Each party publishes its signed package and CA certificate to the **peering directory**
(saltlick), and fetches the other's: `PUT`/`GET /v1/parties/<sovereign_id>/package` and
`/ca`, beside a signed pointer entry naming the package's sha256. Neither side needs a public
host, and nothing is sent by email. The packages are signed and the outcome is digest-compared,
so the directory does not need to be trusted: it carries files, and compose checks them against
the keys read back in step 1. A party that can reach no directory hands the same two files over
by any channel; the checks are unchanged.

### What each side runs

Both parties compose **independently**, from the same two files.

The composed agreement has **no "us" and no "them"**. The two halves are sorted by identifier
into position 0 and position 1, and every derived field is expressed in terms of those
positions. That is the entire reason both sides can compute byte-identical output. If you find
yourself writing `our_` and `their_` fields, stop — you will produce a mirror image of your
counterpart's document and no comparison will be possible.

Composition refuses in exactly seven ways, and a refusal is the mechanism working, not an error:

| # | Refuses when |
|---|---|
| 0 | A signature does not verify under the key bound in step 1 |
| 1 | Either `spec` is not **exactly** `federation-package/v1` — a literal pin, not a comparison of the two packages with each other |
| 2 | The two identifiers are equal, or the two trust domains are equal |
| 3 | Either package has expired. The boundary is **inclusive**: `expires` equal to today composes |
| 4a | Both offer a transport and share none that the preference order names |
| 4b | Both offer a transport and neither will listen |

Two of these have a trap in them. **The `spec` pin is literal.** Two packages both declaring
`v2` agree with each other, so an implementation that only compares the two composes them under
v1 rules — verifying under Ed25519 a signature a later version may specify some other algorithm
for. And **transport compatibility is tested against the preference order, not the raw
intersection**: two parties both offering some transport this version has never heard of do
intersect, and must still refuse.

### Then compare the digest

Each side computes `sha256` over the **canonical bytes of the agreement** — UTF-8, no trailing
newline, no byte-order mark, no leading or trailing whitespace of any kind — and states it as
`sha256:` followed by 64 lowercase hex characters.

Send that number to your counterpart by any means. **This comparison does not need a secure
channel either.** An adversary who alters a digest in transit causes a *visible mismatch*, which
gets investigated. To cause a false *match* they would have to make both sides independently
compute the same wrong agreement, which means forging a signature.

### How you know it worked

**The two digests are character-for-character equal.** That proves both sides hold the identical
agreement, and transcription error between you is now impossible.

If they are not equal, see **When the two digests differ**. Do not proceed to step 4 holding
two different agreements: every step after this one is configured from the agreement, and two
agreements produce two configurations that disagree silently, weeks later, in a way that reads
as a broken link.

---

## Step 4 — Declare conformance

Agreeing is not the same as being able to deliver.

Before any traffic, each party runs **its own** configuration generator against the agreement
**without applying it**, and asserts the output would be valid on its own system. Each then
publishes one line, signed with the same Ed25519 key:

```
conformance ok for sha256:<the agreement digest>
```

At that point, and only then, both properties hold: the two sides provably hold the same
agreement, and each has proved it can implement it.

The proofs at step 7 confirm what the running configuration actually does, and they are not optional.

---

## Step 5 — Bring the tunnel up, and prove it carries nothing else

### What the tunnel is

One dedicated point-to-point tunnel between the two edge hosts, on neither party's management
plane. It carries exactly one thing: your edge's fabric port talking to our edge's fabric port.

### WireGuard is the default. IPsec is the exception, and it costs one declaration.

The composed agreement's `transport.kind` tells you which, and it is derived by walking a fixed
preference order — `wireguard`, then `ipsec-fips` — and taking the first entry both parties
declared. The order is part of the specification, not a local setting.

**If your accreditation requires IPsec, declare `ipsec-fips` and do not declare `wireguard`.**
The intersection then has exactly one member, the preference order is never reached, and the
agreement composes to `ipsec-fips` with nothing changed on either side but your own declaration.
You do not need our agreement to make that choice and we cannot override it.

So: **if you can run IPsec, declare it, and you will get IPsec.** Declare WireGuard as well if
you can run both; you will still get IPsec, and you keep a fallback if we ever cannot.

| | IPsec — the default | WireGuard — by exception |
|---|---|---|
| Declare | `ipsec-fips` in `capabilities.transports` | `wireguard` |
| Suite | IKEv2, AES-256-GCM, SHA-384, ECDSA P-384, certificate authentication | Its own fixed suite; nothing to negotiate |
| Use it when | You can run it. This is the normal case | You cannot run IPsec, or your accreditation forbids it |
| Why it is the default | Layer-3 managed interface, and the suite is available on validated cryptographic builds — which matters if your fabric image is not itself validated | Simpler, but its primitives are not validated, and that has to be waived somewhere |

**There is no post-quantum transform in the tunnel, in either option, and this is deliberate.**
The TLS session inside already negotiates a post-quantum hybrid key exchange end to end, so an
adversary who records the tunnel today and breaks its classical key exchange in 2040 recovers a
stream of records that are still protected. Post-quantum on the outer layer would protect only
the outer layer.

**One case changes that: if your fabric image is not post-quantum capable.** Then the inner
layer is classical and the outer layer is the only place it can live. Declare
`pqc_capable: false` in your capabilities and **ask us** — this case has a designed answer and no
built one, and it needs a conversation before you build anything.

### What you exchange

From the agreement, not from a separate negotiation:

| Value | Where it comes from |
|---|---|
| `transport.kind` | Derived from the preference order |
| `transport.link_subnet` | The `proposed_link_subnet` of **position 0** — the party whose identifier sorts first. If that is not you, your proposal is not used, and this is not a negotiation |
| `transport.mtu` | The **smaller** of the two `measured_mtu` values |
| `transport.listener` | Which party accepts the inbound connection |

Directly between the two of you, outside the package: each side's edge **public address and
UDP port**, and — for WireGuard — each side's public key. Nothing else. **No CA, no identity, no
namespace crosses at this step**, which is what makes a failure here cost only time.

### Who dials

`transport.listener` names the party that **accepts** the connection. The other party
initiates.

- Exactly one party declared `listener: true` → that party listens.
- Both declared `true` → position 0 listens.
- Neither declared `true` → **composition refuses.** There is no link to build: each party has
  said it will dial and neither will answer.

**If you are behind NAT with no inbound port, declare `listener: false`.** Do not declare `true`
and hope; you may win the position tie-break and become the listener, and then nothing connects.

The listener must accept inbound UDP 500 and 4500 for IPsec, or the agreed port for WireGuard,
from the other party's address **only**.

### MTU — measure it, do not assume it

`measured_mtu` is in your package because it is a fact about your path, not a default.

**1380 is the working default and it clears the common cases.** On a 1500-byte physical path,
IPsec's worst-case overhead — 20 outer IPv4, 8 UDP for NAT traversal, 8 ESP header, 8 GCM IV,
16 ICV, up to 3 padding, 2 trailer — leaves about 1435. A path behind PPPoE at 1492 leaves about
1427. WireGuard costs 60 bytes over IPv4 and 80 over IPv6.

**Raise it only against a measured path.** Get this wrong and large messages fragment or vanish
while small ones work perfectly — the worst failure shape there is, because the link tests fine.

To measure, once the tunnel is up:

```bash
# 1352 payload + 8 ICMP + 20 IP = 1380. Increase until it fails; the last success is your MTU.
ping -M do -s 1352 <far side's link address>
```

### What you should see

```bash
# IPsec
swanctl --list-sas                     # the connection, ESTABLISHED, with an INSTALLED child SA
ip -br addr show ipsec-<name>          # the xfrm interface, holding your address from the /31

# WireGuard
wg show                                # a recent handshake, and a byte counter that increases

# either
ping -c3 <far side's link address>     # replies
```

### How you know it worked — the part people skip

**Prove the tunnel carries nothing but the fabric port.** Assert the firewall rules here, before
any trust material moves, because after step 6 you will be debugging the fabric and will assume
the tunnel is fine.

```bash
nc -z -w2 <far side's link address> 7447   # the fabric port: SUCCEEDS
nc -z -w2 <far side's link address> 22     # shell: MUST FAIL
nc -z -w2 <far side's link address> 443    # anything else: MUST FAIL
```

Two more checks that only matter if you also run an overlay mesh on this host, and which fail
silently if you get them wrong:

- **Our public address must not fall inside any route your mesh advertises.** If it does, the
  encrypted packets are sent into the mesh and take a second layer of encapsulation they have no
  MTU budget for.
- **The tunnel interface must not be enrolled in, advertised by, or routed through your mesh.**

---

## Step 6 — Stand up your edge router

One dedicated fabric router process for this peering. Not a second configuration on an existing
router — a separate process, on the edge host, whose only job is this link.

### What it must do

| Requirement | Why |
|---|---|
| Trust **our** certificate authority, in **this process's trust store only** | It must never reach a core router. The access layer matches the common name and cannot see the issuer, so a foreign root in a core router *is* an authorisation grant |
| Accept a peer presenting the common name from the agreement | See below. It is derived, not agreed |
| Permit that identity to **write** only under the prefix in the agreement | The only origin claim that is enforced rather than merely asserted |
| Permit that identity to **read** only the key expressions in the flow where we are the sender | The release set is a whitelist. There is no wildcard form of it |
| Present your own certificate, issued by **your** CA, with the derived name as its subject common name | This is what our access rules match |
| Dial inward to your own core, which trusts only your own CA | Your core never learns about ours |

### The names are derived. Do not agree them, do not type them twice.

Both peering names follow entirely from the two identifiers, which are already in the packages:

```
fed.<issuer>.<counterparty>.01
```

Throughout this section, `xab` stands in for your identifier and `xzz` for ours. Both are from
the standard's reserved user-assigned range, so neither can collide with a real party — substitute
our real identifier, which is in the message that accompanied this document.

If your identifier is `xab` and ours is `xzz`, you present `fed.xab.xzz.01` and we present
`fed.xzz.xab.01`. Both appear in the composed agreement as `parties[n].peering_cn`. **Copy them
from the agreement.** A package does not declare them and neither should you.

This is deliberate, and it removes the single most dangerous step in a hand-run bootstrap. A
mistyped name matches no rule, is then refused **in silence, at both ends, with no error**, and
looks exactly like a broken link. A name that is never typed cannot be mistyped.

Three properties of the name worth knowing before you look at it and wonder:

- The separator is a **full stop**, because an identifier may itself contain a hyphen
  (`nato-xyz`), and a hyphen-separated name would not parse.
- **A rotated certificate keeps its name.** The access rules match the name; changing it on
  rotation severs the link. The ordinal increments only to run a second, *concurrent* identity.
- The name carries **no organisational hierarchy, echelon, or unit lineage** — those are mutable
  and the name is not, and a name shaped like a tree but matched like a string is a trap.

### The publishing prefix is one symmetric rule

**Every party publishes across the link under `from/<its own identifier>/`.** You publish under
`from/xab/`; we publish under `from/xzz/`. Each side grants the counterparty write access under
`from/<counterparty>/` and nothing else.

That is one rule, applied by both, not two conventions. It is derived, it appears in the
agreement as `flows[n].publish_under`, and no package declares it.

**Your release set is not the same thing as the publishing prefix**, and conflating them causes
real confusion:

| | What it is | Where it lives |
|---|---|---|
| `from/<id>/**` | The **wire name**. Where your data appears once it has crossed | On the link, and on the receiver |
| Your release set, e.g. `release/weather/**` | The **local staging name** for data you have agreed to export | Inside your own systems, before anything crosses |

Moving data from the second to the first is a small republishing process **you run on your own
side, over your own data**. It is deliberately not a router feature: the fabric cannot rename
key expressions in transit, and building that would mean forking it. Budget for writing it.

### Certificate requirements

- Subject common name **exactly** the derived name for your side.
- ECDSA P-384, if you are on the IPsec path — the tunnel's certificate, separately from the
  fabric's.
- **The IPsec private key must be unencrypted.** The daemon loads it without a terminal, and a
  passphrase-protected key makes it hang rather than fail — which presents as an unexplained
  timeout, not as an error.

### How you know it worked

Your edge router's log shows an established session with the far side, and the common name it
reports is `fed.<ours>.<yours>.01`. Your **core** router's log shows a session with your edge and
nothing else — it has never seen our name and never will.

---

## Step 7 — Prove the refusals, then open the flows

Run all five. **Four of them are supposed to fail, and a failure that does not happen is the
finding.**

| # | Do this | Expected |
|---|---|---|
| 1 | We publish to the agreed landing prefix | **Arrives**, and is stored |
| 2 | We publish somewhere else | **Refused** |
| 3 | We present a name that is not in the agreement | **Refused** |
| 4 | We present a name one of **your own** routers uses | **Refused** |
| 5 | We subscribe to something outside the agreed release set | **Nothing delivered** |

> **Check your edge router's log, not the absence of data.** When this fabric refuses something
> it does so silently — the sender is told nothing. "Nothing arrived" and "it was refused" look
> identical from outside, so only the log distinguishes a working control from a broken link.
> This is the single most common way to waste an afternoon on this procedure.

**Case 4 is the important one, and it is the reason the names are shaped the way they are.**
Nothing stops our certificate authority issuing a certificate bearing one of *your* routers'
names, and the design assumes we can. The `fed.` prefix and the issuer segment are what make
that refusable as a class: you can write one rule saying nothing matching `fed.*` may ever touch
your operational namespaces, and another saying only `fed.xzz.*` may act as us. Prove both.

### Agree the cut before you need it

**A peering with no agreed cut procedure is one that will be severed badly, at speed, by someone
guessing.** Your package's `cut` block carries your contact, and it is required alongside
`data_offer` for exactly this reason.

Agree three things in writing: **who on each side may sever, how they do it, and who is called
when it happens.** Then confirm your own cut works, on your own host, without asking us:

```bash
# IPsec
swanctl --terminate --ike <connection name>

# WireGuard
wg set <interface> peer <our public key> remove
```

Local, immediate, unilateral, and effective on sessions already established. It needs no
cooperation from us and no convergence anywhere.

Afterwards, and in this order: remove our trust domain from your identity service **first**, or
a refresh timer re-adds our certificate authority within minutes and quietly undoes you; then
remove our CA from your edge router's configuration; then restart the edge router.

**There is no revocation list and no way to revoke a single certificate.** Cutting the tunnel is
the control that actually works. The rest is cleanup.

---

## When the two digests differ

Work down this list in order. Do not skip to the bottom.

**1. Print both agreements and diff them.** The offending field names itself. Loud, in the first
hour, instead of silent for three weeks.

**2. If the two printed agreements are identical, do not re-run anything.** The disagreement is
in the *bytes hashed*, not in the agreement, and a diff has nothing to show. This case reads as
impossible — two identical documents, two digests — and the instinct is to re-run the whole
procedure. Do not. **Measure one number instead:**

```bash
wc -c agreement.canonical.json      # both sides, and compare the two counts
```

| Difference in byte length | Cause |
|---|---|
| **1** | A **trailing newline** on one side. Almost always: the agreement was written to a file before hashing, and an editor, `echo`, a here-document or a standard library appended one |
| **3** | A **byte-order mark** on one side |
| **0**, and the digests still differ | The two sides encoded the same characters differently. The canonical form is **UTF-8** |

**This is the one failure the naive procedure cannot diagnose**, which is why the byte length is
measured before anything else is touched.

**3. If the lengths differ by something else,** you have a real content difference that your
pretty-printer is hiding — most often array ordering (see **What you will get wrong**) or a
number rendered differently. Diff the canonical bytes, not the pretty forms.

---

## What you will get wrong

Six sharp edges, from the specification's own record of what other implementers hit. Each is
listed with the symptom you will actually see, because the symptom is what you will have.

### Sorting arrays with your language's default comparator

Everything this format orders is ordered **by UTF-16 code unit, compared one code unit at a
time, with the shorter string first where one is a prefix of the other.** That is the same
comparison RFC 8785 applies to member names, and there is no second one.

| Language | Default | Correct? |
|---|---|---|
| JavaScript | `Array.prototype.sort()` compares UTF-16 code units | **Yes.** `localeCompare` and `Intl.Collator` are **not** — they are locale-dependent |
| Go | `sort.Strings` compares bytes | **No.** Encode with `utf16.Encode([]rune(s))` and compare the code units |
| Python | `sorted()` compares code points | **No.** `sorted(xs, key=lambda s: s.encode("utf-16-be"))` |

Byte-wise UTF-8 and code-point ordering are the *same* ordering, so Go and Python agree with each
other and both differ from this rule — on exactly one thing. A character above the basic
multilingual plane is a surrogate pair beginning `D800`–`DBFF`, so UTF-16 places it **before**
`U+E000`–`U+FFFF`, where a code-point comparison places it after.

> **Symptom:** every vector passes, every real exchange works, for months. Then one release set
> containing a non-ASCII key expression produces two digests, the two printed agreements differ
> in the order of one array, and nothing looks wrong in either. `compose-collation-utf16` is the
> vector that separates the two orderings; run it.

Two more rules in the same area, so you do not meet them separately:

- **You are obliged to sort and de-duplicate your own set-arrays before signing.** The composer
  sorts and de-duplicates every set-array it reads anyway, whatever order it arrived in. These
  are not alternatives and neither is optional: the signature covers the bytes as signed, so a
  composer cannot repair your package — but the *agreement* is composed output, and normalising
  there is what keeps two composers byte-identical.
- **An empty array is not a declaration.** `"release": []` is a malformed package, not a send
  half that happens to carry nothing.

### The verifying key encoding — it arrives by voice and has no file extension

Covered in step 1 and repeated here because it is the one that costs the most.

> **Symptom:** every signature check fails, on every package, including packages you signed
> yourself and verified locally. Nothing narrows it down, because nothing is field-specific.
>
> **The tell:** the value is 60 characters and begins `MCowBQYDK2Vw`. That is a
> `SubjectPublicKeyInfo` — the right 32 bytes behind a 12-byte DER prefix — and it is what most
> libraries hand you. The correct value is **44 characters ending in one `=`**.

### Writing the agreement to a file before hashing it

The digest input is the canonical form as **bytes**, and nothing else: UTF-8, no trailing
newline, no byte-order mark, no whitespace of any kind.

A canonical form is a byte string, not a text file. The ordinary way to hash one is to write it
out first — at which point most editors, `echo`, here-documents and several standard libraries
add a newline on your behalf.

> **Symptom:** your digest never matches ours, and the two agreements diff identically. See
> **When the two digests differ** and measure the byte lengths; a difference of exactly 1 is
> this.
>
> Concretely, from the specification's worked example: the agreement digests to
> `sha256:b29bb230…`, and the same agreement with one `\n` appended digests to
> `sha256:27f0dcf0…`. Diffing the two shows nothing, because they are the same agreement.

### A number that round-tripped through a float

Integers only, everywhere. **A number carrying a fractional part or an exponent is refused, not
coerced** — `1380.0`, `1.38e3` and `1380.` are each malformed, exactly integral though they are.
The test is on the **token as written in the document**, not on the parsed value: reject any
number whose JSON text contains `.`, `e` or `E`. Also refused: a leading zero (`01`), a leading
plus (`+1`), and negative zero (`-0`).

Every integer must be within ±(2^53 − 1). No rate, size or count this format carries comes near
it.

**In JavaScript this needs care, and the care belongs on the reading side.** JavaScript has one
number type, so by the time a value is a `Number` the distinction between `1380` and `1380.0` is
gone. A JavaScript *producer* is conforming by default — `JSON.stringify(1380)` emits `1380`. A
JavaScript *consumer* must apply this rule to the document **text**, never to the parsed value,
which cannot carry the answer.

> **Symptom:** your package composes perfectly on your side and we refuse it as malformed, with
> no field you can see anything wrong with. The value is right; the token is not.

### Taking `now` from the local clock

**`now` is the current date in UTC**, formatted `YYYY-MM-DD`. Never the local date of whoever
happens to be composing. The expiry boundary is **inclusive** — a package whose `expires` equals
today is valid today and composes.

> **Symptom:** invisible for months. Then, on the one day an `expires` falls due, two parties in
> different timezones composing either side of local midnight reach a refusal on one side and an
> agreement on the other — a digest comparison that cannot even be run, and no field to point at.
>
> It is invisible in development too, because the vectors supply `now` from a file. The one
> fixture whose verdict turns on it is `compose-expiry-boundary`.

### A `null`, a duplicate member name, or a member with no value

Three rules that are all the same rule: **refuse, never repair.**

- **A null is an error at any depth.** `"cut": null`, `"measured_mtu": null`, and a null
  *element* inside an array are each a malformed package. It must be a refusal rather than a
  repair because **the signature covers the null**: a receiver that quietly dropped it would
  verify one document and compose a different one.
- **A member name occurs at most once.** A parser handed the same name twice must refuse. Go's
  decoder keeps the last occurrence and several others keep the first, so a package carrying two
  `expires` members composes into two different agreements and each party believes its own.
- **A member is never emitted without a value.** No `"mtu":,` standing in for something that
  could not be derived. Output that parses nowhere, carrying a clean digest and a nil error, is
  the worst failure this format can have, because **both sides can agree on it**.

And: an escape denoting an unpaired surrogate is refused, as is any input that is not valid
UTF-8. **Never substitute `U+FFFD`** — that is what most decoders do by default, it changes the
bytes and therefore the digest, and it reports nothing. A decoder that replaces invalid input has
composed a document that was never signed.

---

## What we will refuse, and why it will look like a broken link

> **Read this before you spend a day on a link that is working exactly as designed.**

**Denial in this fabric is silent at the far end.** When our side refuses something you sent, you
are told nothing: no error, no reset, no negative acknowledgement. The message simply does not
arrive.

**From your side, a refusal is indistinguishable from a network fault.** Both look like: the
session appears healthy, you publish, and nothing lands. You cannot tell them apart from where
you are standing, and no amount of tracing on your side will separate them — the evidence is in
our log, not on the wire.

We say this plainly rather than leaving you to discover it, because the natural response is to
re-check the tunnel, and the tunnel is fine.

### What we refuse

| We refuse | Because |
|---|---|
| A write to a key expression outside `from/<your id>/**` | That prefix is the only origin claim that is enforced rather than asserted |
| A read of anything outside our release set for you | The release set is a whitelist; there is no wildcard form |
| A certificate whose common name is not `fed.<your id>.<our id>.01` | Matching is on the exact string — case, separators and all |
| A certificate bearing one of **our own** router names | Your CA can issue it, and this is the rule that stops it mattering |
| Anything at all, if our edge does not hold your CA for this peering | And you will see nothing, because the session never establishes |

### The one question that resolves it fastest

Send us this, and nothing else, first:

> **"Did our session appear in your edge router's log at all, and if so, under what common name
> and at what time?"**

It splits the entire problem space in two, in one round trip:

| Our answer | What it means | Where to look next |
|---|---|---|
| **No session at all** | The problem is below the fabric: the tunnel, the TLS handshake, the certificate chain, or our edge not holding your CA | Your side: step 5's port checks, and the certificate chain |
| **Session present, under the name you expect** | The transport and the identity are fine. The problem is the namespace: a write outside the granted prefix, or a read outside the release set | The agreement, field by field, against what you actually published |
| **Session present, under a name you did not expect** | Your edge is presenting the wrong certificate, or presenting the right one from the wrong process | Your edge's certificate, subject common name |

Asking anything more specific before this answer costs a round trip and narrows nothing.

### What to ask us to check, once you know which half it is

| If | Ask us to check |
|---|---|
| No session | That your CA is installed at our edge for this peering, and its fingerprint against the value from step 1 |
| Session, wrong namespace | The exact write prefix and read expressions our edge holds for you, quoted verbatim, and compare them character for character against the agreement |
| Session, everything matches, still nothing | Whether our core has storage declared for the landing prefix — data can be routed correctly and then discarded, which looks the same from your end |

**Ask us:** for our edge router's log excerpt covering the window you are testing in. You cannot
see it and there is no other way to distinguish a refusal from a fault.

---

## Two shapes that are valid and are not yet a working link

Neither is an error, and neither should be read as one:

| Shape | What it means |
|---|---|
| The agreement has **no `transport` member** | You have agreed data terms and no path. Nothing can cross yet. One party willing to connect is not a link |
| The agreement has **`flows: []`** | You have agreed a path that carries nothing |

Both are reachable deliberately — the second is the first pass of the staged approach in step 0.
Both are also reachable by accident, when one side omitted a block it meant to include. Read the
agreement before you build against it.

---

## What still needs a conversation

Everything else is in our package or in the public goatcore repository.

| # | What | Why it is not in the package |
|---|---|---|
| 1 | **Our Ed25519 signing public key and our CA fingerprint**, read aloud on one call | This is step 1 and it is the point. A value from a document or a directory is a value from whoever carried it |
| 2 | **A conversation, before you build anything**, if your fabric image is not post-quantum capable | The designed answer is standalone post-quantum key agreement on the tunnel itself. It has never been run, and it is not something to discover mid-build |
| 3 | **Our validated-cryptography posture**, if your accreditation needs a claim about it | It depends on the host we run your edge on, and on a certificate number read off the public database. It is per-host, not per-product |
| 4 | **Our edge router's log**, whenever a refusal and a fault are indistinguishable from your side | **What we will refuse**. There is no other way |

Our edge's address and port are our package's `transport_offer.endpoint`.

There is deliberately no fifth. **We do not supply a verifier, a library, or a container image**,
and you should not wait for one. The specification and the vectors are the whole of what you
need, and if they are not, that is the defect we want reported.

---

## Field reference — your package at a glance

Everything below is stated fully in the specification; this is the shape, for orientation.

```json
{
  "spec": "federation-package/v1",
  "sovereign_id": "xab",
  "issued": "2026-09-05",
  "expires": "2026-12-05",

  "identity": {
    "trust_domain":   "spiffe://xab.example",
    "ca_fingerprint": "sha256:<64 lowercase hex>"
  },

  "capabilities": {
    "transports":        ["ipsec-fips", "wireguard"],
    "fabric_fips_image": true,
    "pqc_capable":       true,
    "fabric_version":    "1.9.0"
  },

  "transport_offer": {
    "endpoint":             "_fed._udp.xab.example",
    "listener":             true,
    "public_key":           "<base64>",
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

  "signature": "<88 base64 characters>"
}
```

| Always required | Optional as a block; complete once present |
|---|---|
| `spec`, `sovereign_id`, `issued`, `expires`, `identity`, `capabilities`, `signature` | `transport_offer`, `data_offer`, `cut` |

Four things a package **never** carries, and you should not add them:

- **Your peering name.** Derived from the two identifiers (step 6).
- **Your publishing prefix.** Derived from your identifier (step 6).
- **A tier, level or standing claim.** Presence of a block is the declaration. Membership of any
  alliance or coalition is resolved by the *receiver*, from its own register, keyed on your
  identifier — a party asserting its own standing, and a receiver believing it, is recognition
  becoming authorisation.
- **Your verifying key.** A package carrying its own key authenticates nothing. Yours is bound
  in step 1 and nowhere else.

`cut` is required alongside `data_offer`, whichever halves it carries.
