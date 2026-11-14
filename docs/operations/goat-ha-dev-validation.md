# goat-ha-dev — Phase 2 validation runbook

Executable checklist for Phase 2 validation against the temporary EC2 pair (see `docs/design/ha-multi-site.md` §5.8.2).

**Goal:** prove the Spock multi-master + signal federation + coordinated-upgrade design end-to-end at minimum blast radius.

**Environment:** two Ubuntu 22.04 EC2 instances — one x86 amd64 + one arm64 Graviton in two notional cloud regions. Torn down after validation.

**Prerequisite:** `terraform apply` on `terraform/goat-ha-dev.tf`.

---

## Wave 1 — Does NOT need the patched netbird-mgmt image

Can run today even with the CI image-build blocked. Uses stock `netbirdio/management:latest`.

### 1.1 Cluster bring-up

```bash
cd terraform
aws-vault exec legacy --no-session -- terraform apply  # review plan first
```

Capture the four outputs:
- `hadev_amd64_public_ip`
- `hadev_arm64_public_ip`
- `hadev_ssm_connect_amd64`
- `hadev_ssm_connect_arm64`

Enroll both as Netbird peers in the dev mesh (one shell each via SSM):

```bash
# On each node:
sudo netbird up --management-url https://netbird.legacy.example --setup-key $NETBIRD_SETUP_KEY
sudo netbird status  # confirm Connected
```

The inventory entries already exist in `ops/ansible/inventory/mgmt-hosts.ini` under `[hadev_hosts]` (a child group of `[mgmt_hosts]`) with SSM instance IDs, mesh IPs, and per-host `signal_uri` + `site_name` pre-set. Confirm the mesh IPs there match what `netbird status` reports on each node; update if they drifted.

Run `mgmt-host.yml` against them:

```bash
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES aws-vault exec legacy --no-session -- \
  ansible-playbook \
    -i inventory/hosts.ini -i inventory/mgmt-hosts.ini \
    mgmt-host.yml --limit hadev_hosts \
    -e postgres_superuser_password="$HADEV_POSTGRES_SUPERUSER_PASSWORD" \
    -e netbird_encryption_key="$HADEV_NETBIRD_ENCRYPTION_KEY"
```

Both `-e` vars are required (the playbook fail-fasts without them). Generate `HADEV_NETBIRD_ENCRYPTION_KEY` once per cluster via `openssl rand -base64 32`, stash in `.envrc.local`; same key must apply on every node of a multi-master cluster so encrypted setup-keys/tokens in Postgres can be decrypted on any node (see `docs/design/ha-multi-site.md` §5). `signal_uri` is already pre-set per-host in `mgmt-hosts.ini`, no `-e` needed.

**Red flag:** any failure in the Docker-compose up step. **Expected:** pgEdge postgres + signal-server + management containers running on each node.

### 1.2 pgEdge image audit

On each node (via SSM):

```bash
docker exec -it $(docker ps -qf ancestor=ghcr.io/pgedge/pgedge-postgres:16-spock5-minimal) bash
```

Inside the container:

```bash
# Telemetry / phone-home — look for anything unexpected
grep -rE "curl|wget|http[s]?://" /usr/local/bin /usr/local/lib/postgresql 2>/dev/null | grep -v "pg_config\|/docs/" | head -20

# EXPOSE'd ports beyond PG 5432
ps -ef | grep -v "^USER\|grep"

# License file
find / -iname "LICENSE*" 2>/dev/null | head -5
cat $(find / -iname "LICENSE*" -path "*/pgedge*" 2>/dev/null | head -1)

# Snowflake extension?
psql -U postgres -c "SELECT * FROM pg_available_extensions WHERE name IN ('snowflake', 'spock', 'pg_stat_statements');"

# Installed extensions already loaded
psql -U postgres -c "SELECT extname, extversion FROM pg_extension;"
```

**Expected:**
- Spock 5.x available and likely already in the default-loaded list
- No telemetry / phone-home processes
- PostgreSQL License
- Snowflake may or may not be present (one of the open items — document either way)

**Red flag:** any unexplained network callouts; license incompatible with self-hosted.

### 1.3 Spock bidirectional replication between the two nodes

On each node, create the shared replication user + DB-level grants:

```sql
CREATE USER replicator WITH REPLICATION PASSWORD '...' LOGIN;
GRANT CONNECT ON DATABASE netbird TO replicator;
-- Add the usual schema/table grants per pgEdge docs
```

Configure mutual subscriptions:

```sql
-- On amd64 node:
SELECT spock.node_create(node_name := 'hadev_amd64', dsn := 'host=<amd64-mesh-ip> dbname=netbird user=replicator password=...');
SELECT spock.repset_add_all_tables('default', ARRAY['public']);

-- On arm64 node (same plus subscribing to amd64):
SELECT spock.node_create(node_name := 'hadev_arm64', dsn := 'host=<arm64-mesh-ip> dbname=netbird user=replicator password=...');
SELECT spock.repset_add_all_tables('default', ARRAY['public']);
SELECT spock.sub_create(
  subscription_name := 'sub_from_amd64',
  provider_dsn := 'host=<amd64-mesh-ip> dbname=netbird user=replicator password=...',
  replication_sets := ARRAY['default']
);

-- Back on amd64 node:
SELECT spock.sub_create(
  subscription_name := 'sub_from_arm64',
  provider_dsn := 'host=<arm64-mesh-ip> dbname=netbird user=replicator password=...',
  replication_sets := ARRAY['default']
);
```

Validate on each node:

```sql
SELECT subscription_name, status FROM spock.subscription;
-- Expected: 'replicating' on both sides.
```

Write on one node, read on the other:

