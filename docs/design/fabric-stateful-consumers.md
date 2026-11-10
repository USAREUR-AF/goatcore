# Fabric Stateful Consumers — Correlation, Sessions, and Incomplete-State Handling

## Conclusion

Any consumer that correlates multiple events into a session (fire → terminate, request → response, track-appear → track-disappear, enrollment-start → enrollment-complete, RPC-call → RPC-reply) is a stateful consumer. Every stateful consumer in this system uses the pattern in this document so it is production-safe by default: bounded memory, orphaned-session dead-lettering, crash-recoverable state via history replay, and operator-visible diagnostics for when correlation goes wrong.

This is a platform pattern, not a one-off. First caller is a demo game's telemetry (fire → terminate correlation for hit statistics). Future callers: a track-lifecycle consumer (Phase 3B), enrollment-bundle-validator session tracker (`ops/enrollment/`), DDS bridge request/response pairing, any future async RPC-over-fabric pattern.

The skeleton lives at `ops/zenoh-consumer/skeleton/` (to be created in Block 11 Step 9B).

## The problem in one sentence

You receive event A. You expect event B to arrive eventually. B may never arrive. Memory must not leak; data must not silently vanish; the operator must be able to see what's orphaned.

## Design constraints

1. **Bounded memory under any traffic pattern.** An unbounded number of unmatched A-events cannot pin unbounded RAM. There is always an eviction policy.
2. **No silent data loss.** Orphans are observable, queryable, and re-processable.
3. **Crash-recoverable.** Restart of the consumer does not lose in-flight sessions; state is reconstructable from fabric history.
4. **Schema-change safe.** A payload version bump to A or B does not cause the consumer to drop unrelated events. (See fabric-schema-evolution.md.)
5. **Operator-visible.** The consumer exposes orphaned-session lists, completion latency distributions, and reprocess hooks — it is not a black box.

## The pattern

### Declaration

Every stateful consumer declares its correlation shape at startup:

```go
consumer := stateful.New(stateful.Config{
    Name:           "air-defenders-correlator",
    CorrelationKey: "payload.track_id",           // JSONPath into the event payload
    OpenOn:         []string{"game/fire"},  // keyexpr suffix(es) that open a session
    CloseOn:        []string{"game/terminate", "impact"}, // close normally
    SessionTimeout: 30 * time.Second,             // grace period for B after A
    MaxOpenSessions: 10_000,                      // hard memory cap; evict oldest when hit
    PersistencePath: "/data/state.bolt",          // optional — see "Persistence tiers" below
    DLQTopic:       "air-defenders/{site}/_dlq/orphaned-rounds", // where orphans go
})

consumer.OnComplete(func(s Session) {
    // A and B both arrived; process the correlated pair
    emitResult(s.A, s.B)
})

consumer.OnOrphan(func(s Session, reason OrphanReason) {
    // Never silently lose. Publish to DLQ, increment metric, log structured.
})
```

### State lifecycle

```
OPEN  — event A arrived, waiting for B. Holds state in memory (or bolt).
COMPLETE — event B arrived before timeout. Handler fires. State freed.
ORPHANED — timeout expired without B. Published to DLQ. State freed.
EVICTED — memory cap hit before timeout. Published to DLQ with eviction reason. State freed.
LATE — B arrived after session was already closed (orphan or complete). Counted, logged, published to a late-arrival topic for post-hoc analysis.
```

### Hard memory cap

`MaxOpenSessions` is a load-bearing parameter. At cap:

- Oldest open session is evicted (published to DLQ with `reason: evicted_at_cap`)
- `consumer_evicted_sessions_total` counter increments
- Alert fires at sustained evictions → either traffic shape changed or timeout is wrong

Without a cap, a traffic spike or a schema bug could drive OOM. The cap plus the DLQ means worst-case we lose observability on old orphans — not memory.

### Persistence tiers

Three tiers, picked per caller based on the cost of losing in-flight state on restart:

| Tier | Storage | Recovery on restart | When to use |
|---|---|---|---|
| **Memory-only** | RAM | None — all open sessions lost | Game telemetry, low-value analytics. The demo game uses this. |
| **Bolt-backed** | Local BoltDB file | Replays from disk. No fabric query. | Medium-value sessions with <10k concurrent, local-only correlation. |
| **History-replay** | Fabric RocksDB | On restart, queries `session.Get(keyexpr, since=now-timeout-window)` and rebuilds state from stored events. | High-value sessions, cross-site correlation, audit-bearing data. |

History-replay is the most elegant option because it leverages Zenoh storage rather than duplicating state — but it costs a startup-delay proportional to the replay window. A track-lifecycle consumer with a 1-hour session timeout would replay 1 hour of track events on startup (~seconds). Acceptable.

### Reactive tooling — when things go wrong

