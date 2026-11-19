# ADR 0811 — Audit-log integrity — MinIO Object Lock + integrity ledger + daily check

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-25 |
| Repo scope | goat core |
| Related | [`0508-audit-logger-interceptor.md`](0508-audit-logger-interceptor.md), [`0800-open-source-edr.md`](0800-open-source-edr.md), [`../design/egress-proxy.md`](../design/egress-proxy.md) |
| Implementation note 2026-05-31 | The `Decision` §"Storage" line names a "1 year online + 6 years archive" retention split without naming the substrate for the *archive* tier, AND implicitly assumes COMPLIANCE-mode lock on the online tier. [`../design/audit-cold-storage-offload.md`](../design/audit-cold-storage-offload.md) refines both: the substrate is a three-tier shape (router rocksdb 7d / MinIO 30d **GOVERNANCE** online / cold-storage bucket COMPLIANCE-locked operator-triggered offload). The COMPLIANCE bar is preserved at the cold-storage tier (where evidence-grade preservation lives); the online tier moves to GOVERNANCE so the substrate can FIFO-evict oldest under quota pressure instead of failing new writes — the more common operational case. Layered defenses (least-privilege audit-subscriber S3 user, FIFO-eviction service audit-trail, write-rate anomaly alerts) cover the rare attacker case. The 1y/6y/AU-11 bar is preserved: production sites apply a scheduled (e.g., monthly) offload + a 7y COMPLIANCE retention on the cold-storage bucket. Trial fabrics use the same substrate shape with shorter retentions matched to their disk budget. The substrate-shape decision is the same fleet-wide; the retention *values* differ per-deployment. Production deployments without a cold-archive offload cadence can override the middle tier to COMPLIANCE to keep the original ADR 0811 posture (one tier doing both jobs). |

## Fabric Record Scope (2026-10-26)

The local V2 implementation preserves the exclusion of Zenoh payloads: new audit
objects and local audit tails contain metadata only. Replay capture and ordinary
body reads are gated on every tier pending a separate owner-approved governance
design. Historical combined objects remain under their original retention and
are exposed to normal readers only through authenticated metadata projection.
No live rollout or historical inventory was performed for this change.

SourceInfo supplies a publisher assertion. A namespace may be attributed to a
slot by conservative analysis of a rendered ACL; its digest does not prove
installed enforcement. Neither establishes authenticated sender identity or
non-repudiation. Put/delete, receiving site and observed outcome are explicit.
See the [subscriber contract](../design/zenoh-audit-external-subscriber.md#metadata-and-replay-separation-2026-10-26)
and [cutover procedure](../operations/audit-replay-cutover.md). Historical retention
values below are design decisions, not a claim that a control mandates blanket
capture or those exact durations.

## TL;DR (for a layperson)

Audit logs are only useful if they can't be quietly modified.
We store audit records in MinIO with Object Lock enabled,
which makes objects immutable for a defined retention period.
We additionally maintain an integrity ledger — a hash chain
across records — so that any tampering after the fact would
break the chain. A daily CI job verifies the chain.

## Diagram



Source: [`diagrams/0811-audit-log-integrity.mmd`](diagrams/0811-audit-log-integrity.mmd).

## Context

Audit logs that can be modified are evidence-grade only as
far as the modifying authority is trusted. For
contested-environment use cases, audit-record immutability is
critical.

Three layers of integrity:

1. Storage immutability (Object Lock).
2. Hash-chain integrity (ledger).
3. Daily verification (CI).

Each catches different tamper modes.

## Decision

Audit-log integrity tier:

- **Storage**: per-site MinIO with Object Lock in compliance
  mode. Retention: 1 year online + 6 years archive.
- **Cross-mounted peer-mgmt-host storage**: Router-A's MinIO
  cross-mounts to Router-B and vice versa, so a single-host
  loss doesn't lose audit.
- **Integrity ledger**: hash-chain of audit records;
  per-record hash includes prior-record hash; chain head is
  itself periodically signed.
- **Daily tamper-detection CI**: walks the ledger, verifies
  every record, alerts on chain break.
- **Sources** (matching `egress-proxy.md` §5.6 in-scope event
  classes):
  - Egress decisions — nftables, Squid, dante (per [`0806`](0806-egress-proxy-cascade.md)).
  - Auth + sudo on mgmt-host — auditd, journald (per [`0800`](0800-open-source-edr.md), [`0808`](0808-pam.md)).
  - Config changes to egress / ACL policy — git commit + Ansible
    deploy + policy-engine events (per [`0805`](0805-policy-engine-active-passive.md)).
  - Break-glass exceptions — allowlist `break_glass:` blocks.
  - Privilege escalation attempts — auditd.
  - Peer enrollment / de-enrollment — NetBird management API +
    admit-audit sidecar metrics ([`0816`](0816-admit-audit-sidecar.md)
    — note: admit-audit is a *signal generator*, not a tamper-proof
    record source; its alerts feed Alertmanager, while the canonical
    enrollment audit comes from the management API itself).
  - Audit-logger interceptor on the data fabric ([`0508`](0508-audit-logger-interceptor.md)).

  **Out of scope (v1)** per the design doc: application-level logs
  (hosted applications, Zenoh sample payloads), Traefik
  access logs, routine Prometheus scrapes.

Section §5.6 of
[`../design/egress-proxy.md`](../design/egress-proxy.md) is
the design source.

## Alternatives considered

- **Filesystem-only audit logs.** No tamper resistance.
  Rejected.
- **Cloud-only audit storage.** Loses sovereignty / Island
  Mode. Rejected.
- **Hash-chain only, no Object Lock.** Object Lock prevents
  silent overwrites; complementary to hash-chain.

## Consequences

**Positive.** Three-layer integrity. Tamper-resistant audit.
Cross-mounted across A/B routers. Daily verification gives
freshness guarantee.

**Negative.** Storage cost (1 year online + 6 years archive).
Acceptable for the integrity property.

## Assessor view (SCA-R)

Audit-log integrity serves AU-9 (protection of audit
information), AU-10 (non-repudiation — paired with event
signatures from [`0511`](0511-publisher-trust-model.md)),
AU-11 (audit record retention — 1y online + 6y archive),
and SI-7 (integrity).

The comparative-posture artifact identifies "audit retention
integrity" as a dimension where goat is ahead.

## Trigger conditions for revisit

- Storage volume becomes unmanageable. Shorten online
  retention (push to archive faster); do not lose the
  retention bar.
- A specific authorization mandates a different audit
  storage scheme. Add a federation adapter; preserve the
  per-site sovereign tier.

---

*See `GLOSSARY.md` for terms and acronyms.*
