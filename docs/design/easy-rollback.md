# Easy rollback — engineering principle

**Status:** Proposed 2026-05-19.
**Anchor ADR:** [ADR 0859](../adr/0859-easy-rollback-engineering-principle.md).
**Block:** in-flight → easy-rollback (engineering-principle declaration; per-service adoption is a tier-0 sweep).

## The principle

> Every service we deploy can be rolled back to its last 3 known-good image SHAs with one command — by any operator, from any session, without consulting another doc.

This is a one-line operational invariant. It is short on purpose so it survives.

## Why this is a first-class invariant

Three forcing functions:

1. **Pace.** The repo's deploy cadence is high — image bumps land hourly during active days. The window between "the image is live" and "we discover it's broken" is sometimes seconds (a crash loop visible in `docker inspect`) and sometimes minutes-to-hours (a UX regression like the panoscope "real peers missing" gap surfaced 2026-05-19 after PR #238). In both cases the operator's *next minute* matters: every minute of degraded state is a minute the operator can't trust the rest of the fabric.
2. **Surface area.** Tier-0 services (panoscope, portal, zenoh-router, audit-subscriber, isitup, brief, dataflows, airdefenders, dashboard, mgmt, oauth2-proxy) are eleven things. Each can break independently. Each currently has its own ad-hoc recovery dance (find prior SHA, sed compose file, force-recreate). Eleven different dances ≈ no dance.
3. **Multi-agent reality.** Agents push image bumps; an operator across the room sees a 502 and doesn't have the deploy session's context. Without a substrate, the operator and the agent diverge on "what was the last good SHA" — which is exactly the failure mode that left the user staring at `bad gateway` while the agent debugged config split-brain on 2026-05-19.

The principle is the floor. Specific services can build richer rollback (blue-green, canary, schema-aware staged migration) on top — but every service starts at the floor.

## What "rollback" means here

Scoped narrowly on purpose. **In:**

- **Container image substitution.** Compose service flips from current image SHA to a previous one; container recreated; service back on a known-good binary.

**Explicitly out:**

- Schema migrations (DB), runtime config (compose env, role group_vars), persistent state (volumes, MinIO objects, postgres tables). Those need their own rollback discipline; conflating them into "rollback" creates the impression that a single command can undo arbitrary harm. It can't.
- Rolling back the *repo* (revert PR, force-push). That's the [ADR 0109 trunk-based git posture](../adr/0109-git-collaboration-trunk-based.md) territory and uses `git revert`, not this substrate.

A separate sibling design covers config rollback ([Block 63A progressive config rollout with auto-revert, ADR 0224](../adr/0224-progressive-config-rollout-with-auto-revert.md)) — the two compose.

## Two artifacts per service

### 1. Per-service in-repo ledger

`ops/ansible/roles/<service>/deploy-history.yml`:

```yaml
# Last 8 deployed image SHAs for <service>, newest first.
# Owner: whichever PR bumped the role's image pin.
# CI: a workflow PR-checks that any bump to the role's image default
#     also appends a new row here (and prunes >8 oldest rows).
service: panoscope
history:
  - sha: sha-59da3c3
    image: ghcr.io/usareur-af/panoscope@sha256:506421f8de85617aef5f462c87bbb0e9d7ef9570ae1f5990fb0a86b9f134756a
    pr: 238
    merged_at: 2026-05-19T22:00:00Z
    summary: "feat(panoscope): Data-COP visualization"
  - sha: sha-d281c75
    image: ghcr.io/usareur-af/panoscope@sha256:01c84eccebbcb6aba9fc6fa5bc70530e1f00e3dfaad003970703dc162d3b6bcb
    pr: 0  # WIP image, not from a merged PR — flagged DANGER: do-not-roll-to
    merged_at: 2026-05-19T20:23:02Z
    summary: "wip(panoscope-actually-render): handoff baton"
    do_not_roll_to: true
  - sha: sha-90ce0b4
    image: ghcr.io/usareur-af/panoscope@sha256:df60703e8175...
    pr: 228
    merged_at: 2026-05-19T17:40:13Z
    summary: "feat(synth-vendor): notional-namespace collapse"
```

Why in-repo: an operator can `git log -p ops/ansible/roles/panoscope/deploy-history.yml` and see exactly what shipped when. It survives the host. CI keeps it honest (must-append rule + max-8 prune rule).

