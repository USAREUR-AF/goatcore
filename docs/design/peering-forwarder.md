# Peering forwarder: federating when neither edge accepts inbound traffic

| Field | Value |
|---|---|
| Status | **Designed, not built.** The edge-side behaviour it relies on (one side listens, the other dials with a keepalive) is built in `roles/goat-peering-wg`; the forwarder role, its CI guard and the agreement member in §6 are not built |
| Decision | [ADR 1218](../adr/1218-peering-forwarder-when-neither-edge-accepts-inbound.md), amends [ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md) step 1 |
| Companions | [`sovereign-federation-runbook.md`](../operations/sovereign-federation-runbook.md) (step 1 and step 5), [`federation-package-spec.md`](federation-package-spec.md) (`transport_offer`), `roles/goat-peering-wg` |
| Audience | Operators of two goatnets that want to federate; whoever accredits the interconnection |

## 1. The problem in one paragraph

A peering is one WireGuard tunnel between two edge hosts, and WireGuard needs **one** of the two
to accept an inbound UDP packet. When either edge has a public address, nothing here applies:
that side sets `transport_offer.listener: true`, the other dials it, and its keepalive holds the
path open through its own NAT. When **both** edges sit behind NAT with no inbound port (a site
LAN, a satellite terminal, carrier-grade NAT) neither can listen, and composition refuses the
pair with `compose.neither_listener`. A peering forwarder gives one side a public address it
does not otherwise have.

## 2. The design

**A forwarder is a small host with a fixed public address that relays one UDP port to one edge.
It carries ciphertext only and holds nothing of the peering.**

```
partner edge ──wg-fed0 (peering)──▶ forwarder <public>:51820
                                     │ DNAT + SNAT, one port, one destination
                                     ▼
                         wg-carry0 (carrier) ◀── dialled out by our edge, keepalive 25 s
                                     │
                                     ▼
                              our edge wg-fed0 (listener)
```

1. **Our edge dials the forwarder** over a dedicated carrier tunnel, `wg-carry0`, with a 25 s
   keepalive. The carrier is outbound-only from the edge, so it works behind any NAT that allows
   outbound UDP.
2. **The forwarder relays one port.** UDP arriving on `<public>:51820` is destination-translated
   to the edge's carrier address and source-translated to the forwarder's carrier address, so
   replies return through the carrier and not through the edge's own NAT. Nothing else is
   forwarded.
3. **The peering is unchanged and end to end.** Our edge is the listener; its `transport_offer`
   carries `listener: true` and the forwarder's `host:port` as `endpoint`. The partner dials
   that endpoint exactly as it would dial an edge with a public address. The peering WireGuard
   session, and the fabric TLS session inside it, terminate on the two edges.

**The partner changes nothing.** It dials one `host:port`, which every party already supports.

### What the forwarder holds, and what it never holds

| Holds | Never holds |
|---|---|
| Its own carrier WireGuard key (authenticates the carrier, not the peering) | The peering WireGuard key of either edge |
| One nftables ruleset: one inbound port, one destination | Any CA, SVID, fabric certificate or trust bundle |
| A public address | Membership of any goatnet mesh |
| | A fabric router, a storage, or any process that parses peering traffic |

### What a compromised forwarder can and cannot do

It can **drop, delay or count** packets, and it sees the partner's public address, packet timing
and volume. That is an availability and traffic-analysis position, the same one every internet
router on the path already holds. It **cannot read or inject**: the peering WireGuard session
authenticates every packet with keys the forwarder does not have, and the fabric TLS 1.3 session
inside it is mutually authenticated by certificate. The forwarder therefore needs no trust from
either party, and either party may operate it.

## 3. Reachability rules, per host

| Host | Inbound | Outbound |
|---|---|---|
| **Forwarder** | UDP 51820 (peering) from the partner's egress address when it is stable, otherwise from any address. UDP 51821 (carrier) from any address. Nothing else; no SSH. Managed through the provider's agent or console, or out of band on owned hardware | UDP to the edge's carrier address only |
| **Our edge** | On `wg-fed0`: the peering port only from the carrier interface. On `wg-carry0`: nothing initiated from the forwarder | UDP 51821 to the forwarder only, plus what `cross-sovereign-federation.md` §5.3 already allows inward |
| **Partner edge** | Unchanged | UDP 51820 to the forwarder's public address |

**When the partner sits behind carrier-grade NAT or a satellite terminal, its egress address
changes**, so the forwarder's peering port stays open to any source. That is acceptable because
WireGuard answers nothing that does not carry a valid handshake for the edge's key: an
unauthenticated packet is dropped silently and produces no reply.

**On our edge, the peering port is bound to the carrier.** The rule `goat-peering-wg` writes
today ("the tunnel port answers its one counterparty", by source address) cannot apply, because
peering packets now arrive from the forwarder's carrier address. The role gains one variable,
`peering_wg_via_forwarder`, under which the listen port is accepted only on `iifname wg-carry0`.

