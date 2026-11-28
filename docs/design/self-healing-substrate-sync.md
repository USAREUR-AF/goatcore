# Self-healing substrate sync — design (Block 81)

> **Reconciled (2026-11-28):** how this design relates to `goatbuilder standup`, the site cache and the pull converger, and what is kept from it, is in [cold-start-designs-reconciled.md](cold-start-designs-reconciled.md) and [ADR 1277](../adr/1277-one-signed-site-bundle-is-the-cold-start-artifact.md) (Accepted, not built).

**Status:** design-only; no implementation in this branch.
**Added:** 2026-05-12.
**ADR proposal:** TBD (file when implementation phase opens).

## Problem statement

A mgmt-host Orin that is powered down for an extended period (days to
months) and powered back up does **not** auto-update its substrate to
match current trunk. The operational consequence — surfaced on
2026-05-12 — is that any substrate
fix that landed in trunk while a host was offline has to be applied
**by an operator-fired Ansible run**. There is no boot-time mechanism
that says "this host is running stale substrate, pull current trunk
and reconcile."

The shape of failure surfaced on 2026-05-12:

1. F-096 chicken-and-egg: clock starts at 1970 because RTC has no
   battery backup. The `chrony-presync.service` that exists in trunk
   (closes F-096 forward) was never installed on the host because the
   host was last bootstrapped before `ba47f494` (2026-05-09) and no
   substrate-update mechanism re-runs the `ntp-hygiene` role on existing
   hosts.
2. F-093 Docker data-root on eMMC: ditto. The `docker-engine` role
   renders `/etc/docker/daemon.json` with `data-root` on NVMe, but only
   on a FRESH bootstrap. Existing hosts retain the eMMC default until
   an operator runs `f093-retrofit.yml`.
3. WireGuard tai64n replay protection: bad clock → 1970 tai64n is
   rejected by the relay (newer tai64n already on file). Resolves once
   clock is right; cascade with #1.

For routine reboots of hosts whose substrate matches current trunk,
the existing systemd unit set (LUKS auto-unlock per Block 65, then
`chrony-presync` then `docker.service` then `mgmt-host-compose-watch`)
already auto-recovers. The gap is specifically the **stale substrate
case**.

## Today's mechanism (the bridge until Block 81 lands)

Two operator-fired playbooks cover the case manually:

- **`ops/ansible/f093-retrofit.yml`** — landed 2026-05-12. Codifies the
  Docker data-root + containerd root migration ceremony from
  `docs/operations/docker-data-root-migration.md`. Idempotent;
  `meta: end_host` short-circuits when Docker Root Dir is already on
  NVMe. Two modes: `fresh` (rename + reconfigure) and `migrate` (full
  rsync per the doc procedure).
- **`ops/ansible/site-resume.yml`** — landed 2026-05-12. Single-command
  operator entry point. Its first step is a clock-step (only acts when skew > 1d).
  Phase 1 imports `f093-retrofit.yml`. Phase 2 imports
  `site-bootstrap.yml`. Phase 3 reports Spock subscription health +
  advises on operator-fired re-create if invalidated.

These are correct for today's small fleet. They become operator toil
at fleet sizes > ~10 hosts and they do not run AT ALL on hosts the
operator doesn't remember to touch — exactly the case Block 81 closes.

## Block 81 — proposed mechanism

A host-side systemd service that, on first boot after a quiet period:

1. Acquires a real clock (depends on site-resume.yml's clock-step —
   plain NTP step before chrony tries NTS).
2. Fetches current trunk substrate from a trusted source (see
   "Trust-source" below).
3. Compares against host's current substrate signature (a manifest of
   role versions + checksums + last-bootstrap-commit).
4. If drifted, runs the host-local subset of `site-bootstrap.yml`
   against `localhost`. Tasks that depend on group_vars / inter-host
   coordination (Spock peering, federation peers) are skipped — those
   stay operator-fired.
5. Records the new substrate signature.
6. Emits to the snitch collector (so operator dashboard sees fleet
   convergence state).

Out of scope for Block 81 (these stay operator-fired):
- Spock sub_drop + sub_create with synchronize_data (data-loss risk).
- Cert rotation (cert ceremony stays operator-fired per Block 22M).
- Cross-host coordination (federation peer mesh).

## Trust-source for the substrate pull

Three candidate substrates, ranked by sovereignty + survivability:

### Option A: per-site Orin-C Forgejo (current canonical)
The OCI substrate (Block 75) already provisions Forgejo on Orin-C
for first-party `oci.goat/*` images. Adding a goat-ansible git mirror
there is a small extension. Each site has its own Forgejo; no
cross-site coupling.

- Pros: per-site sovereign; survives site partition; no cloud dep
  for the routine case; reuses Block 75 substrate.
- Cons: Orin-C must be up. On a fresh site bringup (no Orin-C yet),
  no source. Single point of failure within a site (Orin-C goes down
  = no self-heal). Doesn't help single-Orin sites (prov-* shape).

### Option B: Cloudflare R2 bucket (image-mirror tier)
Block 68 rapid-bringup substrate already uses R2 for
image mirror. Adding a signed tarball of `ops/ansible/` (current
trunk HEAD) is a small CI extension.

- Pros: globally available; survives any single-site loss; cheap;
  CI-published every commit.
- Cons: requires internet egress at host boot. The "no public-internet
  resident secure plane" invariant (Block 22) limits this to a
  read-only opt-in pull, never an inbound listener. Signature
  verification against the offline-CA root closes the supply-chain
  attack class.

### Option C: hybrid (A primary, B fallback)
Default to Forgejo (sovereign); fall back to R2 if Forgejo
unreachable; fall back to "stay on current substrate" if both fail.

- Pros: best-of-both; never blocks boot.
- Cons: more substrate to maintain. Operator must keep both in sync.