### 2. Per-host runtime ledger

`/var/lib/goat/<service>/deploy-history.jsonl`:

```jsonl
{"ts":"2026-05-19T22:08:54Z","action":"deploy","image":"ghcr.io/usareur-af/panoscope@sha256:506421f8...","prev_image":"ghcr.io/usareur-af/panoscope@sha256:df60703e...","operator":"person-30@example.org","via":"ansible/panoscope.yml"}
{"ts":"2026-05-19T22:11:32Z","action":"rollback","image":"ghcr.io/usareur-af/panoscope@sha256:01c84ecc...","prev_image":"ghcr.io/usareur-af/panoscope@sha256:506421f8...","operator":"person-30@example.org","via":"rollback.sh","reason":"data-cop deploy lost real peers"}
```

Append-only NDJSON. Written by both the Ansible role (on every successful apply) and the rollback script (on every rollback). The host-side log is the source of truth for "what's actually running"; the in-repo ledger is the source of truth for "what we committed to shipping."

When the two diverge — host running a SHA not in the in-repo last-8 — the rollback script refuses (footgun guard) unless `--force` is passed.

## One command

`ops/scripts/rollback.sh <service> [N]` — N defaults to 1 (previous SHA), max N is 7 (8th oldest in the in-repo ledger).

```bash
./ops/scripts/rollback.sh panoscope          # roll back panoscope one step
./ops/scripts/rollback.sh panoscope 2        # roll back panoscope two steps
./ops/scripts/rollback.sh panoscope --list   # show the per-service last-8 with timestamps + PRs
./ops/scripts/rollback.sh panoscope --status # show currently-running SHA on each per-service host
```

Mechanics: SSH to the host (mesh IP from the same `inventory/mgmt-hosts-mesh.ini` ansible uses), `sed` the compose file's `image:` line to the chosen SHA-pinned reference, `docker compose up -d --force-recreate`, append a `rollback` entry to the host runtime ledger. Idempotent: re-running is a no-op if already on the target SHA.

Refusals:
- Target SHA flagged `do_not_roll_to: true` in the in-repo ledger.
- Host's currently-running SHA isn't in the in-repo ledger (host-vs-repo divergence — operator must investigate).
- Service is on a per-site image pin override that the rollback script doesn't know how to flip (escape hatch: print the manual sed + recreate steps and exit).

### Env-var-defaulted compose images

Some services don't hardcode `image:` directly. They render `image: ${SERVICE_IMAGE:-default}` in the compose so an operator can override via env. Examples: isitup, oauth2-proxy-dashboard. For those, `sed`'ing the compose file's `image:` line would replace the *literal* `${SERVICE_IMAGE:-...}` text, which is wrong — Compose would interpret the resulting line as a literal image name `${SERVICE_IMAGE:-...}` and fail to start.

Per-host metadata in `deploy-history.yml` toggles the script onto a different rollback path:

```yaml
hosts:
  - host: 100.127.0.186
    ssh_user: goat
    container_name: isitup
    compose_path: /opt/mgmt-host/docker-compose.isitup.yml
    # Optional — when both env_file and env_var are set, rollback.sh
    # flips ENV_VAR's line in ENV_FILE instead of sed'ing the compose.
    # Compose's ${VAR:-default} substitution then picks the new value
    # at the next `up -d --force-recreate`.
    env_file: /opt/mgmt-host/.env
    env_var: ISITUP_IMAGE
    site: nova-a
```

Mechanics on this path: `sed` the `<env_var>=...` line in `<env_file>` (or append if absent), then `docker compose up -d --force-recreate <container>`. Runtime ledger entry records `via: rollback.sh:env_file` to distinguish from the default sed-on-compose path. The role itself must keep the env_file in sync on every apply (`isitup` role writes `ISITUP_IMAGE={{ isitup_image }}` to `/opt/mgmt-host/.env` as part of its apply tasks).

## Tier-0 services (first wave)

The eleven services that, if broken for a non-trivial period, immediately impact what an operator or partner sees in a browser:

