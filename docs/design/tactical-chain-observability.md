# Daisychain: tactical chain observability, one answer per path from sensor to receiver

**Status: designed, not built.** Code home: goatstuff, inside `goat-feeds` (Fieldkit Feeds), which already holds the
site's link registry; the boundary-node pieces (§5) stay in goat core. Decision record: [ADR 1266](../adr/1266-a-tactical-data-chain-is-declared-source-to-receiver-judged-hop-by-hop-and-pages-when-data-stops-between-two-hops.md),
which amends [ADR 1260](../adr/1260-every-tactical-link-in-one-registry-with-one-health-record.md). The pieces it reads that exist today are
listed in §6.

## 1. What it answers

For every path that carries a sensor's data to a receiver off the fabric, at any moment:

- Is data flowing end to end?
- If not, at which hop does it stop, and what are the counts on each side of that hop?
- Is what leaves for the receiver what the receiver was told to expect (categories, SAC/SIC, port)?
- Does the receiver confirm receipt, or is that unknown?

and it pages a person when the answer changes from flowing to broken.

## 2. A chain

A chain is one source, one receiver, and the ordered hops between them. Example, a radar to a partner's
control center through a simple boundary node:

| # | Hop | Evidence it moves data | Where the evidence comes from |
|---|---|---|---|
| 1 | Source on the fabric | messages in at the first pipe; the source's own heartbeat (CAT034 status) where it has one | pipe health record (ADR 1260 §2) |
| 2 | Pipe (relay, decode, encode) | in, converted, sent, per window | pipe health record |
| 3 | Boundary node receives | `received` per feed | boundary monitor `/api/v1/status` |
| 4 | Boundary node sends | `datagrams` per feed; output SAC/SIC and categories | `/api/v1/status`, `/api/v1/modifications` |
| 5 | Tunnel | WireGuard handshake age, or IPsec SA state; bytes | tunnel-state file on the boundary host (§5) |
| 6 | Receiver | `goat.bn-feedback/v1` acknowledgements, when the receiver sends them | `/api/v1/feedback` |

Chains are generated, not written: the estate names the source key, the processors-pipeline declaration names
the pipes and their outputs, the boundary-node declaration names the feed, its tunnel and its receiver, and the
receiver contract (categories, SAC/SIC, port) comes from the boundary node's profile (`expect_sac`/`expect_sic`,
`dest_port`, the reshape target). A generator joins them on the pipe's output address and the feed's source
port. A link that joins no chain is listed as such.

## 3. The verdict

Daisychain reads each hop's record every 15 s and keeps 1 minute and 15 minute windows.

| Verdict | Rule | Severity |
|---|---|---|
| `flowing` | every hop's count rose in the last 2 minutes, and hop 4's output identities and categories match the contract | none |
| `source-quiet` | hop 1 did not rise, and no later hop rose either | warning |
| `broken at <hop N>` | hop N−1 rose and hop N did not, for 2 minutes | critical |
| `wrong-shape` | hop 4 output differs from the receiver contract | critical |
| `receipt-unconfirmed` | hops 1 to 5 flowing; no acknowledgement from the receiver | none; shown |

A source with a heartbeat that stops is `source-down` (warning, or critical if the chain is declared
continuous). A bursty source with no heartbeat can only be `source-quiet`; the verdict carries the evidence
it used.

## 4. Where people see it, and how they are told

- **Fieldkit Feeds, Daisychain view.** Needs attention first; each chain opens to its hops left to right, with the
  count at each and a link to that hop's own detail. Facets as in [`fabric-feeds.md`](fabric-feeds.md) §11.
- **CLI.** `goat chains ls`, `goat chains show <id>` (JSON with `--json`).
- **API.** `goat-feeds` serves `GET /api/v1/chains` and `GET /api/v1/chains/<id>` on the site host's mesh
  address, behind the external-service door.
- **Pages.** Daisychain exports one metric per chain (`goat_chain_verdict`) and ships alert rules for the
  site's Alertmanager. Critical verdicts page through ntfy on the relay tier
  ([ADR 1168](../adr/1168-self-hosted-paging-and-automatic-oncall-enrolment.md)), warnings go to the warning tier, and a return to `flowing` resolves. The page text names the
  chain, the hop and the two counts.

## 5. What each hop must publish

| Hop | Publishes | Status |
|---|---|---|
| Pipes (`goat-xf-pipe-zenoh`) | ADR 1260 health record on `ops/links/health/<site>/<link>` every 10 s | designed (ADR 1260); counters exist in the unit's status line |
| Boundary node, per feed | received, datagrams, state, input and output identities | built (`/api/v1/status`, `/api/v1/modifications`) |
| Boundary node, tunnel | handshake age or SA state, bytes in and out, written every 15 s by a root timer to `/run/simple-bn/tunnel.json`; the monitor serves it | designed |
| Receiver | `goat.bn-feedback/v1` `ack` per SIC | built on our side; needs each receiver to send |

## 6. Build order

| Slice | What | Ready when |
|---|---|---|
| 1 | Daisychain v0 in `goat-feeds` (goatstuff) on the site host: chains from the declarations; hop 2 read from the pipe units' status lines, hops 3, 4, 6 from the boundary monitor, verdicts and `/api/v1/chains`; Alertmanager rules | a deliberately stopped pipe pages within 3 minutes and resolves on restart |
| 2 | Tunnel-state timer on boundary hosts; the monitor serves it; Daisychain reads hop 5 | a downed tunnel shows `broken at tunnel` |
| 3 | `goat-xf-pipe-zenoh` health records (ADR 1260 §2); Daisychain reads them instead of unit status | slice 1's test passes on records alone |
| 4 | Fieldkit Feeds Daisychain view and `goat chains` | an operator answers "is the receiver getting the radar" from one page |
| 5 | Receivers send acknowledgements | `receipt-unconfirmed` turns into `flowing` for that receiver |

## 7. Rules

- Daisychain reads records, never payloads, and never subscribes to a link's data on a viewer's request
  (ADR 1123).
- Each site judges only its own chains; a goatnet view reads site roll-ups.
- A verdict is never stronger than its evidence: no acknowledgement means `receipt-unconfirmed`, never
  "received".
- Input and output of a hop are reported separately. A hop that changes data (a boundary node's reshape or
  identity rewrite) reports both, so neither side is read as the other.
