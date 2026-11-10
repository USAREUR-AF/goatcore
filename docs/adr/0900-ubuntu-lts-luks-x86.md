# ADR 0900 — Ubuntu 24.04 LTS + LUKS for x86 management hosts

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-25 |
| Repo scope | goat core |
| Related | [`0604-luks-data-at-rest.md`](0604-luks-data-at-rest.md), [`0809-boot-chain-integrity.md`](0809-boot-chain-integrity.md), `0902-goat-drop.md`, [`../design/os-baseline-and-luks.md`](../design/os-baseline-and-luks.md) |

## TL;DR (for a layperson)

x86 management hosts run Ubuntu 24.04 LTS with LUKS full-disk
encryption. We considered an immutable-OS alternative (Kairos)
and dropped it because it could not deliver our FIPS 140-3 +
DISA STIG compliance requirements. Ubuntu Pro provides those
through a well-supported channel. Parity for 22.04 is under
review; see [`../design/orin-os-decision.md`](../design/orin-os-decision.md).

## Context

The x86 management-host OS choice has to satisfy:

- FIPS 140-3 (kernel cryptographic module).
- DISA STIG hardening.
- LUKS data-at-rest.
- Long-term support for the production deployment window.
- Mature ecosystem (Ansible, Docker, Prometheus stack).

Ubuntu 24.04 LTS + Ubuntu Pro meets all five. Earlier
evaluation included Kairos (immutable-OS); see
`0902`.

## Decision

x86 mgmt-hosts run:

- **OS**: Ubuntu 24.04 LTS (mutable root).
- **Storage encryption**: LUKS
  ([`0604`](0604-luks-data-at-rest.md)).
- **Boot chain**: UEFI Secure Boot + signed shim
  ([`0809`](0809-boot-chain-integrity.md)).
- **Compliance channel**: Ubuntu Pro (FIPS 140-3 kernel +
  USG STIG).
- **Hardening**: USG STIG via Ansible role
  `ops/ansible/roles/usg-stig/`.
- **22.04 parity**: under review; tracked in
  [`../design/orin-os-decision.md`](../design/orin-os-decision.md).

The fleet-wide OS baseline (three-tier storage, NBDE, CSfC
DAR framing, FIDO2 break-glass) is at
[`../design/os-baseline-and-luks.md`](../design/os-baseline-and-luks.md).

## Alternatives considered

- **Kairos** (immutable OS). Could not deliver FIPS 140-3 +
  DISA STIG. Dropped — see
  `0902`.
- **RHEL / Rocky Linux**. Mature in DoD; we don't have an
  enterprise subscription path. Held; not chosen for
  primary.
- **Debian stable**. Lacks FIPS 140-3 channel.
  Rejected.

## Consequences

**Positive.** Mature, well-supported, with a clear FIPS +
STIG path. Large operator community.

**Negative.** Mutable root means dm-verity coverage is
read-only-partition only ([`0809`](0809-boot-chain-integrity.md)).
Acceptable; the OS base is well-hardened.

## Assessor view (SCA-R)

Ubuntu 24.04 LTS + Ubuntu Pro serves CM-2 (baseline
configuration), CM-6 (configuration settings — STIG),
SC-13 (cryptographic protection — FIPS kernel), SC-28
(data at rest — LUKS), and SI-7 (integrity — boot chain).
Ubuntu Pro's FIPS 140-3 validated kernel module is the
load-bearing artifact.

## Trigger conditions for revisit

- Ubuntu Pro pricing / governance changes adversely.
- A specific authorization mandates a different OS. Negotiate.

---

*See `GLOSSARY.md` for terms and acronyms.*
