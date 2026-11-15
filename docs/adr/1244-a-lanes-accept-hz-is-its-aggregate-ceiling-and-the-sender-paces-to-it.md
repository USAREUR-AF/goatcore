# ADR 1244 — A lane's `accept_hz` is its aggregate ceiling, and the sender paces to it

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-11-15 |
| Deciders | operator |
| Repo scope | goat core (`docs/design/federation-package-spec.md`, `ops/federation/republisher`, `ops/ansible/roles/goat-zenoh-edge`, `crates/goat-federation/src/render.rs`, `ops/federation/standup/acceptance.sh`) |
| Supersedes | — |
| Superseded by | — |
| Related | [ADR 1157](1157-cross-sovereign-federation-edge-terminated.md) (the edge's ingress `downsampling` and `low_pass_filter`) · [ADR 1033](1033-zenoh-qos-prioritization-reserved-mission-slice.md) (no custom router interceptor) · [`federation-package-spec.md`](../design/federation-package-spec.md) §3, §7 and §8.1 · [`federation-acceptance.md`](../design/federation-acceptance.md) · [`ops/federation/republisher/README.md`](../../ops/federation/republisher/README.md) |

## TL;DR (for a layperson)

When two goatnets federate, each says how fast it will accept data from the other: `accept_hz`. The receiving side enforces it with a filter that lets through at most one message every 1/`accept_hz` seconds on the whole link, not per topic. Our own sending side, meanwhile, allowed up to `accept_hz` messages in each second, all at once if they came at once. So a partner could stay inside the agreed rate and still have messages thrown away, silently, at the other end. This decision says what the number means, the one rate for the whole link, and makes our sender space its messages to it. A message is then delayed rather than lost, and if the link is truly overloaded the sender drops it and counts it, so nothing disappears without a trace.

## Context

The package format gives each receiver three ceilings, `accept_hz`, `accept_bytes_per_message` and `accept_distinct_keys`: "I will accept traffic up to these limits" ([spec](../design/federation-package-spec.md) §3, line 108). The agreement copies them into each direction's `envelope` ("you cannot be obliged to accept more than you declared", §7, line 817; §8.1, line 882). Nothing says what `hz` is per (a key, a release set, the lane) or whether it is a count per second or a spacing.

The two ends read it differently.

- **The receiver.** `goat federate apply` renders one ingress `downsampling` rule per lane, `{ key_expr: "from/<them>/**", freq: <hz> }` (`crates/goat-federation/src/render.rs:661`). In zenoh 1.9.0 the downsampling interceptor keeps its state per **rule**: `ke_state: HashMap<usize, Timestate>`, keyed by rule id, with `compute_id` mapping any matching key to its rule (`zenoh/src/net/routing/interceptor/downsampling.rs`; read from the pinned router's source, which carries no patch to that file). So the lane admits one message per 1/`hz`, whatever its keys, and drops a second message inside that interval with nothing logged.
- **The sender.** The republisher sheds above `hz` messages in each one-second window (`ops/federation/republisher/src/policy.rs:213-224`), and lets a burst through inside the window.

A sender inside its envelope therefore loses data. Two puts a millisecond apart are two a second, far under 100, and the receiver drops the second. The federation test environment shows this. The rotation scenario's once-a-second sample flow and acceptance's own single puts share one lane. Its acceptance checks on that lane failed in two runs ("did not persist", "not delivered"), and a sample was lost inside an acceptance step. The acceptance runner already spaces its own samples for this reason ([`federation-acceptance.md`](../design/federation-acceptance.md), "Rates"), but that spacing cannot cover traffic it does not send.

## Decision

1. **Meaning.** `accept_hz` is the lane's **aggregate** ceiling: across every key under the lane's `publish_under` (`from/<sender>/**`), at most one message per 1/`accept_hz` seconds. It is a minimum spacing, not a count per second, and it is not per key or per release set. The package spec says so in §3 and §8.1. The agreement's fields, bytes and digest do not change.
2. **Receiver.** The receiving edge keeps enforcing it as today: one ingress `downsampling` rule per lane at `freq: accept_hz`. The edge role's documentation names the rule's per-rule state.
3. **Sender.** The republisher **paces** each lane. It queues and spaces its puts so that no two leave less than 1/`hz` apart. The queue is bounded. A message that cannot be sent within the bound is **shed and counted** (`refused_total{reason="rate_shed"}`, or a dedicated reason the implementation names), and logged at most once a minute, never dropped silently. The one-second window in `policy.rs` is replaced by this pacing.
4. **One writer per lane.** Everything that publishes onto a lane from our side, live data, acceptance samples and round-trip probes, writes `release/<set>/…` on our core and reaches the lane only through that lane's republisher. The core's ACL admits the republisher as the only writer of its wire keys. The implementation checks this against the ACL the role renders, so pacing in one place covers every message.
5. **Acceptance, until the pacing is built.** Acceptance's persistence and delivery checks send a fresh value until one is stored, within their existing bounded wait, as the round trip already does. When a single put is not stored, the report names the receiving edge's one-message-per-1/`hz` rule as a possible cause.

## Alternatives considered

| Alternative | Why not |
|---|---|
| One downsampling rule per released set, so each set has its own timer | It multiplies the receiver's ceiling by the number of sets, which the spec does not allow, unless `hz` is divided among them, which the spec does not describe. It still drops a second message inside one set, which is where the test environment's collisions were |
| Leave `hz` out of the edge's ingress filter, or set the filter well above `hz` | The ingress filter is the receiver's own protection: "the one place load arrives that we neither control nor can ask to slow down" (`ops/ansible/roles/goat-zenoh-edge/templates/config.json5.j2:102-103`). A higher filter makes a collision less likely, not impossible |
| Per-key state in zenoh's downsampling, upstream | It answers a different question: 100 Hz per key with 5,000 distinct keys is 500,000 messages a second, with no aggregate bound. A local patch is a custom interceptor, which [ADR 1033](1033-zenoh-qos-prioritization-reserved-mission-slice.md) rejects |
| Only make acceptance tolerant (resend until stored) | It is kept, as item 5, but alone it hides the loss from acceptance and leaves real traffic losing a burst's second message |
| Redefine `accept_hz` as a count per second, with bursts | The receiving edge cannot enforce it: zenoh's filter is a spacing, and there is no burst allowance to configure |

## Consequences

**Positive.**
- One meaning on both ends.
- A partner that stays inside the envelope loses nothing to the receiver's filter.
- Overload is shed where the sender can see and count it.
- The agreement format is unchanged, so no partner has to re-sign a package.

**Negative.**
- A burst now waits. Its last message is delayed by up to N/`hz` seconds, so a lane's latency under bursts depends on its agreed rate.
- The bound on the queue is a new number to choose and measure.
- A partner's own sender, if it is not ours, still has to pace itself. The spec states the meaning, but nothing can make a third-party sender follow it. Their excess is dropped at our edge, as today.

## Assessor view (SCA-R)

This narrows an ambiguity in a cross-boundary flow control and makes data loss visible (SC-7 boundary protection, SI-4 system monitoring, AU-2 auditable events). The receiver's ingress ceiling is unchanged, so the boundary's protection against a flooding partner is not weakened. Loss caused by the sender's own overload becomes a counted, logged event instead of a silent drop at the partner's edge. Residual risk: a non-conforming third-party sender still loses data at our edge without telling us, as it does today; this decision does not change that.

## Trigger conditions for revisit

- zenoh's downsampling gains per-key state or a burst allowance, or a newer router version changes its per-rule behaviour.
- A lane needs bursts above its agreed rate without delay: a measured case, not a supposed one.
- A partner's implementation reads `accept_hz` as per key and the two cannot agree from this text.
