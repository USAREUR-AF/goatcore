# Peer fault handling

**Status:** Partly built, not deployed on any goatnet. The reader, its deploy role, the `peer` domain in `goat health`, `goat top` and panoscope, the neighbor-observed row in the `time` domain, the federation preflight's clock check, and the rendering of the trust-refusal reading in `goat health` and `goat top` are built; attribution to an owner, notices, tracking and closing are designed. Decided in [ADR 1232](../adr/1232-faults-on-peers-we-do-not-own-follow-one-five-stage-path.md).
**Related:** [`environment-health-surface.md`](environment-health-surface.md) ([ADR 1036](../adr/1036-environment-health-surface.md)) · [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) · [`zenoh-ntp-integration.md`](zenoh-ntp-integration.md) · [`anomaly-probation-lifecycle.md`](anomaly-probation-lifecycle.md) ([ADR 1068](../adr/1068-anomaly-probation-lifecycle.md)) · [`zenoh-brokerless-consequences.md`](zenoh-brokerless-consequences.md)

## What this is

A machine that belongs to someone else can connect to a goatnet in a faulty state. The fabric tolerates
the fault, the cost lands on our router, the owner does not know, and nobody on our side is told. Every
such fault follows the same five stages: the router that sees it measures it, the fault is tied to an
owner, the owner and the admins are both told, the fault is tracked with a deadline, and it closes when
the measurement reads clean.

| Stage | What happens | Status |
|---|---|---|
| 1. Measure | A reader beside the router turns evidence the router already produces into a number per peer | Built for the clock fault: the reader is goatstuff `ops/zenoh-trial-peers/peer-health`, deployed by `ops/ansible/peer-health-deploy.yml` (role `peer-health`). Not deployed |
| 2. Attribute | The peer's Zenoh ID is joined to a machine and an owner | The reader fills name and owner from a file it is given (built); nothing generates that file (not built) |
| 3. Tell | Admins, the owner and the peer itself each see the verdict | Pager built; the reader publishes the per-peer verdict key, `goat health --domain peer`, `goat top` and panoscope render the domain, and `goat health` on a peer prints the router's verdict on that machine (built); `goat health --domain peer` and `goat top` show the trust-refusal reading, with candidate peers apart from peers at fault (built, not deployed); notices not built |
| 4. Track | One probation entry of kind `peer` per fault, with a deadline | Probation list built for the identity kind; the `peer` kind is reserved, not built |
| 5. Close | The entry closes when the same measurement reads clean for a set period | The reader reports a peer as cleared after a clean period (built); closing a tracked entry is not built |

A fault kind supplies a detector for stage 1 and a notice text for stage 3. Stages 2, 4 and 5 are the same
for every kind.

## The worked case: a peer whose clock runs ahead

A Zenoh router stamps each sample with its own clock. A receiving router refuses a timestamp more than
500 ms ahead of its own clock. With `drop_future_timestamp: false` it keeps the sample and replaces the
timestamp with its own.

A peer router that runs 17 seconds ahead therefore has every sample re-stamped. Its stored data is ordered
by arrival at our router, a reader cannot tell that the time was replaced, and our router writes two log
lines per sample:

```
WARN uhlc: incoming timestamp from <peer zid> exceeding delta 500ms is rejected: <peer time> vs. now: <our time>
```

That line carries the peer's Zenoh ID and both times, so the offset and the rate can be read from it with
nothing installed on the peer.

## Where the fault appears

The same fault shows up in three places. The place decides who measures it, who can be named as owner,
and how the owner is told. The partner-owned router across a federated connection is the case with the
least under our control, so the build targets it and treats the other two as the simpler cases.

| | Our own router, inside the goatnet | Another organization's machine, enrolled on the goatnet | A partner-owned router, across a boundary node |
|---|---|---|---|
| Who measures | Any neighboring router; the host also reports its own clock through `ntp-health-agent` | The router it connects to | Only the per-partner edge router on our boundary node (`goat-zenoh-edge`, one container per partner). The core router never sees the partner directly |
| What we can see | The machine, by name | The machine, by name | The partner's boundary router only. A fault on a machine deeper in the partner's network arrives through that one connection |
| Owner | Us | The enrolling organization and person, from the mesh and the organization registry | The partner sovereign, through the contact in the federation package. No machine owner is known to us |
| How the owner is told | Health surface and pager | Notice to the person; the machine can read its own verdict on the fabric | Organization-to-organization notice, and one released key the partner subscribes to. Health keys do not cross the boundary: only `release/<set>/` does |
| Who fixes it | Us | That organization | The partner. Our only actions are re-stamping and reporting |
| Whose clock is right | Majority among the goatnet's own machines ([ADR 1037](../adr/1037-fabric-internal-time-synchronization-first.md)) | Same | Two separately synchronized fabrics meet. Neither majority applies to the other, so each side is compared against UTC and the verdict says which side was grounded |
| Check before connecting | Join preflight | Machine enrollment | Federation preflight (`ops/federation/standup/preflight.py`), check `clock` |