### MTU

The peering packet now travels inside the carrier, which costs one more WireGuard header
(60 bytes over IPv4, 80 over IPv6). On a 1500-byte path the carrier MTU is 1420 and the peering
link MTU is at most 1340. Set `measured_mtu` to **1300** in the edge's `transport_offer`; the
role's default of 1380 fragments or drops large samples behind a forwarder.

## 4. Where the forwarder runs

The role is the same on every host; only the placement changes. Each placement is a valid
production arrangement for the parties that agree it.

| Placement | Who operates it | Use |
|---|---|---|
| **Commercial cloud VM with a static address** (smallest instance class, one static IP) | Either party | Bringing a peering up when neither side has a public address. Under the operator's cloud discipline: Terraform root, tags, documented teardown |
| **A site the party owns, with a static public address and one forwarded UDP port** (a government network edge, a data-centre rack, a business internet line) | The party that owns the site | The durable form. No dependency on a commercial provider |
| **One forwarder per sovereign** | Each party, for its own edge | Neither party depends on the other's infrastructure. Either may then be the listener; the agreement picks one |
| **No forwarder** | — | Either edge has a public address. This is still the default |

A forwarder may serve several peerings, one port and one carrier per peering, never a shared
port: the destination of a forwarded packet must follow from the port alone.

## 5. Accreditation

The forwarder sits **outside the authorization boundary** of both parties. It is transport, in
the same position as the internet path it replaces, and the boundary controls stay on the edge:

- **SC-7 enforcement points** in `cross-sovereign-federation.md` §5.3 are unchanged and still on
  the edge host. Point 1 (the edge's outside interface accepts only the peering transport from
  the named counterparty) is enforced on the carrier interface instead of the public one.
- **SC-8 / SC-13** are carried by the fabric TLS session, as before. The forwarder adds no
  cryptographic claim and removes none.
- **SP 800-47 interconnection record.** The forwarder is named in the interconnection agreement:
  who operates it, where it runs, its public address, and the statement that it holds no peering
  key. A forwarder that is not written into the agreement is an undeclared component of the link.

## 6. Declaring it in the agreement

**Today**, version 1 of the federation package has no member for a forwarder. A party behind a
forwarder offers the forwarder's address as `transport_offer.endpoint` with `listener: true`, and
composition accepts it unchanged. The forwarder is declared in the partner declaration file,
`ops/federation/partners/<sovereign-id>.yaml`, under `forwarder:` (operator, placement, public
endpoint), and in the written interconnection agreement.

**Designed, not built:** a reserved member `transport_offer.via`, so that the composed agreement
both parties digest says the endpoint is a forwarder and who operates it:

```json
"transport_offer": {
  "endpoint": "<forwarder host>:51820",
  "listener": true,
  "via": { "kind": "forwarder", "operator": "<sovereign-id>" },
  ...
}
```

It is informational in composition: it changes no negotiated field and no rule, and its presence
is what puts the forwarder into the digest.

**The peering directory** (`saltlick`) never carries the forwarder's address. It is designed to
carry one capability, `inbound: "direct" | "forwarder" | "none"`, so two parties learn before
building that neither can listen and one must run a forwarder
([`peering-directory-service.md`](peering-directory-service.md) §6). The directory stays optional:
two parties can establish the same fact on a call.

## 7. Operating it

- **Bring-up order.** Forwarder up, carrier up from the edge (handshake within 25 s), then our
  package is issued with the forwarder's endpoint, then the partner dials. The forwarder exists
  before step 1 of the runbook, because its address is a value our package carries.
- **Health.** The carrier's latest-handshake age on the edge is the forwarder's liveness signal.
  An age above three keepalive intervals (75 s) means the carrier is down and the peering is
  unreachable from outside, whatever the peering's own state shows.
- **Cutting the peering.** Unchanged: `wg set wg-fed0 peer <key> remove` on our edge. Stopping the
  forwarder, or dropping its carrier peer, also severs the link, and either of those is a cut its
  operator can make alone.
- **Replacing the forwarder** changes our `transport_offer.endpoint`, so it is a new package and a
  re-composed agreement. Keep the address stable; a cloud placement uses a reserved static IP and
  not an ephemeral one.

## 8. Alternatives not taken

| Alternative | Why not |
|---|---|
| An HTTP or SOCKS proxy (Squid and similar) | Carries TCP, not WireGuard's UDP, and terminates the connection it proxies |
| NAT hole punching between the two edges | Fails behind carrier-grade NAT and satellite terminals, and needs a public rendezvous host anyway |
| Relaying through either goatnet's NetBird relay | Puts the peering on one party's management plane, which ADR 1157 step 1 excludes |
| Running the edge itself on the cloud host | Valid, and it needs no forwarder, but the edge then holds the partner's CA and the fabric session in the cloud. The forwarder keeps every key on hardware the party chose |
| A TURN server | Adds a protocol and credentials for what one nftables rule does |
