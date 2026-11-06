# Time-quality attestation propagated through the fabric

## Conclusion

Every emission on the Zenoh fabric carries an inline `time_quality`
envelope alongside the existing `sources` array. Wall-clock timestamps
alone are not enough: the fabric must distinguish between
GNSS-locked-with-σ=200ns, NTS-verified-with-σ=2ms, chrony-steered-with-σ=10ms,
and self-clocking-with-σ=800µs-and-growing. The schema-enforcer
interceptor rejects emissions missing the field; the data dictionary
mandates it; the lineage layer propagates worst-case quality through
derivations.

This is a *propagation* contract, not a new clock-sync mechanism. It
sits on top of the per-peer NTP work already in flight
([`universal-ntp-hygiene.md`](universal-ntp-hygiene.md)) and the
data-dictionary discipline already required
([`data-dictionary.md`](data-dictionary.md)). It exists because GNSS
denial and NTS spoofing are routine threats in the operational
environments goat targets, and downstream fusion code today silently
treats a self-clocking edge identically to a GNSS-locked one.

Decision: ADR [`0514`](../adr/0514-time-quality-attestation.md).
Implementation: `../project/implementation-plan.md` Block 36.

## Why a fabric-layer field

The two existing time disciplines — peer-level chrony / NTS in
[`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) and the
admit-time NTP attestation in
[`zenoh-ntp-integration.md`](zenoh-ntp-integration.md) — answer two
real questions but leave a gap:

| Question | Today | Gap |
|---|---|---|
| "Is this peer's clock disciplined?" | NTP-health-agent + admit-audit | Answer is per-peer-at-enrollment, not per-emission |
| "What's the per-message authoritative timestamp?" | OTLP timestamp + monotonic ns | No quality dimension; consumers cannot weight |
| "Did we lose GNSS during the last hour?" | Prometheus per-peer | Cross-plane joins require manual stitching |

Tactical fusion math, anomaly detection (Tipper —
[`tipper.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/tipper.md)), and partner releases
([`mission-partner-sharing.md`](mission-partner-sharing.md)) all want
the answer per-message, in-band, machine-checkable. The fabric layer
is where every emission already passes through a schema-enforcer
interceptor — the cheapest place to enforce the contract.

## The envelope

Producers add a `time_quality` object to every emission. Required
fields:

| Field | Type | Description |
|---|---|---|
| `source` | enum | `gnss_locked` / `nts_verified` / `chrony_steered` / `peer_attested` / `self_clocking` |
| `sigma_us` | uint64 | Best-case 1σ uncertainty in microseconds at this emission's timestamp |
| `last_external_lock_at` | int64 (UTC ns) | Wall-clock instant of most recent external lock event from a non-self source |
| `seconds_since_external_lock` | uint64 | Convenience: monotonic seconds since `last_external_lock_at` |
| `drift_model_confidence` | float (0.0–1.0) | When `source == self_clocking`, the per-node adaptive drift model's self-reported confidence |

Optional fields (recommended where the producer has them):

| Field | Type | Description |
|---|---|---|
| `gnss_constellation` | string | `gps`, `galileo`, `beidou`, `multi`, etc. — populated only when `source == gnss_locked` |
| `nts_servers` | string[] | NTS servers contributing to the lock (populated when `source == nts_verified`) |
| `peer_attesters` | string[] | Mesh-peer source IDs whose NTS-verified time corroborated this peer's clock (populated when `source == peer_attested`) |
| `holdover_class` | string | When self-clocking, the holdover hardware class — `csac`, `tcxo`, `ocxo`, `none` |

### Source ladder

The `source` enum is ordered worst-to-best and forms an explicit ladder:

1. **`self_clocking`** — no external lock; per-node adaptive drift model is the only signal. `sigma_us` grows monotonically since `last_external_lock_at`.
2. **`peer_attested`** — this peer is self-clocking but has been corroborated by ≥2 mesh peers whose own time is `nts_verified` or better. `sigma_us` is the pessimistic envelope of the corroborators plus path delay.
3. **`chrony_steered`** — chrony has a synchronized state but the upstream is opaque or non-NTS. Acceptable for non-tactical timekeeping.
4. **`nts_verified`** — chrony with at least one NTS-authenticated upstream contributing to the current sync.
5. **`gnss_locked`** — multi-GNSS receiver with current 3D fix and PPS.

Consumers compare `source` ordinally for filter predicates ("only accept fusion inputs with `source >= nts_verified`"). The schema-enforcer treats this enum as a closed set; new ladder rungs require a non-additive schema bump per `fabric-schema-evolution.md`.

### Where the values come from

The per-node `ntp-health-agent`
([`time-hygiene/`](../../ops/time-hygiene/)) already tracks the inputs
and exposes them on `0.0.0.0:9753` for Prometheus. It gains a new
local UDS endpoint that returns the current envelope as a single
syscall — producer libraries call it once at emission time, no
network round trip. Caching is bounded at 100 ms staleness; producers
that emit faster than 10 Hz reuse the cached envelope (acceptable —
`sigma_us` does not move meaningfully in 100 ms).

## Lineage propagation

Every lineage edge in [`lineage.md`](lineage.md) gains a
`worst_case_time_quality` attribute. Rule: a derived emission's
`time_quality` is the *worst* (lowest in the ladder, largest
`sigma_us`) of its inputs and the deriving peer's own envelope at
emission time. The lineage peer
(`cmd/lineage/forward/<topic>` / `cmd/lineage/backward/<topic>`)
exposes the propagation graph so an operator can ask "which inputs
poisoned this fusion track's time quality?" without writing a query
against raw audit storage.

This makes time-quality degradation *visible* — a single
self-clocking sensor producing tracks that fuse into mission-relevant
output is identifiable by a query, not by reading commit logs.

## Schema-enforcer interceptor change

The existing schema-enforcer at
`zenoh-plugins/` gains one rule:

```rust
// zenoh-plugins/schema-enforcer/src/time_quality.rs (sketch)
pub fn validate_time_quality(env: &TimeQualityEnvelope) -> Result<(), ValidationError> {
    if env.source == Source::SelfClocking && env.last_external_lock_at == 0 {
        return Err(ValidationError::NeverHadExternalLock);
    }
    if env.sigma_us == 0 {
        return Err(ValidationError::ZeroSigmaImplausible);
    }
    if env.seconds_since_external_lock > MAX_HOLDOVER_SECONDS
        && env.holdover_class == "none" {
        return Err(ValidationError::HoldoverWithoutHardware);
    }
    Ok(())
}
```

Rejection puts the emission on the DLQ
([`zenoh-dlq.md`](zenoh-dlq.md)) with `reason = time_quality_invalid`
and emits an audit-logger event. The producer gets a structured
NACK — the same mechanism the schema-enforcer already uses for
undeclared topics.

## Data-dictionary requirement

Every entry in `attest/schema/<topic>` and
`attest/schema-declaration/<topic>` adds a required field block:

```yaml
time_quality:
  required: true
  minimum_source: nts_verified  # per-topic, optional override
```

`minimum_source` is per-topic. A pose / track topic might require
`nts_verified` or better (a fusion math invariant). A heartbeat topic
might accept `peer_attested`. The schema-enforcer reads this from the
declaration and rejects emissions below the floor.

## Composition with other systems