```sql
-- On amd64:
INSERT INTO peers (id, name, ...) VALUES (...);

-- On arm64 (within seconds):
SELECT * FROM peers WHERE id = '...';  -- should return the row
```

**Red flag:** subscription in `down` or `error` state; rows don't appear on the peer within ~10s.

### 1.4 Cross-arch correctness

Reverse the write direction. Write on `hadev_arm64` (the Graviton/arm64 node), read on `hadev_amd64` (x86). Verify binary-column and integer-heavy Netbird tables (`group_peers`, `peers`, `events`) serialize/deserialize correctly.

**Red flag:** replication halts with decode errors; numeric values differ; timestamps off by a cast.

### 1.5 Island-mode simulation

Block network between the two instances via security-group edit:

```bash
# Remove the arm64 SG's egress allowance to amd64 EIP (or ingress — either direction breaks replication)
# Easier: just stop spock.subscription temporarily:
# On amd64:
SELECT spock.sub_disable('sub_from_arm64');
# On arm64:
SELECT spock.sub_disable('sub_from_amd64');
```

Write on both sides concurrently. Let WAL accumulate for ~5 min. Re-enable:

```sql
SELECT spock.sub_enable('sub_from_arm64');
SELECT spock.sub_enable('sub_from_amd64');
```

Observe:
- Replication catches up (`replay_lag` shrinks to 0)
- Conflicts logged in Postgres log — not a SQL table in Spock 5.0.6. Check with `docker logs postgres-a 2>&1 | grep 'CONFLICT:'` on each node. Loser-side logs `Resolution: apply_remote`, winner-side logs `Resolution: keep_local`. Default resolution is `last_update_wins` (verify via `SHOW spock.conflict_resolution;`).
- No rows permanently lost (check row counts post-reconcile match the sum)

**Red flag:** replication stays `down` after re-enable; row counts diverge; conflict log grows unbounded.

### 1.6 WAL slot invalidation + `pg_basebackup` rebuild

Set `max_slot_wal_keep_size = 1GB` and generate enough write traffic to invalidate a slot (fastest: disable one subscription, write many rows, watch slot grow past 1GB).

When the slot is invalidated:

```sql
SELECT slot_name, active, wal_status FROM pg_replication_slots;
-- Look for wal_status='lost'
```

Rebuild the returning node:

```bash
# On the node that was offline:
docker compose stop postgres
rm -rf /var/lib/postgresql/data/*
pg_basebackup -h <primary-mesh-ip> -U replicator -D /var/lib/postgresql/data -Fp -Xs -P -R
docker compose start postgres
```

Re-subscribe via Spock. Confirm replication resumes.

**Red flag:** basebackup fails; Spock can't re-subscribe; data diverges silently.

### 1.7 Signal federation

With both `signal-server` instances running (one per node), enroll a test peer against each. Attempt cross-stack P2P:

- Peer A enrolled against `hadev_amd64` signal
- Peer B enrolled against `hadev_arm64` signal
- `netbird status` on both should show them as mesh peers
- `ping` between them should succeed (direct P2P, relayed, or via signal-forwarding — all three valid)

**Red flag:** cross-stack peers cannot discover each other; `signal-server` logs show forward failures.

---

## Wave 2 — REQUIRES patched netbird-mgmt image

Gated on unblocking CI or building locally. Covers the remaining validation item:

### 2.1 Coordinated-upgrade playbook exercise

Prerequisite: a patched `netbird-mgmt:patched-v0.68.3` image available (via GHCR once CI is unblocked, OR built locally via `docker buildx build -t ghcr.io/usareur-af/netbird-mgmt:patched-v0.68.3 ops/netbird/` + `docker push`).

Simulated upgrade flow from vN to vN+1 (pin a specific commit SHA as "vN+1" for the test):

```bash
cd ops/ansible
OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES aws-vault exec legacy --no-session -- \
  ansible-playbook \
    -i inventory/hosts.ini -i inventory/mgmt-hosts.ini \
    coordinated-upgrade.yml \
    -e target_image=ghcr.io/usareur-af/netbird-mgmt:patched-53b04e5 \
    -e migrator_host=mgmt-host-hadev-amd64
```

(Image tag matches the CI short-SHA format `patched-<7char>`; use whichever tag `.github/workflows/netbird-mgmt.yml` actually published — check GHCR to confirm.)

Expected phases:
- Phase 1: both nodes stopped.
- Phase 2: image pulled everywhere; `NETBIRD_MGMT_IMAGE` set in each site's `.env`.
- Phase 3: marker touched on `mgmt-host-hadev-amd64`; it starts on the patched image; the server accepts TCP on :8080 (its readiness signal — no HTTP /health endpoint exists).
- Phase 4: `spock.sub_show_status()` shows all subscriptions `replicating`; `docker logs postgres-a | grep CONFLICT:` returns 0 for the last 5 min.
- Phase 5: the second site's marker is touched; the second site starts and migrates its own database (AutoMigrate changes nothing when the schema is already current); marker removed; its `spock.sub_show_status()` shows no subscription `down` or in error.
- Phase 6: final verification; no recent `CONFLICT:` lines in the postgres log.

**Red flag:** any Phase fails; conflicts appear in the postgres log during/after; replication stops.

---

## Wave 2 findings (run 2026-04-20)

Full end-to-end pass after five pre-run fixes landed: UTF8 DSN (21e0a06), postgres-a mesh binding (052379e), coordinated-upgrade Spock-5.0.6 API corrections (21e0a06), coordinated-upgrade image-swap via `.env` + var-recursion fix (ac798ca), and the HTTP `/api/health` → TCP `wait_for` readiness probe (8ac3360).

