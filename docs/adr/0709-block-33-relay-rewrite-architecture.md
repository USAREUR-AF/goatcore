# ADR 0709 — Block 33 netbird-relay rewrite architecture

| Field | Value |
|---|---|
| Status | Accepted (33A — wire-format library + conformance test surface; **33B kickoff amendment landed 2026-04-29 — D7–D13 firm**; **D3 deploy-shape amendment landed 2026-05-01 — systemd-native, not docker**). 33C / 33D land as separate amendments or supplemental ADRs as decisions firm up. |
| Date | 2026-04-29 (initial) · 2026-04-29 (33B kickoff amendment) · 2026-05-01 (D3 deploy-shape amendment) |
| Repo scope | goat core |
| Supersedes | — |
| Superseded by | — |
| Related | [`0110-license-posture-agplv3-forked-components.md`](0110-license-posture-agplv3-forked-components.md), [`0111-vendor-friendly-license-posture.md`](0111-vendor-friendly-license-posture.md), [`0700-chainguard-first.md`](0700-chainguard-first.md), [`0702-build-from-source-mgmt-plane.md`](0702-build-from-source-mgmt-plane.md), [`0707-rust-toolchain-pinning.md`](0707-rust-toolchain-pinning.md), [`1008-soak-testing-discipline.md`](1008-soak-testing-discipline.md), [`1011-findings-register.md`](1011-findings-register.md), [`1012-load-testing-discipline.md`](1012-load-testing-discipline.md), [`1248-goat-follows-stock-upstream-netbird-and-wireguard-and-retires-its-departures.md`](1248-goat-follows-stock-upstream-netbird-and-wireguard-and-retires-its-departures.md) |

## TL;DR (for a layperson)

We're rewriting the netbird relay's data plane in Rust. The relay is the
hop that mesh peers fall back to when they can't talk directly to each
other (which is most of the time on Satlink because of CGNAT). The
existing relay is written in Go and tops out at about 25-30 megabits per
second per pair of peers, even though the underlying hardware can do
gigabits — measurement evidence showed three quarters of
the relay's CPU time is spent inside the Go runtime's goroutine scheduler
and mutex synchronization, not on actual network work.

This ADR captures the architectural decisions we made for the rewrite:
- We reimplement the wire protocol from a clean spec extraction (no
  copying of upstream Go source) so the new Rust binary has no AGPL §13
  obligations attached to it.
- We split the work into a pure parse/serialize library crate
  (`goat-relay-protocol`) and a binary crate that does I/O (`goat-relay`,
  the 33B work).
- The library crate is runtime-agnostic; the binary crate picks tokio
  with `tokio-uring` (single-threaded runtime per pinned thread, with
  layered protections that make work-stealing structurally impossible),
  quinn for QUIC, tokio-tungstenite for WebSocket, and rustls + the
  `ktls` crate for in-kernel TLS hand-off.
- Conformance is enforced by a dedicated conformance binary
  (`goat-relay-conformance`) that round-trips wire bytes through our
  parser and serializer and asserts byte-equal output for a committed
  set of gold-file fixtures. The same binary runs at smoke (1 iteration)
  and validation (100 iterations) tiers per the soak-testing discipline.

The acceptance bar for the eventual production binary is **at least 3×
the throughput of the tuned Go relay** (the GOMAXPROCS=1 baseline that
landed in M-006), measured on the same VPS shape. That works out to
≥110 Mbps single-flow on a t3.micro relay.

## Context

### Why this work exists

Three intertwined measurement pieces, all from late April 2026, motivated
this rewrite:

- **M-001**:
  the netbird relay caps at ~32 Mbps single-flow on a t3.micro relay
  with t3.nano peers. Multi-flow degrades catastrophically (5 Mbps at
  16 streams) while CPU stays below 45%. Surfaced
  F-015
  — the `rels://` WebSocket transport exhibits classic TCP-over-TCP
  head-of-line-blocking.
- **M-002**:
  RCA disambiguation. Direct TCP between the same peer hosts achieves
  330 Mbps single-flow / 1.03 Gbps 4-parallel. So the cap is not the
  peer NIC, not the kernel TCP buffer, not Docker/docker-proxy. The
  relay binary itself caps at ~25 Mbps. Surfaced
  F-016
  — the binary supports both QUIC and WebSocket but every production
  daemon picks WS.
- **M-005**:
  strace -c on the running production relay during sustained 49 Mbps
  load. Result: 76% of process time in Go runtime overhead
  (futex 51.8% + nanosleep 24.3%); only 24% in actual I/O syscalls.
  Surfaced F-017
  — the bottleneck is structurally the Go runtime, not network I/O.

M-006 added
2026-04-29 found a free 25% improvement from setting `GOMAXPROCS=1` on
the relay container — eliminating inter-thread futex contention without
any code change. Production was tuned same-day, lifting single-flow from
~25 Mbps to ~36.6 Mbps. **This is the new apples-to-apples baseline**
for any rewrite-vs-Go comparison; otherwise the rewrite would get credit
for a one-line env-var change.