## What the router names when the fast clock is behind a partner

Measured on 2026-11-13 with three Zenoh 1.9.0 routers in containers: ours, the partner's boundary router
connected to ours, and a router one hop deeper in the partner's network, with a publisher behind it. A
router's clock was set 17 seconds ahead where the table says fast. Every router stamps samples it
receives from a client; our router ran with `drop_future_timestamp: false`.

| Case | Partner boundary router | Deeper partner router (stamps the sample) | What our router logs | What our readers receive |
|---|---|---|---|---|
| A | fast; stamps the sample itself | none | Rejections naming the boundary router's ID | Every sample, re-stamped by our router |
| B | correct, `drop_future_timestamp: false` | fast | Nothing. The partner's boundary router rejects and re-stamps | Every sample, stamped by the partner's boundary router |
| B2 | correct, `drop_future_timestamp: true` | fast | Nothing | No samples. The partner's boundary router drops them |
| D | fast | correct | Nothing. A router does not re-stamp a sample that already carries a timestamp | Every sample, with the deeper router's correct stamp |
| F | fast | fast | Rejections naming the **deeper** router's ID, a router we hold no session with | Every sample, re-stamped by our router |

Three consequences for the detector.

- **The ID in a rejection line is the router that stamped the sample, not the connection it arrived on.**
  When a whole partner network runs fast (case F), the ID belongs to a router we never see.
- **The connection is named by which of our routers logged the line.** One reader runs beside each
  per-partner edge router, so every rejection that reader counts belongs to that one peering, whatever
  ID it carries. On a core router the reader joins the ID to the session list and reports a line whose
  ID holds no session as arriving through another router.
- **A fast clock one hop inside a correct partner network never reaches us** (cases B and B2). The
  partner's own boundary router sees it. Our reader reports only what crosses the boundary.

## Stage 1: measure at the router we own

The router is the only vantage point we control for a machine we do not own.

- **One reader beside each router that terminates an outside connection.** It follows the router's log
  stream and admin space, counts fault events per peer Zenoh ID, and publishes the result. It stores
  nothing and enforces nothing. If it stops, the fabric behaves as before.
- **One play deploys it.** `ops/ansible/peer-health-deploy.yml -e target=<group>` installs the reader and a
  host unit that feeds it. The unit follows the router container's log, keeps the timestamp-rejection
  lines in a file the reader follows, and writes the router's session list from its admin space. The
  reader connects with its own leaf (`goat-peer-health-trial-peer`); the router's access rules let that
  leaf publish `ops/peer/health/**` and `ops/peer/verdict/**` and nothing else. Built, not deployed.
- **It publishes on the existing health contract.** A new domain `peer` on `ops/peer/health/<site>` as
  `EnvHealthV1` gains a `PeerFaultDetailV1` detail. `goat health`, panoscope and stethoscope render a new
  domain without a new surface ([`environment-health-surface.md`](environment-health-surface.md) §4.4).
- **The clock reading goes in the `time` domain**, on `ops/time/health/<site>`, as a row marked observed
  by a neighbor. It is one more pairwise offset for the majority comparison in
  [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) §5.2. Built: the time roll-up (goatstuff
  `ops/zenoh-trial-peers/time-health`) reads its site's `peer` reading and adds the row with
  `observed_by_neighbor` set. The row is outside the site's color and host count, which grade the site's
  own clocks.
- **The reading is graded before anyone is blamed.** If the router's own host clock is not synchronized,
  or most peers show an offset in the same direction, the verdict is "our clock". If the reader cannot
  measure, it reports unmeasured and never green.

| Fault on the peer | Evidence at our router | Reading |
|---|---|---|
| Clock ahead of the limit | Timestamp rejection lines with the peer's Zenoh ID | Offset in seconds; rejections per minute |
| Trusts a root the router no longer presents | A source address that keeps opening connections to the router port and holds no session; a declared peer absent from the session list | Time since last session; connection attempts per minute |
| Certificate close to expiry | The certificate the peer presents at connect | Days remaining |
| Connects and drops in a loop | Admission events per peer | Connects per minute |
| Publishes above its envelope | Per-peer message and byte rates | Rate against the declared envelope ([`qos-throughput-envelopes.md`](qos-throughput-envelopes.md)) |

