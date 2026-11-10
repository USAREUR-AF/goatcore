# snitch-bundle ansible role

Bundle-driven snitch probe install for wg-cp0-onboarded peers (Block 61
/ ADR 0316 / offline-enrollment.md §11.6a). **Block 61H multi-platform
builds:** Linux + macOS supported via per-OS task
dispatch; Windows handled by a sibling installer (see *OS support*
below).

**When to use this role:** any peer that came up via the bootstrap +
offline-enrollment bundle pair. The snitch binary is read from the
unpacked bootstrap bundle on the host; the per-device probe API key is
read from the offline-enrollment CBOR bundle (or, for ad-hoc test envs,
passed via inventory).

**When NOT to use this role:** dev workstations, snapshot-debug peers,
or any peer onboarding via setup-key-only (no bundle). Those still use
the legacy `curl <collector>/install | bash` flow at
[`../../../snitch/install-probe.sh`](../../../snitch/install-probe.sh).

## OS support

| OS / family | Status | Service manager | Install path | Per-OS task file |
|---|---|---|---|---|
| Linux | supported | systemd | `/usr/local/bin/snitch` + `/etc/systemd/system/snitch-probe.service` | [`tasks/Linux.yml`](tasks/Linux.yml) |
| Darwin | supported | launchd | `/usr/local/bin/snitch` + the LaunchDaemon plist `tasks/Darwin.yml` installs | [`tasks/Darwin.yml`](tasks/Darwin.yml) |
| Windows | **not supported in this role** | sc.exe / Windows Service | n/a | use [`../../../snitch/install-probe-windows.ps1`](../../../snitch/install-probe-windows.ps1) |

### Why Windows is a sibling installer, not part of this role

Ansible can manage Windows hosts, but the connection layer is WinRM
rather than SSH and the task surface uses `win_*` modules with a
mostly-disjoint shape from the POSIX modules used here (`win_copy` /
`win_template` / `win_service` etc.). That's a structural enough
divergence that bolting it on would force every Linux/Darwin task to
carry `when: ansible_facts['system'] != 'Windows'` guards or live in a
nested block, which obscures the much simpler Linux+Darwin path. We
also expect Windows snitch deployments to be operator laptops handled
ad-hoc rather than fleet-managed today, so the PowerShell installer is
the right ergonomic shape (run-once, no inventory). When fleet-managed
Windows peers become a real concern, add a sibling `snitch-bundle-win`
role with the WinRM-shaped tasks; do not retrofit this one.

## Variables

See `defaults/main.yml`. Key knobs:

- `snitch_bundle_unpack_dir` — where the bootstrap bundle was unpacked.
- `snitch_enrollment_bundle_path` — where the per-device CBOR bundle lives.
- `snitch_collector_url` — collector URL reachable via wg-cp0.
- `snitch_relay_urls` — comma-separated relay URLs for transport assertion.
- `snitch_probe_key_inventory` — override for ad-hoc envs (avoids needing `bundle-extract`).

## Idempotency

Re-running the role on an already-onboarded host:

- Binary copy: idempotent (only re-copies if bundle binary's hash differs).
- API key extraction: idempotent (no_log; same input → same output).
- Service unit + env file: re-rendered; handler restarts service only on change.
- Service: `state: started` (Linux) / `launchctl kickstart` (Darwin) is no-op if already running.

## Bundle binary expectations

The bundle delivered to a host MUST carry the `binaries/snitch` (or
`binaries/snitch.exe` on Windows) appropriate for the target host's
GOOS+GOARCH. `bundle-create` is the source of truth for this: at
issuance time it picks the right cross-compiled binary from the
collector's per-OS-arch tree (Block 61H `/app/bin/<os>-<arch>/` layout)
and embeds it in the bundle. Bundles issued before Block 61H ship the
linux/<arch> binary only; for Darwin re-enrollment, re-issue the
bundle.

## Dependencies

- [`bundle-extract`](../../../enrollment/cmd/bundle-extract/) helper installed on the host (or `snitch_probe_key_inventory` set as inventory override for ad-hoc test envs).
  The helper reads a CBOR field from the offline-enrollment bundle. Built from `ops/enrollment/cmd/bundle-extract/main.go`; deployed to the host via the same bootstrap bundle that delivers the snitch binary itself (Block 61, [`bootstrap-bundle.md` §2](../../../../docs/design/bootstrap-bundle.md)).
- The offline-enrollment bundle on the host must carry the `snitch_probe_key` CBOR field (per [`offline-enrollment.md` §11.6a](../../../../docs/design/offline-enrollment.md)). Bundles issued before Block 61 don't have the field; for those peers, set `snitch_probe_key_inventory` instead and run.
- `wg-cp0` interface up + reaching the collector. The role does NOT
  enroll the host — assume Block 22 wg-cp0-client role has already run.

## Cross-references

- Block 61 in implementation-plan.md
- Block 61H multi-platform builds
- F-043 finding — snitch lacked relay-transport visibility
- [offline-enrollment.md §11.6a](../../../../docs/design/offline-enrollment.md)
- [bootstrap-bundle.md §2 (snitch binary delivery)](../../../../docs/design/bootstrap-bundle.md)
- [Per-OS installers in the snitch repo subtree](../../../snitch/) — `install-probe.sh` (Linux), `install-probe-darwin.sh` (macOS), `install-probe-windows.ps1` (Windows)