| Service              | Role                                  | Why tier-0                                          |
|----------------------|---------------------------------------|------------------------------------------------------|
| panoscope            | ops/ansible/roles/panoscope/          | Operational picture; "is the fabric working" view    |
| goat-zenoh-portal    | ops/ansible/roles/zenoh-portal/       | Vendor self-service; partner-facing                  |
| zenoh-router         | ops/ansible/roles/zenoh-router/       | The fabric; everything else depends on it            |
| audit-subscriber     | ops/ansible/roles/audit-subscriber/   | Compliance integrity                                 |
| isitup               | ops/ansible/roles/isitup/             | Operator-facing status page                          |
| brief                | ops/ansible/roles/brief/              | Partner briefing surface                             |
| dataflows            | ops/ansible/roles/dataflows/          | Partner briefing surface                             |
| airdefenders         | ops/ansible/roles/airdefenders/       | Demo/partner-facing                                  |
| netbird-dashboard    | ops/ansible/roles/netbird-dashboard/  | Mgmt console (admins)                                |
| netbird-mgmt         | ops/ansible/roles/netbird-mgmt/       | Mgmt API; peer enrollment                            |
| oauth2-proxy-dashboard | ops/ansible/roles/oauth2-proxy-dashboard/ | L7 admin fence; failure = wide-open dashboard or denied-all dashboard |

Tier-0 sweep: each of these gets a `deploy-history.yml` + a CI workflow check + Ansible-role wire-up to write the host runtime ledger. Adoption sequence: panoscope first (it's the service that motivated this), then the rest in parallel.

## CI substrate

One reusable workflow `.github/workflows/deploy-history-check.yml`:

- Triggers on PRs that modify any `ops/ansible/roles/*/defaults/main.yml`.
- For each modified role: if the image-pin default changed (`<role>_image: ...`), require the matching `deploy-history.yml` to have a new top row matching the new SHA, and require the ledger to have ≤8 rows.
- Failure mode: clear PR-comment explaining the missing append.

## Open questions

1. **Where does the rollback script live in the operator's `$PATH`?** Today `ops/scripts/rollback.sh` requires `cd` into the repo. Probably fine for v1; eventually fold into `work` CLI as `work rollback <service> [N]`.
2. **Does the rollback script need a "rollback-of-rollback" guard?** I think yes — if the operator rolls back, sees the same problem, the script should warn "you just rolled to this SHA 2 min ago; the problem may not be image-side." Cheap to add.
3. **Per-site vs per-fleet ledger?** Today the role's `deploy-history.yml` is one file; in production we'll want per-site overrides (one site on a different SHA than another during a staged rollout). v2.
4. **Image-pin discovery for services that use `:latest`?** Panoscope currently pulls `:latest`, no SHA pin in the role default. The rollback substrate REQUIRES SHA pinning. First step of tier-0 sweep: convert every `:latest` to a SHA pin (one PR per service; image-pin bump PRs are already this shape).

## Composition

- **Ansible discipline ([CONTRIBUTING.md](../../CONTRIBUTING.md)):** roles append host runtime ledger on every successful apply.
- **Progressive config rollout ([ADR 0224](../adr/0224-progressive-config-rollout-with-auto-revert.md)):** sibling pattern for *config* rollback (not image).
- **Per-service failure mode declaration ([ADR 0317](../adr/0317-per-service-failure-mode-declaration.md)):** every tier-0 service's role declares whether it's fail-open / fail-closed / fail-static. Rollback is the operator response when fail-open turned out to be the wrong default.
- **PR governance (Block 66):** the deploy-history CI check is a branch-protection candidate once stable.

## Why not just "operator rolls back manually"

Because the manual dance is repo-knowledge-intensive:

- Find the prior SHA: `gh run list --workflow=<service>-image.yml --branch=main` + cross-reference timestamps + check that nothing destructive landed in between.
- Find the compose path on host: not standardized today (some at `/opt/<service>/`, some at `/opt/docker-compose/<service>/`).
- Edit compose: `sed` with no validation that the SHA exists locally on the host (rollback-to-image-not-yet-pulled = race condition).
- Force-recreate: docker compose syntax varies by version.

Each step is a place an operator under pressure can make an irreversible mistake. The 2026-05-19 panoscope incident hit three of these in the same hour. The script encapsulates the dance.

## What this is not

- A substitute for canary deploys, blue-green, progressive rollout. Those are higher-tier patterns; rollback is the floor.
- A schema-migration tool. Schema migrations need forward-fix or down-migration; image rollback alone won't undo a destructive schema change.
- A multi-tenant tool. Rollback is per-service per-host. Cross-service coordination (rollback A but only if B is also on version X) is out of scope.
