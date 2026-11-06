# Operator runbook: rebuild a site's pub/sub state

**Use when:** a site has been reflashed / rebuilt and the freshly-stood-up
`goat-zenoh-portal` is empty. You need to restore every vendor, topic,
schema body, and code example to the same state the prior portal held
— so the synthetic vendor publishers, panoscope, and audit subscriber
all reconnect to a live fabric, not to an empty one.

**Design context:** [`docs/design/pubsub-state-as-code.md`](../design/pubsub-state-as-code.md) (Block 86).

**Pre-condition:** the rebuilt portal is up, healthy, and reachable
(`curl $PORTAL_URL/api/health` returns 200). Its `--operator-api-key`
is set and you know the value (it's the new portal's key; it does NOT
have to match the old one).

## Three rebuild paths

The state-of-record lives in three tiers (in precedence order). Pick
the first one available; the rest is identical.

### Path A — local Forgejo (default; island-mode-compatible)

```bash
git clone http://<utility-host>:3030/goat-pubsub-state/<site>.git pubsub-state
pubsub-apply --from ./pubsub-state \
  --portal-url $PORTAL_URL \
  --api-key   $PORTAL_OPERATOR_API_KEY
```

Expected output: one `created` log line per vendor + per topic + per
schema + per example, followed by exit 0. Re-running prints `no
changes; portal already matches repo`.

### Path B — MinIO snapshot tarball (fallback when Forgejo was lost)

```bash
mc cp local-<site>/pubsub-state-snapshots/latest.tar.gz .
mkdir pubsub-state
tar xzf latest.tar.gz -C pubsub-state --strip-components=1
pubsub-apply --from ./pubsub-state \
  --portal-url $PORTAL_URL \
  --api-key   $PORTAL_OPERATOR_API_KEY
```

The MinIO bucket is on the same utility host as the prior portal; if both
are gone, fall through to Path C.

### Path C — offline tarball from rapid-bringup bundle (true cold start)

Every rapid-bringup bundle (Block 68)
carries a `pubsub-state.tar.gz` artifact under
`bundle/state-of-record/` reflecting the last sync-point before the
bundle was sealed. This is the "we lost everything including Forgejo
and MinIO" recovery path.

```bash
tar xzf bundle/state-of-record/pubsub-state.tar.gz
pubsub-apply --from ./pubsub-state \
  --portal-url $PORTAL_URL \
  --api-key   $PORTAL_OPERATOR_API_KEY
```

The bundle's snapshot may be days/weeks stale — any vendor onboarding
that happened post-bundle will need to be re-run from
[`ops/ansible/operator-onboard.yml`](../../ops/ansible/operator-onboard.yml).

## What `pubsub-apply` does (and doesn't)

It restores **schema definitions, topic registrations, vendor metadata,
and code examples**. After it returns 0, the portal's `/vendor.html`
and `/schema.html` look the same as they did before the rebuild.

It does **not** restore:

- **Vendor API keys + bundles.** Every vendor gets a freshly-minted
  one-time bundle URL on rebuild — those keys are secrets, by
  construction not in the mirror. The portal's create-vendor response
  in the apply log carries the new `api_key` + `bundle_url`; re-share
  them with each vendor.
- **Per-vendor X.509 certs.** Same: the offline CA re-mints, fresh
  serials, fresh bundle. Vendors download + install the new cert
  before they can publish again.
- **Runtime telemetry** (last_seen_at, publish_count). Those
  reconstitute from real publishes after the rebuild.
- **Audit log history.** That's in the `audit-integrity` MinIO bucket
  on the utility host (or its sibling), separate from the state-of-record per
  [ADR 0811](../adr/0811-audit-log-integrity.md).

## Dry-run before applying

`--dry-run` prints the plan and exits with code 2 if anything would
change. Run it first when you're unsure whether the portal is already
populated:

```bash
pubsub-apply --from ./pubsub-state --dry-run \
  --portal-url $PORTAL_URL --api-key $PORTAL_OPERATOR_API_KEY
```

Exit code 0 + `no changes` means the portal already matches; you're
done. Exit code 2 + a list of `would: POST ...` lines means there are
deltas; review them, then drop `--dry-run` and re-run.

## Scoped rebuild (one vendor at a time)

When restoring partner connections one vendor at a time:

```bash
pubsub-apply --from ./pubsub-state --scope <vendor-prefix> \
  --portal-url $PORTAL_URL --api-key $PORTAL_OPERATOR_API_KEY
```

`--scope` narrows the desired-state reading + the deletion sweep so
other vendors on the portal are untouched.

## Verifying the rebuild

After exit 0, three quick checks:

```bash
# 1. Portal vendor list matches.
curl -H "Authorization: Bearer $PORTAL_OPERATOR_API_KEY" \
     $PORTAL_URL/api/vendors | jq '.vendors | length'

# 2. Re-running apply is a no-op.
pubsub-apply --from ./pubsub-state \
  --portal-url $PORTAL_URL --api-key $PORTAL_OPERATOR_API_KEY
# expect: "no changes; portal already matches repo"

# 3. Schema viewer renders for at least one topic.
curl -H "Authorization: Bearer $PORTAL_OPERATOR_API_KEY" \
     "$PORTAL_URL/api/schemas/render/<vendor>/<topic>/v1" | jq '.messages[0].name'
```

## Restoring the mirror timer

After a rebuild, the `pubsub-state-mirror.timer` is what keeps the
Forgejo repo + MinIO snapshot fresh. Re-apply the role:

```bash
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES aws-vault exec legacy --no-session -- \
  ansible-playbook -i inventory/<site>.ini nova-pubsub-state-repo-bootstrap.yml
```

The role is idempotent and will simply confirm the timer is enabled.

## When this runbook fails

- **`pubsub-apply` returns "operator API key required"** — set
  `$PORTAL_OPERATOR_API_KEY` (the *new* portal's key) and re-run.
- **Vendor create returns 500** — the portal's offline-CA dependency
  (cert-sign + age identity) wasn't wired up. Check
  `journalctl -u goat-zenoh-portal -n 100` and fix the cert-mint
  config before re-running apply.
- **MANIFEST hash mismatch** — the tarball / repo was modified between
  fetch and apply. Re-fetch and try again. The reconciler still works
  on a hash-mismatched tree (it diffs entry-by-entry, not by hash);
  the warning is informational.
- **`pubsub-apply` exits 2 on a clean apply** — `--dry-run` is set;
  drop the flag and re-run.
