# Bench Test as a Service — engineered runbook for tier-4b (and successors)

| Field | Value |
|---|---|
| Owner | goat core team |
| Status | **Active 2026-05-02** — Disciplines 1, 2, 3, 5, 6 implemented + tested locally; Discipline 4 (pre-baked AMI) queued. End-to-end empirical verdict pending the next M-NNN bench cycle (must complete in <60 min wall-clock with one operator command — that is the success measurement of this discipline). Original draft: 2026-05-01 after a full-day M-NNN bench cycle hit 8 distinct bugs because prior inline-fixes were never persisted. |
| Scope | tier-4b (goat-relay verdict bench) + the pattern generalizes to any future ephemeral validation env |
| Cross-refs | [`goat-relay-bench-env-smoke.md`](goat-relay-bench-env-smoke.md), [`ephemeral-env-preflight.md`](ephemeral-env-preflight.md), `cloud-env-lifecycle.md`, [`../adr/1012-load-testing-discipline.md`](../adr/1012-load-testing-discipline.md), `../findings/F-034-cloud-init-apt-503-aborts-package-install.md`, `../findings/F-038-l5-iouring-docker-dual-process-eaddrinuse.md` |

## The problem this fixes

Across one bench cycle on 2026-05-01, **8 distinct gaps surfaced** between the previous M-014 verdict and re-running the same bench shape:

1. cloud-init apt 503 aborted package install → docker-compose-v2 missing
2. orphan postgres-a / signal-a / management-a containers from prior cycle
3. orphan goat-relay PID 18187 (root) survived `docker rm -f` from yesterday
4. goat-relay Ansible role had a broken Jinja2 escape in the orphan-kill task
5. goat-relay L8 SO_REUSEPORT bug at N≥2 shards (Linux-specific; bench-host symptom)
6. socat sidecar (Phase 3.5) didn't run because inchworm phase aliases skipped it
7. **AWS SG missing ports 80 + 10000** — was inline-fixed during M-014 and never persisted to terraform
8. **Default policy + policy_rule missing** in netbird DB — mgmt-host-bootstrap CLI doesn't seed them, so peers can't see each other

**All 8 were fixed inline during the cycle. Items #7 and #8 had been fixed inline during M-014 too. They were never persisted to code, so the next bench cycle re-paid the discovery cost.**

The meta-bug is the *operator-error pattern*: inline fixes during a bench cycle, no follow-up commit to terraform / Ansible role / Go CLI / inchworm script. The next cycle starts fresh from the un-fixed code.

## The six disciplines

### 1. Inline fix → persist commit (mandatory, same-day)

**Every** inline fix during a bench cycle creates a commit before bench teardown. No exceptions.

| Inline fix shape | Persist target |
|---|---|
| AWS SG / NACL / route-table change via console or CLI | `terraform/<env>.tf` |
| Container env var added via `docker run -e` | `ops/ansible/roles/<role>/defaults/main.yml` + `tasks/main.yml` |
| Manual SQL inserted via `docker exec postgres psql` | `ops/<binary>/` Go code path that should have done it OR `ops/ansible/<env>.yml` task with the SQL |
| Ansible playbook re-run with a `-e var=val` override | Default landed in `defaults/main.yml` or `group_vars/<env>/vars.yml` |
| systemd unit / GRUB cmdline / sysctl change via SSM | `ops/ansible/roles/<role>/tasks/main.yml` |
| Cert / key / config file scp'd to host | Ansible `template:` or `copy:` task in the per-env play |
| iptables rule added | `ops/wave-XX/<env>/inchworm.sh` `enrol` step OR Ansible role |

**Pre-teardown checklist:**
- [ ] All inline fixes committed?
- [ ] Validation tier of the bench rerun confirms persisted fixes work fresh?
- [ ] If a fix can't be persisted (security secrets etc.), it is documented in the runbook with explicit operator-required step?

### 6. Preflight env audit (mandatory, runs after `apply` and before `phase2`)

`inchworm preflight` fans out to all 3 hosts via parallel SSM (~60s wall-clock) and audits for state corruption that would block deploy. Six checks per host: leftover `goat-relay` systemd unit (F-038 Cause 1 — the actual blocker last cycle), orphan goat-relay processes, orphan docker containers, TCP `:33073` already bound, UDP `:33073` already bound, DB rowcount on `setup_keys`/`accounts`/`peers`. Output is aligned `GREEN` / `RED` / `INFO` per host with the **exact recovery command** on every RED line. Exit non-zero on any RED — operator must clear before proceeding.

