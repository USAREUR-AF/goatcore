# Federating two sovereign environments — operator runbook

**Audience: an operator of a goat environment who wants to exchange data with another
environment run by someone else.** You need no relationship with the authors of this software and
no access to anyone else's systems. Everything here is done on machines you control, using
material both sides are content to publish.

**Read this with your counterpart.** Both sides run the same steps, in the same order, against
each other. Nothing in this procedure is asymmetric, and neither side needs privileged access to
the other.

Design behind it: `cross-sovereign-federation.md`.
Decision: [ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md).

**Status: this procedure has not yet been executed between two real environments.** The
certificate behaviour it relies on is proven (`ops/federation-smoke/assert-pki.sh`, 7/7); the
live-fabric rehearsal is written and unrun. Treat it as a first-run procedure, not a routine one.

---

## What you are building

One extra data router on each side, dedicated to the other party.

```
your core fabric ── your edge router ══ tunnel ══ their edge router ── their core fabric
   trusts YOUR CA       trusts THEIR CA              trusts YOUR CA        trusts THEIR CA
```

Your edge router is the only thing that trusts their certificate authority. Your core fabric
never does. Their side mirrors it exactly.

## Before you start

- [ ] Both sides run this software with **their own** certificate authority. If one side does not
      have its own CA, stop — you do not have two sovereign environments, and a different pattern
      applies.
- [ ] **One** side can accept inbound UDP on one port, directly or through a forwarder. The other
      side only dials out. If both edges sit behind NAT with no inbound port, one side runs a
      peering forwarder first, because its address is a value that side's package carries:
      [`peering-forwarder.md`](../design/peering-forwarder.md) ([ADR 1218](../adr/1218-peering-forwarder-when-neither-edge-accepts-inbound.md)).
      No other reachability is required.
- [ ] You have agreed, in writing, **what data flows** and **in which direction**. Everything
      below is mechanical; that is the decision. The names each router presents are derived from
      the agreement, not agreed by hand (Step 2).
- [ ] A host you control to run the edge router on. It does not need to be powerful. It must not
      be a machine that already runs your core fabric.

---

## Step 1 — Run the driver, and read its plan first

Everything below is what the driver does, written out for a reader. **Do not
follow it by hand** — a peering stood up by hand on 2026-09-07 needed about a
dozen improvised interventions, and four defects were invisible
until the GENERATED artifacts were the thing deployed.

```bash
ops/scripts/peering-standup.sh <peering> --plan     # prints every phase, runs nothing
ops/scripts/peering-standup.sh <peering>            # stops at phase 4, which is yours
ops/scripts/peering-standup.sh <peering> --from 5   # resume after the exchange
```

| # | Kind | Phase | Worked when |
|---|---|---|---|
| 1 | GATE | preflight, fail-closed | prints `READY`; a failure names the input and where it was looked for |
| 2 | T | `init` — our half, measured | a signed package; exit 23 means a value could not be derived and the list is the output |
| 3 | T | `lint` | every required member present, signature shape valid |
| 4 | **MANUAL** | exchange packages; bind their key **out of band** | their package is on disk and their verifying key came from a channel authenticated independently of this link |
| 5 | T | `compose` | one digest, and argument order does not change it |
| 6 | T | `verify` | the derived names re-derive; the ingress prefix and republish target are stated |
| 7 | T | mint the peering leaves | three certificates, names derived from the agreement, SANs covering every address that will be dialled |
| 8 | T | `apply --confirm` | the partner declaration, the edge group_vars, the republisher vars and the ACL fragment |
| 9 | A | deploy the edge | the edge dials our core and accepts the partner's CN |
| 10 | A | open the transport the AGREEMENT selected | the link is up. `wireguard` runs `peering-wg.yml`, `ipsec-fips` runs `peering-edge.yml`; the phase never substitutes one for the other. A party that needs IPsec declares `ipsec-fips` and NOT `wireguard`, so the intersection selects it |
| 11 | GATE | acceptance | carries both ways, refuses what the agreement does not name, and round-trips |