| Phase | Result |
| --- | --- |
| 1 stop everywhere | ✅ both nodes stopped in ~3 s |
| 2 pull + env swap | ✅ `patched-53b04e5` pulled on both, `NETBIRD_MGMT_IMAGE` written to each `.env` |
| 3 migrator start | ✅ amd64 started on patched image, TCP :8080 reachable within the retry budget, marker cleaned |
| 4 Spock health | ✅ `sub_from_arm64` `replicating`; 0 CONFLICT lines in postgres log last 5 min |
| 5 non-migrator start (serial) | ✅ arm64 started on patched image (SKIP_MIGRATIONS suppressed its AutoMigrate); TCP :8080 reachable |
| 6 final verification | ✅ Spock still replicating on both sides; 0 CONFLICT lines in last 10 min |

Post-run state confirmed on both nodes: `management-a = ghcr.io/usareur-af/netbird-mgmt:patched-53b04e5`, `postgres-a` healthy, `spock_test` row count unchanged at 16 each side (no data movement during upgrade — expected since the "upgrade" is effectively the same code with the skip-migrations patch).

### §2.1 operational notes captured during the run

- **Netbird management has no HTTP health endpoint.** v0.68.3 (and our patched fork) 404s every HTTP path (`/`, `/api`, `/metrics`, `/health`, `/api/health`, `/livez`, `/readyz`, `/api/v1/health`). Readiness is "TCP :8080 listens and doesn't immediately close" — which is what `ansible.builtin.wait_for state: started` measures. Any future health-based alerting must speak gRPC (management implements `grpc.health.v1`) or probe the TCP listener directly.
- **Spock 5.0.6 has no SQL-queryable conflict/subscription tables with the names the earlier playbook assumed.** `spock.subscription` the *table* exists but its columns are `sub_name` / `sub_enabled`, not `subscription_name` / `status`. Use `spock.sub_show_status()` the function for the expected column names. `spock.conflict_log` the table doesn't exist at all; filter `docker logs postgres-a` for `CONFLICT:` instead. Both noted in the findings from §1.5.
- **Image-swap via compose regex rewrites is fragile.** The initial playbook tried to `lineinfile` the `image:` line in `docker-compose.yml` with a regex that required `netbird-mgmt` in the current image name. A first-time upgrade from the upstream stock `netbirdio/management` didn't match and the swap silently no-op'd, so the migrator restarted on the OLD binary and the "upgrade" ran invisibly. Replaced with an `.env` write of `NETBIRD_MGMT_IMAGE=<target>`; the compose's `${NETBIRD_MGMT_IMAGE:-netbirdio/management:0.68.3}` default-var pattern handles the rest, idempotent across any starting image.
- **Ansible `vars:` self-reference is a template recursion trap.** `vars:\n  x: "{{ x | default(...) }}"` triggers "Recursive loop detected in template" on resolution. Inline `default()` on the task-level keyword instead. Not Spock-specific — this is the general Ansible pattern to remember.
- **Patched binary reports `version = "development"`.** Our Dockerfile doesn't stamp an ldflags version tag, so `INFO running gRPC server` logs version "development". Cosmetic today; before Phase 3 we should stamp the patched ref so `netbird status` on peers shows a recognizable server version.
- **The coordinated-upgrade playbook is safe to re-run.** Phase 1 stops are idempotent; Phase 2 `docker pull` + `lineinfile` are idempotent; Phase 3+5 start-then-wait handle the "already started" case via `docker compose up -d` (creates iff missing, reuses otherwise). This matters because real-world rollouts will have retries and partial-completion resumes.

---

## Wave 3 — Gate 2 extensions (risk-driven deep tests)

Extends Waves 1 + 2 with the deeper probes required before Gate 2 in `../design/ha-multi-site-validation.md` §7. Each step maps to one load-bearing assumption (see that doc §1). Some steps depend on pgBackRest (debt item 4) and observability (debt item 5) being live — marked **[obs]** and **[bkp]** below; everything else runs on the same EC2 pair as Waves 1–2.

### 3.1 72h real-ops soak (extends 1.3, 1.4)

Proves Spock replicates the actual GORM write pattern, not just toy `INSERT`s.

Drive a load generator against both management REST APIs for 72h. Per 15-min tick, distributed across both sites:
- 1 peer enroll (rotating setup keys)
- 1 setup-key create + 1 delete
- 1 group modify (once per hour)
- 1 policy modify (once per 4h)
- 1 route modify (once per 8h)
- Deliberate conflicts at T+24h and T+48h: same policy row hit from both sites within 100ms

At T+72h, diff every replicated table across both sites:

```bash
for t in accounts users peers groups group_peers policies policy_rules \
         setup_keys personal_access_tokens network_addresses nameservers \
         dns_settings routes installations; do
  docker exec postgres-a psql -U postgres netbird -c \
    "SELECT '$t' AS tbl, count(*), md5(string_agg(t::text, '' ORDER BY 1)) FROM $t t;"
done
# Repeat on postgres-b; diff the outputs.
```

**Pass:** row counts equal on both sides for every replicated table; MD5s equal; only unexplained divergence is `spock.resolutions` entries matching the two injected conflicts.

**Red flag:** any other divergence; replication halt during the 72h; manual PG intervention needed.

### 3.2 24h network-level island-mode with client actions (extends 1.5)

Wave 1.5 uses `sub_disable` (application-layer). This step cuts the network and performs client actions during the blackout.

```bash
# Block all traffic between instances at the SG level. From a non-test admin shell:
# In the AWS console: remove peer-SG ingress on both sides.
# Or via iptables inside each instance:
sudo iptables -I INPUT -s <peer-eip> -j DROP
sudo iptables -I OUTPUT -d <peer-eip> -j DROP
```

