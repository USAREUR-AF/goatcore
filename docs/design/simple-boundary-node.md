# Simple boundary node, pseudo boundary node, and the boundary monitor

**Status: WIP.** The sender and the monitor are built and run on one goatnet with synthetic data only. The TCP transport, the
synthetic-only guard, fixed per-feed source ports, the ansible role (`ops/ansible/simple-bn.yml`) and the egress relay
(`--input relay`) and the fabric input (`--input fabric`) are built. The WireGuard peer step and the pseudo boundary node
are designed, not built. The gate that lifts WIP: one non-synthetic feed leaves through the WireGuard stage and the monitor shows it for
seven days without a gap. Decision record: [ADR 1253](../adr/1253-simple-boundary-node-pseudo-boundary-node-and-one-boundary-monitor.md).

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

A simple-bn sends in three stages. Synthetic data may use stage 0. Anything else needs stage 1 or 2.

| Stage | Transport | Use | Status |
|---|---|---|---|
| 0 | plain UDP to the receiver's public address | integration testing with synthetic data only (SIM bit set) | built |
| 1 | a point-to-point WireGuard tunnel from the simple-bn to the receiver, separate from NetBird | any real data | interface and key step built (`simple-bn-wg-prepare.yml`); the peer step designed |
| 2 | the receiver's own protected transport (for example their VPN or an IPsec gateway) | when the receiver mandates it | designed, per case |

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
receiver's transport presents.

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
picture holds the newest record of each track. The time of each report is the track's own I062/070.

```json
"radar-a": {"input": "relay", "src_port": 41001, "data_class": "real", "expect_sac": 7, "expect_sic": 42,
            "cats": [62], "reshape": {"to": "radar", "position_env": "RADAR_A_POSITION"}}
```

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
  encryption: {stage: 0}                              # 1 = wireguard: our_pubkey, peer_pubkey, peer_endpoint, tunnel /30
  wireguard: {interface: sbn-<receiver>, listen_port: <udp port, not 51820>}   # stage 1 tunnel, made by simple-bn-wg-prepare.yml
  feeds: [<name>, <name>]
  profiles: <path to the profiles JSON in the overlay>
  sac: <0-255>
  input: notional                                     # notional | relay; stage 0 (encryption none) refuses real data
  scene: {reference: [<lat>, <lon>]}                  # notional: one shared scene; radars have a profile `site`, the fused feed `kind: fused`
  relay: {from_env: <ENV VAR>}                        # relay: the inside senders' mesh addresses, comma-separated, from the store
  fabric: {context: <goat context dir on the host>, relay_dest_env: <ENV VAR>}   # fabric: the relay's mesh address
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
6. **Give the receiver the facts the run printed:**
   - our source address;
   - the protocol and their port;
   - the format;
   - for each feed, its SAC, SIC and source port.

   These are everything a receiver's firewall and decoder need. The receiver's own address is never printed.
7. **Open the inbound ports**, each from the receiver's address only: the `feedback_port`, if declared, and the
   WireGuard `listen_port`. Both are rules in the host's cloud security group, in the goatnet's Terraform tree. On a
   relay-health host they go in the module's `extra_ingress`, because that group defines its rules inline and a
   separate rule resource would be removed on the next apply.
8. **Before real data,** do the WireGuard stage (§4) with the receiver:
   - run `ops/ansible/simple-bn-wg-prepare.yml`, which makes the interface and key pair and prints our public key;
   - the WireGuard port differs from NetBird's 51820, which `wt0` holds on every mesh host;
   - send the receiver the key-exchange sheet, and take their public key and tunnel endpoint back;
   - bring up the peer and change the sender's destination to the receiver's tunnel address.

`-e state=absent` stops the services and removes the files.

Checks on the host: `systemctl is-active simple-bn-sender bn-monitor`, `curl http://<mesh address>:8740/healthz`.
Tests: `python3 -m unittest discover -s ops/simple-bn/tests`.