**Recommended:** Option C, with the cost-benefit revisited after
Block 75 (OCI substrate) lands fully — until then, Option B is the
cheapest cut.

## Implementation phasing

Block 81 implementation will be staged. Each phase is independently
valuable + ships independently:

- **81.0 — preconditions:**
  Block 75 OCI substrate must be at least at Forgejo-on-Orin-C state.
  Block 68 rapid-bringup R2 mirror must be terraform-applied.

- **81.1 — substrate signature manifest:**
  CI emits `ops/ansible/.substrate-manifest.json` on every commit to
  `ops/ansible/` that touches roles + playbooks. Format: list of
  `{role_name, checksum, current_trunk_commit}`. Host signature is a
  read of the same file from disk. Comparison is "are the trunk
  commits I've seen for these roles equal to the trunk commits on
  disk?" Plus a manifest-of-state-on-host that records what was last
  applied.

- **81.2 — `goat-self-heal.service` (one-shot pull-and-compare):**
  systemd one-shot on boot. Pulls current manifest from trust-source.
  Compares against on-disk manifest. If drifted, EMITS to bus + snitch
  but does NOT auto-apply yet (gate on operator confirmation). Lets
  us watch drift accumulate fleet-wide before flipping on auto-apply.

- **81.3 — operator-toggle for auto-apply:**
  Per-host group_var `self_heal_auto_apply: false` (default) → host
  emits drift signal + waits. Flip to `true` per-site after observing
  81.2 metrics for two weeks. Auto-apply runs the local-only subset
  of site-bootstrap (no cross-host coordination).

- **81.4 — alerting on stuck drift:**
  Prometheus alert when a host has been at non-zero drift for > 14
  days (says "operator-fired site-resume.yml needed for site X").
  Composes with Block 27 wide-event observability and the snitch
  dashboard.

- **81.5 — Spock subscription auto-recreate (deferred):**
  ONLY when 81.4 has shown stable. The sub_drop + sub_create dance
  is destructive on the data path; not safe to auto-fire until we
  have months of 81.4 observation.

## Open questions

1. **What's the right interval?** Block 81.2 says "on boot." For a
   long-running host that doesn't reboot, drift accumulates silently.
   A periodic timer (weekly?) would catch drift on never-rebooting
   hosts. Trade-off: more cron-style autonomous work → more attack
   surface. Default: boot-only at first; revisit after 81.4 data.

2. **Versioning the manifest:** what if trunk's manifest schema
   changes? Need a version field + back-compat handling, OR a
   "manifest is per-trunk-commit, schema is implicit" approach
   (simpler, less robust to long-lived hosts).

3. **Failure mode of self-heal itself:** what happens when 81.3
   auto-apply fails partway? Today's site-bootstrap is idempotent so
   re-run is safe, but a fail mid-state could leave the host in a
   worse-than-stale state. Mitigation: 81.3 only auto-applies tasks
   marked `host_local_safe: true`; anything else stays operator-fired.

4. **Composition with Block 38 chaos:** Block 38 (continuous
   architectural chaos) substrate could in principle USE 81 as the
   recovery primitive — simulate "host offline for a week" then
   verify 81 brings it back. Worth checking after 81.3 ships.

## Composition (existing substrate this builds on)

- **Block 65** — LUKS multi-modal continuity. Without auto-unlock, no
  boot at all. Already landed.
- **Block 22K + 22M** — offline-CA-signed bundles. The trust anchor
  for verifying the pulled manifest comes from the bundle's
  `tls_trust_anchor` field. Already landed.
- **`chrony-presync.service`** (in `ntp-hygiene` role) — clock at
  boot. Already landed.
- **`docker-engine` role** — F-093 forward-only fix. Already landed.
- **`mgmt-host-compose-watch`** — auto-restart compose stack on
  reboot. Already landed.
- **Block 27 wide-event observability** — substrate-drift events emit
  to OpenObserve. Provides the alerting surface.
- **Block 75 OCI substrate** — Forgejo locus for Option A. Partly
  landed (substrate spine + first consumer); full deployment queued.
- **Block 68 rapid-bringup** — R2 image-mirror locus for Option B.
  Substrate landed; not yet terraform-applied.

## Why not other approaches

### Periodic fleet-roll (operator-fired)
Today's de-facto pattern. Operator runs `site-bootstrap.yml` against
each site on a cadence (no formal cadence today). Pros: no new
mechanism; works today. Cons: doesn't scale; operator-dependent;
hosts that go offline between rolls don't get covered.

### Phoenix-native rebuild
Each long-offline host is rebuilt from bootstrap-bundle (Block 68).
Pros: clean state; phoenix-architecture-aligned. Cons: data loss
(pg state, ory state, certs, secrets); requires the host to be
brought down for a full reflash. Reserve for catastrophic-substrate-
divergence, not routine drift.

### Polling-style config agent (Puppet / Chef shape)
A daemon on each host polls a config server every N minutes and
reconciles. Pros: actively explored pattern. Cons: heavyweight;
attack surface; requires server-side agent. Strictly worse than
Block 81's "pull on natural boundaries" for our scale.

## Cross-refs

- [`ops/ansible/f093-retrofit.yml`](../../ops/ansible/f093-retrofit.yml)
- [`ops/ansible/site-resume.yml`](../../ops/ansible/site-resume.yml)
- [`docs/operations/docker-data-root-migration.md`](../operations/docker-data-root-migration.md)
- `docs/findings/F-093-docker-data-root-default-plus-spock-unbounded-wal.md`
- `docs/findings/F-096-nts-only-chrony-cant-bootstrap-from-implausible-clock.md`
- [`docs/operations/post-partition-convergence-runbook.md`](../operations/post-partition-convergence-runbook.md)
