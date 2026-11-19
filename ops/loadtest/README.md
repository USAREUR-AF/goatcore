# Zenoh fabric load-testing apparatus

Measures the goat Zenoh data fabric's throughput and persistence behaviour
against the **real** nova-a router, for extrapolating capacity. Design of record +
the pre-registered predictions this scores against:
[`docs/design/zenoh-loadtest.md`](../../docs/design/zenoh-loadtest.md) (ADR 1012).

## What it measures (three axes, not one number)

- **Locking** → expect ≈ 0. Put-latency stays flat while storage lag grows.
- **Latency** → higher than zenoh but bounded (rocksdb low-ms; MinIO ~2.5 s = the
  5 s batch window, by design). Unbounded growth = a lane saturating.
- **Throughput** → scalable per lane. The current single-lane drain rate is the
  *unit of scaling*; aggregate = per-lane × lanes, capped by the shared NVMe.

Plus, for each durable sink (rocksdb, MinIO): **complete copies?** (distinct
stored seq ÷ published seq) and **publish→storage delay?**

## Isolation + safety (read before running on nova-a)

A router host may carry the OS, Docker, rocksdb, and MinIO on **one filesystem** — a
runaway run can fill the disk and wedge the host. Guards:

- Load traffic lives only under the `loadtest/**` namespace; the leaf cert ACL is
  scoped to it (a wrong prefix fails closed at the router).
- Persistence sinks are dedicated and **whole-unit wipeable**: one rocksdb subdir
  (`/opt/docker-data/zenoh/loadtest/`) and one ephemeral, quota-capped MinIO
  bucket (`loadtest-ephemeral`) served by a *second* instance of the real
  audit-subscriber. The real `audit-integrity` bucket is never written.
- `run-loadtest.sh` refuses to start if free space (or the projected run size)
  would breach the disk floor, and a live watchdog aborts the run if it does.
- `wipe-loadtest.sh` is **allowlist-only, dry-run by default**, and structurally
  cannot delete `audit-integrity` / `telemetry-otlp` / `runtime-detection-archive`
  or any real vendor's rocksdb dir — see the asserts in `lib-loadtest.sh`.
