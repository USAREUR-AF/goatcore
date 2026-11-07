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

One extra data router on each side, dedicated to the other party, and one small process beside
each core that names your data for the crossing.

```
 release/<set>/**                                                      release/<set>/**
       │ republisher (on your core host)          republisher (on their core host) │
       ▼                                                                           ▼
 from/<you>/<set>/**                                                 from/<them>/<set>/**
your core fabric ── your edge router ══ tunnel ══ their edge router ── their core fabric
   trusts YOUR CA       trusts THEIR CA              trusts YOUR CA        trusts THEIR CA
```

Your edge router is the only thing that trusts their certificate authority. Your core fabric
never does. Their side mirrors it exactly.

**The republisher** runs on your core host, the host your edge dials, and both of its sessions
dial your core router. It subscribes to each release set you offer, `release/<set>/**`, and
publishes every sample under `from/<you>/<set>/**`, the wire name. It is the only writer of
`from/<you>/**` on your side; your core admits it by its own certificate name, and your edge
carries the wire name out. Several sets share the one wire prefix, the set kept as a path
segment: `release/REL-ALL/<rest>` crosses as `from/<you>/REL-ALL/<rest>`.

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
      be a machine that already runs your core fabric. It is a peer of your own mesh in node class
      `PEERING-EDGE`, which lets it reach only your core router's fabric port and mesh DNS (ADR 1219).

---

## Step 1 — Run the driver, and read its plan first

Everything below is what the driver does, written out for a reader. **Do not
follow it by hand** — a peering stood up by hand on 2026-09-07 needed about a
dozen improvised interventions, and four defects were invisible
until the GENERATED artifacts were the thing deployed.

```bash
ops/scripts/peering-standup.sh <peering> --plan     # prints every phase, runs nothing
ops/scripts/peering-standup.sh <peering>            # stops at phase 5, which is yours
ops/scripts/peering-standup.sh <peering> --from 6   # resume after the exchange; stops at phase 10
ops/scripts/peering-standup.sh <peering> --from 11  # resume after the review
```

`goatbuilder federate <peering>` runs the same phases with the same numbers and `--from`/`--to`.

| # | Kind | Phase | Worked when |
|---|---|---|---|
| 1 | GATE | preflight, fail-closed | prints `READY`; a failure names the input and where it was looked for. When the declaration offers a release set, the inventory group `<peering>_core` names the core host and sits under `[peering_cores:children]` |
| 2 | A | prepare what the package measures: the federation interface and the SPIRE bundle | the interface and its key exist with no peer on them; the bundle is exported when this side runs SPIRE |
| 3 | T | `init` — our half, measured | a signed package; exit 23 means a value could not be derived and the list is the output |
| 4 | T | `lint` | every required member present, signature shape valid |
| 5 | **MANUAL** | exchange packages; bind their key **out of band** | their package is on disk and their verifying key came from a channel authenticated independently of this link |
| 6 | T | `compose` | one digest, and argument order does not change it |
| 7 | T | `verify` | the derived names re-derive; the ingress prefix and republish target are stated |
| 8 | T | mint the peering leaves | four certificates, names derived from the agreement, SANs covering every address that will be dialled: outward, inward, core, and `republisher.pem` with CN `fed.<us>.<them>.republisher.01` |
| 9 | T | `apply --confirm` | the partner declaration, the edge group_vars, the republisher vars (`federation-<them>-republish.yml`) and the ACL fragment, which carries the edge's subject and the republisher's `fed-rp-<them>` subject |
| 10 | **MANUAL** | review what apply rendered; admit the edge on our core | the fragment is spliced into the core's access control and the core restarted on it |
| 11 | A | open the transport the AGREEMENT selected | the link is up. `wireguard` runs `peering-wg.yml`, `ipsec-fips` runs `peering-edge.yml`; the phase never substitutes one for the other, and any other transport refuses. A party that needs IPsec declares `ipsec-fips` and NOT `wireguard`, so the intersection selects it |
| 12 | A | deploy the edge, then the outbound republisher (`peering-zenoh-edge.yml`, then `peering-republisher.yml` with `--limit <peering>_core`) | the edge dials our core and accepts the partner's CN; the republisher container `goat-fed-republisher-<them>` runs on the core host, its `/metrics` on loopback reads `goat_fed_republisher_local_subscribed 1` and `goat_fed_republisher_edge_linked 1`, and its log has one `release release/<set>/** -> wire from/<us>/<set>/**` line per offered set |
| 13 | A | federate the SPIRE trust domains (ADR 1157 step 2) | our server refreshes their trust domain across the link, or the play says the step was not done for a side with no SPIRE server |
| 14 | GATE | acceptance | carries both ways, once per offered set: a sample staged under `release/<set>/…` reads back under `from/<sender>/<set>/…` on the receiving side; refuses what the agreement does not name; and round-trips once per set |

**Phases 5 and 10 are manual on purpose.** The exchange channel is the counterparty's to choose,
and a verifying key taken from the package it verifies proves nothing. The review is the control,
and only this party can change this party's core.

