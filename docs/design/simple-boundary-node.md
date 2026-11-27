# Simple boundary node, pseudo boundary node, and the boundary monitor

**Status: WIP.** The sender and the monitor are built and run on one goatnet with synthetic data only. The TCP transport, the
synthetic-only guard, fixed per-feed source ports, the ansible role (`ops/ansible/simple-bn.yml`) and the egress relay
(`--input relay`), the fabric input (`--input fabric`), encryption stage 1 (WireGuard interface, key and peer) and stage 2
with IPsec are built. The pseudo boundary node is designed, not built. The gate that lifts WIP: one non-synthetic feed
leaves through an encrypted stage and the monitor shows it for seven days without a gap. Decision records:
[ADR 1253](../adr/1253-simple-boundary-node-pseudo-boundary-node-and-one-boundary-monitor.md), and
[ADR 1263](../adr/1263-simple-boundary-node-partner-ipsec-gateway-fails-closed-and-real-data-needs-a-validated-module.md) for
stage 2 with IPsec and which stage may carry real data.

## 1. Three kinds of boundary

A boundary node is the one machine through which data leaves a goatnet. goat now has three kinds, and each is named
for what it does on the far side.

| Kind | Short name | Far side | What crosses | Decision |
|---|---|---|---|---|
| Boundary node | `bn` | another sovereign's goatnet | Zenoh, through a dedicated edge router per partner, `release/<set>/**` republished as `from/<us>/<set>/**` | [ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md), [`03-boundary-node.md`](../tutorials/03-boundary-node.md) |
| Simple boundary node | `simple-bn` | a system that is not goat and does not speak Zenoh | plain UDP or TCP messages in the receiver's own format (for radar: ASTERIX CAT034 and CAT048), one way | ADR 1253 |
| Pseudo boundary node | `pseudo-bn` | another goatnet of the same sovereign (another environment) | whatever the crossing declares; may join the far goatnet instead of opening a tunnel | ADR 1253 |

**Detail.** The canonical boundary node federates two fabrics and so carries the full trust model of
[`federated-trust-model.md`](federated-trust-model.md) and [ADR 1157](../adr/1157-cross-sovereign-federation-edge-terminated.md): SPIRE federation, a per-partner router, a deny-by-default
ACL. A simple-bn federates nothing. The receiver never sees a Zenoh key, a certificate or a mesh address; it gets a stream
of datagrams on one port. The pseudo boundary node sits between the two: the far side is ours, so trust is already shared,
and the only questions are how the bytes travel and what protection they need on the way.

## 2. What a simple-bn does

A simple-bn takes named feeds and sends each one as messages to one declared receiver, and writes its own counters to a
status file that the boundary monitor (§6) shows.

```
release/<set>/** on the fabric ──► simple-bn ──► encoder (ASTERIX CAT034/048) ──► UDP or TCP ──► receiver
   (built: --input fabric)          (built)            (built)                        (UDP built)
                                       │
                                       └──► /run/simple-bn/status.json ──► bn-monitor (mesh address only)
```

**Detail.**

- Code: [`ops/simple-bn/simple_bn_sender.py`](../../ops/simple-bn/simple_bn_sender.py), Python standard library only.
- **One socket per feed, from a fixed source port.** Each feed declares `src_port` (1024-65535, unique) in its
  profile and always sends from it, so a receiver's firewall can allowlist our address and those ports. Each feed also
  has its own ASTERIX source identity (SAC/SIC), so a receiver, a packet capture and the monitor can each tell the
  feeds apart. A kernel-chosen port would change on every restart and could not be allowlisted, so the role refuses a
  feed without one.
- **The socket is connected.** If the receiver host answers ICMP port-unreachable, the next send fails with
  `ECONNREFUSED`. The sender records this as `receiver-refusing`. It is the only signal a one-way UDP stream gives.
- **Feeds today are notional sensors built into the sender.** Three generic sensor classes, each with its own update
  period, range and target mix:
  - `hemispheric-cuas`: drones, helicopters, aircraft, 1 s updates.
  - `counterfire`: projectile salvos on ballistic arcs, 1 s updates.
  - `surveillance-3d`: drones, helicopters, aircraft, 2 s updates.
- **A deployment names its own feeds** with a profiles file (`--profiles`, JSON) that sets `base`, `title`, `sic` and
  `src_port` per feed. Deployment names stay in the private overlay, never in this repo.
- **Every CAT048 report sets the SIM bit** in I048/020, so a receiver can tell the reports are simulated. Each report
  carries:
  - Target report: I048/010, I048/140, I048/020 and I048/040.
  - Cooperative targets add Mode-3/A and flight level (I048/070, I048/090).
  - Tracks add the track number, Cartesian position, velocity and status (I048/161, I048/042, I048/200, I048/170).
  - Every report has 3D height (I048/110).
- **CAT034 sends a North Marker every update period**, carrying the period in I034/041.
- **The receiver address is a secret-class value.** It is passed in a mode-600 environment file
  (`/etc/simple-bn/receiver.env`) and is never logged, never written to the status file and never committed. The private
  overlay holds a 1Password reference to it.

**The fabric input (built).** The sender inside the goatnet (`--input fabric`) subscribes to each feed's declared key,
which must be under `release/` (profile `key`), and sends each sample's payload unchanged. Only released keys leave, for
the same reason the canonical boundary node republishes only `release/`
([ADR 1208](../adr/1208-sovereign-environments-release-namespaces-and-grant-encryption.md)): what crosses a boundary is
a deliberate act of the data's owner, not a property of where the boundary host sits. The payload is raw ASTERIX, as the
ingress adapter publishes a keyless radar's frames ([ADR 1251](../adr/1251-every-source-keeps-its-identity-through-a-pod-carriage-rules-provenance-at-entry-and-pod-local-authority.md),
`<slot>/stream/asterix/<cat>/raw/v1`) and a producer with a release grant carries them into the lane. Each payload is
checked like the relay checks it (§3) and sent, one datagram per sample, to the egress relay's mesh address at the
feed's `src_port` (`--relay-dest`). That hop is on the goatnet mesh, which encrypts it; the relay enforces encryption to
the receiver. The session is a zenoh client with the machine's own goat context (`profile.toml`, `mtls.cert.pem`,
`mtls.key.pem`, `ca-roots.pem`), so the router's grants decide what it can read. The role installs the pinned
`eclipse-zenoh` in its own virtual environment and hands the context to the service as systemd credentials.

