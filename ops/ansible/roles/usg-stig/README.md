# usg-stig

Apply DISA STIG hardening via Canonical's Ubuntu Security Guide (USG). Builds on the `ubuntu-pro-fips` role (which enables the `usg` Pro service). Part of the fleet OS baseline — see [`docs/design/os-baseline-and-luks.md`](../../../../docs/design/os-baseline-and-luks.md) §5.

## What it does

1. Install `usg` if not present (requires `pro enable usg` — done by `ubuntu-pro-fips`).
2. Select the appropriate DISA STIG profile for the Ubuntu version detected.
3. Run `usg audit` to baseline current state, capture report.
4. Run `usg fix` to remediate.
5. Re-audit; capture post-remediation report.
6. Record exceptions file for controls that cannot be auto-remediated (expected on L4T; expected on Docker-heavy hosts).

## Platform notes

- **Ubuntu 22.04:** `disa_stig_ubuntu2204` profile is current (V1R2).
- **Ubuntu 24.04:** DISA-published STIG not yet final; uses Canonical's USG 24.04 SRG-mapped profile preview. Document as "SRG-compliant pending DISA STIG publication" in evidence.
- **L4T arm64:** many STIG controls assume stock kernel paths, audit config, and PAM layout. Expect 10-20 controls to fail or need manual review. Log each exception in the per-host `stig-exceptions.md`.
- **dm-verity rootfs** (Phase 3 target): audit config needs writable overlay; role currently assumes writable root. Phase 3 extends with overlay handling.

## Variables

| Variable | Default | Description |
|---|---|---|
| `usg_profile` | auto-selected by Ubuntu version | Override if you need a specific profile. |
| `usg_fix_enabled` | `true` | Set `false` to audit only, no remediation. |
| `usg_exception_path` | `/var/log/stig-exceptions.yml` | Site-specific exceptions (documented justifications). |
| `usg_audit_report_path` | `/var/log/stig-audit-{{ ansible_date_time.date }}.html` | HTML report output. |

## Evidence

Each run writes:

- `/var/log/stig-audit-YYYY-MM-DD-pre.html` — state before fix
- `/var/log/stig-audit-YYYY-MM-DD-post.html` — state after fix
- `/var/log/stig-exceptions.yml` — operator-maintained exceptions list (persists across runs)


## TODO

- Add explicit Ubuntu 24.04 STIG profile switch once DISA publishes it.
- Add L4T exception bundle — pre-populated exceptions for known L4T gaps (audit paths, kernel params).
- dm-verity / overlay-aware remediation for Phase 3.
- CI job that runs `usg audit` non-destructively and fails on drift.