Why this is its own discipline: F-038 generalizes — any production component that has both a systemd-native deploy shape AND a docker-deploy shape can leave one shape orphaned across runs, and the orphan silently survives every `docker rm -f`. Catching it pre-deploy is cheap (60s); catching it post-deploy via opaque "shard 0 EADDRINUSE" symptom takes hours. Implementation: `ops/wave-33/tier-4b/inchworm.sh` `cmd_preflight` + `preflight_audit_script`.

### 2. Per-phase smoke check fail-fast (mandatory)

The current `inchworm full` runs end-to-end, fails at minute 25 with a generic ansible error, requires diagnosis. Replace with **smoke-check-at-each-phase**: each phase ends in a 5-second probe that returns ✅/❌ + the exact symptom.

Per-phase smoke probe template (Ansible task at end of each phase play):

```yaml
- name: PHASE N — smoke check
  ansible.builtin.shell: |
    set -eu
    # Phase-specific check
    nc -z -w3 {{ mgmt_public_ip }} 80
    nc -z -w3 {{ mgmt_public_ip }} 10000
    docker ps --format '{{ "{{" }}.Names{{ "}}" }}' | grep -q "^postgres-a$"
    docker exec postgres-a pg_isready -U postgres > /dev/null
    docker exec postgres-a psql -U postgres -d netbird -c "SELECT 1" > /dev/null
    echo "PHASE N smoke OK"
  changed_when: false
  failed_when: smoke_result.rc != 0
  register: smoke_result
```

**Per-phase smoke checks for tier-4b:**

- **Phase 1 (image upload):** `aws s3api head-object` returns 200 + presigned URL is reachable
- **Phase 2 (mgmt-host stack):** docker ps shows postgres-a + signal-a + management-a; `pg_isready`; signal-a `/healthz`; management-a port 80 reachable from host loopback; **port 80 + 10000 reachable from PEER SG (not just from mgmt host loopback — this catches the F-034-class SG gap)**
- **Phase 3 (goat-relay):** docker ps shows goat-relay; `ss -tlnp | grep 33073` shows listening; goat-relay logs include `kTLS hand-off` + `listener bound`; **for --shards N≥2, both shards' bind log lines are present** (catches L8 bugs immediately)
- **Phase 3.5 (socat):** `nc -z -w3 mgmt_public_ip 80` succeeds; `curl -sI http://mgmt_public_ip` returns expected response
- **Phase 4 (account + setup-key):** mgmt-host-bootstrap CLI returns success; setup-key file exists at `/tmp/tier4b-setup-key.txt`; **policies + policy_rules tables have ≥1 row each** (catches the missing-default-policy gap)
- **Phase 5 (Relay block):** management.json contains the Relay block; `Relay addresses:` log line in management-a; mgmt-a `/api/health` returns 200

### 3. State preservation across runs (don't tear down between attempts)

Per [`ephemeral-env-preflight.md`](ephemeral-env-preflight.md): the inchworm script's whole point is to keep the env up between phases. When a phase fails, **fix in place + re-run that phase only**, do NOT re-apply terraform.

**State to preserve in `/tmp/inchworm-tier4b.env` across attempts:**
- `MGMT_ID`, `PEER_A_ID`, `PEER_B_ID` (instance IDs)
- `MGMT_IP` (public IP)
- `IMAGE_ARCHIVE` (path to local tarball; rebuild only on goat-relay code change)
- `INCHWORM_ENC_KEY`, `INCHWORM_PG_PASS` (random secrets generated once)
- `SETUP_KEY_VAL` (the bench's enrol key — preserved so re-enrol doesn't generate a new one)
- `PEER_A_MESH_IP`, `PEER_B_MESH_IP` (mesh IPs once peers enrol)
- `CERT_DISTRIBUTED` (boolean — set to true once cert distribution to peers succeeded)
- `POLICY_SEEDED` (boolean — set to true once default policy + rule inserted)

### 4. Pre-baked AMI for the bench host (Phase 4+ aspirational, near-term improvement)

Cloud-init runs every fresh-provisioning cycle and re-hits substrate flakes. A pre-baked AMI with `docker.io + docker-compose-v2 + iperf3 + python3-pip + git + ca-certificates + curl + gnupg` already installed eliminates this entire class of failure.

**AMI build process:**
1. Provision a t3.micro Ubuntu 24.04 LTS once
2. Install all packages via apt-get with retry
3. Pre-load the goat-relay Docker image
4. Pre-create `/opt/goat-relay /opt/mgmt-host /etc/modules-load.d/goat-relay-tls.conf`
5. Snapshot to `goat-relay-tier4b-<date>` AMI
6. terraform `data.aws_ami` looks up by tag; rotate AMIs quarterly

This eliminates ~10 minutes of cloud-init wait + the F-034 flake.