**Phase 4 is manual on purpose.** The channel is the counterparty's to choose,
and a verifying key taken from the package it verifies proves nothing.

**Phases 1–8 run where the fabric is; phases 9–10 are ansible from the operator's
machine.** Use `--to 8` and `--from 9` when those are different hosts.

### Optional: look the partner up in a peering directory first

A peering directory (`saltlick`) is a public listing outside every goatnet that holds
**pointers, not permissions**: where each party's signed package can be fetched, and its sha256.
It grants nothing and is never an input to the check. Until a directory is reachable by both
sides, the first exchange is out of band, and everything above works without one.

| Set | To |
|---|---|
| `FED_DIRECTORY_URL` | the directory, `https://…` |
| `FED_PACKAGE_URL` | where you serve your package, as a static file behind TLS on a public host you run — never on a mesh. Copy the file phase 3 wrote there unchanged |
| `FED_COUNTERPARTY_ID` | the partner's `sovereign_id` |
| `FED_SIGNING_KEY` | the key that signed your package; it signs the pointer too |

With those set, the exchange phase, before it stops:

1. checks your package is served byte for byte at `FED_PACKAGE_URL`, then publishes your pointer;
2. looks up the partner's pointer, fetches their package, checks its sha256 against the pointer
   and both signatures against the key in your keyring, and places it where compose reads it if
   nothing is there yet;
3. prints a **compatibility report**: spec, fabric and tool version, the transport the agreement
   will select, FIPS image on both sides, post-quantum capability, trust domains, and whether
   either edge accepts inbound traffic. Each row is PASS, WARN, FAIL or UNKNOWN with its fix.

A directory that is down, refuses, or disagrees with the package is a warning, never a stop.
Your CA certificate and SPIFFE bundle still move directly.

**What a partner is never asked by message.** The package carries their `sovereign_id`, trust
domain, CA fingerprint, transports, FIPS image, post-quantum capability and versions; peering
names are derived from the agreement. A question list sent to a partner means one of these was
missed. The one act that stays out of band is **reading the verifying-key fingerprint, and after
compose the agreement digest, aloud** — group count first, read back. The directory replaces
neither. Design: [`peering-directory-service.md`](../design/peering-directory-service.md) §6.

## Step 2 — The names and prefixes come from the agreement

None of these is asked of the partner. Each is in a package or derived by `compose`, and both
sides derive the same values from the same agreement.

| Value | Example | Whose |
|---|---|---|
| Your sovereign identifier | `xab` | yours, in your package |
| The name your edge router presents to them | `fed.xab.xaa.01` | derived from the agreement (`peering_cn`) |
| Where their data lands on your side | `from/xaa/**` | derived from the agreement |
| What you will send them | `release/all/**` — the **staging** name, internal to you | yours |
| The name that data carries once it crosses | `from/<your sovereign>/**` — the **wire** name; the republisher lifts the staging name onto it before anything reaches the edge, and it is what the counterparty's ingress prefix admits | derived |

Their values mirror these. **The name is load-bearing**: the other side's permission rules
will match it exactly, character for character, which is why it is derived rather than typed. A
change to a `sovereign_id` changes every derived name and means a new agreement on both sides.

Do **not** choose a name that already exists in your own fabric. Their router presenting a name
your own routers use is the failure mode this whole design is built to prevent.

## Step 3 — Exchange trust material

Each side publishes its **public** certificate authority material and pulls the other's. No
private key is exchanged, ever, in any step of this procedure. If anyone asks you to send a
private key, the procedure is being done wrong.

The first exchange is out of band — email, a call, a physical handover, whatever your policy
allows. Verify the fingerprint by a **different channel** than the one that carried the file:

```bash
openssl x509 -in their-ca.pem -noout -subject -fingerprint -sha256
```

Read the fingerprint to your counterpart aloud, or check it against a value they published
separately. A trust anchor accepted without out-of-band verification is not a trust anchor.

After the first exchange, refresh is automatic: each side re-pulls the other's published material
on a timer, so **the other side replacing their certificate authority does not break the link**.
That is the reason for using the identity service rather than copying a file into place.

