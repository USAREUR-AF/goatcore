# ADR 1270 — first-bite is the home of capture and inspection, and every end node taps its sensor link by default

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-27 |
| Deciders | operator |
| Repo scope | first-bite (`mind --tap-only`, `mind goat watch --save`, the `first-bite-tap@` unit), goat core (this decision; the `goat-zenoh` role's write and read rules), node builds that install the tap (goatbuilder's first-bite target, sensor node repos), private overlays (reader lists, portal records) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1032](1032-zenoh-volatile-non-persisted-namespace-class.md) (`volatile/`, routed never stored), [ADR 1123](1123-client-demand-must-not-amplify.md) (client demand never amplifies), [ADR 1105](1105-pubsub-org-admin-co-publish-write-grants.md) (single-owner write roots), [ADR 1040](1040-zenoh-advanced-pubsub-liveliness-adoption.md) (`@`-prefixed chunks), [ADR 1027](1027-partner-release-set-governance.md) (release sets), [`zenoh-data-fabric.md`](../design/zenoh-data-fabric.md) §"No one subscription may match the whole keyspace" |

## TL;DR (for a layperson)

When a sensor's data looks wrong on the map, the first question is what the sensor actually
sent. Today the answer means someone logging into the node beside the sensor and recording
traffic by hand, after the problem has passed. This decision makes every node that sits beside
a sensor record that sensor's traffic all the time, keep the last hour on its own disk, and
send each finished minute over the fabric to a short named list of people allowed to read it.
Anyone on that list can then save the exact bytes and check the work themselves. The recording
tool is first-bite, which already decodes sensor traffic; no new tool is added.

## Context

first-bite's `mind` already captures and inspects sensor links: `mind --tap-only` writes the
link to rotating pcap files and publishes each finished file to a Zenoh key, `mind --pcap-file`
replays one, `mind discover` surveys an unknown sensor's traffic, and `mind goat watch` reads
the fabric. The first sensor nodes that ran the tap published to keys under their own slot
anchor (`<anchor>/<sensor>/pcap/<node>`). That placement has three defects. The anchor's
`<anchor>/**` storage persists every slice on the router, which is the disk-fill
[ADR 1032](1032-zenoh-volatile-non-persisted-namespace-class.md) exists to prevent. The
universal peer read rule (`*/**`) covers those keys, so every peer on the goatnet can read a
vendor's raw bytes. And nothing turns the tap on for the next node: it was installed by hand.

Raw sensor bytes are more sensitive than the decoded tracks made from them. They carry the
vendor's protocol, configuration and timing, which the vendor has not released, and they are
never data a partner receives.

## Decision

1. **first-bite is the home of capture and inspection.** Capturing, saving, replaying and
   surveying a sensor link are first-bite `mind` commands. A node build, a node repo or an
   operator tool that needs a capture runs `mind`; it does not carry its own capture code.

