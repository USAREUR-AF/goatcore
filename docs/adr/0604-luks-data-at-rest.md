# ADR 0604 — LUKS data-at-rest with NBDE and multi-mode unlock

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-25 |
| Repo scope | goat core |
| Related | [`0103-crypto-boundary-discipline.md`](0103-crypto-boundary-discipline.md), [`0605-op-tee-key-custody.md`](0605-op-tee-key-custody.md), [`0606-tamper-response-zeroization.md`](0606-tamper-response-zeroization.md), [`../design/os-baseline-and-luks.md`](../design/os-baseline-and-luks.md) |

## TL;DR (for a layperson)

Every disk in the system is encrypted with LUKS — Linux's
standard full-disk encryption. There are five different ways
the disk can be unlocked at boot, and we choose which apply
per host class: a passphrase typed by an operator, a TPM
2.0 chip, a FIDO2 hardware token, a USB-key, or a split-secret
threshold scheme. This flexibility lets us deploy in
attended environments (passphrase) and unattended ones
(TPM 2.0 + Network Bound Disk Encryption) with the same
underlying primitive.

## Diagram



Source: [`diagrams/0604-luks-data-at-rest.mmd`](diagrams/0604-luks-data-at-rest.mmd).

## Context

Disk encryption has competing constraints:

- **Attended boot** wants a passphrase / FIDO2.
- **Unattended boot** (after a power cycle, no operator on
  site) wants automatic unlock from a measured / attested
  source.
- **Network-bound unlock (NBDE / Tang / Clevis)** uses a
  network-reachable Tang server to release the unlock key
  if and only if the host is on the right network.
- **Multi-host SSS (Shamir's Secret Sharing)** spreads the
  unlock key across multiple devices; M-of-N reconstruct.

We need all five modes available, chosen per host class /
deployment context.

## Decision

LUKS is the primitive everywhere. Multi-mode unlock policy:

| Mode | When |
|---|---|
| **passphrase** | Operator-attended deployment (workstations, lab benches). |
| **tpm2** | Unattended mgmt-hosts in physically secured racks; binds to PCR measurements. |
| **fido2** | Operator-attended deployment with hardware-token requirement (FIDO2 + LUKS). |
| **usb** | Air-gapped initial bring-up; key on a removable USB device. |
| **sss** | Multi-host threshold deployments where no single device should hold the unlock key. |

Implementation:

- **Ansible role:** `ops/ansible/roles/luks-unlock/` parameterized
  on mode.
- **NBDE (Tang + Clevis):** layered on top where network-bound
  unlock is desired; mgmt-host can act as a Tang server for
  the rack's other LUKS volumes.
- **Boot chain integrity** ([`0809`](0809-boot-chain-integrity.md))
  is paired with LUKS — confidentiality (LUKS) + integrity
  (Secure Boot + dm-verity).
- **Tamper response** ([`0606`](0606-tamper-response-zeroization.md))
  zeroes LUKS header on physical intrusion.

Full design at
[`../design/os-baseline-and-luks.md`](../design/os-baseline-and-luks.md).

## Alternatives considered

- **Single-mode LUKS.** Doesn't fit the variety of host
  classes. Rejected.
- **Filesystem-level encryption (eCryptfs / fscrypt).**
  Doesn't cover the boot partition or swap; weaker than
  full-disk LUKS. Rejected.
- **No data-at-rest encryption.** Disqualifies the system
  from handling controlled data. Rejected.

## Consequences

**Positive.** One primitive, five operational modes. Fits
attended and unattended deployments. NBDE for unattended
recovery without operator presence at the rack.

**Negative.** Five modes is a lot of operational ground.
Mitigated by the parameterized Ansible role and per-host-
class default policy.

## Assessor view (SCA-R)

LUKS data-at-rest serves SC-28 (protection of information at
rest). FIPS 140-3 validated kernel cryptographic module
(via Ubuntu Pro on x86 and Ubuntu-for-Tegra Pro on Orin)
provides the cryptographic primitives; LUKS uses them.
NBDE / Tang+Clevis serves CP-2 (contingency — unattended
recovery within the right network).

NIST SP 800-111 (storage encryption) maps onto the
deployment patterns.

## Trigger conditions for revisit

- A new mode emerges (e.g. attestation-based key release
  via TEE). Add it to the role.
- Operational evidence shows a mode is unused. Trim the
  role; do not bloat.

---

*See `GLOSSARY.md` for terms and acronyms.*