The second row, measured on Zenoh 1.9.0 (2026-11-14): when a client ends the TLS handshake because it does
not trust the router's root, the router logs one line from `zenoh_link_tls::unicast` that ends
`received fatal alert: UnknownCA`. Stock Zenoh logs it as a warning at the default level. A router build
with a patched accept loop logs it only at debug, so that router needs
`goat_zenoh_rust_log: "info,zenoh_link_tls::unicast=debug"` (role `goat-zenoh`) for the line to exist. The
line never carries the client's address or Zenoh ID, at any log level. The detector therefore counts the
lines, and names as candidates the declared peers that hold no session; it cannot name the client from
the log. Naming it for certain needs the network as evidence: the sources of new connections to the
router port compared with the sources in the router's session list. The feed keeps the line (built); the
detector in the reader and the network comparison are not built.

## Stage 2: attribute the peer to an owner

A Zenoh ID is not actionable. Each reading is joined to a machine name and an owning organization before
it is shown: Zenoh ID to the session's source address (router admin space), to the mesh peer name, to the
machine's declared owner in the goatnet's registry. A peer that maps to no declared owner is itself a
fault, the kind [`anomaly-probation-lifecycle.md`](anomaly-probation-lifecycle.md) reserves.

Across a boundary node the join stops at the partner: the reading is attributed to the peering, and the
owner is the partner's contact in the federation package.

## Stage 3: tell three audiences

- **Admins** see the peer row turn amber or red on the health surfaces. A fault that stays red pages
  ([ADR 1168](../adr/1168-self-hosted-paging-and-automatic-oncall-enrolment.md)).
- **The owner** receives a notice from the goatnet's own mail sender naming the machine, the measured
  value, the limit and the fix. One template per fault kind. An operator approves the first notice for
  each fault. Reminders and the closing notice are sent automatically.
- **The peer itself** can read its own verdict. The reader publishes each peer's reading on a key that
  peer is allowed to read, so `goat health` run on the owner's machine prints the fault. For a federated
  partner the key is one released key per partner under that partner's release set.

## Stage 4: track each fault as one entry with a deadline

Each fault becomes a probation entry of kind `peer` with a first-seen time, an owner, a deadline and a
state: detected, owner notified, cured, or escalated when the deadline passes. Escalation flags and pages.
It never disconnects the peer; disconnecting is an operator decision.

## Stage 5: close automatically

The entry closes when the measurement that opened it reads clean for a set period, and the owner is told
it closed. For the clock fault that is zero rejected timestamps from the peer for the period. A fault that
returns reopens the same entry with its history.

## Stopping the fault before it connects

- **Federation preflight checks the clock.** Two federated fabrics are synchronized separately, so neither
  side can measure the other before the tunnel exists. Check `clock` (GB-268) reads each edge and core
  host's own time service and fails a host that is unsynchronized or more than the budget from UTC
  (`FED_CLOCK_BUDGET_MS`, default 250 ms, half of the router's 500 ms limit). Built, not yet run against a
  live peering ([`federation-preflight.md`](federation-preflight.md)).
- **Join preflight gains the same check** for a machine joining a goatnet. `goatbuilder join-node` phase 3
  reads the node's own time service before anything on the node changes and refuses an unsynchronized node
  or one over the budget (GB-297), by the rule of check `clock`. Built, not yet run against a live node.
- **A reader can see a replaced timestamp.** A sample the router re-stamped is marked as such when the
  per-sample time class of [`zenoh-ntp-integration.md`](zenoh-ntp-integration.md) §3 ships. Designed.

## Build order

Each step is one revertible change.

1. **Measure the federated case.** Measured; see "What the router names when the fast clock is behind a partner".
2. **The reader with the clock detector**, beside a per-partner edge router and beside the core router.
   Result: the clock fault shows on `goat health` and panoscope with the machine or the peering named.
   Built, not deployed.
3. **The per-peer verdict key** and the `goat health` line on the peer's side. Built.
4. **Probation entries of kind `peer`** with owner, deadline, automatic close, the pager on escalation,
   and the owner notice template.
5. **The second detector: a source that connects and holds no session.** Building a second detector on
   the same frame is the test that the method is general. The renderers are built, not deployed:
   `goat health --domain peer` prints the trust verdict (clean, unattributed, candidates or not
   measured), the TLS alert counts per name, and the candidate peers in their own block, never counted
   as peers at fault; `--json` carries `trust_verdict`, `tls_alerts`, `declared_peers_absent` and each
   row's `source_address` and `absent_since_unix_ms`; the `goat top` PEER chip shows the site grade,
   which the trust reading can turn amber or unmetered, never green.
6. **The clock check in join and federation preflight.** Built for federation and for join; neither yet run
   against a live peering or node.