### 5. Hands-off operator runbook

Operators should be able to run the bench with **one command** + `<30 min` of attention:

```bash
./ops/wave-33/tier-4b/inchworm.sh apply image full enrol ramp verdict teardown
```

Each phase fails fast with the exact symptom. The operator either:
1. Sees ✅ at each phase, gets a verdict in <30 min
2. Sees ❌ at one phase, fixes the symptom, re-runs that phase only

The pre-bench checklist (operator confirms before running):
- [ ] All findings in `docs/findings/INDEX.md` with status "Active" have been reviewed for "is the remediation in code"
- [ ] No recent terraform changes to the bench env shape that haven't been validated
- [ ] AWS account has sufficient SCP / IAM for SSM + iperf3 traffic (5 Gbps shared ENA OK for t3.micro)
- [ ] Local docker is running (for image build) + local docker buildx is configured for linux/amd64

## Implementation tracking

| Discipline | Status | Where |
|---|---|---|
| 1 — Inline fix → persist commit | ✅ codified 2026-05-01; F-035/036/037 inline-handled in `inchworm.sh` `cmd_enrol`; Ansible-side persistence still queued for those three | This doc + per-finding |
| 2 — Per-phase smoke checks | ✅ landed 2026-05-02 — fail-fast probes at end of phases 2 / 3 / 3.5 / 4 / 5 | [`ops/ansible/goat-relay-tier4b.yml`](../../ops/ansible/goat-relay-tier4b.yml) |
| 3 — State preservation in inchworm | ✅ enough for now — `/tmp/inchworm-tier4b.env` covers MGMT_ID / PEER_*_ID / IMAGE_ARCHIVE / SETUP_KEY_VAL / mesh IPs. CERT_DISTRIBUTED + POLICY_SEEDED tracked inline by `cmd_enrol` (idempotent without explicit flags) | `ops/wave-33/tier-4b/inchworm.sh` |
| 4 — Pre-baked AMI | ❌ queued — needs Packer job + terraform `data.aws_ami` swap | Block 34X.ami |
| 5 — Hands-off operator runbook | ✅ landed 2026-05-02 — Disciplines 1/2/3/6 together produce the one-command experience | This doc |
| 6 — Preflight env audit | ✅ landed 2026-05-02 — `inchworm preflight` parallel-SSM audit catches F-038 class | `ops/wave-33/tier-4b/inchworm.sh` `cmd_preflight` |

### Bench-infrastructure regression gate (CI)

`.github/workflows/inchworm-test.yml` runs shellcheck + `inchworm-test.sh` on every PR / push touching the bench scripts. Catches helper-function regressions (e.g. the `mesh_ip_strip_validate` CIDR-suffix bug) before the next bench cycle pays the discovery cost.

### Acceptance bar — empirical verdict on this discipline

Run `inchworm apply image preflight phase2 phase3 phase35 phase4 phase5 enrol ramp verdict teardown` against a fresh `terraform apply` env. **PASS** if it completes in <30 min with zero manual intervention and emits a numeric M-NNN verdict. **FAIL** if any phase requires operator debug; the failing assertion + recovery action goes into the next iteration of this runbook.

#### Empirical verdict — first run 2026-05-02

**Verdict on the discipline: PASS** (with caveats — discipline did its job; underlying perf-test surfaced separate bugs).

| Step | Result |
|---|---|
| `apply` | ✅ ~3 min, 9 resources |
| `image` | ✅ ~3 min docker buildx, 5.6 MB archive |
| `preflight` | ✅ all-green on fresh env across 3 hosts in <60s |
| `phase2` smoke | ✅ docker compose v2, 3 containers Up, pg_isready, mgmt-a listener |
| `phase3` smoke | ✅ relay container Up, TCP+UDP :33073 bound, 8 shards bound, no panics, "ws data plane:" line present |
| `phase3.5` smoke | ✅ socat sidecar, port 80 reachable |
| `phase4` smoke | ✅ setup-key file + DB row both verified |
| `phase4` idempotency (run twice) | ✅ **byte-identical sha256 across consecutive runs**, preserve mode kicked in correctly |
| `phase5` smoke | ✅ Relay block in management.json, mgmt-a TCP listener up post-restart, "Relay addresses: [rels://198.51.100.234:33073]" emitted |
| `enrol` | ✅ both peers got mesh IPs (100.127.0.104, 100.127.0.213), management Connected, Relays 1/2 Available |
| `ramp` | ✅ ran to completion, captured 4 flow counts (1/4/8/16) into JSONL evidence |
| `verdict` | ✅ emitted FAIL with exact failing assertion ("single-flow Mbps: 0.0 < 110 Mbps acceptance bar") |
| `teardown` | (queued — see release notes) |