## Step 4 — Stand up your edge router

One process, dedicated to this partner. It is the same router software you already run, started
with a different configuration.

Set these, and nothing else:

| Setting | What it is |
|---|---|
| `goat_zenoh_edge_partner_ca_path` | **their** CA material — and only theirs |
| `goat_zenoh_edge_partner_cn` | the name from Step 2 that they will present |
| `goat_zenoh_edge_ingress_key_exprs` | where their data may land, e.g. `from/xab/**` |
| `goat_zenoh_edge_egress_key_exprs` | what they may receive, named as it appears **on the wire**: `from/<our sovereign>/**` — never the `release/**` staging name |
| `goat_zenoh_edge_core_endpoints` | your own core, which this edge dials inward to |
| `goat_zenoh_edge_outward_cert` / `_key` | what you present to them |
| `goat_zenoh_edge_inward_cert` / `_key` | what you present to your own core |

**Do not point the trust refresh at your core router.** It must write to the edge router's trust
file. Pointing it at the core puts a foreign certificate authority on the machine that holds
everything — the single most damaging mistake available in this procedure.

**Do not reuse your core router's configuration as a starting point.** Use the edge template. The
permission engine stops at the first rule that allows something, so a permissive rule inherited
from your core cannot be taken back by adding a narrower one afterwards.

## Step 5 — Open the tunnel

One encrypted tunnel between the two edge hosts. It belongs to neither side's internal network:
do not enrol the other party's machine into your own network management, and do not join theirs.

Firewall it to exactly this, on the edge host:

- **inbound**: the tunnel port, from their address only. Everything else denied and logged.
- **outbound**: the tunnel port, to their address only.
- **nothing else crosses** — no management, no shell, no administrative interface.

**Behind a forwarder**, the side that listens receives the tunnel port on its carrier interface
(`wg-carry0`), not from the partner's address, and sets `peering_wg_via_forwarder: true` so the
rule binds to that interface. Its only outbound is the carrier port to the forwarder. The side
that dials sends to the forwarder's address. Set the link MTU to 1300 on both sides
([`peering-forwarder.md`](../design/peering-forwarder.md) §3).

## Step 6 — Sealed data for this partner, if they need it

**Skip this unless a release set has to be readable only by a named audience.** Unsealed exchange
is the default: the lane the agreement grants is already partitioned and mutually authenticated, and
`grant-keyed-encryption.md` §0a is explicit that using the mechanism is the exception rather than
the norm. Naming no audience renders no rule, which is a deliberate configuration and not a gap.

When a partner does need it, three things have to line up.

**1. Admit their trust domain.** A federated partner's identity is in THEIR SPIFFE trust domain, so
`grant-keyd` refuses it until the domain is named:

```json
"federated_trust_domains": ["xaa.fed.lab"],
"release_sets": { "weather": ["member/a", "spiffe://xaa.fed.lab/member/partner"] }
```

A member id written as a full SPIFFE URI is a federated identity, used verbatim; a bare id is one of
ours. Their certificate lives at `<cert_dir>/federated/<their domain>/<path>.pem`. Admitting a
domain lifts exactly one check — the SVID still has to chain to a root in the trust bundle, still
has to be time-valid, and still has to match its id exactly.

**Admission is deliberate and separate from federating trust bundles.** The bundle carries their
root because two SPIRE servers federate, which authenticates the LINK. It says nothing about who may
read sealed data, and ADR 1169 keeps that decision entirely ours.

**2. Name the audience when you apply.**

```bash
goat federate apply <agreement> --as <us> --edge-group <group> --partner-audience weather
```

This renders `goat_zenoh_edge_key_bundle_key_exprs: ["keys/weather/**"]` — scoped per release set,
never `keys/**`, which would hand them every audience's bundle. Withdrawing the scope is re-running
apply without that set.

**3. Narrow the core's key storage to match.** This is the one that will catch you. A queryable
declared wider than the granted lane is refused **silently**, and the natural configuration stores
every set under one `keys/**` storage:

