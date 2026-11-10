# ubuntu-pro-fips

Attach Ubuntu Pro and enable FIPS-validated kernel + crypto modules on the target host. Part of the fleet OS baseline — see [`docs/design/os-baseline-and-luks.md`](../../../../docs/design/os-baseline-and-luks.md) §5.

## What it does

1. Install `ubuntu-advantage-tools` if not present.
2. Attach Ubuntu Pro subscription (token supplied via `ubuntu_pro_token` variable).
3. Enable `fips-updates` service. This replaces the running kernel with a FIPS 140-3 validated kernel + OpenSSL FIPS module on next boot.
4. Verify post-reboot: `cat /proc/sys/crypto/fips_enabled` returns `1`.
5. Record outcome to `/var/log/ubuntu-pro-fips-state.json` for evidence collection.

## Platform notes

- **x86_64:** expected to work cleanly. Canonical FIPS kernel cert covers stock Ubuntu 22.04 / 24.04 amd64 kernels.
- **arm64 stock (RPi 5 on Ubuntu 24.04):** Canonical has arm64 FIPS coverage; verify specific kernel build at enrollment time.
- **arm64 L4T (Jetson Orin / Thor):** `pro enable fips-updates` may not produce a FIPS kernel because L4T uses NVIDIA's kernel, outside Canonical's cert. Role sets `ubuntu_pro_fips_state.kernel_fips = false` and continues; compliance gap logged for the evidence bundle.

## Variables

| Variable | Default | Description |
|---|---|---|
| `ubuntu_pro_token` | _(required)_ | Ubuntu Pro subscription token. Provide via `-e` or `group_vars/all/vault.yml`. |
| `ubuntu_pro_fips_enforce` | `true` | Fail the play if `fips_enabled=0` after role runs on a platform where FIPS is expected. Set `false` for L4T. |
| `ubuntu_pro_state_path` | `/var/log/ubuntu-pro-fips-state.json` | Path to write state evidence. |

## Evidence

Each run writes JSON like:

```json
{
  "timestamp": "2026-04-22T14:00:00Z",
  "hostname": "mgmt-host-vega-a",
  "pro_status": "attached",
  "pro_services": ["esm-apps", "esm-infra", "fips-updates", "livepatch", "usg"],
  "kernel": "5.15.0-1042-tegra",
  "kernel_fips_enabled": false,
  "openssl_fips_enabled": true,
  "platform": "l4t-arm64",
  "gap_documented": "L4T kernel outside Canonical FIPS cert scope"
}
```

Evidence file is picked up by the evidence-bundle Ansible task.

## TODO

- Add automatic reboot + post-reboot verification task (currently requires operator to reboot and re-run).
- Extend for 24.04 arm64 — verify specific Canonical arm64 FIPS kernel build number.
- Integrate with `systemd-pcrlock` for TPM re-seal on kernel updates (Phase 3).
