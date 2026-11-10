# oci-substrate-t2-sync

Per-site Orin-C cron that syncs the canonical-set from T1 (R2 + ECR)
to T2 (Orin-C Forgejo) on a 48h interval. Pull-through cache (Block 71's
[`docker-registry-mirror`](../docker-registry-mirror/)) covers urgent
cache-misses between cron runs; this cron is the drift-safety-net per
ADR 0838 §"Sync model"
+ design doc §6.2.

## Status

**Wired, still switched off.** What changed and what did not:

Landed:

- **T1 endpoints carry real values** — `oci_t1_r2_endpoint` and
  `oci_t1_ecr_endpoint` are set in `ops/ansible/group_vars/all.yml`, the
  cluster-shared home (the provisioning tier is one cross-goatnet service;
  that file explains why it is not per-goatnet).
- **The credential is named** — `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY`,
  account-scoped, read via `lookup('env')`, asserted by the role.
- **`meta/main.yml`** with the `rollback-ledger` dependency, plus an empty
  `deploy-history.yml`, so enabling the role no longer widens the
  rollback-coverage gap.
- **A playbook references it** — `oci-substrate-deploy.yml` phase 3, gated on
  `oci_t2_sync_enabled` at the role application so a disabled role writes no
  ledger row.

Not landed, which is why `oci_t2_sync_enabled` is still `false` everywhere:
no R2 credential minted, no first `rclone` seed, ECR repos not created (no
`aws_ecr_*` resource exists in any terraform tree), the `crane copy` registry
semantics unproven against the R2 bucket, and the deploy-time → cron-time
credential delivery hop unresolved. Full checklist:
`rapid-site-bringup.md` §5
"What the light-up still needs (live-side)".

Meanwhile the first-consumer flow (Block 75E admit-audit) still bridges
directly from ghcr.io → Orin-C Forgejo without going through T1. That is a
one-time bootstrap, not the steady-state path.

## Variables

| Variable | Default | Purpose |
|---|---|---|
| `oci_t2_sync_enabled` | `false` | Master switch. Flip to `true` per-site when T1 is wired. |
| `oci_t2_forgejo_endpoint` | `localhost:3030` | Local Forgejo OCI registry (this Orin-C). |
| `oci_t1_r2_endpoint` | `""` | R2 endpoint for the substrate's R2 mirror (set when T1 R2 lands). |
| `oci_t1_ecr_endpoint` | `""` | ECR endpoint (set when T1 ECR lands). |
| `oci_t2_sync_interval_hours` | `48` | Cron interval; tunable per-site (24h–168h). |
| `oci_t2_sync_offset_hours` | `""` | Stagger offset across sites; auto-computed if empty. |
| `oci_canonical_set_path` | `/etc/oci-goat/canonical-set.yaml` | Canonical-set on this host. |
| `oci_sync_script_path` | `/usr/local/bin/oci-goat-sync.sh` | Sync script. |
| `oci_sync_log_path` | `/var/log/oci-goat-sync.jsonl` | Per-run JSONL log. |

## Operator-fired manual run

Once T1 is wired:

```bash
ssh orin-c.<site>.<zone> /usr/local/bin/oci-goat-sync.sh
tail -f /var/log/oci-goat-sync.jsonl
```

## Cross-refs

- ADR 0838
- Design doc §6
- Block 75D in implementation-plan.md