### Why a rewrite specifically

A rewrite is structurally indicated, not just nice-to-have:

- The 76% Go-runtime cost is **uniform across the binary regardless of
  transport**. quic-go runs in the same Go runtime with the same
  goroutine-scheduler + mutex-contention pattern. Forcing QUIC instead
  of WS (which would be a daemon-side config change, not a relay change)
  doesn't address the runtime cost.
- Per-syscall optimizations (kTLS, io_uring) target only the 24% I/O
  slice. Even if implemented inside the existing Go relay, they would
  leave the 76% slice untouched.
- A Rust binary has no goroutine scheduler, no GC, and no runtime futex
  pressure of the shape strace measured. kTLS + io_uring on top of that
  get the additional 24% I/O slice.

The structurally-cleanest fix is the Rust rewrite. Per-syscall
optimizations help less than runtime-layer changes do.

### Why the small team is doing this (vs handing off)

Per `vendor-handoff-pickup-plan.md`,
this work was scoped as a vendor-handoff candidate — bounded, measurable,
standard-engineering, ~4-7 weeks. The small team is shipping the first
cut to keep the validation surface honest and the operating discipline
intact; handoff to a larger org (Army Software Factory, FFRDC, or a
specialist Rust-systems consultancy) is available at any sub-block
boundary if scope or velocity demand it. The seven-artifact acceptance
gate (code, ADR, findings, soak driver, M-NNN measurements,
validation-state entry, runbook) applies equally to small-team and
recipient-org output.

## Decision

This ADR captures thirteen load-bearing architectural decisions for
Block 33. D1-D6 landed at 33A. D7-D13 firmed at 33B kickoff
(2026-04-29) — runtime, kTLS hand-off, QUIC + WS library choices,
RSS / cross-core distribution timing, workspace structure, and unsafe
code perimeter.

### D1 — Clean-room reimplementation, not patched fork

Block 33's Rust binary is a **clean-room reimplementation of the wire
protocol**, not a patched fork of upstream's Go source.

The wire protocol itself (message-type bytes, peer-ID format, transport
choices, handshake FSM) is **not copyrightable subject matter** under
Oracle v Google (US Supreme Court, 2021) and Sega v Accolade (9th Cir.,
1992) — protocols, APIs, and methods of operation are uncopyrightable.
We extract the spec from upstream source, document it in our own words
in `docs/design/netbird-relay-wire-protocol.md`, and implement against
the spec in Rust without transliterating code structure.

Practical hygiene during 33A's reading work:

- BSD-3-Clause subtrees first (`shared/relay/messages/`,
  `shared/relay/healthcheck/`, `shared/relay/tls/`) — permissive license,
  no clean-room burden, can be referenced freely. Wire-format byte
  layouts, magic constants, peer-ID format, keepalive timings all
  extracted from this side.
- AGPLv3 subtree (`relay/server/`) read second, with hygiene applied.
  Extract semantics in our own words: routing decisions, FSM transitions,
  validator interface contract, lifecycle. Do **not** transliterate Go
  code structure into Rust.
- The Rust crate's per-module doc comments name which upstream files
  were read and at what license tier — provenance is auditable.

Result: the new Rust binary has **no AGPL §13 obligations attached** —
it is not a derived work of `relay/server/` Go source. Patches against
the existing upstream `management/` binary remain governed by ADR 0110
unchanged.

### D2 — Apache-2.0 own code per project default

`goat-relay-protocol` and `goat-relay` (the 33B binary crate, when it
lands) are Apache-2.0 — project default for own code per
[ADR 0111](0111-vendor-friendly-license-posture.md). Transitive deps
must be Tier-1 to Tier-4 per ADR 0111 (Tier-5: SSPL / Commons Clause /
Elastic v2 / restrictive BSL / non-commercial custom — disqualified).
The protocol crate's current dep set (`bytes`, `thiserror`, `proptest`)
is Tier-1 (Apache/MIT). 33B's runtime + WS + QUIC + kTLS deps must be
checked against the same bar before merge.

### D3 — Process-boundary deployment, not in-process replacement

The new Rust binary runs as a separate process alongside the existing
Go relay during canary rollout. **Production deployment shape is
systemd-native, not docker** (see "D3 amendment 2026-05-01" below).
Rollback is a one-line `goat-relay.service` ExecStart flip + `systemctl
restart goat-relay`.

This pattern is consistent with ADR 0110's process-boundary discipline
for AGPL components. It also gives us:

- A clean A/B comparison surface (Go relay on one VPS, Rust on another,
  same DNS round-robin load).
- No need to "swap" code in any running daemon — daemons reconnect to
  whatever relay DNS round-robin selects on next handshake.
- Production-ready rollback path that's mechanical, not architectural.

#### D3 amendment 2026-05-01 — systemd-native deployment, not docker

