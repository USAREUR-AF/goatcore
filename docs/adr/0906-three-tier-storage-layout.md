# ADR 0906 — Three-tier storage layout per site

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-25 |
| Repo scope | goat core |
| Related | [`0905-nvme-per-site.md`](0905-nvme-per-site.md), [`0604-luks-data-at-rest.md`](0604-luks-data-at-rest.md), [`0809-boot-chain-integrity.md`](0809-boot-chain-integrity.md), [`../design/os-baseline-and-luks.md`](../design/os-baseline-and-luks.md) |

## TL;DR (for a layperson)

Each mgmt-host's storage is laid out in three tiers: a small
read-only system partition (verified by dm-verity), a
LUKS-encrypted mutable system partition (for /var, logs,
state), and a separate LUKS-encrypted data partition on NVMe
(/opt/docker-data, for database / backup / audit). This
separation makes each tier's controls distinct and reviewable.

## Diagram



Source: [`diagrams/0906-three-tier-storage-layout.mmd`](diagrams/0906-three-tier-storage-layout.mmd).

## Context

A single LUKS-encrypted disk holding everything is
operationally simple but conflates three different concerns:

- **System read-only**: integrity-verified by dm-verity.
- **System mutable**: `/var`, system logs, package state.
  Encrypted, mutable.
- **Data**: database, backups, audit logs. Encrypted,
  high-IO, separate storage tier.

Separating them lets each be encrypted, monitored, and
backed up appropriately.

## Decision

Three-tier storage layout per mgmt-host:

| Tier | Purpose | Protection |
|---|---|---|
| **1. Read-only system** | OS root partition | Signed kernel + dm-verity ([`0809`](0809-boot-chain-integrity.md)). |
| **2. Mutable system** | /var, /home, /tmp, /etc | LUKS ([`0604`](0604-luks-data-at-rest.md)). |
| **3. Data** | /opt/docker-data — DB, backups, audit, persistent volumes | LUKS on NVMe ([`0905`](0905-nvme-per-site.md)). |

Properties:

- **Per-tier monitoring**: capacity / health alerts per tier.
- **Backup discipline**: only the data tier is backed up
  (system tiers are reproducible from images / IaC).
- **Tier 3 cross-mount**: A/B routers cross-mount each
  other's `/opt/docker-data/audit` for the audit-integrity
  cross-mount pattern ([`0811`](0811-audit-log-integrity.md)).

Full layout in
[`../design/os-baseline-and-luks.md`](../design/os-baseline-and-luks.md).

## Alternatives considered

- **Single partition.** Conflates concerns. Rejected.
- **Two tiers (system + data).** Loses dm-verity coverage
  for read-only. Rejected.
- **Four+ tiers.** Diminishing returns. Three is the
  sweet spot.

## Consequences

**Positive.** Each tier has appropriate controls.
Reviewable per tier. Backup discipline naturally bounded.

**Negative.** More partitions to manage at install time.
Acceptable; Ansible role handles it.

## Assessor view (SCA-R)

Three-tier storage serves SC-28 (data at rest — appropriate
per tier), SI-7 (integrity — dm-verity at tier 1, LUKS at
tier 2/3), and CP-9 (backup — backup discipline scoped to
data tier).

## Trigger conditions for revisit

- A new tier emerges (e.g. read-mostly cache). Add a
  tier; do not collapse.

---

*See `GLOSSARY.md` for terms and acronyms.*
