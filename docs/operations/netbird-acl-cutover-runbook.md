# NetBird L4 ACL cutover — operator runbook (Block 83C.2-L4)

Operator-facing runbook for replacing NetBird's wide-open `Default All→All accept` policy with explicit per-group permits on a target site. Closes the L4 portion of F-132.

> **Before you edit the matrix: you cannot.** `ops/ansible/netbird_acl_matrix.yml`
> is generated from `ops/warrant/` (ADR 1129) and CI rejects a hand-edit. This
> runbook covers *applying* the ACL to a goatnet; to *change what it says*, see
> [`warrant-runbook.md`](warrant-runbook.md) first, then come back here to cut
> it over.

| Field | Value |
|---|---|
| Status | Operator-fired cutover — recommend a dedicated session with the live mesh quiesced (no in-flight demos / vendor onboarding ceremonies). |
| Reversible? | By the declared ACL, never by going wide open. There is no Default All→All on any goatnet, not even in an emergency ([ADR 1185](../adr/1185-no-default-all-to-all-standup-applies-declared-acl.md)). Undo a bad change by reverting its PR and re-running `mode=apply`; `mode=rollback` is retired and refuses. |
| Mesh impact | On a goatnet stood up since ADR 1185 there is no Default to begin with: `mode=apply` is what gives peers reach. On an older goatnet that still carries one, `mode=propose` + `mode=apply` leave it in place. `mode=enforce` applies the declared `default_policy_action` to Default (disable or delete) — anything not in the explicit permits is dropped. |

## Why this exists

Before cutover: `Default All→All accept` is enabled; every peer can reach every other peer on every port. NetBird group membership is a label only (used for L7 sidecar group-claim derivation, dashboard auditing, and the mint-time `auto_groups` orchestration), not an L4 gate. Port-differentiated services — Zenoh router :7447, SSH :22 — are wide-open. This is the L4 leak that F-132 names; this cutover closes it.

After cutover:

- ADMIN-class peers (operator laptops): full mesh access including SSH + Zenoh.
- VENDOR-class peers (third-party publishers): Zenoh router :7447 + Traefik :443 only.
- OPERATOR-class peers (observability humans): Traefik :443 + Zenoh :7447 (no SSH, no netbird-mgmt-API L4).
- OBSERVER-class peers (snitch / monitoring): Traefik :443 only.
- MGMT-HOST (the mgmt host itself): full reach to every other peer for health-probing.

## Source files

- Matrix (declarative permits): [`ops/ansible/netbird_acl_matrix.yml`](../../ops/ansible/netbird_acl_matrix.yml)
- Group taxonomy (UUID source): [`ops/ansible/netbird_acl_taxonomy.yml`](../../ops/ansible/netbird_acl_taxonomy.yml)
- Cutover playbook: [`ops/ansible/netbird-acl-cutover.yml`](../../ops/ansible/netbird-acl-cutover.yml)
- Intent matrix (design): `docs/design/mesh-acl-posture.md`

## Pre-flight (before any mode)

- [ ] Mesh is quiesced (no in-flight vendor onboarding ceremonies, no panoscope demo for an outside audience, no partner demo flow scheduled in the next hour).
- [ ] The site's NetBird API token is in your shell.
- [ ] `netbird-acl-bootstrap.yml -e target_site=<site>` has been re-run recently and is idempotent (no group churn pending).
- [ ] You have a second mesh-connected device on hand (so if your primary device gets disconnected by a misconfigured permit, you can roll back from the second one).
- [ ] Your account has an `ADMINS` group claim in Kratos (so the rollback path through the netbird-dashboard works, in addition to via the playbook).

## Step 1 — Survey + plan (mode=propose, ALWAYS START HERE)

```bash
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ansible-playbook \
  -i localhost, --connection local \
  netbird-acl-cutover.yml -e target_site=<site>
```

Read the output carefully. Two summaries print:

1. **Plan summary** — every policy that would be created, with source / dest / proto / port / action.
2. **Peer coverage report** — for each live peer, whether it falls under a SOURCE in any synthesized rule.

**Stop and decide** before moving to Step 2:

- Any peer marked `UNCOVERED` will lose connectivity when Default is disabled. Cross-check against the intent matrix. If a peer is in a group not represented in the matrix (e.g. an orphan from an older mint), either add it to a covered group or extend the matrix.
- The Default policy's name should match the pattern. If `Default policy id: (none found)` and the site's `default_policy_action` is `disable`, the auto-created policy has a different name in NetBird's mgmt-server — fix `default_policy_name_pattern` in the matrix YAML. For `default_policy_action: delete`, absence is the declared end state; the enforce run asserts that no enabled All→All accept policy remains under a different name before printing the 'nothing to do' message. A failing assert means the pattern (or its warrant source) is wrong and must be fixed before proceeding.
- If you find a missing permit (something legitimate that's not in the matrix), add it now: edit `netbird_acl_matrix.yml`, commit, re-run propose. Don't band-aid by minting a policy in the dashboard manually — the rollback path can't reverse a manual permit.

Observed UNCOVERED peers from your first propose run go here:

```
<paste output here when running>
```

## Step 2 — Additive burn-in (mode=apply)

```bash
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ansible-playbook \
  -i localhost, --connection local \
  netbird-acl-cutover.yml -e target_site=<site> -e mode=apply
```

This **adds** the explicit permits but leaves Default enabled. Mesh continues to work exactly as before. Verify in the dashboard:

- All N policies from the matrix render correctly under Policies tab.
- Each policy's source / destination resolves to the expected groups.
- Each policy is `enabled: true`.

Exercise live mesh traffic for ~10 min to make sure burn-in doesn't surface any latent UI / API misbehavior. The Default policy is still permissive, so nothing should break.

Burn-in observed:

```
<paste any observations during burn-in>
```

## Step 3 — Cutover (mode=enforce — THE KILL SWITCH)

**This is the step that removes any Default.** With `default_policy_action: disable` it DISABLES the Default `All→All accept` policy but keeps the row; with `default_policy_action: delete` it DELETES the Default policy row and treats its absence as the intended end state after asserting there is no other enabled All→All accept policy; with `default_policy_action: keep` the playbook refuses `mode=enforce` in pre-flight instead of touching Default at all.

```bash
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ansible-playbook \
  -i localhost, --connection local \
  netbird-acl-cutover.yml -e target_site=<site> \
    -e mode=enforce -e confirm_cutover=YES
```

Without `confirm_cutover=YES` (literal, uppercase), the playbook refuses to proceed. This is the speed bump that forces an explicit "yes I mean it" rather than an accidental flip.

Immediately after enforce:

- [ ] **Smoke test from operator laptop (ADMIN):** `ssh <user>@<mgmt-host-mesh-ip>` succeeds; `curl https://dashboard.<site>.<zone>/` returns 302 → OAuth.
- [ ] **Synth-vendor publishers (VENDOR):** check panoscope page — vendor publish series should keep flowing. If they go dark, vendor peer can't reach Zenoh :7447 → rollback immediately and fix the matrix.
- [ ] **Panoscope subscriber:** reach panoscope at `https://panoscope.<site>.<zone>/` from your laptop — packet animations should still render live.
- [ ] **Unauthorized SSH attempt (NEGATIVE check):** from a non-ADMIN peer (e.g. a VENDOR mesh peer or the synthetic-vendor host), `ssh -o ConnectTimeout=5 <mgmt-host-mesh-ip>` should hang or refuse — that's the cutover proving itself.

Smoke-test observed:

```
<paste smoke-test results here>
```

## Step 4 — Undo (revert, then apply)

`mode=rollback` is retired ([ADR 1185](../adr/1185-no-default-all-to-all-standup-applies-declared-acl.md)). It used to re-enable or recreate a Default All→All and delete every declared policy, which undid a bad cutover by making the whole goatnet flat. It now refuses and prints this procedure.

If a change broke a peer you care about:

1. `git revert` the PR that changed `ops/warrant/` and merge the revert (it carries the regenerated matrix).
2. Re-apply the reverted declaration:

   ```bash
   OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ansible-playbook \
     -i localhost, --connection local \
     netbird-acl-cutover.yml -e target_site=<site> -e mode=apply -e reap_orphans=true
   ```

   `apply` re-shapes drifted policies to the matrix; `reap_orphans=true` removes tagged policies the reverted matrix no longer declares.
3. Confirm with `python3 ops/scripts/netbird-policy-posture.py <site> --url https://<public_mgmt_fqdn>`: exit 0 means no All-group rule and nothing undeclared.

If a peer needs its reach restored, check the scope's membership authority.
In an enforcing warrant scope, [prepare and review the declaration](warrant-runbook.md#4a-membership--which-peer-holds-which-group)
and deploy it through the normal converger workflow. In other scopes,
`goat admin members move` is available where direct CLI writes are authorized.

## Day-N maintenance

The matrix is **the** policy source of truth post-cutover. To extend (add a new permit, add a new group):

1. Edit `netbird_acl_matrix.yml`.
2. Re-run `mode=propose` to see the diff.
3. Re-run `mode=apply` — idempotent (skips policies that already exist).
4. Verify in dashboard.

There is currently no `mode=remove` or `mode=update`. To retract a permit:

- Edit the matrix to remove the rule.
- Run `mode=apply -e reap_orphans=true`, which removes cutover-tagged policies the matrix no longer declares, OR run `mode=rollback` then re-run the propose → apply → enforce cycle. On a goatnet under ADR 1189 the everyday dashboard is read-only; a hand deletion is a `NETBIRD-ADMIN` break-glass edit at `admin-dashboard.<site>` and shows up in `ops/warrant/reconcile.py` until the matrix agrees.

A future Block 17A.5 Policy Engine consumes this YAML directly and ages out manual editing of policies in the dashboard. Until then, this playbook is the way.

## Cross-refs

- F-132 — the leak this closes
- `docs/design/mesh-acl-posture.md` — intent matrix this YAML is the machine-readable form of
- `docs/design/nova-l7-admin-fence-implementation.md` — sibling L7 fence (closed for dashboard + grafana)
- Block 83C.2-L4 — implementation tracking
- [ADR 0411](../adr/0411-ory-idp-mvp.md) — the Kratos / Hydra MVP this matrix is the L4 face of