During the 24h blackout:
- Enroll a new Netbird client against each management server using a local setup key.
- Modify one policy at each site.
- Establish a new WireGuard tunnel between two peers at the same site (intra-site signal must work).

Restore connectivity. Verify:

```sql
SELECT subscription_name, status FROM spock.subscription;
SELECT * FROM spock.resolutions;  -- expect entries for the policy conflict
SELECT count(*) FROM peers;       -- compare both sides; new peers present on both
```

**Pass:** both new clients visible from both management servers within 5 min; policy LWW applied; `spock.resolutions` alert fires **[obs]**; intra-site tunnels held throughout.

**Red flag:** client enrollment fails during blackout; new intra-site P2P fails during blackout; row counts diverge beyond the known conflicts.

### 3.3 Slot-rebuild runbook + second-operator drill (extends 1.6)

Wave 1.6 proved the mechanics. Step 3.3 proves the mechanics are **usable under stress**.

- Use [`docs/operations/post-partition-convergence-runbook.md`](post-partition-convergence-runbook.md) §A "Slot rebuild" — the rebuild sequence + diagnostic queries are codified there.
- An operator *other than the author* executes the rebuild cold — no notes beyond the runbook.
- Time the rebuild end-to-end.

Extrapolate the rebuild time to the field link's bandwidth. If the estimate exceeds 12h, design mitigation (incremental base backup, physical-media pre-stage, shorter partition tolerance) before Gate 2.

**Red flag:** runbook requires ad-hoc improvisation; rebuild substantially longer than expected without clear cause; any data divergence post-rebuild.

### 3.4 Signal federation deadline + partial-failure load (extends 1.7)

**Prerequisite:** `signal-server` has an explicit `Forward` deadline in code. Proposed: 500ms total, 200ms per peer. If absent, file an issue and fix before this step.

Three signal servers (two on EC2 pair + one additional on a third instance). Load generator issues ~100 `Send`/sec with random target keys across all three registered-peer populations.

Scenarios (run sequentially, record p50/p99 Send latency + error rate + time-to-recover):
1. Kill one signal server (`docker kill signal-server-<target>`). Measure impact on the other two.
2. Inject 2s latency into one server's Forward responses (`tc qdisc add dev eth0 root netem delay 2000ms`).
3. Restart the killed server; verify clients re-register and Forwards resume.

**Pass:** p99 Send latency stays bounded by `Forward deadline + local lookup time`; no sustained blocking; clean recovery.

**Red flag:** Send hangs past deadline; one bad peer degrades signaling at other sites.

### 3.5 Adversarial coordinated upgrade (extends 2.1)

Wave 2.1 is the happy path. Step 3.5 proves the procedure fails **loudly** when done wrong.

For each misuse, run the upgrade and record what happens:
1. Skip the "all replicas caught up" precheck (start with known replication lag).
2. Start both sites' new-version containers simultaneously.
3. Pull the new image on only one site (image version mismatch).
4. Start the new version on the non-migrator site first.

**Pass:** each misuse is detected either by playbook checks or by immediate visible errors (column-not-found, replication halt, alert fire); recovery procedure works.

**Red flag:** any misuse produces silent data loss; any misuse goes undetected until long after.

After 3.5, file playbook improvements for any class of misuse that wasn't caught — add preflight + postcheck assertions to [`coordinated-upgrade.yml`](../../ops/ansible/coordinated-upgrade.yml).

### 3.6 Observability alert fire-tests **[obs][bkp]**

**Prerequisite:** Prometheus + Alertmanager + pgBackRest all deployed on the EC2 pair.

Deliberately induce each condition; verify the alert fires to the configured channel within SLO:

| Alert | Inducement | Fire SLO |
|---|---|---|
| Replication lag >60s | `tc qdisc add dev eth0 root netem delay 500ms 100ms` | <2 min |
| Slot WAL retained >80% of `max_slot_wal_keep_size` | 24h partition + synthetic WAL load | <5 min |
| `spock.resolutions` row appears | Conflict from §3.1 | <1 min |
| `spock.conflict_log` row appears | Conflict from §3.1 | <1 min |
| Clock skew >5s | `timedatectl set-ntp false; date -s '5 minutes ago'` | <1 min |
| Signal federation miss rate >10% / 1 min | Partition federation peer | <2 min |
| PG container down | `docker kill postgres-a` | <1 min |
| Disk >80% | `fallocate -l 4G /var/lib/postgresql/data/filler` | <5 min |
| Backup age >25h | Pause pgBackRest cron for 26h | <1 hour |

**Pass:** every alert fires within SLO.

**Red flag:** any alert fails to fire, or fires in a way operators cannot distinguish from noise.

### 3.7 Rollback-to-streaming drill **[bkp]**

Execute the streaming-replication fallback from `../design/ha-multi-site.md` §17 end-to-end on a **third throwaway EC2 pair** (do not disturb the Spock cluster — this is a fresh-build exercise, and the output is the written procedure itself).