**Phases 2 and 11–13 run ansible** against `ops/ansible/inventory/peering-edges.ini`: the edge
group `<peering>_edge`, and for the republisher the core group `<peering>_core`. The rest run
`goat` and the standup scripts where the working directory is.

## Step 2 — The names and prefixes come from the agreement

None of these is asked of the partner. Each is in a package or derived by `compose`, and both
sides derive the same values from the same agreement.

| Value | Example | Whose |
|---|---|---|
| Your sovereign identifier | `xab` | yours, in your package |
| The name your edge router presents to them | `fed.xab.xaa.01` | derived from the agreement (`peering_cn`) |
| Where their data lands on your side | `from/xaa/**` | derived from the agreement |
| What you will send them | `release/all/**` — the **staging** name, internal to you | yours |
| The name that data carries once it crosses | `from/<your sovereign>/**` — the **wire** name. Your republisher, on your core host, lifts `release/<set>/<rest>` onto `from/<your sovereign>/<set>/<rest>` before anything reaches the edge, every set under the one prefix; it is what the counterparty's ingress prefix admits | derived |

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

### The republisher, on your core host

Phase 12 deploys it with `peering-republisher.yml` on the inventory group `<peering>_core`, which
names the core host your edge dials. The agreement's values (`goat_fed_republisher_sovereign_id`,
`_partner_id`, `_cn`, `_publish_under`, `_release`) come from `apply`. Set these in
`group_vars/<peering>_core/`, and nothing else:

| Setting | What it is |
|---|---|
| `goat_fed_republisher_image` | the image, digest-pinned: `components.goat-fed-republisher.fleet` in `ops/pins/fleet-pins.yaml` |
| `goat_fed_republisher_core_endpoint` | your core router as this host dials it, e.g. `tls/<core address>:7447`, with no `#` fragment |
| `goat_fed_republisher_core_ca_src` | **your** CA, the roots your core's certificate chains to, as a path on the machine running the driver |

Its certificate is phase 8's `republisher.pem`, and the agreement is phase 6's; the role copies both
from the working directory. Your core admits it through the fragment's `fed-rp-<them>` subject: read
the offered `release/<set>/**`, write `from/<you>/**`, nothing else. A core configured from
`roles/goat-zenoh` carries the same grant through `zenoh_sovereign_peerings[].republisher_cn` and
`republisher_release_key_exprs`.

Its `edge_linked` and `dropped_total{reason="edge_down"}` metrics describe its session to **your
core**. They do not see the partner link; phase 14's round trip does.

To stop your data crossing at once without touching the edge, stop the container
`goat-fed-republisher-<them>` on the core host: nothing else writes your wire name.

## Step 5 — Open the tunnel

One encrypted tunnel between the two edge hosts. It belongs to neither side's internal network:
do not enrol the other party's machine into your own network management, and do not join theirs.

Firewall it to exactly this, on the edge host:

- **inbound**: the tunnel port, from their address only. Everything else denied and logged.
- **outbound**: the tunnel port, to their address only.
- **nothing else crosses** — no management, no shell, no administrative interface.

**Behind a forwarder** (status: built, not yet run between two goatnets), the side that listens
receives the tunnel port on its carrier interface (`wg-carry0`), not from the partner's address,
and its only outbound is the carrier port to the forwarder. The side that dials sends to the
forwarder's address. The forwarder is up before step 1
([`peering-forwarder.md`](../design/peering-forwarder.md) §7):

| Host | Play | Variables |
|---|---|---|
| Forwarder (group `peering_forwarders`) | `ops/ansible/peering-forwarder.yml` (`roles/goat-peering-forwarder`) | `peering_forwarder_enabled: true`; `peering_forwarder_peerings`, one entry per peering: `name`, `edge_carrier_public_key`, optional `partner_sources`. Ports 51820/51821 and carrier 169.254.30.0/31 are defaults |
| Our edge | `peering-prepare.yml` and `peering-wg.yml` (`roles/goat-peering-wg`) | `peering_wg_via_forwarder: true`, `peering_wg_forwarder_carrier_public_key`, `peering_wg_forwarder_endpoint: <forwarder>:51821` |

With `peering_wg_via_forwarder: true` the edge's firewall binds the tunnel port to `wg-carry0`,
and phase 11 refuses unless this side listens and the link MTU is at most 1340. Offer
`measured_mtu = 1300`. Declare the forwarder under `forwarder:` in the partner file.

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

The round trip is the one check the sending side can make alone. Stage a nonce under one of your
release sets, let your republisher lift it onto your wire name, and wait for their acknowledgement
to come back under theirs:

```bash
# on their side, once: watch for your probes, stage the ack under one of THEIR sets
python3 ops/federation/roundtrip/responder.py from/<you> release/<one of their sets>

# on yours, once per set you offer (phase 14 does this for every set)
ops/federation/roundtrip/fedcheck.sh release/<one of your sets> from/<them>
```

A pass means the sample crossed, **their own router delivered it to a process on their side**, the
payload arrived with the digest you sent, and the answer came back over the same agreement. It uses
no access the agreement does not already grant — the probe goes out on your wire name and the ack
returns on their landing prefix, which are the two lanes you just built. Because the probe is
staged, a pass also proves your republisher lifted that set; with the republisher stopped it fails.

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