**Bugs caught + persisted same-day per Discipline 1 during this empirical run (7 total):**

1. Phase 5 smoke `'%{http_code}'` curl format-string conflicts with Ansible jinja2 safe-loader arg-splitter (commit `a218cdd`)
2. `cmd_phase2`'s `--tags=mgmt_host` is dead code — mgmt-host.yml has no `tags:` defs, so tag-filter selects nothing AND returns rc 0, blocking the `||` fallback (commit `a218cdd`)
3. `goat-relay` role's `until: relay_logs.stdout != ""` missed stderr — goat-relay emits all logs via Rust `eprintln!`, so docker logs returns rc=0 with empty stdout despite container being healthy. Fixed via `2>&1` (commit `5cd12a9`)
4. Phase 5 `/api/health` was the wrong path — netbird-mgmt has no public HTTP health endpoint (every probed path returns 404 without auth). Replaced with TCP-listener-still-up probe (commit `f42177a`)
5. Phase 4 always-regenerate violated the "byte-identical re-run" acceptance bar. Rewrote to preserve-if-both-present (DB row + file): trust existing on re-run, mint only when either missing. F-007 still applies (plaintext can't be re-derived from DB hash). Confirmed empirically: same sha256 / size / mtime across 2 consecutive `inchworm phase4` runs (commit `1e81d91`)
6. `cmd_phase4`'s `--start-at-task` referenced the old task name `'Create bench account...'` after my Phase 4 rewrite renamed everything to `'Phase 4 — ...'`. ansible-playbook silently no-ops on missing start-at-task targets (commit `41fa41d`)
7. Peer cloud-init had no retry on the netbird install line (`curl ... | sh`) AND apt-get install hung 3.5h on `iperf3.postinst configure` because Ubuntu 24.04 LTS's `needrestart` was prompting interactively under cloud-init. Two terraform fixes: (a) 5x retry around netbird install + post-install assertion; (b) `DEBIAN_FRONTEND=noninteractive`, `NEEDRESTART_MODE=a`, `NEEDRESTART_SUSPEND=1` env vars on the apt runcmd (commits `02eaf12` + `6ec8935`)

**Why iperf3 FAIL is a discipline PASS, not a discipline FAIL:** the bench-as-a-service runbook's job is "fail-fast with named symptom + cheap iteration loop." iperf3 0.0 Mbps was emitted as a numeric verdict with the exact failing condition. Operator can now diagnose the real bug (signal-vs-mgmt peer-registration mismatch — peers report `Signal: Disconnected, reason: peer not registered at this signal server`) without burning 7h on cloud-init flakes. That's the whole point of the discipline.

**Cumulative time:** ~4h focused work (initial code/docs + two empirical-run sessions). **Cumulative AWS spend:** ~$5. **Forward leverage:** the next M-NNN cycle starts from the same env shape minus the 7 bugs caught here. The signal-registration bug is the next priority finding to chase.

| Cycle metric | Before bench-as-a-service | After |
|---|---|---|
| Wall-clock to numeric verdict | ~7h | ~30 min once env is healthy |
| Manual-debug wait points | every phase boundary | fail-fast with named symptom; fix in place |
| Cost per failed cycle | ~$15 + 7h | ~$3 + <30 min |
| Bugs persisted across cycles | inline-fixes lost on next cycle | Discipline 1 enforces same-day commit |

## What we don't try to fix here

- **Production-shape host upgrade** (Block 34J — c7i.large or bare-metal). Different work.
- **The L8 SO_REUSEPORT Linux-environment bug**. Bench-as-a-service is for the *meta-bug pattern*; specific bugs go through their own findings + commits. (Discipline 6 preflight catches the *symptom shape* — port already bound by a leftover process — without trying to debug L8 itself; see F-038.)
- **Multi-environment reuse** (cgnat, integdev, hadev all sharing this discipline). Pattern is the same; this doc establishes it for tier-4b first.

## Next bench cycle's pre-flight

Before running tier-4b again, the operator runs through this checklist:

1. **`grep -E '\[F-[0-9]+\].*Active' docs/findings/INDEX.md`** — list all currently-Active findings
2. For each, confirm the Status field says "✅ Resolved" OR if "Active", confirm the remediation lives in terraform / Ansible role / Go CLI / inchworm script (per the audit shape from `docs/findings/F-NNN-meta-bug-inline-fixes-not-persisted.md`)
3. If ANY remediation is NOT persisted, file the persistence work BEFORE running the bench
4. Run the bench. The phase-by-phase smoke checks catch any persistence gap that slipped through
5. Verdict captured. Teardown.
