# bootstrap — Block 68 rapid-site-bringup substrate

> **Reconciled (2026-11-28):** nothing in `goatbuilder standup` calls this directory today. What is kept, combined or superseded is in [`docs/design/cold-start-designs-reconciled.md`](../../docs/design/cold-start-designs-reconciled.md).

Tooling for the 1 h single-Orin / 2 h three-Orin rapid-bringup
commitment. Companion to the bootstrap-bundle artifact spec at
[`docs/design/bootstrap-bundle.md`](../../docs/design/bootstrap-bundle.md);
procedure + budget at
`docs/design/rapid-site-bringup.md`.

## Layout

| Path | Block | Purpose |
|---|---|---|
| [`rebuild.sh`](rebuild.sh) | bootstrap-bundle.md §6 | Tag → tarball → sign → age-encrypt → optional T1 S3 push for the bootstrap bundle |
| `recipients.txt` | bootstrap-bundle.md §4.1 | age recipient roster + ssh-keygen `allowed_signers` (dual-purpose) |
| [`first-boot/`](first-boot/) | 68A.2 | On-host first-boot script + USB-userdata schema + systemd unit + Docker-based integration test |
| [`first-boot/integration-test/`](first-boot/integration-test/) | 68A.2 | End-to-end test of first-boot.sh in a container — no real Orin/Karbon needed; `./run-test.sh` |
| [`build-image-orin.sh`](build-image-orin.sh) | 68B.2/3 | Pre-baked Orin image build (orin-mgmt + orin-obs share one image, role-at-boot); Step 4 invokes NVIDIA L4T `flash.sh --no-flash --sign` when `--l4t-bsp-path` provided |
| [`build-image-karbon.sh`](build-image-karbon.sh) | 68B.4 | Pre-baked Karbon 801 image build (Ubuntu 24.04 + systemd-boot + Secure Boot via mkosi) |
| [`pull-image.sh`](pull-image.sh) | 68C.3 | Multi-tier image puller (T2 R2 → T1 S3 → T3) with Ed25519 signature verification — handles tier fallback client-side, no Cloudflare Worker required |
| [`drill/`](drill/) | 68D | Drill harness — auto mode polls target's first-boot.jsonl over ssh; manual mode is operator-paced |

## Substrate elsewhere

- [`docs/operations/image-builder-host.md`](../../docs/operations/image-builder-host.md)
  — operator runbook for the build host
- `docs/project/block-68a-config-input-spike.md`
  — UX decision record (USB userdata + serial-console secrets prompt)
- `terraform-prod/r2-mirror.tf`
  — T1 S3 + T2 Cloudflare R2 mirror infrastructure

## Status

**Substrate fully landed.** Hardware-independent pieces are
runnable today; only NVIDIA's `flash.sh` invocation in `build-image-orin.sh`
Step 4 requires the L4T BSP on the builder host.

What works end-to-end without hardware:

- ✅ `rebuild.sh --dry-run` builds a bundle tarball; full flow self-verifies
- ✅ `first-boot.sh` validated end-to-end via Docker integration test
  ([`first-boot/integration-test/`](first-boot/integration-test/)) —
  7/7 PASS: USB userdata absorbed, schema validated,
  expect-driven secrets prompt, JSONL emission, mode-0600 envrc,
  group_vars overlay, state marker, re-run safety
- ✅ `pull-image.sh` does multi-tier fallback (T2→T1→T3) and signature
  verification client-side — no Cloudflare Worker needed
- ✅ Drill driver supports auto mode (ssh-poll target's
  `/var/log/goat/first-boot.jsonl` for phase boundaries) and
  manual mode (operator-paced)
- ✅ `build-image-karbon.sh` mkosi config validates; produces a real
  raw IMG when run on a Linux builder host with mkosi installed
- ✅ `build-image-orin.sh` Steps 1–3 (snapshot + image cache + first-boot
  staging) runnable today; Step 4 (NVIDIA L4T flash) invokes flash.sh
  end-to-end when `--l4t-bsp-path` is provided

What still needs hardware or live cloud creds:

- 🟡 First real-Orin boot smoke (NVIDIA L4T BSP + target Orin in recovery)
- 🟡 First real-Karbon boot smoke (x86 cloud-init VM stand-in works in the interim)
- 🟡 First S1 / S2 PASS drill against real Orins
- 🟡 `terraform apply` of `terraform-prod/r2-mirror.tf` (operator-fired
  with `TF_VAR_cloudflare_account_id` from `.envrc.local`)
- 🟡 First end-to-end push to T1 S3 + T2 R2 with operator's offline-CA
  YubiKey signature
- 🟡 68C.2 — T3 sovereign mirror substrate selection per
  [ADR 0220](../../docs/adr/0220-multi-cloud-multi-substrate-posture.md)
- 🟡 68F ADR promotion — gated on two PASS drills

**68C.1 close-out (revised):** Cloudflare Worker for
tier-fallback URL rewriting was originally planned. On reflection,
`pull-image.sh` already does T2→T1→T3 fallback + signature verification
client-side, so the Worker is redundant for v1. The R2 bucket serves
ciphertext via custom domain `image-mirror.example` with no
server-side logic. Worker can land later if rate-limiting / audit / or
side-channel logic is needed; not gating.
