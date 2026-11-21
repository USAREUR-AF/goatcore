# Simple boundary node, pseudo boundary node, and the boundary monitor

**Status: WIP.** The sender and the monitor are built and run on one goatnet with synthetic data only. The TCP transport and the
synthetic-only guard are built. The fabric subscription, the WireGuard stage, the ansible role and the pseudo boundary
node are designed, not built. The gate that lifts WIP: one non-synthetic feed leaves through the WireGuard stage and the monitor shows it for
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
   (designed, not built)            (built)            (built)                        (UDP built)
                                       │
                                       └──► /run/simple-bn/status.json ──► bn-monitor (mesh address only)
```

**Detail.**

- Code: [`ops/simple-bn/simple_bn_sender.py`](../../ops/simple-bn/simple_bn_sender.py), Python standard library only.
- **One UDP socket per feed.** Each feed therefore has its own source port as well as its own ASTERIX source identity
  (SAC/SIC), so a receiver, a packet capture and the monitor can each tell the feeds apart.
- **The socket is connected.** If the receiver host answers ICMP port-unreachable, the next send fails with
  `ECONNREFUSED`. The sender records this as `receiver-refusing`. It is the only signal a one-way UDP stream gives.
- **Feeds today are notional sensors built into the sender.** Three generic sensor classes, each with its own update
  period, range and target mix:
  - `hemispheric-cuas`: drones, helicopters, aircraft, 1 s updates.
  - `counterfire`: projectile salvos on ballistic arcs, 1 s updates.
  - `surveillance-3d`: drones, helicopters, aircraft, 2 s updates.
- **A deployment names its own feeds** with a profiles file (`--profiles`, JSON) that sets `base`, `title` and `sic` per
  feed. Deployment names stay in the private overlay, never in this repo.
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

**The fabric input (designed, not built).** The sender subscribes to the declared `release/<set>/<feed>/**` keys and
encodes each sample. Only released keys leave, for the same reason the canonical boundary node republishes only
`release/` ([ADR 1208](../adr/1208-sovereign-environments-release-namespaces-and-grant-encryption.md)): what crosses a
boundary is a deliberate act of the data's owner, not a property of where the boundary host sits. Until it is built, the
notional feeds are the only input, and the notional feeds are synthetic.

**TCP (built).** `--transport tcp` sends the same encoder output over one long-lived connection per feed. ASTERIX blocks
carry their own length, so the stream needs no extra framing. A lost connection is retried with backoff from 1 s up to
30 s. Blocks generated while it is down are counted as `dropped`, never queued. The status fields report `transport`,
`connected`, `reconnecting`, and the bytes the receiver's kernel accepted.

**Synthetic-only guard (built).** The sender refuses to start when `--encryption none` (stage 0) is combined with any
input other than `notional`. It also refuses `--input fabric` until the fabric input is built. The monitor's encryption
field shows `--encryption`.

## 3. Where a simple-bn runs

The host is a machine in the goatnet that is always up and connected, and that is not a core-services machine, not a
host-site node and not a person's laptop.

**Detail.** A boundary host is the one place a goatnet talks to an outside party, so it is kept off every machine whose
failure or compromise would take the goatnet with it. The same rule puts the canonical boundary node's edge router on its
own host and never on the core router (F-424, [`03-boundary-node.md`](../tutorials/03-boundary-node.md)). A cloud instance in the goatnet's own Terraform tree fits:
it has a fixed public address a partner can allowlist, and it is up when site power or a site link is not. The chosen host
is declared per goatnet in the private overlay.

## 4. Encryption

A simple-bn sends in three stages. Synthetic data may use stage 0. Anything else needs stage 1 or 2.

| Stage | Transport | Use | Status |
|---|---|---|---|
| 0 | plain UDP to the receiver's public address | integration testing with synthetic data only (SIM bit set) | built |
| 1 | a point-to-point WireGuard tunnel from the simple-bn to the receiver, separate from NetBird | any real data | designed |
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
| application acknowledgement | the receiver returns the last sequence number it processed; the monitor shows the gap | designed, needs the receiver's agreement |

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
    transport: udp                                  # udp | tcp
    format: ASTERIX CAT034/CAT048
  encryption: {stage: 0}                              # 1 = wireguard: our_pubkey, peer_pubkey, peer_endpoint, tunnel /30
  feeds: [<name>, <name>]
  profiles: <path to the profiles JSON in the overlay>
  sac: <0-255>
  input: notional                                     # stage 0 (encryption none) refuses anything else
```

## 8. Install by hand (until the role exists)

```
install -d -m 755 /opt/simple-bn; install -d -m 700 /etc/simple-bn
install -m 644 simple_bn_sender.py bn_monitor.py /opt/simple-bn/
install -m 600 receiver.env /etc/simple-bn/receiver.env
install -m 644 systemd/*.service /etc/systemd/system/
systemctl daemon-reload; systemctl enable --now simple-bn-sender bn-monitor
```

Checks: `systemctl is-active simple-bn-sender bn-monitor`; `curl http://<mesh address>:8740/healthz`;
`python3 -m unittest discover ops/simple-bn/tests`.
