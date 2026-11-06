# ADR 0711 — tokio-uring migration for io_uring SQPOLL + multishot recv (Block 33C.1 lever L5)

| Field | Value |
|---|---|
| Status | Proposed (2026-05-01) |
| Repo scope | goat core (goat-relay binary) |
| Supersedes | none |
| Superseded by | none |
| Related | [`0709-block-33-relay-rewrite-architecture.md`](0709-block-33-relay-rewrite-architecture.md) (Block 33 architecture; D7 runtime decision; D11 amendment 2026-05-01 multi-shard SO_REUSEPORT), [`0218-relay-substrate-bare-metal-strategy.md`](0218-relay-substrate-bare-metal-strategy.md) (substrate strategy — SQPOLL gain compounds harder on bare-metal), [`1012-load-testing-discipline.md`](1012-load-testing-discipline.md) (measurement discipline) |

## TL;DR (for a layperson)

The goat-relay forwards bytes between mesh peers. Today's Linux kernel offers two ways to do that:

1. **The classic way** (epoll + read/write syscalls): every packet causes a context switch from userspace into the kernel and back. At 100k packets/sec that's 200k context switches/sec — measurable CPU overhead.
2. **The new way** (io_uring + SQPOLL): the relay submits packet requests into a shared memory ring. A kernel thread polls the ring without context-switching. The relay can submit and receive thousands of packets without entering the kernel via syscall.

The current goat-relay uses the classic way (via vanilla tokio). To use the new way we need to swap the async runtime from tokio's epoll-based reactor to `tokio-uring`'s io_uring-based reactor. This ADR captures that decision: we will migrate, the migration is bounded to the goat-relay binary (not the wider project), and the expected throughput uplift is 10-30% on syscall-heavy paths per AWS published benchmarks.

## Context

[ADR 0709 D7](0709-block-33-relay-rewrite-architecture.md) selected vanilla tokio (current_thread runtime, single thread per shard) as the async runtime for the goat-relay rewrite. That decision was right for the first cut: tokio is the most-deployed Rust async runtime, has the broadest ecosystem (rustls, quinn, tokio-tungstenite all native), and gets us to a working binary fastest.

The Block 33C.1 lever cluster catalogues the per-byte efficiency levers we expect to compound to multi-Gbps per host. Lever **L5 — io_uring SQPOLL + multishot recv** is the one we deferred at 33B kickoff: vanilla tokio's reactor uses epoll + per-syscall reads, which is fine but leaves measurable CPU on the table at high packet rates.

M-005 measured the upstream Go relay's syscall profile and found ~24% of process time in actual I/O syscalls (read + write + epoll_pwait); the rest was in Go runtime overhead (F-017). Block 33B's Rust rewrite eliminated the 76% Go-runtime slice. The remaining slice — the I/O syscall path — is what L5 attacks.

The Block 33C.1 implementation plan parked L5 with a "needs tokio-uring migration first" note. This ADR is the architecture decision for that migration; the migration itself is the implementation work.

### What L5 actually is

Three io_uring features compound:

1. **SQPOLL** (kernel-side polling): a kernel thread polls the submission queue without the userspace process having to issue an `io_uring_enter()` syscall to wake it. At sustained traffic the userspace process never enters the kernel; it just writes SQEs into shared memory. AWS published benchmarks show ~10-30% throughput uplift at high packet rates from this alone.
2. **Multishot recv**: one `IORING_OP_RECV_MULTISHOT` SQE generates a CQE per arriving packet without re-arming. Reduces SQ pressure to one entry per connection lifetime instead of one per packet.
3. **Registered buffers / buffer rings**: pre-pinned buffer pool that io_uring can DMA into directly. Combined with kTLS-bypassed AEAD (already landed per ADR 0709 D8), the receive path is "NIC DMA → kernel → registered buffer → userspace pointer," with no per-packet allocation or copy.

On hypervisor-virtualized VMs the SQPOLL gain shrinks because the hypervisor scheduler preempts the SQPOLL kernel thread (per [ADR 0218](0218-relay-substrate-bare-metal-strategy.md) §"Lever multipliers"). On bare-metal the gain is preserved end-to-end. L5 is therefore the lever whose value depends most on the substrate decision.