Steps (to be authored as part of this drill — that's the point):
1. Tear down Spock on both fresh nodes.
2. Configure physical streaming replication (primary + standby).
3. Promote standby manually, validate failover.
4. Verify Netbird management runs against either node.

**Pass:** drill completes in one working day; the written procedure is tested end-to-end.

**Red flag:** any step requires ad-hoc invention; drill exceeds one day. Either means the fallback isn't real — block Gate 2 until resolved.

### 3.8 Zenoh cross-site federation over Netbird mesh

**Proves:** tactical data fabric (Zenoh router-to-router) works across distinct mgmt-plane sites, using the Netbird mesh as transport. Retires the open "does Zenoh federation work on top of our Netbird federation" question without committing to a production topology yet. Gate for Phase 3B (DDS bridge + CDS conditioning per `../design/zenoh-data-fabric.md`).


**Prerequisite:** hadev pair still up (it is, per current terraform tag `TeardownBy=Wave-3-validation-complete`). §3.1 soak completion not required — Zenoh containers don't touch postgres-a / signal-a / management-a.

**Setup:**

1. Add a `docker-compose.zenoh.yml` overlay to `ops/compose/mgmt-host/` that adds a single service `zenoh-router-a` running upstream `eclipse/zenoh:1.9.0` pinned by content digest per [ADR 0858](../adr/0858-zenoh-image-upstream-pivot.md). The rocksdb backend `.so` is downloaded by the Ansible role from the matching upstream `zenoh-backend-rocksdb` 1.9.0 release and bind-mounted via the host plugin dir — same pattern as `ops/ansible/roles/zenoh/`. Bind `:7447` on `0.0.0.0` (intra-host SG gate only). Mesh-IP dial for the PEER router.
2. Render a small `zenoh-config.json5` from a Jinja template with each host's OWN mesh IP for `listen.endpoints` + the PEER's mesh IP in `connect.endpoints`. Router-to-router peer topology, one-hop federation.
3. Deploy to both hadev hosts via SSM (source-push + docker compose up — same pattern as `ops/wave3/scripts/start-soak.sh`).
4. Verify `zenohd` process listening on 7447 each side; verify peer establishes (log line `New peer link` or similar).

**Test:**

```bash
# On hadev_amd64 (publisher):
zenoh-publish demo/site/test "hello from amd64 at $(date -u +%s)"

# On hadev_arm64 (subscriber — started FIRST):
zenoh-subscribe 'demo/**' | tee /tmp/sub.log
```

The publish binary + subscribe binary are in `signal-server/cmd/zenoh-probe/` (to be added; small Go program using the upstream `zenoh-go` bindings) OR use the Zenoh CLI shipped in the container (`zenohd` image's `zenoh-publish` / `zenoh-subscribe`).

**Success signals:**

- Subscriber receives publisher's message within `< 500 ms p99` across the full Netbird mesh hop (cross-region (notional)).
- Router `/status` endpoint reports the peer link as `Connected` on both sides.
- Killing one router: subscriber sees connection close; restarting: reconnects automatically.
- `docker stats` on each host shows Zenoh containers staying under 50 MB RAM at steady state.

**Red flag:** messages drop silently; peer link flaps; Zenoh router RAM unbounded growth.

**Pass:** one full publish cycle delivers cross-site under SLO + peer-failure recovery clean. Documents that federated-Zenoh-on-federated-Netbird is a real capability, unblocking Phase 3B + future site-expansion plans.

**Not in scope:** DDS bridge, CDS conditioning interceptors, Rust plugins. Phase 3B.

### 3.9 Steady-state contract — shared health assertion

**Proves:** every Wave 3 scenario that claims PASS leaves the mesh in a state any *other* Wave 3 scenario would recognize as healthy. Catches the class of failure where a scenario's bespoke checks succeed while the underlying system has regressed in a way the scenario doesn't look for (e.g. §3.4 recovery bands pass but Spock subscriptions have fallen behind; §3.2 reconcile succeeds but federation counters have stopped moving).

**Not a separate test run.** `ops/wave3/steady-state.sh` is a shared assertion wired into the other §3.x scripts at the end of their choreography. It checks three things, each tied to a load-bearing assumption from `../design/ha-multi-site-validation.md` §1:

| Check | Load-bearing assumption | Signal |
| --- | --- | --- |
| Spock `sub_show_status()` = `replicating` on both hadev hosts | §1.1 Spock replicates Netbird's real schema, both directions | DB-layer federation live |
| Peer-count parity across sites within `PEER_TOLERANCE` rows | §1.3 Island-mode reconciles; §1.6 Slot rebuild converges | Application-state federation live |
| `signal_server_forward_{sent,received}_total{outcome=delivered}` > 0 on both sides | §1.5 Signal federation fails partial, not total | Signal-layer federation live |

Single JSON verdict on stdout, exit 0/1. Wired into `stop-partition.sh` (§3.2) and `scenario-3-recovery.sh` (§3.4). Add to any new §3.x script via one line after the scenario-specific checks:

```bash
"$(cd "$(dirname "$0")/.." && pwd)/steady-state.sh" > scenario-X-steady-state.json || exit 1
```

`PEER_TOLERANCE` is zero by default; set it to the known number of in-flight enrollments when the scenario intentionally creates rows that may not yet have converged.

**Why not broader.** A four-check version would also look at WAL lag in bytes and clock skew. Deliberately not included: WAL bytes diverge transiently during normal catch-up, and NTP health is already covered by `ops/ansible/roles/ntp-hygiene/` alerts. Three checks is the minimum that catches the silent-divergence class the scenarios are each individually blind to.

---

## Teardown

When validation is complete (findings captured in the doc), destroy:

```bash
cd terraform
aws-vault exec legacy --no-session -- terraform destroy \
  -target=aws_instance.hadev_amd64 \
  -target=aws_instance.hadev_arm64 \
  -target=aws_eip.hadev_amd64 \
  -target=aws_eip.hadev_arm64 \
  -target=aws_security_group.hadev_amd64 \
  -target=aws_security_group.hadev_arm64 \
  -target=aws_iam_instance_profile.hadev_ssm \
  -target=aws_iam_role_policy_attachment.hadev_ssm \
  -target=aws_iam_role.hadev_ssm
```

Record findings in `docs/design/ha-multi-site.md` §5.8 (update the "open items" list) before destroying.

## Findings log (update as we go)

| Date | Test | Result | Notes |
|---|---|---|---|
| _pending_ | _(fill in as tests run)_ | | |

---

## Gotchas encountered during Wave 1 bring-up (2026-04-20)

Recorded for future re-deployment. None were architecturally blocking; all had clear fixes.

### 1. `management-a` read-only-mount + missing DataStoreEncryptionKey

Netbird management generates a `DataStoreEncryptionKey` on first boot and tries to persist it back to `/etc/netbird/management.json`. Our compose mounts that file **read-only** (`:ro`), so the write fails and the container enters a crash loop.

**Fix:** pre-supply the key in `management.json`. `ops/ansible/templates/management.json.j2` now has a required `netbird_encryption_key` var (32-byte base64 from `openssl rand -base64 32`). Same key must be used on every node of a multi-master cluster — encrypted setup-keys/tokens in Postgres can only be decrypted with the shared key. Stashed in `.envrc.local` as `HADEV_NETBIRD_ENCRYPTION_KEY`.

### 2. `postgres-a` pg_hba.conf doesn't allow `postgres` user from docker bridge

The baked pg_hba.conf only has entries for a `netbird` role that the init scripts don't actually create. Meanwhile our compose DSN uses `user=postgres` (Phase 1-2 pattern per §5.5 in the design doc). Result: every management-a connection attempt → `FATAL: no pg_hba.conf entry for host "172.18.0.4", user "postgres"`.

**Fix:** add `host all postgres 0.0.0.0/0 scram-sha-256` to `ops/postgres/spock/conf/pg_hba.conf` (committed). For already-running containers, apply live with `docker exec -u root postgres-a bash -c 'echo ... >> /etc/postgresql/pg_hba.conf'` then `pg_reload_conf()`.

### 3. `docker exec` drops to the container's USER by default

`docker exec postgres-a bash -c 'echo ... >> /etc/postgresql/pg_hba.conf'` fails with permission denied because the postgres container's USER is `postgres`, which can't write to root-owned files. **Fix:** `docker exec -u root postgres-a ...`.

### 4. GHCR packages must be public for anonymous pull

Docker pull from `ghcr.io/usareur-af/*` fails with "unauthorized" until the package visibility is set to Public in the GitHub UI: `https://github.com/users/goat-maintainer/packages/container/<name>/settings` → Change visibility → Public. We hit this for both `postgres-spock` and `signal-server`.

Alternative: plumb a GHCR read token via Ansible + `docker login ghcr.io` before pull. Simpler to just make packages public since the repo is public anyway.

### 5. Ansible `become: true` required on Ubuntu EC2 hosts

`ops/ansible/mgmt-host.yml` originally set `become: false` (the original Orin pilot OS had the bringup user in the `docker` group by default). Ubuntu EC2 hosts' default `ubuntu` user needs `sudo` for `/opt/mgmt-host` creation. **Fix:** `become: true` at the play level. Docker commands still work — root has all group memberships. (Under the new Ubuntu-for-Tegra 22.04 target for Orin — see [`../design/orin-os-decision.md`](../design/orin-os-decision.md) — the Orin behaves the same as Ubuntu EC2 here, so `become: true` is the correct setting everywhere.)

### 6. `netbirdio/dex` replace directive requires `CGO_ENABLED=1` in CI

Netbird's `go.mod` has `replace github.com/dexidp/dex => github.com/netbirdio/dex v0.244.0`. The fork has a `File` field on `storage/sql.SQLite3` that upstream `dexidp/dex` doesn't. With `CGO_ENABLED=0` in a cross-compile buildx path, Go doesn't resolve the replace directive cleanly and fails with "unknown field File".

**Fix:** `.github/workflows/netbird-mgmt.yml` restructured into matrix strategy (amd64 on `ubuntu-24.04`, arm64 on `ubuntu-24.04-arm`) with native per-arch `docker build --platform linux/<arch>`, CGO=1, Ubuntu final stage (glibc). A final `merge` job uses `docker buildx imagetools create` to combine into a multi-arch tag. Pattern: [docs.docker.com/build/ci/github-actions/multi-platform](https://docs.docker.com/build/ci/github-actions/multi-platform/).

### 7. Netbird mesh IP retrieval

`netbird status --json` shows peers (others), NOT the node's own IP. Use `netbird status | grep "NetBird IP"` OR `ip -4 addr show wt0 | grep -oP '(?<=inet )[0-9.]+'`. We grab it post-enrollment via SSM send-command to populate `.envrc.local` (`HADEV_AMD64_MESH_IP`, `HADEV_ARM64_MESH_IP`).

### 8. Upstream PR scope inflation via force-rebased base

An upstream PR's initial diff appeared to include commits from other upstream contributors that were actually already on `main` — just at different SHAs (upstream force-rebased after tagging v0.68.3). GitHub's PR diff is commit-based: if the commit SHAs in the PR branch aren't in the target branch's reachable set (even if equivalent content is), they show up as "added" lines.

**Fix:** `git rebase upstream/main` our feature branch, `git push --force-with-lease`. PR auto-updated to clean one-commit diff.

---

## Wave 1 findings (run 2026-04-20)

First actual Wave 1 execution against `goat-ha-dev`. Results below supersede the "Not yet run" placeholders in the per-step sections above.

| Step | Result |
|---|---|
| 1.1 Cluster bring-up | ✅ postgres-a + signal-a + management-a Up on both hosts after the 8 gotcha fixes in the section above |
| 1.2 pgEdge image audit | ✅ Spock 5.0.6 available; **Snowflake 2.4 IS bundled** (closes the ha-multi-site §5.8 open item — can use the `-minimal` tag for centralized-events case too if we want it); only TCP 5432 EXPOSEd; 893 MB disk / 266 MB content; no `ps` in image (minimal); no observable telemetry |
| 1.3 Spock bidirectional replication | ✅ mutual `replicating` state between hadev_amd64 ↔ hadev_arm64 with `default_insert_only` repset. **INSERT on amd64 → appeared on arm64 in ~6 s; INSERT on arm64 → appeared on amd64 in ~6 s.** |
| 1.4 Cross-arch correctness | ✅ amd64 (x86) ↔ arm64 (Graviton) replication works end-to-end with no decode errors, correct values, correct timestamps |
| 1.5 Island-mode simulation | ✅ Disabled both subs, dual-wrote with 2 shared PKs (amd64 at T0, arm64 at T0+17s) + private rows each side, re-enabled. Convergence in <15 s. Both nodes identical: 7 rows, shared PKs resolved to arm64's payload (later `ts` wins). Conflicts logged with symmetric LWW — amd64 log: `Resolution: apply_remote` ×2; arm64 log: `Resolution: keep_local` ×2. Zero row loss. |
| 1.6 WAL slot invalidation + basebackup | ✅ Simulated a "slot-lost" arm64 by dropping the sub pair (same end-state from amd64's perspective), wrote 10 new rows on amd64 during the rebuild window, ran `pg_basebackup` from amd64 → fresh arm64 volume via one-shot container through the Netbird mesh (48 MB / ~20s over WireGuard), restarted arm64, dropped inherited `hadev_amd64` node identity, recreated as `hadev_arm64`, recreated subs on both sides. Both nodes converged to 16 rows (5 baseline + 10 rebuild-window + 1 post-rebuild); both subs `replicating`. **Exposed a critical compose bug: pgdata-a was mounted at `/var/lib/postgresql/data` but pgedge image uses `/var/lib/pgsql/16/data` — postgres state was ephemeral all along.** Fixed in 2031804 (compose path) + 3c82c22 (pg_hba baked entries). |
| 1.7 Signal federation | ✅ First run required wiring `--federation-peers` into `signal-a` command and moving the gRPC port binding off 127.0.0.1 (294da3c). After that, a new `cmd/federation-probe` binary registered a fake peer key on arm64's `signal-a` via `ConnectStream`, issued a `Send` via amd64's `signal-a` targeting that key; amd64 missed locally, `Broadcast` via Forward RPC to arm64, arm64 delivered the message to the probe's receive stream. **End-to-end round-trip 182 ms (cross-region (notional), Netbird WireGuard)**. Metrics confirm: amd64 `signal_server_relay_total{outcome=forwarded}=1`, `signal_server_forward_sent_total{outcome=delivered}=1`; arm64 `signal_server_forward_received_total{outcome=delivered}=1`. |

### §1.3 / §1.4 operational notes captured during the run

- **Spock node DSN caching:** `spock.node_create` stores a DSN that's later used for provider-to-subscriber callbacks. If the replicator password is changed after node_create, the stored DSN becomes stale and `sub_create` fails with `password authentication failed` — **even though the immediate provider_dsn passed to sub_create is correct and hand-probed works fine**. Fix: drop the Spock node and recreate with the current password in the DSN, then recreate subscriptions. Documented here so the next operator doesn't chase the same red herring.
- **Replication user password stability:** generate the replicator password once and stash it persistently in `.envrc.local` as `HADEV_REPL_PWD`. Loop-regenerating it per host creates mismatches (happened during this run — each ALTER USER set a different password, auth failed).
- **pg_hba.conf entries needed for Spock:** in addition to the Phase 1-2 superuser-access-anywhere line (gotcha #1), add `host all replicator 0.0.0.0/0 scram-sha-256` AND `host replication replicator 0.0.0.0/0 scram-sha-256`. Both are needed — the first for regular queries (spock.node_info, schema introspection); the second for the replication slot protocol itself.
- **Replicator role needs explicit `GRANT USAGE ON SCHEMA spock`** and `GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA spock`. Without these, the provider-side spock.node_info() call from the subscriber returns `permission denied for schema spock`.
- **Replication set membership is not auto-updated for new tables.** `spock.repset_add_all_tables('default_insert_only', ARRAY['public'])` adds tables that exist AT CALL TIME. Tables created later (e.g., `spock_test` for this validation) need `spock.repset_add_table(set_name := 'default_insert_only', relation := 'spock_test'::regclass)` specifically. Worth codifying in the Ansible Spock setup when we get there — some kind of "re-run repset_add_all_tables on every playbook apply" pattern.
- **Quoting in `spock.repset_add_table`:** the `set_name` and `relation` args are string-typed. Passing them as double-quoted strings causes Postgres to interpret them as column names and fail with "column does not exist". Use single quotes or named-arg syntax (`set_name := '...'::text, relation := '...'::regclass`).
- **Cross-mesh TCP path validation:** `bash -c '</dev/tcp/<peer-mesh-ip>/5432'` works — we see the TCP connection succeed, verifying the Netbird WireGuard tunnel transports Spock traffic correctly. Useful pre-flight before attempting `spock.sub_create`.

### §1.5 operational notes captured during the run

- **No `spock.conflict_log` table in Spock 5.0.6.** The runbook (and a lot of older pgEdge blog posts) assume a queryable `spock.conflict_log` or `spock.conflict_log_summary` table. In 5.0.6 those don't exist. Conflicts go to the PostgreSQL log file via `spock.conflict_log_level = log` — filter with `docker logs postgres-a 2>&1 | grep 'CONFLICT:'`. Alerting in Phase 3 will need a log-scraping rule (Promtail/Loki or `pg_log_errors`), not a SQL-based exporter.
- **Conflict log message format is symmetric and informative.** Example: `CONFLICT: remote INSERT EXISTS on relation public.spock_test (local index spock_test_pkey). Resolution: apply_remote.` The loser-side node logs `apply_remote` (accepts the incoming row) and the winner-side logs `keep_local` (discards the incoming). With `spock.conflict_resolution = last_update_wins` (the default in this image), "winner" means higher `ts`/commit timestamp.
- **Default conflict resolution is LWW out of the box.** `SHOW spock.conflict_resolution;` returns `last_update_wins` without any explicit config in our compose. That's the mode we rely on for the HA design — worth verifying after any image bump.
- **Convergence time post-reconnect was ≈12–15 s** after `spock.sub_enable()` on both sides for a test with ~6 rows (3 per side, 2 of them in conflict). Consistent with the single-INSERT latencies in §1.3. Expect linear scaling with WAL volume during the partition; §3.2 will extend this with 24h of accumulated writes.

### §1.6 operational notes captured during the run

- **CRITICAL: pgedge image uses Red Hat PGDATA path.** Our compose mounted `pgdata-a:/var/lib/postgresql/data` (the Debian convention used by the Docker Official postgres image). The pgedge image uses `/var/lib/pgsql/16/data` instead, so the mount was a no-op — all postgres state was actually stored on ephemeral container-layer storage. Every `docker compose down` or container restart silently destroyed the cluster. Discovered when pg_basebackup filled the "volume" but the restarted container saw its real PGDATA empty and re-`initdb`'d from the image init scripts. Fixed in 2031804 (compose) — both `docker-compose.yml` and `docker-compose.phase2.yml` now mount to `/var/lib/pgsql/16/data`. **Pilot hardware/Orin deployments need to verify this fix is in before any production data lands.**
- **`pg_hba.conf` patches that were "committed" per Gotcha #2 were never actually baked into the image template** — only runtime-patched. Fixed in 3c82c22 which adds `host all postgres`, `host all replicator`, and `host replication replicator` rules to `ops/postgres/spock/conf/pg_hba.conf`. Broad CIDR is acceptable because mesh-IP scoping is enforced by the Netbird overlay + EC2/Orin SG rules; pg_hba is defense-in-depth. The `host all postgres` rule is flagged for tightening before Phase 3 once management runs as a least-privilege role.
- **pg_basebackup through the Netbird mesh works.** 48 MB cluster copy completed in ~20 s from amd64 region (notional) to arm64 region (notional) over WireGuard. No tuning, no tunnel tricks. This is the concrete proof that Spock-rebuild-after-slot-loss is operationally viable once bandwidth numbers are measured.
- **pg_basebackup one-shot container recipe:** `docker run --rm --network=host --user 0:0 -e PGPASSWORD=... -v <volume>:/var/lib/pgsql/16/data:rw <pgedge-image> bash -c 'chown -R postgres:postgres /var/lib/pgsql/16/data && su postgres -c "pg_basebackup -h <provider-mesh-ip> -U replicator -D /var/lib/pgsql/16/data -Fp -Xs -P"'`. The `--user 0:0` is required because the pgedge image starts as `postgres` by default, which can't chown a freshly-created empty volume. `--network=host` exposes the `wt0` Netbird interface inside the container.
- **Dropping the provider-side sub first is required** when tearing down a peer for rebuild — the logical replication slot on the provider is "active" as long as the consumer sub exists, even if the consumer is unreachable. Drop the peer's sub first (or on the provider side), then the orphan slot cleans up automatically. Skipping this step leaves the slot accumulating WAL on the provider forever.
- **After pg_basebackup, the restored cluster inherits the provider's Spock node identity.** In our case arm64 came up thinking it was `hadev_amd64`. `spock.node_drop('hadev_amd64')` + `spock.node_create(node_name := 'hadev_arm64', ...)` to re-identify before creating subscriptions. Mark this step in the runbook explicitly — a fresh operator would not guess it.
- **Restored container needs the pg_hba runtime patch (today).** The image still ships without the `host all postgres` / replicator rules (until the next image build consumes 3c82c22), so after starting a fresh postgres-a from the basebackup we had to re-patch `/etc/postgresql/pg_hba.conf` in the container. Goes away after the next image rebuild.

### §1.7 operational notes captured during the run

- **Signal-server federation wasn't wired before this run.** The stock compose passed `--grpc-addr=:10000` and `--metrics-addr=:9090` but no `--federation-peers=...`, so each `signal-a` ran standalone — peers enrolled at one site could not reach peers at another regardless of the Netbird mesh being up. Fixed in 294da3c: added `FEDERATION_PEERS` env var (.env-sourced per site) → `--federation-peers=${FEDERATION_PEERS}`.
- **Port 10000 was bound to 127.0.0.1.** This silently blocked cross-site federation even with `--federation-peers` configured, because the peer couldn't open the gRPC connection. Fixed in 294da3c: bind `10000:10000` (0.0.0.0). SG rules keep the port off the public internet; Netbird mesh is the intended audience. `/metrics` stays on 127.0.0.1 for host-local Prometheus scrapes.
- **Live federation probe is `signal-server/cmd/federation-probe`.** Run as `go run ./cmd/federation-probe --peer-a=<sender> --peer-b=<receiver>`. Exit 0 = Peer A missed locally, Broadcast to Peer B via Forward RPC, Peer B delivered. Keep this in the runbook for any future signal-server change — it's the canonical runtime check that federation still works end-to-end on live mesh.
- **Observed cross-site latency: 182 ms (p50, n=1)** amd64 region (notional) → arm64 region (notional) via Netbird WireGuard. Consistent with the ~100-200 ms intra-region inter-EC2 budget we expected. The §3.4 load test will surface p99 and the Forward deadline behavior; the `Broadcast` code path today has no per-peer timeout — note this as a finding for `ops/signal-server` hardening before Phase 3.