| Core storage | Partner fetch |
|---|---|
| `keys/**` | `[]` — the declaration is wider than the grant and never reaches them |
| `keys/weather/**` | the bundle |

A goatnet serving a federated partner needs a key storage per released release-set
(F-499).

**Worked when:** the partner queries `keys/<set>/**` and gets a bundle whose `members` include their
SPIFFE id. With no audience named the same query returns `[]`.

## Step 7 — Prove it before you trust it

Run all five. **Four of them are supposed to fail**, and a failure that does not happen is the
finding.

| # | Do this | Expected |
|---|---|---|
| 1 | They publish to the agreed landing prefix | **Arrives**, and is stored |
| 2 | They publish somewhere else | **Refused** |
| 3 | They present a name you did not agree | **Refused** |
| 4 | They present a name one of *your* routers uses | **Refused** |
| 5 | Their side subscribes to something not in the agreed list | **Nothing delivered** |

> **Check the router's log, not the absence of data.** When this software refuses something it
> does so silently — the sender is told nothing. "Nothing arrived" and "it was refused" look
> identical from outside, so only the log distinguishes a working control from a broken link.

Case 4 is the important one. Nothing stops your counterpart's certificate authority issuing a
certificate bearing one of your router's names, and the design assumes they can. Prove it is
refused.

## Step 8 — The round trip, and it is the last thing you do

**Every check in Step 7 needs shell on both sides.** You read their store, you read their log. A
real publisher has neither, and none of those checks is available to them.

The round trip is the one check the sending side can make alone. Publish a nonce under your own
wire name and wait for their acknowledgement to come back under theirs:

```bash
# on their side, once
python3 ops/federation/roundtrip/responder.py from/<them> from/<us>

# on yours, whenever you want the answer
ops/federation/roundtrip/fedcheck.sh from/<us> from/<them>
```

A pass means the sample crossed, **their own router delivered it to a process on their side**, the
payload arrived with the digest you sent, and the answer came back over the same agreement. It uses
no access the agreement does not already grant — the probe goes out on your wire name and the ack
returns on their landing prefix, which are the two lanes you just built.

Run it last, because it is the only stage that fails when an earlier one silently half-works. A
peering can be ESTABLISHED, fully authenticated, and carrying nothing at all with no denial logged
anywhere (F-493,
F-494,
F-177). One-way checks pass
in some of those states. This one does not.

Measured on a live two-region peering 2026-09-07: **PASS in 538 ms** healthy; **FAIL in 8 s** with
the partner's responder stopped; **FAIL in 8 s** with the edge stopped; **PASS again** on restore.
The failures are the point — a check that cannot fail is not a check
(F-478).

## Step 9 — Ending it

Remove them from the tunnel on your own edge host. It takes effect immediately, cuts sessions
already open, and needs no cooperation from them.

Then, in this order:

1. Remove their trust domain from the identity service **first** — otherwise the refresh timer
   re-adds their certificate authority within minutes and quietly undoes you.
2. Remove their CA from the edge router's configuration and re-apply.
3. Restart the edge router.

There is no certificate revocation list and no way to revoke a single certificate. Cutting the
tunnel is the control that actually works; the rest is cleanup.

---

## If something does not work

| Symptom | Most likely cause |
|---|---|
| Connection refused immediately | Their certificate does not chain to the CA you installed. Re-check Step 3, and check you installed *their* CA and not your own. |
| Connects, but nothing arrives | The name they present does not exactly match `goat_zenoh_edge_partner_cn`. Matching is exact — check case, hyphens and whitespace. |
| Arrives but is not stored | Your core has no storage declared for the landing prefix. Data is being routed and discarded. |
| Worked, then stopped after weeks | A certificate expired, or their CA rotated and the refresh is not running. Check the refresh timer first. |
| Works in one direction only | The two sides' agreed prefixes do not mirror. Compare Step 2 values side by side. |

**When diagnosing, look at the router log on the receiving side.** A refusal is only visible
there. This is the single most common way to waste an afternoon on this procedure.