**One scene, several radars, fused tracks (built).** With `scene: {reference: [lat, lon]}` the notional feeds share
one air picture instead of each inventing its own targets (`ops/simple-bn/scene.py`). Each feed whose profile has a
`site` (east and north metres from the reference point) is a radar at that site watching the same true targets: it
sends CAT034 with its position (I034/120) and CAT048 for what it detects, with range and azimuth noise (profile
`noise`), a detection probability that falls off in the outer fifth of its range (`pd`), clutter plots, and only the
kinds it is declared to see (`detects`; by default its base profile's). A profile with `"kind": "fused"` is the fused
feed: every radar's tracked detections go to one tracker, which correlates them into system tracks and sends them as
CAT062 once per period from the fused feed's own SAC/SIC and source port.

**Detail.** The tracker is a central track processor: greedy global nearest-neighbour association on the squared
normalised distance inside a 4-sigma gate; a constant-velocity Kalman filter per horizontal axis whose process noise is
wider for projectiles; confirmation after 3 updates from any radars; coasting, then deletion after 8 s without an update
(3 s for projectiles, 3 s for tentative tracks); and a merge of two confirmed tracks that sit inside each other's gate
on the same heading. It takes the radars' tracked detections, not their clutter plots, as a central processor fed by
radars with their own plot-to-track stage does. Each CAT062 record carries I062/010, 015, 070, 105 (WGS-84), 100
(Cartesian from the reference point), 185, 040, 080 (MON for a track only one radar fed in the last 5 s; SRC 3D radar;
SIM set), 060 when a radar read a Mode 3/A code and 130 when the altitude is known. Measured on simulated time
(`tests/test_scene.py`, three radars, 4 minutes, two seeds): more than 85 % of visible targets held as one confirmed
track, fewer than 0.5 false or duplicate tracks on average, mean position error under 120 m (45 to 58 m in the runs
that set the gate).

**A simulated scene in the real feeds' form (built).** A receiver that will later take real relayed feeds can be
validated first on simulated ones that differ only in SAC (0) and the SIM bit. A scene profile with `"shape": "real"`
sends what a relayed feed sends after reshape (§6b): for a radar, confirmed tracks only, the same items in every plot,
one plot per datagram, and CAT034 every `status_period_s` (4 s); for the fused feed, CAT062 without Mode 3/A. A profile
with `"kind": "blue"` is one friendly drone reporting its own position as CAT062 with no height. `"area"` gives radars
that are far apart their own targets. `dest_port` sends a feed to its own receiver port, as for a relayed feed.

**TCP (built).** `--transport tcp` sends the same encoder output over one long-lived connection per feed. ASTERIX blocks
carry their own length, so the stream needs no extra framing. A lost connection is retried with backoff from 1 s up to
30 s. Blocks generated while it is down are counted as `dropped`, never queued. The status fields report `transport`,
`connected`, `reconnecting`, and the bytes the receiver's kernel accepted.

**Synthetic-only guard (built).** The sender refuses to start when `--encryption none` (stage 0) is combined with any
input other than `notional`, except a relay that holds (`--dest none`), which sends nothing. It also refuses
`--input fabric` until the fabric input is built. With `--encryption wireguard` it refuses to start unless the kernel
routes the receiver's address through the declared tunnel (`--wg-interface`, checked with `ip route get`). The monitor's
encryption field shows `--encryption`.

## 3. Where a simple-bn runs

The host is a machine of its own: always up and connected, joined to the goatnet, and running the simple-bn and nothing
else. It is never a core-services machine, a host-site node, a person's laptop, or a host shared with another service.

**Detail.** A boundary host is the one place a goatnet talks to an outside party, so it is kept off every machine whose
failure or compromise would take the goatnet with it. The same rule puts the canonical boundary node's edge router on its
own host and never on the core router (F-424, [`03-boundary-node.md`](../tutorials/03-boundary-node.md)). A host shared with another service shares its failures,
its public exposure and its teardown, so a boundary host serves nothing else. The module `terraform/modules/boundary-host`
makes one: a cloud instance in its own Terraform root, joined to the mesh, managed over SSM, with a static Elastic IP a
partner allowlists once and inbound closed except the rules declared for that partner. It is up when site power or a
site link is not. The host is declared per goatnet in the private overlay.

**With a real sensor.** The sender moves inside the goatnet, reading released keys from the fabric, and the boundary host
becomes an egress relay (`input: relay`, built). The receiver's allowlist does not change when the sender moves.

**Detail.** Each relayed feed listens on the host's mesh address (`wt0`) at the feed's `src_port` and sends from the same
`src_port` on every address, so the receiver sees the Elastic IP and the same fixed ports as before. A datagram is
forwarded unchanged only when it came from one of the declared inside senders' mesh addresses, and every ASTERIX data
block in it is whole (its length field matches), of a declared category (`cats`, default 34 and 48) and, when the profile
names `sac` or `sic`, from that radar (I010 of the block's first record). Everything else is counted
(`rejected_source`, `rejected_format`) and dropped. Nothing is queued or stored.

| Relay state | Meaning | Health |
|---|---|---|
| `waiting-for-input` | nothing from a declared sender (relay) or the released key (fabric sender) for 30 s | 503 |
| `holding` | no receiver declared (`--dest none`): input is checked and counted as `held`, nothing leaves | 200 |
| `sending` | forwarding to the receiver | 200 |

A relay that forwards needs encryption stage 1 or 2 and at least one declared sender. A relay that holds needs neither, so
a goatnet can stand its boundary host up, prove the inside path end to end into the `held` counter, and only then
declare the receiver.

## 4. Encryption

A simple-bn sends in three stages. Synthetic data may use any stage. Real data never uses stage 0, and which encrypted
stage it may use depends on who the receiver is ([ADR 1263](../adr/1263-simple-boundary-node-partner-ipsec-gateway-fails-closed-and-real-data-needs-a-validated-module.md)).

| Stage | Transport | Real data allowed | Status |
|---|---|---|---|
| 0 | plain UDP to the receiver's public address | never: integration testing with synthetic data only (SIM bit set) | built |
| 1 | a point-to-point WireGuard tunnel from the simple-bn to the receiver, separate from NetBird | between goatnets of our own sovereign; to an outside receiver only where an authorizing official has accepted it for that interconnection | built: interface and key, and the peer when `wireguard.peer` is declared (`simple-bn-wg-prepare.yml`) |
| 2 | the receiver's own protected transport; built for an IPsec gateway (IKEv2, strongSwan `swanctl`) | to a receiver outside our organization, with strongSwan's cryptography from a CMVP-validated module | IPsec built and measured against a live route-based gateway, as responder behind NAT (§8a); the validated module is not yet on the boundary-host image; other transports designed, per case |

**Which stage carries real data.** WireGuard's primitives (ChaCha20-Poly1305, Curve25519, BLAKE2s) are not on the FIPS
Approved list, and the system's WireGuard waiver excludes traffic across an organizational boundary. Real data to a
receiver outside our organization therefore uses stage 2, with strongSwan's `openssl` plugin against a FIPS-validated
OpenSSL on the boundary host, and the partner authenticated by certificate, or by a pre-shared key that an authorizing
official has accepted for that interconnection. The boundary-host image is stock Ubuntu 24.04 today, so this needs
Ubuntu Pro FIPS on the host first. Each interconnection is recorded in the private overlay before real data crosses it
(ADR 1263, decision 9).

**Detail, stage 1.**

- **Tunnel.** The simple-bn runs one WireGuard interface (`sbn-<receiver>`) with one peer, the receiver. It uses the
  same role the canonical boundary node uses for its partner tunnel, `goat-peering-wg`.
- **Keys.** Each side generates its own key pair, and only the public keys are exchanged, out of band: a signed email
  or a call that reads the key back. Neither side ever holds the other's private key. Our public key, the receiver's
  public key, the receiver's tunnel endpoint and the tunnel addresses are recorded in the private overlay. Our private key
  lives only on the simple-bn host, never on disk elsewhere ([ADR 1186](../adr/1186-credentials-at-rest-are-references-only.md)).
- **Addressing.** The tunnel carries only the receiver's port (`AllowedIPs` = the receiver's tunnel address /32). The
  sender's destination changes from the public address to the tunnel address. Nothing else in the sender changes.
- **What the receiver gets.** A one-page instruction sheet: our public key, our public endpoint, the tunnel addresses and
  the port.

**Stage 2** is decided with the receiver and recorded in the declaration. The sender's destination is whatever address the
receiver's transport presents. For an IPsec gateway the role builds it from `encryption.ipsec` (§7).

**Detail, stage 2 with IPsec (built).**

- **What the role does.** On Ubuntu 24.04 only, it installs `charon-systemd` and `strongswan-swanctl`, renders
  `/etc/swanctl/conf.d/simple-bn.conf` (mode 600, root) and loads it with `swanctl --load-all`. The connection is
  IKEv2, in policy or route mode and as initiator or responder (next items). It then reports whether the tunnel is up
  (`swanctl --list-sas`: IKE and child state, bytes in and out, no addresses) and never fails the run on it, because before the receiver configures their side the far end is not
  there yet. `-e state=absent`, or a declaration back at stage 0 or 1, removes the connection, the interface and the
  packages.
- **Policy or route mode.**

  | | Policy mode (`mode: policy`, default) | Route mode (`mode: route`) |
  |---|---|---|
  | What is encrypted | what matches the selectors: our public /32 to the receiver's inside address | what is routed into the XFRM interface `sbn-xfrm` (the child SA's `if_id_in`/`if_id_out`, default 42) |
  | Selectors | `local_ts` our public /32; `remote_ts` the receiver's inside address | `0.0.0.0/0` both sides unless `local_ts`/`remote_ts` are declared |
  | Sender's source address (`--src-addr`) | our public address, on a dummy interface `sbn-ipsec` | our tunnel address (`tunnel_local`) on `sbn-xfrm` |
  | Route to the receiver | strongSwan's own (table 220) | the role's: the receiver's inside address via `tunnel_remote` on `sbn-xfrm`; charon installs no routes (`install_routes = no`) |
  | Fits | a policy-based gateway that names our public address in its proxy IDs | a route-based gateway with a point-to-point tunnel subnet (for example a /30: their tunnel interface .1, ours .2) |

  A route-based gateway with no proxy IDs proposes `0.0.0.0/0` both ways, and route mode accepts that: the selectors stop
  deciding what is encrypted, the routing table does. The wide selectors are why charon must not install its own routes
  in route mode (a `0.0.0.0/0` route in table 220 would pull every destination into the tunnel); the role sets
  `install_routes = no` in `/etc/strongswan.d/simple-bn.conf` and restarts charon when that changes. The tunnel
  addresses (`tunnel_cidr`, `tunnel_local`, `tunnel_remote`, each literal or from an env var) are private and agreed
  with the receiver; the role refuses route mode without them, with addresses outside `tunnel_cidr`, or with the two
  equal. `receiver.dest_env` must then hold the receiver's inside IPv4 address, which is what the role routes into the
  tunnel.