**Decision:** `goat-relay` ships as a single static-linked binary +
systemd unit on the relay VPS, **not** as a containerized workload.
Distribution is via a versioned tarball (`.tar.gz` containing the
binary + the unit file + a per-environment `.env`); rollout is
`systemctl restart goat-relay` after a tarball swap. Rollback is the
inverse.

**Why the change.** Two findings from M-009 + M-014 made the docker
path operationally fragile for *this specific workload*:

- F-027:
  `tcp_available_ulp` is per-netns; the F-023 runtime probe inside a
  bridged container correctly reads "no tls" and silently falls back
  to userspace TLS — kTLS bypass invisibly disabled.
- F-023:
  Ubuntu 24.04 doesn't auto-load the `tls` module on boot; needs
  `/etc/modules-load.d/goat-relay-tls.conf`.

The combined invariant for kTLS to work in a docker shape is "host
module load + `network_mode: host` + bind mounts for cert/key + nonroot
user perms juggling for the Chainguard runtime." That's three layered
non-defaults. The systemd-native shape needs only one: the
`modules-load.d` snippet. Net surface area drops; failure modes drop;
rollback path stays the same.

**Why this doesn't generalize.** The mgmt-host stack stays on docker
compose per [ADR 0207](0207-docker-compose-orchestration.md) — that
workload doesn't touch kTLS, doesn't need `network_mode: host`, and
benefits from per-service netns isolation. **The carve-out is specific
to data-plane workloads that need host-netns access for kernel-bypass
optimizations** (kTLS now; AF_XDP / DPDK if Block 33C.1 L5 lands;
io_uring SQPOLL similar). The same carve-out applies at scale to the
future Tipper / replay-substrate / Layer-6 analytics paths if they
need data-plane host-netns access.

**Implementation.** Block 33D ships:

- `ops/goat-relay-systemd/goat-relay.service` (Type=simple, dynamic
  user, hardened `ProtectSystem=strict` + `NoNewPrivileges` + minimal
  AmbientCapabilities, host-netns by definition, restart-on-failure)
- `ops/ansible/roles/goat-relay-systemd/` (replaces the docker-tag-
  swap path; pulls tarball from S3 presigned URL or repo-side
  versioned object; verifies sha256; atomic symlink swap; `systemctl
  daemon-reload && systemctl restart goat-relay`)
- Existing `ops/goat-relay/Dockerfile` + `ops/ansible/roles/goat-relay/`
  retire after Block 33D production rollout completes; kept in tree
  for the `local-Linux validation harness` use case (`docker-compose.yml
  tester` profile) until L8 multi-shard work needs `network_mode: host`
  in the harness too.
- Tarball signing: rides the bootstrap-bundle Ed25519 + age pattern
  per [`../design/bootstrap-bundle.md`](../design/bootstrap-bundle.md)
  for cross-region distribution; on-host verification before unpack.

The bench-env path (tier 4a / 4b inchworm) **stays containerized**
for short-lived ephemeral envs because the operator burden of pulling
a fresh tarball per env burn is higher than the docker-tag-pull
ergonomics. The carve-out is "production-deploy = systemd-native;
ephemeral-bench = docker still OK" — same posture as the rest of the
project's container-orchestration discipline.

**Closes F-027**
as a structural concern (the finding stays Active for any future
container-shaped deploy that wants kTLS, but ceases to gate goat-relay
production posture).

### D4 — Library crate / binary crate split

`goat-relay-protocol` (lib crate) is **runtime-agnostic** — pure
parse / serialize / state-machine logic, no I/O, no async runtime, no
WS or QUIC dependencies. It is the test surface for the conformance
binary and the dependency for the 33B binary crate.

`goat-relay` (binary crate, 33B output) owns the I/O loop, the
WS / QUIC adapters, the routing table, the per-shard task model, and
the kTLS handoff. It depends on `goat-relay-protocol` for wire types.

Rationale:

- The library crate's API is testable without spinning up a network
  socket. Property tests, gold-file fixtures, conformance round-trips —
  all run under `cargo test` with no orchestration.
- The async-runtime decision (D7) is contained to the binary crate;
  swapping glommio for tokio-uring or vice versa doesn't ripple through
  the protocol library.
- Future Rust users of the wire format (e.g., a peer-side replacement,
  a monitoring tool that decodes captured pcap) can depend on the lib
  crate alone without pulling in the relay server's I/O machinery.

The conformance binary `goat-relay-conformance` lives in
`goat-relay-protocol/src/bin/` (lib + bin in one crate) — it depends
only on the lib's public API and exists primarily to validate that API.
33B's binary lives in a sibling workspace crate `goat-relay/` per D12.

### D5 — Wire-format conformance as a discrete, committed-fixture test surface

Block 33A's close-out criterion is byte-stable wire-format conformance,
not relay-server behavior. The conformance binary
`goat-relay-conformance` exposes two subcommands:

- `gen <dir>`: emit a hand-crafted set of gold-file `.bin` fixtures
  covering every active message type plus boundary cases.
- `check <dir>`: for each `.bin` file, parse via `Frame::parse`,
  re-serialize via `Frame::marshal`, assert byte-for-byte equal to the
  input. Emit one JSONL line per fixture; exit 0 iff every fixture
  round-trips.