2. **Every end node taps its sensor link by default.** A node whose role is to sit beside a
   sensor (a sensor node, a tap machine, a node that translates a sensor's feed) installs
   first-bite from its attested release and enables one `first-bite-tap@<sensor>` instance per
   sensor it serves, filtered to that sensor's address and port. The tap only listens; it never
   opens a connection to the sensor and never transmits on the sensor link. Turning it off is
   a declared per-node exception with a reason, not the default.

3. **Key grammar: one key per node and sensor, in `volatile/`.**

   ```
   volatile/<anchor>/stream/@pcap/<sensor>/<node>
   ```

   | Chunk | Value |
   |---|---|
   | `volatile/<anchor>/stream/` | the node's own non-persisted stream space ([ADR 1032](1032-zenoh-volatile-non-persisted-namespace-class.md)); no router storage and no audit storage keep it |
   | `@pcap` | a verbatim chunk: Zenoh's `*` and `**` never match a chunk that starts with `@`, so no wildcard rule reaches these keys and no wildcard subscription receives them unless it names `@pcap` |
   | `<sensor>` | the sensor's name on that node, the same name as the tap instance |
   | `<node>` | the node's system name |

   One sample is one finished pcap file, content type `application/vnd.tcpdump.pcap`, written
   by the tap and published unchanged.

4. **One-minute slices, bounded.** A slice closes at 60 seconds or at 1 MiB, whichever comes
   first (`--pcap-slice-seconds 60 --pcap-slice-mb 1`), so a published sample stays under the
   [ADR 1032](1032-zenoh-volatile-non-persisted-namespace-class.md) chunk ceiling.

5. **A local ring is the record.** The node keeps the last 60 slices on its own disk
   (`--pcap-keep 60`, one hour at one-minute slices, at most 60 MiB per sensor). A slice that
   fails to publish stays in the ring. The fabric copy is a convenience; the ring is what an
   investigation falls back to.

6. **A reader group, default deny.** The router grants subscription and delivery on
   `volatile/*/stream/@pcap/**` only to the subjects in a per-goatnet list,
   `zenoh_capture_readers`, each row with an owner, a reason and an `until:` date, the same
   shape as `zenoh_fabric_wide_read`. An empty list means nobody reads. Because of the verbatim
   chunk, neither the universal peer read (`*/**`) nor the named fabric-wide grant (`**`)
   reaches these keys, so being allowed to read the whole fabric does not make a reader a
   capture reader. The write grant is the node's own anchor only:
   `volatile/<anchor>/stream/@pcap/**` is added to the node's existing write rule, because
   `volatile/<anchor>/**` does not include a verbatim chunk.

7. **Never released.** Capture keys are never placed under `release/` and never named in a
   release set. A partner who needs to check a sensor gets decoded data or a capture handed
   over deliberately, not a fabric grant.

8. **Raw is shed before decoded.** Captures ride the bulk class of `volatile/` (congestion
   control Drop, lowest priority), and decoded tracks ride the mission class. Under bandwidth
   pressure the router and links drop capture samples first; the node's ring still holds them.
   A node that must save its uplink for decoded data stops publishing (`--pcap-publish`
   removed) and keeps the ring.

9. **Existing keys move to the grammar.** A tap that publishes to `<anchor>/<sensor>/pcap/<node>`
   changes its `--pcap-key` to `volatile/<anchor>/stream/@pcap/<sensor>/<node>` in the same
   change that grants its write rule. There is no period of publishing to both keys. Slices
   already stored under the old keys stay in that anchor's router storage until its retention
   removes them; deleting them sooner is a separate operator decision.

### How this respects the fabric's rules

| Rule | How the tap meets it |
|---|---|
| Brokerless, no central storage | `volatile/` is never stored on a router or in the audit bucket. A reader who wants files saves them on their own disk with `mind goat watch <key> --save <dir>`. |
| No subscription matches the whole keyspace | A reader names a node and sensor, or one sensor across nodes (`volatile/*/stream/@pcap/<sensor>/*`). `mind goat watch` refuses an expression with no literal chunk. |
| Client demand never amplifies fabric load ([ADR 1123](1123-client-demand-must-not-amplify.md)) | The tap publishes once per slice at the sensor's own rate, whoever is reading. It answers no queries and keeps no queryable, so a reader cannot make a node send more or send old slices. Fan-out is bounded by the named reader list. |

## Built and not built

| Piece | State |
|---|---|
| `mind --tap-only` with ring, slice limits and publish to a Zenoh key through a local REST endpoint, retried, kept in the ring on failure | built, in first-bite main and its release candidate 1.2.0-rc.2 |
| `first-bite-tap@<sensor>` systemd unit and installer | built (first-bite `deployment/tap`) |
| `mind goat watch <key> --save <dir>`: each received capture saved unchanged, proven by a hash test | built, in review (first-bite #93) |
| `mind --pcap-file`, `mind discover` on a saved slice | built |
| Taps on sensor nodes | first installs use the release candidate and publish to the node's local router; which hosts run one is recorded in each goatnet's private overlay. Taps installed earlier by hand used the old key shape |
| Write rule `volatile/<anchor>/stream/@pcap/**` and the `capture-r` read rule from `zenoh_capture_readers` in `roles/goat-zenoh` | not built |
| An `acl-evaluate.py` vector proving `*/**` and `**` do not include a `@pcap` key and `capture-r` does | not built; the read and write rules land with it |
| Tap enabled by default in node builds (goatbuilder's first-bite target, sensor node repos) | designed in this ADR; not built |
| Native Zenoh publish from the tap (no local REST endpoint) | designed, not built |

## Alternatives considered

- **A new capture tool or a capture role inside each node repo.** Rejected: first-bite already
  captures, rotates, publishes and decodes. A second tool splits the place an operator looks
  and duplicates the capture code that must stay passive.
- **Keep captures under the anchor (`<anchor>/…/pcap/…`).** Rejected: the anchor's storage
  persists every slice, and the universal read gives every peer the vendor's raw bytes.
- **`volatile/<anchor>/stream/pcap/…` without the verbatim chunk, with a deny rule for
  everyone else.** Rejected: Zenoh rules match by inclusion and a deny wins over an allow, so
  "deny all except the readers" cannot be written; the universal `*/**` read would grant it.
  The verbatim chunk keeps the keys outside every wildcard rule, so the reader rule is the only
  way in.
- **Central capture storage that readers query.** Rejected: it re-adds a broker, and a query
  path lets reader demand pull bulk data across the fabric (ADR 1123).
- **Capture only on request.** Rejected: the traffic that explains a problem has passed by the
  time someone asks. The ring costs at most 60 MiB per sensor.

## Consequences

**Positive.** Anyone on the reader list can save the exact bytes a sensor sent in the last
minute and check a decoder, a converter or a claim about the sensor themselves. The last hour
is on the node even when the uplink is down. Raw bytes leave the universal read and router
storage.

**Negative.** A reader not on the list sees nothing and gets no error (silent denial, as for any
ungranted key), so adding a reader is a router configuration change and restart. Every sensor
node spends up to 60 MiB of disk and one capture process per sensor. Captures are outside the
audit trail by construction, as all of `volatile/` is.

## Assessor view (SCA-R)

Access to raw sensor bytes is a named, dated, owned list enforced by the router's access
control (AC-3, AC-6); it is default deny and outside the universal read. Captures are never
part of a release set (AC-4, AC-21). They are not stored centrally and are outside the audit
bucket (AU-2 scope stated, not silent). The tap is receive-only on the sensor link (SC-7).