- **Initiator or responder.**

  | | Initiator (default) | Responder (`initiator: false` or `role: responder`) |
  |---|---|---|
  | Who dials | we dial the gateway | the receiver's gateway dials us |
  | Gateway address | `gateway_env`, required | `gateway_env` optional; without it `remote_addrs = %any` |
  | Who is admitted | the gateway address and `remote_id` (default: that address) with the key | only `remote_id`, required, with the key |
  | Child SA | `start_action = start`, `dpd_action = restart`, `close_action = start` (swanctl's name for restart) | `start_action = none` (wait), `dpd_action = clear`, `close_action = none`; `unique = replace` |
  | Rekeys | strongSwan defaults unless set | the initiator drives them (below) |

  A responder suits a receiver whose gateway sits behind NAT with no fixed public address: we cannot name an address to
  dial, so we wait for theirs. `remote_id` is the identification their gateway sends (an FQDN or key-id string agreed
  with them); with `remote_addrs = %any` it is, with the key, what admits them, so the role refuses a responder without
  it and refuses a `%` wildcard. `start_action = trap` is wrong here: a trap dials out when traffic matches, and there
  is no address to dial. `unique = replace` lets their gateway reconnect from a new NAT mapping without the old IKE_SA
  blocking it.
- **Responder timers.** The initiator drives every rekey; we never start one, so the two sides never collide. IKE
  `rekey_time = 0s` (reauthentication is off by default), so an initiator rekeying its IKE_SA at its own lifetime (for
  example 8 h) is never raced by ours, and a dead initiator's IKE_SA is removed by DPD. Child `rekey_time = 0s` with a
  hard `life_time = 75m`: a 1 h ESP lifetime, a common gateway default, plus 15 minutes, so the initiator always rekeys
  first, and a child SA it failed to rekey is closed rather than used indefinitely. Set `child_life_time` a little above
  the receiver's ESP lifetime, and `ike_rekey_time` or `child_rekey_time` only to override these.
- **Behind 1:1 NAT.** The boundary host has a private address; the cloud maps the static public address onto it. So:
  - IKE leaves from whatever local address routes to the gateway (`local_addrs = %any`), and strongSwan's NAT detection
    moves IKE and ESP to UDP 4500 by itself. Our firewall admits UDP 500 and 4500 from the gateway only (the
    `boundary-host` module's `ipsec_gateway_cidrs`); no ESP (protocol 50) rule is needed. For a responder whose initiator
    has no fixed address, use their NAT's public egress address /32 if they can name it (ask them first); otherwise
    `0.0.0.0/0`, and the declared `remote_id` with the key is then what admits them.
  - Our IKE identity defaults to the public address, which is what the receiver configures as their peer; it is never
    the private one.
  - Policy mode: our traffic selector (`local_ts`) is the public address /32, the address the receiver allowlists for
    the feeds. The role puts that address on a dummy interface (`sbn-ipsec`, systemd-networkd) and the sender binds
    every source port to it (`--src-addr`), so each datagram's inner source is the public address and matches the
    selector. A selector of the private address would make the receiver allowlist an address that means nothing outside
    our cloud account. Route mode needs none of this: the inner source is our tunnel address.
- **Fail closed.** Nothing reaches the receiver in the clear while the tunnel is down; a send fails instead
  (`send-error` on the monitor, retried every send).
  - Policy mode: an unreachable route to the receiver's inside address. While the tunnel is up, strongSwan's route
    (table 220, looked up first) carries the traffic into it.
  - Route mode: with the interface up and no SA, the kernel drops what is routed into `sbn-xfrm`. A rule sends every
    datagram from our tunnel address to table 4101, which holds only the route into the tunnel and an unreachable
    default, so it never falls back to the plain default route. If the interface goes, its address goes with it, and
    a send from that address fails.
- **The traffic selectors (policy mode).** `remote_ts` is the receiver's inside address: by default the host of `receiver.dest_env`,
  or the addresses in `encryption.ipsec.remote_ts_env`. The role refuses a gateway address that lies inside it: the
  tunnel carries traffic to an address behind the gateway, never to the gateway itself.
- **Proposals.** Default IKE `aes256gcm16-prfsha384-ecp384`, ESP `aes256gcm16-ecp384`; a receiver's gateway may need
  others, set in the declaration. An initiator's lifetimes are strongSwan's defaults unless `ike_rekey_time`,
  `child_rekey_time` or `child_life_time` is set; a responder's are above.
- **Authentication.** Certificates are the default for real data; a pre-shared key is used when the partner's gateway
  requires one (ADR 1263, decision 7). `psk`: generate the key (at least 32 random characters), keep it in the secret
  store and give it to the partner by a one-time link. The key is read from the env var `psk_env` names (resolved from
  the secret store by `with-secrets`), refused if shorter than 16 characters, written only into the mode-600 connection file (base64,
  swanctl's `0s` form) by a `no_log` task, and never printed. `pubkey`: our certificate in `/etc/swanctl/x509/`, our key
  in `/etc/swanctl/private/` (made on the host; it never leaves it) and the receiver's CA in `/etc/swanctl/x509ca/` or
  their certificate in `/etc/swanctl/x509/`; the role checks the files are there and does not place them.
- **Hosted variant.** When the receiver has no IPsec gateway, the same responder serves an IPsec client on the receiving
  host itself, host to host, authenticated by a certificate we sign from a per-interconnection authority and given its
  tunnel address by our end (ADR 1263 decision 10; set-up in §8b). The role builds the responder side today for a client
  that holds a static tunnel address; the address pool and the authority are designed, not built.
- **Refusals.** A stage 2 declaration with `ipsec` and a missing or malformed value stops the run with the names of the
  keys and env vars at fault, never their values.
- **The monitor** shows `encryption: ipsec`. Tunnel state on the page (up or down, peer, bytes) is designed, not built: the
  monitor runs unprivileged and reading strongSwan's state needs root, so it would need a new privileged writer.
- **Measured.** Route mode as responder has carried simulated feeds into a live route-based gateway that initiates from
  behind NAT (IKEv2, PSK, FQDN peer identity, `0.0.0.0/0` selectors, a /30 tunnel subnet): about 30 datagrams/s into
  `sbn-xfrm`, no send errors, the receiver's capture matching ours. Policy mode and the initiator have not been run
  against a live gateway. CI loads the rendered initiator (PSK and pubkey), responder and route-mode connections into a
  real charon (`boundary-and-door.yml`, job `ipsec`).

## 5. Pseudo boundary node

A pseudo boundary node moves data to another environment of the same sovereign (another goatnet of ours) rather than to a
partner. Because both sides are ours, the crossing's protection is chosen case by case and recorded with the crossing.

| Crossing mode | What happens | When |
|---|---|---|
| `join` | the pseudo-bn is enrolled in both goatnets and publishes into the far fabric under its own identity there; no new tunnel | the far goatnet's own mesh encryption is enough, and the far side accepts this host as a member |
| `tunnel` | a dedicated WireGuard tunnel to a receiver in the far goatnet, as in §4 stage 1 | the two goatnets must stay unjoined, or the far side has a different encryption requirement |
| `plain` | plain UDP or TCP, as in §4 stage 0 | synthetic data between test environments only |

**Detail.**

- **The `join` mode is a deliberate act.** A host in two goatnets is a path between them. It is therefore declared on
  both sides:
  - Our side declares an outbound crossing.
  - The far side admits the host as a machine of its own ([ADR 1096](../adr/1096-delegated-machine-enrollment.md)), with a
    recorded owner and a scoped group.
  - The far-side ACL lets the host write only its declared keys.
- **One NetBird client serves one goatnet.** A pseudo-bn in `join` mode therefore runs a second client for the far
  goatnet in its own network namespace. The same separation is used for concurrent environment access in
  [`env-doors.md`](env-doors.md) (ADR 1117).
- **The criteria differ from a cross-sovereign crossing.** No SPIRE federation and no release negotiation are needed,
  because both sides already share the identity root.
- **The release rule still applies.** Only declared `release/<set>/**` keys cross. The far goatnet's data owners
  decide what reaches them, as they would for any producer.
- **The crossing mode is a field in the declaration** (`crossing.mode`), not a property of the host.

## 6. The boundary monitor

Every boundary kind writes one status document per boundary service. One small read-only page, `bn-monitor`, shows the
documents for the node it runs on, visible only from inside the goatnet.

**Detail.**

- **Code:** [`ops/simple-bn/bn_monitor.py`](../../ops/simple-bn/bn_monitor.py), standard library only.
- **Routes:**
  - `GET /`: the page, which refreshes itself every 5 s.
  - `GET /api/status`: the documents as JSON.
  - `GET /healthz`.
- **Binding:** the node's mesh address only (`wt0`), never `0.0.0.0`, on port 8740.
- **The receiver's view (`monitor.receiver: true`, built).** A second unit, `bn-monitor-receiver`, serves the same
  read-only page and status API on our tunnel address (stage 1: `wireguard.peer.address`; stage 2 route mode:
  `tunnel_local`), port 8740. The tunnel has one peer, the receiver, so only they reach it, and only through the tunnel;
  nothing is opened on the public address. The documents carry feed names, states and counts and no addresses. The role
  refuses `receiver: true` without a tunnel address, and removes the unit when it is not declared. The receiver reads
  `/api/v1/health` (200 when every feed is sending; 503 naming each feed that is not, `waiting-for-input` meaning
  nothing is arriving from upstream) and `/api/v1/status` (per feed: `received` from upstream, `datagrams` forwarded to
  them).
- **Visibility:** the mesh ACL is the outer gate. For a named audience, publish the monitor as an external service
  with `door: admit` and `host_class: INFRA` ([ADR 1254](../adr/1254-external-service-admit-door-sign-in-and-admission-by-organization-nation-group-or-email.md),
  [`external-service-golden-path.md`](external-service-golden-path.md) §"Door `admit`"). goat then signs people in and
  admits them by organization, nation, group or e-mail. Status: built; the router switch to the door is a follow-up change.
- **What each feed card shows:**
  - name, SAC/SIC, source port and update period
  - state: `sending`, `stalled`, `receiver-refusing`, `send-error` or `stopped`
  - datagrams, detections, tracks, service messages, bytes, send errors and refusals, over 1 minute, 15 minutes, 1 hour,
    24 hours and since start
  - a 60-minute per-minute bar chart
  - live targets by kind
  - last send
  - acknowledgement state
- **The node header shows:** service uptime, the receiver's label (never its address), transport and format, and the
  encryption stage.
- **Colours tell feeds apart:** each feed card takes its own colour, used for its top rule and its chart.

**Schema `goat.boundary-status/v1`.** The status file is the contract between any boundary service and the monitor.

```json
{"schema": "goat.boundary-status/v1", "kind": "simple-bn | bn | pseudo-bn", "node": "...", "service": "...",
 "receiver": {"label": "...", "transport": "udp | tcp | zenoh", "format": "...", "encryption": "none | wireguard | ..."},
 "started": 0, "now": 0, "uptime_s": 0,
 "feeds": [{"feed": "...", "title": "...", "sac": 0, "sic": 0, "src_port": 0, "update_period_s": 1.0,
            "state": "...", "last_send": 0, "last_error": null,
            "ack": {"mode": "none | tcp | app", "note": "...", "last_refused": null},
            "live_targets_by_kind": {}, "totals": {}, "windows": {"1m": {}, "15m": {}, "1h": {}, "24h": {}},
            "per_minute": [[0, 0]]}]}
```

**The canonical boundary node (designed, not built).** A small adapter beside the edge router reads the router's admin
space (sessions, and messages in and out per key) and writes the same document with `kind: bn`, one feed per released set.
The same page then shows a canonical boundary node and a simple-bn side by side.

**Acknowledgement.**

| Mode | What the monitor can say | Status |
|---|---|---|
| UDP, no reply channel | "no acknowledgement", plus `receiver-refusing` when the receiver host rejects the port | built |
| TCP | connected or reconnecting, bytes the kernel accepted, blocks dropped while down | built |
| application acknowledgement | the receiver's `ack` messages (§6a): N received of M sent per SIC and window | built; needs the receiver to send them |

## 6a. Receiver feedback and the status API

The receiver can tell the boundary node what it sees, so integration questions are answered by the two systems rather
than by people out of band. Anyone can read the same answers from a status API without opening the page.

**Feedback channel, `goat.bn-feedback/v1` (built).** The receiver sends one JSON object per UDP datagram, at most 4 KiB.

| Type | Fields | What the monitor shows |
|---|---|---|
| `hello` | `system`, `contact`, `version` | who the receiver is |
| `ack` | `sic`, `received` (datagrams carrying that SIC), `window_s`, `last_tod` | per feed: the receiver received N of the M we sent in the window |
| `status` | `receiving`, `decoding`, `sics`, `positions_set`, `note` | whether they decode, and which SICs they see placed |
| `request` | `sac`, `sic`, `transport`, `wg_public_key`, `wg_endpoint`, `note` | what they want changed, including their WireGuard public key |

- **Two ways in, both from the receiver's address only:**
  - **A reply to a feed's own source port.** The feed's socket is connected to the receiver, so the kernel delivers only
    datagrams from the receiver's address and port. A stateful cloud firewall already admits these replies, so no
    firewall change is needed.
  - **The dedicated `receiver.feedback_port`.** Our firewall must admit UDP to that port from the receiver's address,
    and the sender also checks the source address.
- **Malformed input is dropped.** A message that does not parse, names an unknown type, or comes from another address is
  counted as `rejected` and dropped. Strings are cut to 200 characters, and unknown keys are dropped.
- **The receiver's address is the trust.** Nothing in a message changes the sender's behavior. A `request` is shown to
  the operator, who decides.

**Status API (built), on the monitor's port:**

| Route | Returns |
|---|---|
| `GET /api/v1/status` | every status document on the node: feeds, counts, windows, ack state, feedback |
| `GET /api/v1/feeds/<feed>` | one feed |
| `GET /api/v1/feedback` | what each receiver has reported |
| `GET /api/v1/modifications` | per feed, measured from the blocks: each SAC/SIC and category that arrived, each that went to the receiver, and what this node changed between them (`format` reshape, `identity` rewrite, `sim-bit`); `unchanged` when a relay passes data as it came |
| `GET /api/v1/health` | `200 {"ok": true}` when every feed is sending, otherwise `503` with the problems |

| `GET /api/v1/openapi.json` | the OpenAPI 3 description of the four routes above, with examples |
| `GET /api/docs` | Swagger UI over that description, with "Try it out" on, so each call can be made from the page |

`/api/status` remains as an alias of `/api/v1/status`. The Swagger page loads its script and stylesheet from a pinned `swagger-ui-dist` version with integrity hashes; the description and every call stay on the node.

## 6b. Matching the receiver's formats (reshape)

A receiver is configured for the shape of data it was first given. A CRC that displays simulated radars
sent as CAT048 plots with CAT034 status, and one tracker sent as CAT062, may accept real radars sent as
CAT062 without drawing them: the datagrams arrive, nothing is displayed. Rather than ask the receiver to
reconfigure, a relayed feed can declare `reshape` in its profile, and the node re-encodes what arrives
into the shape the receiver already displays.

| `reshape.to` | Arrives | Leaves |
|---|---|---|
| `radar` | CAT062 tracks | CAT048 target reports (I048/040 range and bearing from the radar, I048/042, 161, 200, 170, 110), plus a CAT034 north marker every `status_period_s` (default 4 s) carrying the radar position in I034/120 |
| `system` | CAT062 | CAT062 with I062/015 service identification (`service_id`, default 1) and I062/100 Cartesian position from a reference point added where missing |

Every reshaped record carries the feed's declared SAC/SIC (`expect_sac`, `expect_sic`), whatever the
source stamped, and the SIM bit clear (set when the profile declares `"simulated": true`). A batched system
picture holds the newest record of each track. A profile's `"dest_port"` sends that feed to its own
port at the receiver. The time of each report is the track's own I062/070.

```json
"radar-a": {"input": "relay", "src_port": 41001, "data_class": "real", "expect_sac": 7, "expect_sic": 42,
            "cats": [62], "reshape": {"to": "radar", "position_env": "RADAR_A_POSITION",
                                      "geoid_undulation_env": "RADAR_A_GEOID_UNDULATION"}}
```

**Height datum of a radar reshape.** I048/110 is a height above mean sea level (CAT048 edition 1.32:
"The height shall use mean sea level as the zero reference level"). I062/130 is a height above the
WGS-84 ellipsoid (CAT062 edition 1.21: "Vertical distance between the target and the projection of its
position on the earth's ellipsoid, as defined by WGS84"). They differ by the geoid undulation N of the
place, tens of metres in most of the world. The profile names an environment variable holding N in
metres for the feed's area (`geoid_undulation_env`), and the reshape writes `I062/130 - N` to I048/110.
With no undulation declared, a height that came from I062/130 is not written: the report leaves without
I048/110 and the feed's status counts it (`heights_withheld`, beside `geoid_undulation_set`). A
height in the wrong datum would read as a valid height tens of metres off, and nothing downstream could
tell; a missing height is seen. A height from I062/136 (a flight level) is written as before. The role
refuses to deploy when a named variable is not set, and the value is never printed or shown.

**Positions are not in the profile.** A sensor's position is a near-secret (ADR 1196). The profile names an
environment variable; the value, `lat,lon[,height_m]`, lives in the secret store and reaches the deploy
through `with-secrets`. The role checks every named variable is set and well formed (the value is never
printed) and writes it into the node's `receiver.env` (mode 600), which the sender reads. The status API
shows `reshape.to`, the stamped SAC/SIC and `position_set`, never the position.

**Finding the difference.** `ops/simple-bn/asterix_inspect.py <pcap>` prints, for each source port, the
categories, SAC/SIC and data items sent. Capture a feed the receiver displays and one it does not, on the
node's egress interface, and compare. The usual causes, most likely first: a SAC/SIC the receiver has not
defined; a category the receiver does not take from that kind of source (CAT062 from a source defined as a
radar); an item the display needs that is missing (I062/015, I062/100); the SIM bit.

## 6c. Real or simulated, when the data does not say (out-of-band sim/real status)

Some sources can run a simulation and send it in exactly the form of real data, with no flag a receiver
or the node can read. A receiver then cannot tell a training run from a real track, and the node cannot
set the SIM bit correctly on its own. For these feeds the node carries a second, out-of-band statement of
what the source is doing, declared by a person from what the source's operators report, and serves it
beside the data. **Built.**

**The plain rule:** the SIM bit on the wire stays what the profile sets (clear for a real feed); the
declaration is shown in the status API next to it, with who set it and when, and never changes what is sent.

| Part | What it is |
|---|---|
| Profile | `"source_can_simulate": true` marks a real feed whose source can simulate; `"source_marks_simulation": true` once the source flags it in its own data (default false) |
| Declaration file | `/etc/simple-bn/sim-status.json` (`goat.sim-status/v1`, root-written, mode 644, no secret), per feed `mode` = `real`, `simulated` or `unknown`, with `set_at`, `set_by`, `note`, and the last 100 changes |
| Declaration tool | `simple-bn-sim-status set <feed> real\|simulated\|unknown --by <who> [--note <text>]`, `clear <feed>` (back to the profile), `show`; on the node, as root. The API has no write endpoint |
| Status | the sender reads the file every 5 s; each feed's status carries `sim_status`: `declared`, `basis` (`profile` when nobody declared, `operator` otherwise), `source_can_simulate`, `source_marks_simulation`, `sim_bit_sent` (`null` when records pass unchanged) and `agrees` (does the SIM bit match the declaration; `null` for `unknown` or pass-through) |
| API | `GET /api/v1/sim-status`, one row per feed; the monitor page shows the declaration on each feed card and flags a SIM bit that disagrees |
| From the feeds service | the people who run a source work in the feeds service's page (goatstuff `goat-feeds`), so they declare there, on the received feed. With `sim_status_url` in the declaration the sender reads that service's `GET /api/sim-status` every 15 s; a profile links a feed to the service's feed id with `"sim_status_from": "<id>"`. For each feed the newest declaration of the host file and the service wins, and `sim_status.source` says which (`host` or `feeds-service`). An unreachable service keeps its last declarations; `sim_status_url` in the status shows whether the last read worked |

```bash
sudo simple-bn-sim-status set drone-a simulated --by "ops desk" --note "source operators report a simulation run"
curl -s http://<node mesh address>:8740/api/v1/sim-status
```

**In-band, from the source (designed, not built).** The lasting answer is a flag in the source's own data,
which the node's converter carries into the SIM bit. For a MAVLink 2 source the standard carrier is
`MAV_MODE_FLAG_HIL_ENABLED` (32) in `HEARTBEAT.base_mode`, with HEARTBEAT sent at 1 Hz on the same stream
as the position messages. When a source sends it, its profile sets `source_marks_simulation`, the converter
sets SIM from it, and the out-of-band declaration becomes a cross-check.

## 7. Declaration

A goatnet declares each simple-bn in its private overlay. Nothing about a receiver appears in this repo.

```yaml
# group_vars/<boundary host group>/simple-bn.yml  (private overlay)
simple_bn:
  node: <host>
  crossing: {kind: simple-bn, mode: plain}          # pseudo-bn: kind pseudo-bn, mode join | tunnel | plain
  receiver:
    label: <receiver name shown in the monitor>
    ref: op://<vault>/<item>                          # host, port, protocol, format live in the store
    dest_env: <ENV VAR>                               # the receiver's HOST:PORT, from the store
    our_source_env: <ENV VAR>                         # our public source address, given to the receiver
    feedback_port: <udp port or 0>                    # optional dedicated port for goat.bn-feedback/v1
    transport: udp                                  # udp | tcp
    format: ASTERIX CAT034/CAT048
  encryption: {stage: 0}                              # 1 = wireguard: our_pubkey, peer_pubkey, peer_endpoint, tunnel /30; 2 = ipsec below
  wireguard: {interface: sbn-<receiver>, listen_port: <udp port, not 51820>}   # stage 1 tunnel, made by simple-bn-wg-prepare.yml
  feeds: [<name>, <name>]
  profiles: <path to the profiles JSON in the overlay>
  sac: <0-255>
  input: notional                                     # notional | relay; stage 0 (encryption none) refuses real data
  scene: {reference: [<lat>, <lon>]}                  # notional: one shared scene; radars have a profile `site`, the fused feed `kind: fused`
  relay: {from_env: <ENV VAR>}                        # relay: the inside senders' mesh addresses, comma-separated, from the store
  fabric: {context: <goat context dir on the host>, relay_dest_env: <ENV VAR>}   # fabric: the relay's mesh address
```

Stage 2 with IPsec (§4) adds `ipsec` under `encryption`. Values that name a party's address or a key come from env vars:

```yaml
  encryption:
    stage: 2
    ipsec:
      mode: policy                                    # policy | route (an XFRM interface; tunnel_* below)
      initiator: true                                 # false (or role: responder): the receiver's gateway dials us
      gateway_env: <ENV VAR>                          # the receiver's gateway address (IKE peer), from the store; responder: optional
      our_public_env: <ENV VAR>                       # optional; default receiver.our_source_env (our static public IPv4)
      remote_ts_env: <ENV VAR>                        # optional; default the host of receiver.dest_env, as /32
      auth: psk                                       # psk | pubkey
      psk_env: <ENV VAR>                              # psk: the pre-shared key, from the store (16 characters or more)
      pubkey: {cert: <file>, key: <file>, remote_cacert: <file>}   # pubkey: file names on the host; or remote_cert: <file>
      local_id: <id>                                  # optional; default our public address
      remote_id: <id>                                 # initiator: optional, default the gateway address; responder: REQUIRED
      ike_proposals: aes256gcm16-prfsha384-ecp384     # optional (this is the default)
      esp_proposals: aes256gcm16-ecp384               # optional (this is the default)
      dpd_delay: 30s                                  # optional; ike_rekey_time, child_rekey_time, child_life_time likewise
      # route mode only: each of tunnel_cidr, tunnel_local, tunnel_remote is literal or <key>_env
      tunnel_cidr: <tunnel subnet, e.g. a /30>
      tunnel_local: <our tunnel address>              # the sender's source address
      tunnel_remote: <their tunnel address>           # the receiver's inside address is routed via it
      if_id: 42                                       # optional; the XFRM interface ID
      local_ts: 0.0.0.0/0                             # optional in route mode (this is the default); remote_ts likewise
```

A route-based gateway behind NAT that dials us, with an 8 h IKE and 1 h ESP lifetime on its side and a /30 tunnel
subnet (their tunnel interface .1, ours .2); `receiver.dest_env` holds their inside address and port:

```yaml
  encryption:
    stage: 2
    ipsec:
      mode: route
      initiator: false
      auth: psk
      psk_env: <ENV VAR>
      remote_id: <the gateway's own identification, agreed with them>
      tunnel_cidr_env: <ENV VAR>                      # e.g. 198.51.100.0/30
      tunnel_local_env: <ENV VAR>                     # e.g. 198.51.100.2
      tunnel_remote_env: <ENV VAR>                    # e.g. 198.51.100.1
      child_life_time: 75m                            # their ESP lifetime + 15 min (the responder default)
```

The fabric sender's profiles name each feed's `key` (under `release/`), `src_port` (the relay's intake port for that
feed) and, when known, `sac`, `sic` and `cats`.

A relay's profiles name each feed's `src_port` and, when the radar is known, its `sac`, `sic` and `cats`. A relay with no
receiver yet leaves `receiver.dest_env` out and holds.

## 8. Establishing a simple-bn

A simple-bn is set up the same way on every goatnet: one declaration, one profiles file, two secret-store references,
one playbook run. The run ends by printing the facts the receiver needs.

1. **Choose the host** by the rule in §3.
2. **Write the profiles file** in the overlay. Give each feed a `base`, a `title`, a `sic` and a fixed `src_port`.
3. **Write the declaration** (§7) in the boundary host group's vars. Name two env vars for the secret-store values:
   `receiver.dest_env` (the receiver's HOST:PORT) and `receiver.our_source_env` (our public source address).
4. **Store both values** in the secret store and add their references to `.envrc.local.d/*.refs`.
5. **Run the playbook** from an assembled tree:
   `ops/ansible/simple-bn.yml -i inventory/<boundary host inventory> -e target=<boundary host group>`.
   The role:
   - refuses an incomplete declaration, a feed without a unique `src_port`, and stage 0 with non-notional input;
   - installs the sender and the monitor, and renders `/etc/simple-bn/receiver.env` (mode 600);
   - checks that every feed is sending from its declared source port and that the monitor answers on the mesh address.

   Run it a second time: that run must report no changes.

   Or build the host as a simple-boundary machine: `goatbuilder join-node <goatnet> --site <site> --node <host>
   --class simple-boundary` (through `ops/scripts/with-secrets`) takes a reachable Ubuntu host to a machine on the mesh,
   and runs this playbook on it in phase 13 (`ops/goatbuilder/targets/simple-boundary.yaml`, gate J10).
6. **Give the receiver the facts the run printed:**
   - our source address;
   - the protocol and their port;
   - the format;
   - for each feed, its SAC, SIC and source port.

   These are everything a receiver's firewall and decoder need. The receiver's own address is never printed.
7. **Open the inbound ports**, each from the receiver's address only: the `feedback_port`, if declared, the
   WireGuard `listen_port`, or for IPsec UDP 500 and 4500 from their gateway (`ipsec_gateway_cidrs` on the
   `boundary-host` module). Both are rules in the host's cloud security group, in the goatnet's Terraform tree. On a
   relay-health host they go in the module's `extra_ingress`, because that group defines its rules inline and a
   separate rule resource would be removed on the next apply.
8. **Before real data,** move to the encrypted stage §4 allows for this receiver. For a receiver outside our
   organization that is stage 2 on a validated module (below); for one of our own goatnets, the WireGuard stage:
   - run `ops/ansible/simple-bn-wg-prepare.yml`, which makes the interface and key pair and prints our public key;
   - the WireGuard port differs from NetBird's 51820, which `wt0` holds on every mesh host;
   - send the receiver the key-exchange sheet, and take their public key and tunnel endpoint back;
   - bring up the peer and change the sender's destination to the receiver's tunnel address.

**When the receiver mandates IPsec instead** (stage 2), take their gateway's values (address, proposals, PSK or
certificates, their inside address), put the address and key in the secret store, declare `encryption.ipsec` (§7),
add the gateway to `ipsec_gateway_cidrs`, and run the playbook. Give them our public address as both peer address and
identity, and as our traffic selector (/32). The run reports whether the tunnel came up. When their gateway has no
fixed address and dials us, declare a responder (`initiator: false`) with the `remote_id` agreed with them, and set
`ipsec_gateway_cidrs` to their NAT's egress /32, or `0.0.0.0/0` if they cannot name one; the run then reports the
tunnel as not up until they connect. When their gateway is route-based, declare `mode: route` with the tunnel subnet
and the two tunnel addresses agreed with them, and give them our tunnel address and the feeds' source ports.

`-e state=absent` stops the services and removes the files.

Checks on the host: `systemctl is-active simple-bn-sender bn-monitor`, `curl http://<mesh address>:8740/healthz`.
Tests: `python3 -m unittest discover -s ops/simple-bn/tests`.

## 8a. Repeating a feed into a partner's IPsec gateway

The same connection recurs: a partner wants our feeds on one UDP port of a host behind their firewall, delivered
through an IPsec tunnel their firewall terminates, from one dedicated address of ours. It is set up in seven steps with
the same questions, the same declaration keys and the same checks each time. Only the answers change, and each answer
has one home in the declaration.

**1. Ask once, in one message.** Each answer lands in one place.

| Ask the partner | Why | Where the answer goes |
|---|---|---|
| The receiving host's inside address and UDP port | the feeds go to that host, not to their tunnel end | `receiver.dest_env` (secret store) |
| Their gateway's public address, or "none, we are behind NAT" | decides initiator or responder | `gateway_env`, or `initiator: false` |
| Route-based or policy-based; for route-based, the tunnel subnet and which address is theirs | decides `mode` | `mode`, `tunnel_cidr`, `tunnel_local`, `tunnel_remote` |
| IKE version, proposals, IKE and ESP lifetimes | our proposals and timers | `ike_proposals`, `esp_proposals`, `child_life_time` (their ESP lifetime + 15 min) |
| PSK or certificates | authentication | `auth`; the key goes to the secret store, never chat |
| The IKE identity their gateway sends, type and exact string | with `%any` as the peer address it is what admits them | `remote_id` |
| SAC/SIC values they want, or acceptance of ours | their decoder | the profiles file |

**2. Choose the environment before anything is built.** Simulated data and real data never share a host: a partner
validating the format gets simulated feeds (SAC 0, SIM set) from a preproduction boundary host; real feeds come later
from a separate production boundary host with its own tunnel, after the partner has validated the format and each
feed's owner has approved its release. Write the choice into the partner documents so they never have to ask which
one they are connected to.

**3. Bring our side up first.** Deploy the feeds and the tunnel end before the partner configures theirs. A responder
waits with UDP 500/4500 open; route mode fails closed, so the feeds stop at `sbn-xfrm` and nothing leaves in the clear.
The partner's first connection attempt then brings the data at once, with nothing left to switch on our side.

**4. Send one sheet.** Our public address, both IKE identities (ours is our public address), the proposals and
lifetimes, the tunnel addresses, the receiving port, the table of feeds (SAC, SIC, category, our source port) and which
environment it comes from. The key goes separately, by a one-time secret-store link. Never resend a key in a later
revision of the sheet: a revision changes the text, not the key.

**5. Watch the bring-up from the charon log.** `journalctl -u strongswan -f` on the boundary host is the source of truth for
what their gateway sends. Read three facts from it before changing anything: the IKE version, the source address and
port (a NAT shows a port other than 500), and the identity in their `IDi`. Then `swanctl --list-sas` shows the IKE and
child state. On our side, `ip -s link show sbn-xfrm` shows packets leaving through the tunnel.

**6. Validate with a capture both sides take over the same minute.** On the boundary host:
`tcpdump -i sbn-xfrm -U -G 60 -W 1 -w /tmp/feed.pcap 'udp and dst port <their port>'`, then
`python3 ops/simple-bn/asterix_inspect.py /tmp/feed.pcap` for datagrams, categories, SAC/SIC and items per source port.
Send the capture with its window in UTC and the expected counts; the partner captures on the receiving host over the
same window and compares. A feed that groups records (the fused system tracks) has far fewer datagrams than records;
say so on the sheet.

**7. Narrow and record.** Once their gateway has connected, set `ipsec_gateway_cidrs` to the source address the charon
log showed (/32) and apply. Record in the goatnet's runbook for this receiver: the tunnel's state and the date it came
up, the answers from step 1, the secret-store references, and any new rule. Before real data crosses, the
interconnection also needs the records ADR 1263 decision 9 lists: a system security plan row, its ports in the ports and
protocols register, an interconnection security agreement, and the first-run evidence from step 6.

**Rules that hold for every such connection.**

- The peer identity is whatever the charon log shows their gateway sending. A spelling relayed by people can differ
  from the configured value; an identity that differs by one character fails authentication as "no matching peer
  config".
- An open UDP 500 receives internet scanners' IKE probes, often IKEv1. Attribute a packet to the partner only by its
  source address and identity, never by its arrival time.
- Ask for IKEv2-only on their gateway. A gateway left on "IKEv1 or IKEv2" may open with IKEv1, which the responder
  refuses.
- Many firewall gateways start a tunnel only when traffic matches it, and our traffic flows the other way. Ask them to
  enable their tunnel monitor (or an equivalent keepalive), or to start the tunnel by hand, once their side is configured.
- Ask for the receiving host's address, not the far tunnel address. If they forward from their tunnel address, the
  receiving host is still the one to capture on.
- In route mode a receiving host inside the tunnel subnet is reached by the subnet's link route in table 4101; one
  outside it, by the route via `tunnel_remote`. Both are rendered by the role.
- `tcpdump` under Ubuntu's AppArmor profile does not stop on a signal, so a `timeout` wrapper leaves it running; use
  `-G <seconds> -W 1`, which stops by itself.

## 8b. When the partner has no IPsec gateway: goat hosts the connection

Some receivers have no firewall that ends IPsec, or will not reconfigure it for one feed. Then goat hosts the whole
connection: our boundary host is the responder exactly as in §8a, and the receiving host runs an IPsec client that dials
it (ADR 1263 decision 10). The receiver changes no firewall; it installs a client and two certificates.

| | Partner gateway (§8a) | Hosted (this section) |
|---|---|---|
| Their tunnel end | their firewall or gateway | an IPsec client on the receiving host |
| Who configures their end | the partner, from our sheet | the partner, from our client sheet and the files we sign |
| Authentication | certificate by default; pre-shared key if their gateway requires one | certificate only, from an authority used for this interconnection alone |
| Their tunnel address | agreed /30, configured by them | handed out by our end (IKEv2 configuration payload, a pool of one address) |
| Firewall rule on our side | UDP 500/4500 from their gateway's address | UDP 500/4500 from the receiving host's public egress address |
| Status | built and measured | responder with certificate auth built; address pool and authority designed, not built |

**Steps.**

1. **Ask** for the receiving host's operating system, its public egress address (or "none fixed"), and the UDP port the
   feeds should reach.
2. **Make the authority** for this interconnection: a key pair and a self-signed certificate, the key stored in the
   secret store as `simple-bn-<receiver>-ca` and never on a host. Sign our boundary host's certificate with it (the host
   makes its own key and a signing request).
3. **Sign their certificate.** The receiving host makes its own key pair and sends a signing request; sign it with the
   authority. Its subject is their identity, declared as `remote_id`. Their private key never leaves their host.
4. **Declare** `encryption.ipsec` with `role: responder`, `mode: route`, `auth: pubkey` and `pubkey.remote_cacert` naming
   the authority's certificate; with the pool built, a one-address pool for their tunnel address.
5. **Send one client sheet:** the client to use (strongSwan on Linux, the built-in IKEv2 client on Windows), our public
   address and identity, the authority certificate, their signed certificate, the proposals, and the port.
6. **Validate and narrow** as in §8a steps 5 to 7: watch the charon log, take the joint capture, narrow the firewall rule
   to the address their client connects from, and record the interconnection.

**Revocation.** Remove the authority's certificate from the boundary host (or replace the one client certificate) and
reload; the authority signs nothing else, so no other receiver is affected. A lost receiving host is a new key pair and
a new signature, never a reused key.