The 17 hand-crafted fixtures are **committed to the repo** at
`ops/wave-33/relay-conformance/fixtures/`. Rationale: deterministic
third-party reproducibility, regression-watch baseline,
no-build-step-needed-to-test. `run.sh` regenerates from current code
into a tempdir and asserts byte-equal to the committed set — catching
silent wire-format drift in code changes.

This design is **a deliberate narrowing** of the original Block 33A
conception ("drive same packet stream into Go relay AND Rust relay;
diff outputs"). The original was premature because the Rust relay
binary doesn't yet exist (that's 33B). The narrower test gives
meaningful 33A signal — *the wire format is byte-stable* — without
needing 33B or the bench env. Real-traffic replay (against captured
pcap from the bench env) is reserved for a later canonical tier and
fires once 33B's first cut exists.

### D6 — Three-tier soak per ADR 1008

The conformance soak driver `ops/wave-33/relay-conformance/run.sh`
implements the three-tier shape from
[ADR 1008](1008-soak-testing-discipline.md):

| Tier | Iterations | Coverage | Use |
|---|---|---|---|
| smoke | 1 × 17 fixtures | Byte-stable round-trip per fixture | CI on every PR; pin-regression hook on dependency bumps |
| validation | 100 × 17 = 1700 | Determinism (no HashMap-iteration-order leak) | Default verdict gate before 33A → 33B transition |
| canonical | TBD × N captured fixtures | Real production-traffic forwarding equivalence | Pre-prod gate before 33C canary cutover; gated on 33B's Rust relay binary existing as a consumer |

Smoke and validation tiers are operational as of 2026-04-29 (1700/1700
ok, evidence at `docs/reports/2026-04-29-relay-conformance-soak.jsonl`).
Canonical tier is stubbed pending 33B.

### D7 — Async runtime: tokio + tokio-uring (firmed 33B kickoff 2026-04-29)

**Decision:** tokio with `tokio-uring`. Each shard runs as a pinned OS
thread executing its own `tokio::runtime::Builder::new_current_thread()`
runtime instance. No multi-threaded tokio runtime exists anywhere in the
binary; work-stealing is not possible at the runtime level because there
is no scheduler that could steal.

**Alternative considered (not chosen): glommio.** Architecturally
glommio's "tasks bound to executors at the type-system level" property
is the cleanest expression of shard-per-core. We chose tokio anyway
because the cascading library cost (s2n-quic over quinn,
async-tungstenite over tokio-tungstenite, manual setsockopt over the
`ktls` crate) outweighs the architectural elegance for a small-team,
small-codebase first cut. Tokio's ecosystem reach also makes the
eventual vendor-handoff candidate pool (per
`vendor-handoff-pickup-plan.md`)
materially larger.

**Mitigation — five layers, defense in depth.** The "loss" of glommio's
type-system enforcement is recovered through configuration + type-system
+ lint + review + runtime assertion:

| Layer | What it does | Bypass cost |
|---|---|---|
| **1. Runtime config** | `Builder::new_current_thread()` per pinned thread. No multi-threaded runtime exists in the binary. Work-stealing is **structurally impossible** at runtime, not just disabled. | Requires deliberately constructing a second multi-threaded runtime. Visible in any code review. |
| **2. Type system** | Shard-local state is `Rc<RefCell<RoutingTable>>`. `Rc` is `!Send`. The compiler **refuses to compile** code that tries to move shard-local state across threads. | Requires switching the type to `Arc<Mutex<...>>`, which is a semantic change visible in diffs and a violation of the architecture comment. |
| **3. Clippy lint** | `clippy::disallowed_methods` blocks `tokio::spawn` and `tokio::task::spawn` in the hot-path module's `clippy.toml`. Only `local_set.spawn_local()` permitted. CI fails if anyone introduces them. | Requires editing `clippy.toml`, also visible in diffs. |
| **4. Architecture comment** | Hot-path module preamble documents the invariant: "Tasks here run on one core for their lifetime. Shard-local state is `Rc<RefCell<...>>`. Spawn via `local_set.spawn_local`. See ADR 0709 D7." | Documentation. The reviewer's job. |
| **5. Boot-time assertion** | `assert_eq!(Handle::current().metrics().num_workers(), 1)` per shard runtime. Panics if misconfigured. | Catches accidental config drift even if all four layers above were bypassed. |

Layer 1 alone makes work-stealing impossible at runtime. Layer 2 alone
makes the bug class a compile error. Layers 3-5 are defense-in-depth.
For the bug to ship, an engineer would have to deliberately switch the
runtime to multi-threaded **and** change shard-local state from `Rc`
to `Arc<Mutex>` **and** edit `clippy.toml` **and** ignore the architecture
comment **and** survive review. That is structural enforcement at three
independent layers, not "discipline by policy."