| System | How it composes |
|---|---|
| [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) | Source of truth for the per-peer envelope. New UDS endpoint added; existing health-agent code stays. |
| [`data-dictionary.md`](data-dictionary.md) | Mandatory field; per-topic `minimum_source`. |
| [`lineage.md`](lineage.md) | Worst-case propagation along edges; new query verb on the lineage peer. |
| [`tipper.md`](https://github.com/usareur-af/goatstuff/blob/main/docs/design/tipper.md) | Tipper anomaly models weight inputs by `(source, sigma_us)`; suppresses spurious detections from self-clocking inputs. |
| [`scout.md`](https://github.com/usareur-af/goatdocs/blob/main/docs/design/scout.md) | Scout cites `time_quality` per source in answer assembly; consumers can filter low-quality citations. |
| [`mission-partner-sharing.md`](mission-partner-sharing.md) | Partner release configs can require `minimum_source`; the bridge refuses below-floor releases. |
| ADR [`0822`](../adr/0822-wide-event-observability-otlp-openobserve.md) | OTLP attribute `dbb.time_quality.*` set on every wide-event emission; SQL filter on quality dimension is direct. |
| ADR [`0511`](../adr/0511-publisher-trust-model.md) | Publisher trust model gets a time-quality dimension — a publisher that consistently emits `self_clocking` past the holdover budget is downgraded. |

## What this is not

- **Not a replacement for NTP / NTS / GNSS.** It rides on top of them.
- **Not a CMVP claim.** No new validated cryptography. The signature on the envelope (when present) reuses the same ed25519 chain as the rest of fabric publisher trust.
- **Not a distributed-clock algorithm.** No new clock-sync protocol; no Lamport / Hybrid Logical Clock layer. Each peer's envelope is independent; lineage propagation is the only cross-peer mechanism.
- **Not opt-in.** The schema-enforcer rejects emissions without it. Phoenix-native peer discipline ([`phoenix-architecture.md`](phoenix-architecture.md)) means a peer that cannot produce the envelope cannot publish — by construction.

## Sequencing

| Phase | What |
|---|---|
| **36A** | Envelope schema + `ntp-health-agent` UDS endpoint + producer SDK helpers (Go + Rust). Land first; reversible. |
| **36B** | Schema-enforcer interceptor change + DLQ wiring. Test against existing fabric traffic with `minimum_source` set to lowest tier. |
| **36C** | Data-dictionary spec update + per-topic `minimum_source` in production schemas. |
| **36D** | Lineage `worst_case_time_quality` edge attribute + lineage-peer query verbs. |
| **36E** | Backfill: existing producers (gatewatch, audit-logger, NTP health, future airdefenders-on-Zenoh) emit the envelope. |
| **36F** | Consumers wired: Tipper weighting, Scout citations, partner-release floor enforcement. |

Each phase is independently complete — 36A ships value (operators can query envelope) before 36B ships enforcement before 36C ships discipline. Reversal at any point leaves the fabric coherent.

## Open questions

- **Holdover class detection on commodity hardware.** Most current peers are commodity x86 / Orin without a CSAC. The `holdover_class` field defaults to `none`; the question is whether to procure CSACs at sites where time quality is mission-critical. Tracked as a procurement decision in `procurement-ntp-gps.md`.
- **Cross-site peer-attestation latency.** The `peer_attested` rung relies on mesh-peer corroboration. Across sites the path delay is measurable (10s of ms via Spock-replicated mgmt traffic). Whether `peer_attested` is meaningful cross-site is an open empirical question — the first Phase 3 multi-site soak is the venue.
- **`time_quality` on bundles.** Offline-enrollment bundles ([`offline-enrollment.md`](offline-enrollment.md)) are signed at bundle-creation time; the envelope at consumption time is what matters for the first emission. The bundle pipeline does not need to carry time quality, but the first emission does.

## Related artefacts

- ADR [`0514`](../adr/0514-time-quality-attestation.md) — the decision
- ADR [`0804`](../adr/0804-three-tier-ntp.md) — three-tier NTP strategy
- ADR [`0511`](../adr/0511-publisher-trust-model.md) — publisher trust composition
- [`universal-ntp-hygiene.md`](universal-ntp-hygiene.md) — peer-side mechanics
- [`zenoh-ntp-integration.md`](zenoh-ntp-integration.md) — admit-time attestation (complementary)
- [`data-dictionary.md`](data-dictionary.md) — mandatory schema declaration
- [`lineage.md`](lineage.md) — propagation graph
- `fabric-schema-evolution.md` — versioning rules

---

*See `../adr/GLOSSARY.md` for terms and acronyms.*