1. **Orphaned-session listing endpoint.** Every consumer exposes `GET /api/sessions/orphaned?since=T&limit=N` — returns the last N orphans with the A-event + orphan reason. Operator looks at it directly.
2. **Dead-letter topic.** All orphans + evictions publish to `{domain}/_dlq/{consumer-name}` on the fabric with the full session state. Never silently lost. DLQ topics get 30-day retention by default.
3. **Reprocess hook.** `POST /api/reprocess?since=T&until=T` — queries fabric history for both A and B events in the window, re-runs the correlator, emits correlated outputs. Used for backfills after bug fixes or schema migrations.
4. **Completion-latency histogram.** `consumer_completion_latency_seconds{name}` — distribution of A→B delays. Sudden shift tells you something upstream changed before you see orphans.
5. **Late-arrival topic.** `{domain}/_late/{consumer-name}` — B events that arrived after their session closed. Low-rate, high-signal: a non-trivial late-arrival rate indicates the session timeout is too tight.
6. **Session-debug endpoint.** `GET /api/sessions/{correlation_key}` — returns current state of one session by ID. Lets an operator inspect a specific active correlation during an incident.

### Metrics (every consumer emits these)

```
consumer_open_sessions_gauge{name,domain}
consumer_completed_sessions_total{name,domain}
consumer_orphaned_sessions_total{name,domain,reason}   # reason: timeout|evicted
consumer_late_arrivals_total{name,domain}
consumer_completion_latency_seconds{name,domain}        # histogram
consumer_state_memory_bytes{name,domain}
consumer_dlq_publish_total{name,domain}
consumer_reprocess_runs_total{name,domain}
consumer_reprocess_correlations_emitted{name,domain}
```

Prom alert rules ship as part of the skeleton:

- `ConsumerOrphanRateHigh` — orphan rate > 5% of open rate for 10m
- `ConsumerEvictionSustained` — any eviction for 5m (means cap is wrong)
- `ConsumerLateArrivalsSustained` — late arrivals > 1% for 10m (means timeout is wrong)
- `ConsumerDLQStalled` — DLQ publish rate is zero while orphans counter climbs (means DLQ broken)

## Real-world use cases beyond the demo game

| Use case | A-event | B-event | Correlation key | Session timeout | Persistence tier |
|---|---|---|---|---|---|
| Demo game result | `game/fire` | `game/terminate` | `track_id` | 30s | Memory |
| Track lifecycle | `track/appear` | `track/disappear` | `uid` | 1 hour | History-replay |
| Enrollment bundle session | `enrollment/start` | `enrollment/complete` | `device_cn` | 10 min | Bolt |
| DDS bridge RPC | `rpc/request` | `rpc/reply` | `request_id` | 30s | Memory |
| Node health breach | `health/threshold_hit` | `health/threshold_cleared` | `node_id:metric` | 15 min | Bolt |
| Peer rekey | `rekey/initiate` | `rekey/complete` | `peer_id` | 60s | Bolt |

Every one of these is an instance of the same pattern. One skeleton; per-consumer config.

## Non-goals

- **Cross-consumer transactions.** The skeleton does not provide atomicity across multiple consumers. If you need that, you need something that isn't a stateful consumer.
- **Exactly-once delivery.** The skeleton provides at-least-once with deduplication by `(correlation_key, schema_version)`. True exactly-once requires a transactional outbox at the publisher, which is out of scope.
- **Ordering guarantees beyond per-key.** Events for different correlation keys may arrive in any order. Within a key, Zenoh preserves order per-publisher; the consumer does not reorder across publishers.

## Failure modes and their mitigations

| Failure | Proactive mitigation | Reactive tool |
|---|---|---|
| Consumer crashes mid-session | Persistence tier (bolt or history-replay) | Restart rebuilds state; late arrivals handled |
| Memory explosion from unmatched A-events | `MaxOpenSessions` cap + eviction | DLQ captures evicted state; alert fires |
| Schema change breaks payload access | Schema version check at entry | Consumer refuses unknown version; stale-schema-events alert |
| A-events arrive without matching B forever (upstream bug) | Session timeout + DLQ | Orphan rate alert; operator inspects DLQ |
| Clock skew causes false "late" | NTP attestation (see `event-telemetry-schema.md` §NTP) | Late-arrival topic; admit-audit validates timestamps |
| Consumer falls behind → backpressure | Bounded input queue + drop-oldest policy | `consumer_input_dropped_total`; alert fires |
| DLQ topic itself is unsubscribed → silent accumulation | Required DLQ subscriber (zenoh-prom-exporter scrapes count) | DLQ stall alert |

## File layout (to be created in Step 9B)

```
ops/zenoh-consumer/
├── skeleton/
│   ├── stateful/          — Go module: State, Session, Config, lifecycle
│   ├── persistence/       — memory / bolt / history-replay implementations
│   ├── metrics/           — Prom metrics scaffolding
│   └── http/              — /api/sessions endpoints (orphaned, reprocess, debug)
├── dashboards/
│   └── stateful-consumer.json.tmpl  — Grafana template auto-populated per consumer name
└── alerts/
    └── stateful-consumer.yml        — Prom rules
```

Every stateful consumer imports from `ops/zenoh-consumer/skeleton/`, declares its config, registers its `OnComplete` and `OnOrphan` callbacks. Everything else is free.