- **Per-producer audit rate meter (GHSA-hq6c).** Every audit-subscriber meters
  each producer against 500 records/s and 128 MiB/min by default. It is
  observe-only unless `audit_subscriber_producer_quota_enforce` is true, in which
  case over-budget samples are dropped. The dedicated loadtest subscriber from
  `zenoh-loadtest.yml` runs with metering off. A run that sends load through the
  **production** subscriber (as M-052's 10→2000 msg/s sweep did) must first raise
  or zero `audit_subscriber_producer_quota_records` / `_bytes` for that goatnet
  and re-apply the audit-subscriber role, and must **restore the defaults and
  re-apply it again when the run ends**. Otherwise `AuditProducerOverRate` fires
  for `loadtest`, and on an enforcing goatnet the run measures the quota, not
  the fabric. A production subscriber left with metering off for 2 h fires
  `AuditProducerMeteringOff`.
- **Upgrading a host that already runs the loadtest subscriber.** Its compose
  project is now `audit-subscriber-loadtest` (it used to collide with the
  production `audit-subscriber` project). A container started before the
  upgrade still belongs to the old project, so the first
  `docker compose up -d` after the upgrade fails with a container-name
  conflict and the new `down` does not remove it. Remove it once by hand:
  `docker rm -f audit-subscriber-loadtest`, then re-run the play.

## Prerequisites

1. Build + publish the `goat-loadgen` image (`ops/zenoh-trial-peers/loadgen/`).
2. Provision the loadtest stack on nova-a:
   `ops/ansible/zenoh-loadtest.yml` (loadtest cert + `loadtest/**` ACL +
   rocksdb storage entry + loadtest audit-subscriber → ephemeral bucket; narrows
   the real audit-subscriber off `**`).
3. On nova-a: `mc` alias configured for the ephemeral bucket (the play sets this up
   as `loadtest-minio`), and `jq` present.

## Run

```
# single rate point
ops/loadtest/run-loadtest.sh --rate 100 --payload-bytes 100 --duration 60 --sessions 4

# rate sweep that brackets the 51 msg/s MinIO batch-timer crossover and climbs
# toward the drop knee (the durable-throughput capacity number)
ops/loadtest/run-loadtest.sh --rate-sweep 10,50,100,500,2000,10000 --duration 60 --sessions 4

# raw routing ceiling (unbounded; watchdog-gated, no projection possible)
ops/loadtest/run-loadtest.sh --rate 0 --duration 30 --sessions 8
```

Evidence is written to `${LOADTEST_OUT_DIR:-/opt/loadtest/out}/<date>-zenoh-fabric-load-<run>.jsonl`.

### Scaling check (turns a point into a slope)

Re-provision the ansible play with `--lanes 2` (two audit-subscriber shards + two
rocksdb storages over key sub-ranges) and re-run the same sweep. Aggregate per-lane
rate ≈ doubling confirms scalable throughput; sub-linear names the shared resource.

## Summarize

```
ops/loadtest/summarize.py <evidence.jsonl> --m-number 049
```

Writes `<evidence>.report.html` (self-contained, inline-SVG charts, verdict line
first — readable cold) and `<evidence>.M.md` (an M-NNN stub with a
predicted-vs-observed table). Fill the M-stub, commit it under `docs/measurements/`,
and RCA any row outside its predicted band (do not file numbers blindly).

## Wipe

```
ops/loadtest/wipe-loadtest.sh            # dry-run: shows exactly what would go + sizes
ops/loadtest/wipe-loadtest.sh --apply    # delete the loadtest rocksdb subdir + ephemeral bucket
```

## Files

| File | Role |
| --- | --- |
| `lib-loadtest.sh` | shared config + the safety-assert primitives (the wipe contract) |
| `run-loadtest.sh` | operating-range orchestrator: gates, watchdog, scrape loop, probes, evidence |
| `benchmark-ceiling.sh` | **reusable ceiling benchmark** (one command): stand up isolated sink → pause prod subscriber → rate-ladder sweep measuring rocksdb + MinIO **separately** → restore + teardown → rich HTML. First runs: M-052/M-053. |
| `rca-throughput-decomposition.sh` | **self-contained throughput RCA harness** (one command, no LLM, no prod impact): throwaway routers on a private network → zenoh `z_pub_thr`/`z_sub_thr` reference + fixed pipelined loadgen route-only + durable rocksdb sweep → JSONL + HTML. The reproducible apparatus behind M-054 (proves the ~7k "ceiling" was an un-pipelined-generator artifact; real route ~1.2M/s, durable ~32k/s). |
| `render-rca.py` | deterministic stdlib renderer for the RCA evidence → self-contained HTML (verdict + decomposition + durable curve + per-stage non-blocking scaling model + hypotheses). |
| `probe-rocksdb.sh` | rocksdb GET-back via REST: completeness + freshest-key lag |
| `probe-minio.sh` | MinIO bucket walk: completeness + publish→object lag |
| `summarize.py` | evidence → HTML report (inline SVG; operating-range + ceiling modes, hypotheses scored validated/rejected) |
| `wipe-loadtest.sh` | allowlist-only, dry-run-default teardown |

## Ceiling benchmark (reusable)

```
ops/loadtest/benchmark-ceiling.sh                  # default ladder 2k→20k→200k→unbounded
ops/loadtest/benchmark-ceiling.sh --ladder 2000:8:15,20000:16:12
```

Run on the router host. Idempotent; it stands up the isolated sink, pauses the
production audit-subscriber for the window, sweeps, then restores + tears down +
renders the rich HTML report (the end product). It honors the wipe contract
(only ever touches the dedicated loadtest subscriber + a `loadtest-*` bucket).