### What's blocking L5 today

Vanilla tokio drives I/O via `mio`'s epoll-based reactor. Every TcpStream / UdpSocket read or write is an epoll-mediated syscall. There is **no path to use io_uring under vanilla tokio** — the runtime simply does not have the plumbing. To use io_uring we must use a different runtime.

Three candidate runtimes:

| Runtime | Status | Pros | Cons |
|---|---|---|---|
| **tokio-uring** | Tokio-team maintained; v0.5 (Q1 2026) | Same `tokio::sync` primitives (mpsc, watch, oneshot, RwLock); same `tokio::task::spawn_local` model; ergonomic migration; preserves ADR 0709 D7's single-threaded-runtime-per-pinned-thread invariant. | Per-task isolation — each task is bound to its own runtime instance. Some tokio APIs (TcpStream::connect, UdpSocket::bind) replaced with tokio-uring equivalents. quinn doesn't natively support tokio-uring — needs a custom AsyncUdpSocket adapter. tokio-tungstenite uses tokio's AsyncRead/AsyncWrite — needs a small adapter from tokio-uring's owned-buffer API. |
| **glommio** | Datadog-maintained; thread-per-core by design | Mature io_uring shape; explicit thread-per-core matches our shard model; published bench numbers for high-throughput shapes. | Different runtime philosophy from tokio (everything is built on glommio's executor); ecosystem fork is real (no rustls/quinn/tokio-tungstenite native support); large-scope migration. |
| **monoio** | ByteDance-maintained; thread-per-core | Mature io_uring; single-threaded. | Even smaller ecosystem than glommio; ABI-incompatible with rustls/quinn. |
| **Custom io_uring runtime** | Build our own atop the `io-uring` crate | Maximum control; no third-party API constraints. | Substantial engineering effort (~weeks); reinvents what tokio-uring already provides. |

## Decision

**Migrate to `tokio-uring` for the goat-relay binary.**

- The binary's per-shard runtime changes from `tokio::runtime::Builder::new_current_thread()` to `tokio_uring::Runtime`.
- All `tokio::net::TcpListener` / `TcpStream` / `UdpSocket` call sites in `goat-relay/src/listener/` migrate to `tokio_uring::net::*` equivalents.
- `quinn` keeps its own runtime / AsyncUdpSocket trait; we wire a thin `quinn::AsyncUdpSocket` adapter on top of `tokio_uring::net::UdpSocket`. The adapter lives in `goat-relay/src/listener/quic_uring.rs` (cfg-gated to Linux; non-Linux platforms keep the vanilla tokio path).
- `tokio-tungstenite` keeps its own dependence on `AsyncRead + AsyncWrite`. A small adapter wraps a `tokio_uring::net::TcpStream` to expose those traits; the adapter uses tokio-uring's owned-buffer API internally (`read(buf) -> (Result<usize>, Buf)`) and handles the buffer hand-off.
- `rustls` is runtime-agnostic; the rustls `ServerConnection` driving stays unchanged (it consumes byte slices, not async traits — the runtime adapter handles the async I/O).
- `kTLS` continues to work — kTLS hand-off is kernel-level and runtime-agnostic. After hand-off, the post-TLS plaintext stream rides directly on the `tokio_uring::net::TcpStream`.
- The `Shard::spawn` wrapper in `goat-relay/src/shard.rs` updates to construct a `tokio_uring::Runtime` instead of a `tokio::runtime`. Tasks still spawn via `tokio::task::spawn_local` — tokio-uring re-exports the same `LocalSet` / `spawn_local` APIs because they're tokio crates. ADR 0709 D7 invariants stay intact:
  - **Layer 1** — single-threaded runtime per pinned thread: tokio-uring's `Runtime` is single-threaded by design.
  - **Layer 2** — per-connection state `!Send`: `Rc<RefCell<KeepaliveSender>>` from `goat-relay-protocol` continues to enforce this on connection-task fields not in the routing table.
  - **Layer 3** — clippy lint blocks `tokio::spawn` / `tokio::task::spawn`: unchanged.
  - **Layer 4** — architecture comment: updated.
  - **Layer 5** — boot-time assertion: unchanged.

**Migration is bounded to the goat-relay binary.** The mgmt-host stack (signal-server, netbird-mgmt, db-policy-engine, mgmt-host-bootstrap, gatewatch, airdefenders, ntp-health-agent, admit-audit) stays on vanilla tokio. ADR 0709 D7's runtime decision applies to those workloads; their shape is request-scoped, not per-byte hot-path, and the io_uring uplift is structurally smaller. **The carve-out matches the language-decision rule in [ADR 0112](0112-tech-strategy-workload-shape-language-boundary.md): per-byte hot path + on-data-path sidecar gets the heaviest tooling; per-request control + CLI keeps the ergonomic default.**

**Sequencing:** L5 lands AFTER L1-L4, L6, L7, L8 are landed and re-measured against M-NNN (the L8 verdict re-test). The L5 implementation effort is ~4-8 hr focused work plus a re-measurement against the same iperf3 ramp. The expected delta versus the L8-stack baseline determines whether L5 was worth it (predicted: 10-30% lift on syscall-heavy paths; falsifiable).

## Consequences

- **Runtime ABI:** the goat-relay binary now requires Linux 5.6+ (io_uring availability) for production deploys. macOS dev builds keep the vanilla tokio path via cfg-gated module selection — same pattern as the kTLS hand-off (cfg(target_os = "linux")) per ADR 0709 D8.
- **Build matrix:** CI's Linux job builds the io_uring path; macOS job builds the vanilla path. Both paths cover the same routing / handshake / keepalive logic; only the I/O reactor differs.
- **Test coverage:** existing integration tests run on the vanilla path on macOS dev. A separate Linux-only test runner exercises the io_uring path. The conformance fixtures from Block 33A are runtime-agnostic and pass on both.
- **Dependency risk:** `tokio-uring` is at v0.5 (per Q1 2026 release). Smaller ecosystem than vanilla tokio; some libraries (sqlx, hyper, etc.) don't ship with first-party tokio-uring support. We don't depend on those — the goat-relay binary's deps are tokio (sync primitives), rustls, quinn, tokio-tungstenite, bytes, futures-util. All are runtime-agnostic enough to wrap.
- **Rollback path:** if the L5 lift is below threshold or operational issues surface, the vanilla-tokio code path stays in tree (cfg-gated) and we flip back via a build-feature flip + redeploy. Same shape as ADR 0709 D8's `aws-lc-rs` migration deferral pattern.
- **Bare-metal compounding:** Per [ADR 0218](0218-relay-substrate-bare-metal-strategy.md) the SQPOLL gain compounds harder on bare-metal than on hypervisor-VMs. L5's full value is realized after Phase 3 substrate migration; on the AWS VM bench today we'd expect a smaller delta. This is consistent with the per-lever measurement discipline — measure on each substrate.
- **Re-measurement plan:** Block 34 sub-block (allocate at run time per docs/measurements/INDEX.md) — single-flow + 4-flow + 16-flow + 64-flow ramp; flame graph + perf top + strace -c profile. Compare to the L8-stack baseline (the M-NNN re-test currently in flight). Expected: 10-30% throughput lift on the syscall-bound slice; lower on hypervisor VMs, higher on bare-metal.

## Cross-refs

- ADRs: [0709](0709-block-33-relay-rewrite-architecture.md) D7 + D11 + D8; [0218](0218-relay-substrate-bare-metal-strategy.md) lever multipliers; [0112](0112-tech-strategy-workload-shape-language-boundary.md) language-decision rule
- Findings: F-017 (the syscall slice this ADR addresses)
- Measurements: M-005 (24% I/O syscall slice baseline); M-014 (post-rewrite ceiling)
- Implementation plan: Block 33C.1 lever L5 row (status flips from Queued → In progress when this ADR is Accepted and the migration commit lands)