**Migration cost if we pivot back to glommio later:** approximately 1-2
weeks. The wire-protocol crate is already runtime-agnostic
(`goat-relay-protocol/` has no I/O dependencies); only the binary
crate's I/O layer would need swapping. This is the trigger condition for
revisit at 33C if pressure-testing shows the discipline is not holding.

### D8 — kTLS handoff: rustls + the `ktls` crate (firmed 33B kickoff 2026-04-29)

Linux ≥5.10 supports kernel TLS (kTLS) for TLS 1.3. Production
relays run Ubuntu 24.04 with kernel 6.8 — kTLS is available. After the
rustls handshake completes, the binary hands the AEAD session keys to
the kernel; per-packet AEAD then runs in-kernel via `sendmsg`/`recvmsg`,
bypassing userspace TLS framing on the steady-state hot path.

**TLS library — rustls.** Apache-2.0, pure-Rust, exposes
`ConnectionTrafficSecrets` post-handshake. Tier-1 license, Tier-1 maturity.

**Hand-off mechanism — the `ktls` crate.** Apache-2.0 / MIT, tokio-shaped
(now compatible because D7 firmed tokio). Wraps the
`setsockopt(SOL_TLS, TLS_TX, ...)` + `TLS_RX` call pattern in a safe API
and consumes a `ConnectionTrafficSecrets` from rustls. Saves us writing
~30 lines of `unsafe` setsockopt code by hand and provides a
maintained, audited integration point.

**Fallback — manual setsockopt.** If `ktls` lacks something we need
(e.g., a specific cipher suite, kernel capability, or tokio-runtime
integration edge), we fall back to inline `setsockopt(SOL_TLS, ...)`
calls in our own `unsafe` block per D13. This is the only place in the
binary where we'd expect `unsafe` for kTLS reasons.

**Crypto provider — `ring` for 33B; `aws-lc-rs` migration deferred
(firmed step 2, 2026-04-29).** rustls supports two crypto providers:

- **`ring`** — pure-Rust, no C build deps, the rustls default. Tier-1
  license (ISC + MIT + OpenSSL — Apache-2.0-compatible per
  [SPDX guidance](https://spdx.org/licenses/)). What 33B step 2 ships.
- **`aws-lc-rs`** — BSD-3-Clause / Apache-2.0, FIPS-validated via the
  AWS-LC C library, requires a C toolchain + CMake at build time.
  Smaller community than ring but AWS-backed.

We choose `ring` for 33B step 2 to keep the Chainguard image build
chain simple (no C toolchain, no CMake, smaller image surface). The
project's load-bearing FIPS-validated crypto is anchored at the
**WireGuard outer tunnel** (and at the boundary node's strongSwan +
wolfSSL/AWS-LC stack per `fips-pqc-path.md`),
not the relay's inner TLS — which sits *inside* the WireGuard tunnel.
The relay's inner TLS is therefore not on the FIPS critical path
today; `ring` is consistent with the comparative-posture framing in
`comparative-posture.md`.

**`aws-lc-rs` migration is owed when any of the following fires:**

- The Block 19 boundary node ships and the relay's inner TLS becomes
  FIPS-relevant by extension (e.g., a partner declares the relay
  in-scope for their compliance assessment).
- A vendor-handoff recipient's compliance posture requires
  FIPS-validated crypto throughout the binary
  (`vendor-handoff-pickup-plan.md`).
- The `aws-lc-rs` build-time cost is measured in our Chainguard
  pipeline and found acceptable (no longer a build complication
  blocker).

The migration itself is small in the rustls hot path — rustls's
`CryptoProvider` abstraction lets a `default_provider` swap drop in —
but touches `Cargo.toml`, the Chainguard `Dockerfile`, and CI's
build-time-budget. Track in
`block-33-progress.md` decisions log
when triggered.

### D9 — QUIC library: quinn (firmed 33B kickoff 2026-04-29)

**Decision:** [`quinn`](https://crates.io/crates/quinn) (Apache-2.0 / MIT).

**Rationale:**

- Tokio-native — pairs cleanly with D7 (tokio) without the runtime-bridge
  work that s2n-quic or quiche would require.
- Most mature Rust QUIC implementation by production miles. Used by
  cloud-native projects across the ecosystem; large user base means
  more debugged edge cases.
- RFC 9221 unreliable datagrams supported via the `Connection::send_datagram`
  / `Connection::read_datagram` API. Verified during 33A spec extraction
  ([wire-protocol §2](../design/netbird-relay-wire-protocol.md)) that
  the netbird relay's QUIC path uses datagrams (not streams), so the
  required QUIC API surface is exactly what quinn exposes.
- Pluggable TLS backend — quinn can use rustls (consistent with D8) or
  a custom provider. We use rustls.

**Alternatives considered:**

- **s2n-quic** (AWS-backed, BSD-3-Clause via s2n-tls): runtime-flexible,
  FIPS-friendly via s2n-tls. Compelling if FIPS posture mattered at the
  binary level — but the relay sits inside the WireGuard tunnel
  (load-bearing crypto), so s2n-quic's FIPS angle is double-counting.
  Smaller community, fewer published debugging stories.
- **quiche** (Cloudflare): lower-level, more control, more work per byte.
  We don't need that control.

### D10 — WebSocket library: tokio-tungstenite (firmed 33B kickoff 2026-04-29)

**Decision:** [`tokio-tungstenite`](https://crates.io/crates/tokio-tungstenite)
(Apache-2.0 / MIT).

**Rationale:**

- Tokio-native — pairs cleanly with D7.
- Most-deployed Rust WS library by production miles.
- Supports binary frames, ping/pong, close codes — the entire surface
  area required by the netbird WS path per
  [wire-protocol §1](../design/netbird-relay-wire-protocol.md).
- WS framing is **not the hot-path bottleneck** per M-001/M-002/M-005.
  Picking the most ergonomic + most-deployed option, not the
  fastest-on-microbenchmarks option, is correct.

**Alternatives considered:**

- **fastwebsockets** (Cloudflare): faster on microbenchmarks but smaller
  user base. Speed difference is dwarfed by the workload's real
  characteristics (bulk data forwarding inside WS binary frames).
- **async-tungstenite** (runtime-agnostic via `futures` traits):
  necessary if we'd picked glommio at D7. We didn't.

### D11 — NIC RSS / cross-core distribution: deferred to 33C (firmed 33B kickoff 2026-04-29)

**Decision:** 33B's first cut targets the **single-flow ≥110 Mbps**
acceptance bar with **N=1 shard active**. Multi-shard infrastructure is
in place (per-thread runtimes, MPSC cross-shard rings, shard-id field on
the routing table) but the binary boots with a single shard.

**Rationale:**

- 33B's acceptance bar is single-flow per the M-006-tuned baseline. RSS
  + cross-core distribution does not affect single-flow numbers.
- M-001/M-002 evidence: t3.micro relay CPU never exceeded 33% under
  sustained load. We don't need cross-core distribution to land 33B's
  bar; the bar is hit on a single core.
- 33C is where multi-flow + many-peer scaling lives. RSS / eBPF
  redirect / `SO_REUSEPORT` load-balancing all live there, with bench
  validation against M-007's verdict on per-pair-isolated vs relay-wide
  bottleneck.

**33B preserves shard-per-core architecture from day one.** The binary
supports N shards (configurable; defaults to N=1) and the second-shard
path is exercised at least once in the test suite to prevent rot. 33C
wires RSS / SO_REUSEPORT / eBPF and runs the multi-shard validation.

#### D11 amendment 2026-05-01 — multi-shard SO_REUSEPORT bind landed (Block 33C.1 lever L8)

**Decision:** Multi-shard with `SO_REUSEPORT` is now wired in. The binary
defaults to `N = number of cores` shards, overridable via `--shards <n>`.
Each shard pins to its own core, runs its own current-thread tokio
runtime, and binds the WS+QUIC listeners with SO_REUSEPORT — the kernel
distributes incoming TCP accepts and UDP datagrams across the shards'
sockets by source-tuple hash.

**Routing-table-side change:** The `RoutingTable` Internals went from
`Rc<RefCell<...>>` (shard-local) to `Arc<RwLock<...>>` (cross-shard).
The hot path `forward()` takes the read lock, clones the
`mpsc::Sender<Bytes>` (cheap; tokio MPSC sender is `Arc`-y inside),
drops the lock, then `try_sends`. Lock-free under no register/
deregister contention. Register/deregister briefly take the write lock.

**Type-system invariant change (D7 layer 2 amendment):** `RoutingTable`
becomes `Send + Sync`. Per-connection task fields not in the routing
table (e.g., `Rc<RefCell<KeepaliveSender>>` from goat-relay-protocol)
remain `!Send`, so connection tasks themselves still cannot migrate
across threads. ADR 0709 D7 layers 1, 3, 4, 5 retain full strength;
layer 2 is loosened to "the routing-table-only object is Send+Sync;
connection-task fields not in the routing table preserve `!Send`."

**Implementation:**

- `goat-relay/src/routing.rs` — `Rc<RefCell>` → `Arc<RwLock>` refactor
  (commit `ada7968`); two new tests: `routing_table_is_send_and_sync`
  + `cross_shard_forward_via_arc_clone`.
- `goat-relay/src/listener/ws.rs` — bind via `tokio::net::TcpSocket`
  with `set_reuseaddr(true)` + `set_reuseport(true)` before
  `socket.bind()` + `socket.listen(1024)`.
- `goat-relay/src/listener/quic.rs` — bind via `socket2::Socket` with
  `SO_REUSEADDR` + `SO_REUSEPORT` (cfg-gated to unix), then hand the
  `std::net::UdpSocket` to `quinn::Endpoint::new()` with the tokio
  runtime. New dep `socket2 = "0.5"` with the `all` feature flag for
  `set_reuse_port`.
- `goat-relay/src/main.rs` — `--shards <n>` CLI flag (default = num
  cores; clamps to available cores); spawns N `Shard`s, each cloning
  the same `RoutingTable` and binding its own listener pair.

**CBPF (BPF program attached to the SO_REUSEPORT group via
`setsockopt(SO_ATTACH_REUSEPORT_CBPF, ...)`):** **deferred** to a
follow-up. Default kernel distribution is source-tuple hash, which is
adequate for a per-peer-one-connection workload. CBPF would let us
control distribution policy (e.g., consistent-hashing on dest peer-id
to land peer-pair coordination on the same shard, reducing cross-shard
register/deregister broadcasts). Land it if profiling shows
cross-shard subscription notification overhead matters.

**Validation:**

- 22/22 unit + integration tests pass; clippy clean.
- Both ignored multi-peer-pair concurrency tests
  (`ws_listener_multi_peer_pair_routing` /
  `quic_listener_multi_peer_pair_routing`) pass under release with
  the new `Arc<RwLock>` routing internals.
- Cross-shard reachability: `cross_shard_forward_via_arc_clone` test
  verifies register on one clone, forward from another → frame arrives.
- Production-shape verdict (single-flow + aggregate at N=cores) is
  Block 33D / 34J / Block 34K depending on which shape gets there
  first.

### D12 — Workspace structure: top-level Cargo.toml + sibling crates (firmed 33B kickoff 2026-04-29)

**Decision:** Cargo workspace at the repo root with two members:

```text
/Cargo.toml                  # [workspace] members = ["goat-relay-protocol", "goat-relay"]
/Cargo.lock                  # unified lockfile
/goat-relay-protocol/        # existing lib crate (untouched at 33B kickoff)
/goat-relay/                 # NEW binary crate, depends on goat-relay-protocol via path
```

**Rationale:**

- `goat-relay-protocol` stays exactly where it is; its `Cargo.toml` stays
  a crate manifest, not a workspace manifest. No churn to existing 33A
  artifacts (85 unit tests + 17 fixtures + conformance binary all keep
  working unchanged).
- `cargo test --workspace` runs the lib's 85 tests + the binary's tests
  in one command — drives `run.sh` regression-watch posture cheaply.
- Unified `Cargo.lock` means runtime + QUIC + WS + kTLS dep selections
  are visible in a single auditable file.
- Separate `target/` directory shared across both crates speeds
  incremental builds.

### D13 — Unsafe code perimeter (firmed 33B kickoff 2026-04-29)

**Decision:** Layered enforcement.

- **`goat-relay-protocol/Cargo.toml`** keeps `unsafe_code = "forbid"`
  (already there, lib crate). The protocol library has no business with
  `unsafe`.
- **`goat-relay/Cargo.toml`** sets `unsafe_code = "deny"`. Per-block
  `#[allow(unsafe_code)]` opt-ins permitted with mandatory `// SAFETY:`
  justification. `deny` (not `warn`) means accidental `unsafe` in a
  casual edit is a build error.
- **Anticipated unsafe sites** (subject to revision as 33B implements):
  kTLS `setsockopt` fallback path (if `ktls` crate doesn't cover what we
  need); raw socket buffer reuse for the in-place `peer_id` rewrite
  preserved as decision 12 in `block-33-progress.md`'s decisions log.
- **Pattern:** wrap `unsafe` in narrow safe wrappers. E.g.,
  `fn enable_ktls_tx(fd: RawFd, secrets: ConnectionTrafficSecrets) -> io::Result<()>`
  exposes only safe API; the caller never sees the raw `setsockopt`.
- **`// SAFETY:` per usage** documents the invariant. Reviewer must
  validate the invariant on every change to an `unsafe` block.

## Alternatives considered

### A1 — Patch upstream Go relay in place (kTLS + io_uring + shard-per-core)

Rejected. Would address the 24% I/O slice but leave the 76% Go-runtime
slice untouched. Per F-017's diagnosis, the structural ceiling stays
where it is. Would also extend the AGPLv3 fork-and-patch surface
significantly — every upstream Go change becomes a merge problem,
versus our current pattern of small surgical patches in
`ops/netbird/patches/` (per ADR 0110).

### A2 — Force daemon to use QUIC (config-level fix, no relay change)

Rejected as the *primary* fix; preserved as an operational watchlist
item. F-016 surfaced the 100% WS / 0% QUIC production split. Per spec
§2's headline finding, QUIC uses datagrams (not streams), so the F-015
TCP-over-TCP HOL pathology does not apply on the QUIC path. Forcing
QUIC could substantially improve multi-flow degradation in production
even before any rewrite work — but the F-017 Go-runtime overhead
remains the dominant ceiling cost regardless of transport. M-003 (force
QUIC and re-measure) is queued as an interim operational test;
findings will inform 33B's transport priorities.

### A3 — Replace netbird relay with Tailscale's DERP

Rejected. DERP is BSD-3-Clause and architecturally cleaner (UDP
datagrams, lock-free routing) — but it has its own peer-protocol
requirements that don't match netbird's. Adopting DERP would require
peer-side daemon changes ("bring DERP support to netbird"), which
breaks the Block 33 scope constraint of "don't change peer code."
Available as a deeper-rewrite escape valve if 33B's first cut shows
the wire protocol itself is the binding constraint, but not the default.

### A4 — Build a relay-side multiplexer that opens N connections per
peer-pair and load-balances across them

Rejected as a stand-alone fix. Adds protocol complexity (peer needs to
know to open N conns); brittle under partial failures; mostly addresses
multi-flow but not single-flow ceiling; doesn't reduce per-VPS cost
because each conn still rides Go-runtime overhead.

## Consequences

**Positive:**

- Wire-format conformance is enforced by a discrete, scriptable test
  surface that does not depend on the relay binary itself. Smoke runs
  on every PR; full-fleet upgrade requires re-running validation tier.
- Process-boundary deployment + canary rollout + tag-flip rollback gives
  us a low-risk path from 0 to 100% production traffic.
- Library / binary split keeps the protocol API testable in isolation
  and lets future Rust consumers of the wire format depend on a clean,
  small dependency.
- Clean-room hygiene means the Rust binary has no AGPL §13 strings on
  it. Distribution + redistribution + commercial licensing are all
  unencumbered by the AGPL component running next to it.
- The library crate's `unsafe_code = "forbid"` lint provides a strong
  safety baseline. If 33B's hot path needs unsafe (raw socket buffers,
  SIMD, kTLS setsockopt wrappers), it goes in the binary crate with
  explicit per-usage `// SAFETY:` justifications.

**Negative:**

- Two crates instead of one to maintain. Mitigated by the lib crate's
  smaller surface area and the conformance binary giving a regression
  baseline that catches drift cheaply.
- Async-runtime + QUIC + WS library choices firmed at 33B kickoff
  (D7-D10) commit the binary crate to the tokio ecosystem. The
  five-layer mitigation in D7 (runtime config + type system + clippy
  lint + architecture comment + boot-time assertion) recovers
  glommio's "shard-per-core by construction" property; pivot back to
  glommio costs ~1-2 weeks if pressure-testing later shows the
  discipline is not holding.
- The conformance binary uses a tiny ad-hoc JSON emitter rather than
  serde to keep the lib crate's dep tree minimal. If JSON output needs
  grow (richer structure, nested data, escaping edge cases), swap in
  serde. Today's surface is small and controlled.
- Fixture files committed to the repo (~30 KB total) — small but worth
  acknowledging. Regeneratable via `goat-relay-conformance gen`; the
  commit is a deterministic-reproducibility decision, not a design
  necessity.

## Assessor view (SCA-R)

The clean-room hygiene + provenance discipline (D1) is consistent with
NIST SP 800-160 vol. 1 supply-chain provenance expectations. The
conformance test surface (D5, D6) provides an audit-ready evidence
chain — JSONL output is timestamped, version-pinned (Cargo.lock for the
binary crate at 33B), and reproducible by anyone with the repo. The
process-boundary deployment (D3) means a security-relevant defect in
goat-relay does not implicate the existing Go relay; rollback is a tag
flip. The `unsafe_code = "forbid"` lint at the lib crate level
documents the safety perimeter explicitly.

## Trigger conditions for revisit

- **33B's first cut shows the wire protocol itself is the ceiling.**
  Action: revisit A3 (DERP-style replacement) — would require peer-side
  daemon changes, breaks current scope. Open a supplementary ADR.
- **rustls licensing or maintainership changes.** Action: re-evaluate
  D8 against `aws-lc-rs` (Apache-2.0, FIPS-validated, AWS-maintained).
- **AGPL posture on AGPL upstream changes.** Action: per ADR 0110's
  trigger conditions — re-evaluate the `relay/` reading hygiene if
  upstream relicenses to a stricter form.
- **Acceptance bar materially shifts.** M-006 already moved the bar
  from 75 Mbps to 110 Mbps. If a future M-NNN measurement (e.g., a
  bigger relay instance, M-003 force-QUIC verdict) shifts it again,
  amend the bar in 33D's validation table.
- **A vendor-handoff opportunity arrives** for 33B / 33C / 33D.
  Action: per `vendor-handoff-pickup-plan.md`, the seven-artifact
  acceptance gate applies equally to recipient orgs. The small team's
  33A foundation does not need to be renegotiated — it stays as-is.
- **Tokio's per-shard-isolation discipline (D7) does not hold under
  pressure-testing.** Symptoms: routing-table data race surfaced under
  load, observable cross-core task migration despite the 5-layer
  mitigation, or compilation of `Arc<Mutex>`-shaped shard-local state
  by accident. Action: pivot to glommio per the D7 escape valve
  (~1-2 weeks; wire-protocol crate stays runtime-agnostic so only the
  binary's I/O layer swaps).
- **`ktls` crate maintainership or compatibility breaks.** Action: fall
  back to the manual `setsockopt` path called out in D8 with explicit
  `unsafe` per D13.

---

*See `GLOSSARY.md` for terms and acronyms.*
