# portal-vendor-progress runbook

Operator-facing setup for the goat-zenoh-portal `/progress.html`
swimlane (Block 88). Design: [`docs/design/portal-vendor-progress.md`](../design/portal-vendor-progress.md).

## What this gives you

A single page at `https://portal.nova.netbird.nova.example/progress.html`
that shows every external vendor and where each one is on the
onboarding curve:

- **L1** — bundle minted (intrinsic; lights up the moment you `POST /api/vendors`).
- **L2** — vendor's netbird peer has joined the mesh.
- **L3** — vendor ran the bundled `first-publisher.py` (or anything matching its fingerprint).
- **L4** — vendor published something other than the hello example.

Synthetic vendors (the `notional` umbrella + any other `synthetic=true`
rows) are hidden by default; a checkbox at the top flips them in.

L1 always works after a portal restart. L2/L3/L4 require the two
operator-fired bits below.

## Light up L2 (netbird-mgmt peer poll)

1. Mint a NetBird PAT in the dashboard at
   `https://netbird.nova.example/` → Settings → Personal Access
   Tokens. Name it `portal-vendor-progress-readonly`. Scope: any
   identity with read on `/api/peers`. There is no permissions filter
   inside the dashboard today; the PAT inherits the minting user's
   scope.

2. Export the token in your `.envrc.local.d/30-site-services.envrc`:

   ```
   export NOVA_NB_API_TOKEN="nbp_..."
   ```

3. Apply the portal role (no new playbook — the existing
   `portal-deploy.yml` picks the env var up):

   ```
   cd ops/ansible
   OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES aws-vault exec legacy --no-session -- \
     ansible-playbook \
       -i inventory/mgmt-hosts.ini \
       -i inventory/mgmt-hosts-mesh.ini \
       portal-deploy.yml
   ```

4. Verify L2 lights up after one 60 s poll cycle:

   ```
   curl -fsS -H "Authorization: Bearer $PORTAL_OPERATOR_API_KEY" \
     https://portal.nova.netbird.nova.example/api/progress \
     | jq '[.[] | select(.l2_at)] | length'
   ```

   You should see a positive number for every enrolled vendor that
   carries an AutoPeerName-shaped peer in the mesh. `netbird poll
   failed` lines in `docker logs goat-zenoh-portal` mean the PAT was
   rejected — re-mint and re-apply. Success lines log at slog Debug
   and do not appear in container logs by default.

## Light up L3 / L4 (MinIO audit tailer)

The portal reads the same `audit-integrity` bucket the
audit-subscriber writes to. Audit-write
uses a `PutObject`-only policy (`ops/minio/policies/audit-write.json`);
the portal needs a *separate* user with a *read-only* policy on the
same bucket — minted declaratively via `minio_extra_users` on the
existing `roles/minio` (see
`group_vars/nova_hosts/minio.yml`).

1. Generate a secret + add both env vars to your local envrc:

   ```
   # in .envrc.local.d/30-site-services.envrc
   export NOVA_PORTAL_AUDIT_S3_ACCESS_KEY=goat-portal-audit-read
   export NOVA_PORTAL_AUDIT_S3_SECRET_KEY=<openssl rand -hex 24>
   ```

   `direnv reload` so the values are live in your shell.

2. Apply the MinIO bootstrap — the role's pre-flight asserts the
   secret env var is non-empty, then mc-init mints the user from
   `ops/minio/policies/audit-read.json`
   and attaches the policy (idempotent on re-run):

   ```
   cd ops/ansible
   OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES aws-vault exec legacy --no-session -- \
     ansible-playbook \
       -i inventory/mgmt-hosts.ini \
       -i inventory/mgmt-hosts-mesh.ini \
       minio-bootstrap.yml
   ```

3. Re-apply `portal-deploy.yml` — picks up the matching
   `NOVA_PORTAL_AUDIT_S3_*` env vars and wires them into the container.

4. Verify the tailer ran:

   ```
   ssh goat@100.127.0.186 'sudo sqlite3 /opt/goat-zenoh-portal/registry.db \
     "SELECT subscriber_id, last_key, updated_at FROM audit_replay_cursor"'
   ```

   The cursor `last_key` should advance every 30 s (success lines log
   at slog Debug so don't appear in `docker logs` by default). On first
   boot the tailer pulls every historical NDJSON in the bucket;
   ~8 hours of replay per minute, so a 5-day backlog clears in ~15 min.

## Verify the swimlane

```
curl -fsS -H "Authorization: Bearer $NOVA_PORTAL_OPERATOR_API_KEY" \
  https://portal.nova.netbird.nova.example/api/progress | jq .
```

Or open `https://portal.nova.netbird.nova.example/progress.html`
in a browser. The L2 column lights up after one 60 s poll cycle once
the vendor enrolls. L3 lights up the next time the tailer poll catches
a hello-world publish on `<prefix>/hello/v1`. L4 lights up the next
poll after any other publish.

## What can go wrong

- **L2 column always says "—" for every vendor** → no netbird PAT
  configured, or the token is rejected. Check `docker logs` for the
  poller's warning.

- **L3/L4 columns say "—" for every vendor** → MinIO credentials are
  missing or rejected; or the audit-subscriber isn't writing (check
  its `goat_audit_write_errors_total` metric per the
  audit-subscriber runbook).

- **One vendor sits stuck at L1 for days** → that vendor never
  installed the bundle. Reach out via your `FEEDBACK_CHANNEL`.

- **A vendor jumps straight to L4 with no L3** → they skipped the
  example. The UI renders L3 as "(implied)" so the row isn't
  visually broken. No action needed — they're on the fabric.

## Rollback

This change is additive to the portal — schema-additive (two new
tables, no column drops), all goroutines no-op when their env vars
are unset, and the page is a separate URL. To roll back the swimlane:

- Roll the portal image to the previous SHA via
  `ops/scripts/rollback.sh goat-zenoh-portal 1`.
- The new tables stay (they're unused; idempotent CREATE...IF NOT EXISTS
  on a future re-up).
- No data loss; nothing else changes.
