# zenoh-pqc image — PQ hybrid TLS (X25519MLKEM768) zenoh router

Design + measurements: [`docs/design/zenoh-pqc-tls.md`](../../../docs/design/zenoh-pqc-tls.md).
Prototype status + rollout tracking: `docs/project/zenoh-pqc-tls-implementation.md`.

## What this is

The digest-pinned upstream `eclipse/zenoh:1.9.0` image with `/zenohd` rebuilt
from the same 1.9.0 source tag plus three patches:

- [`patches/zenohd-pqc-aws-lc-rs.patch`](patches/zenohd-pqc-aws-lc-rs.patch)
  (14 lines, confined to the `zenohd` binary crate — no zenoh library crate is
  touched). Installs the rustls **aws-lc-rs** CryptoProvider as the process
  default before any link initializes, which makes every TLS/QUIC link offer
  and prefer the **X25519MLKEM768** hybrid post-quantum key-exchange group
  while still accepting classical groups from unmodified ring-provider peers.
- [`patches/zenohd-f253-accept-throttle.patch`](patches/zenohd-f253-accept-throttle.patch)
  (goat F-253). The stock TLS accept loop
  (`zenoh-link-tls/src/unicast.rs`) sleeps 100 ms on **any** `accept()` error,
  including per-connection handshake failures. Under an mTLS reconnect storm
  where peers abandon the handshake mid-flight, that serialized all admission
  to ~10 opens/s and stalled legitimate new sessions past the 10 s
  `open_timeout` (the router "seizes" — see the finding). This patch throttles
  only genuine listener-level (fd-exhaustion) errors and drops failed
  handshakes without delay. It touches the `zenoh-link-tls` **library** crate,
  so unlike the PQC seam it is a small local library patch, not a `main()`
  hook. Root-cause + repro: `docs/findings/F-253-*`
  and the rig at `ops/zenoh/wedge-repro/`.
- Upstream [eclipse-zenoh#2635](https://github.com/eclipse-zenoh/zenoh/pull/2635) (querier/session
  self-deadlock) ships in 1.10.0; its backport was dropped at the 1.10.2 bump.

## What this is NOT

- **Not a zenoh fork.** zenoh's link crates install ring with
  `.install_default().ok()` — first installer wins by upstream design. The
  patch exploits that seam from `main()`; rebasing it onto a future zenoh tag
  is re-applying 14 lines.
- **Not a FIPS claim.** This image builds non-FIPS aws-lc-rs. The upstream
  certificate condition IS now met (CMVP #5298/#5314, June 2026, AWS-LC FIPS
  3.1.0 with ML-KEM in-boundary), and the variant built on the validated
  module now exists (below) — but even that carries "built on the validated
  module" language only; see the design doc §5 before repeating any
  compliance language.

## FIPS variant (`Dockerfile.fips` → `ghcr.io/usareur-af/zenoh-pqc-fips`)

Same 14-line seam, different provider: rustls `fips` → aws-lc-rs `fips` →
**aws-lc-fips-sys =0.13.11**, which vendors exactly the AWS-LC-FIPS-3.1.0
source that CMVP **#5298**/**#5314** validate with ML-KEM in-boundary.
**Never float that pin** — 0.13.12+ vendor 3.2.0/3.3.0/3.4.0, which are
still in the CMVP queue, and the exact version string on the certificate is
what the claim hangs on.

**Coupled pin:** `aws-lc-rs` must be held at `=1.17.1` alongside it.
1.17.3 raised its `aws-lc-fips-sys` floor to `^0.13.16`, which `=0.13.11` can
no longer satisfy — the image build dies at resolution (exit 101). 1.17.1
still floors at `^0.13.1`, so it accepts the validated 0.13.11 module. The
Dockerfile pins `aws-lc-rs` DOWN *before* the `aws-lc-fips-sys` precise pin;
bump the two only in lockstep, and only to a CMVP-validated `aws-lc-fips-sys`.

Differences vs the non-FIPS image, all forced by the module:

- **glibc runtime** (debian:bookworm-slim) — the FIPS build chain
  (cmake + Go + Perl, delocated shared objects) does not fit alpine/musl,
  so the upstream image cannot be reused as the runtime layer.
- **Matched-pair plugins** — rest + storage_manager + the rocksdb backend
  are built from the same tag/toolchain and BAKED into the image
  (`/libzenoh_plugin_*.so`, `/opt/zenoh/plugins-baked/`). Deploying this
  variant requires the role to skip the musl rocksdb bind-mount and point
  `backend_search_dirs` at the baked path.
- **Fail-closed startup** — the patched `main()` asserts
  `try_fips_mode()` before serving and logs
  `goat FIPS: AWS-LC FIPS module active`.
- **Mixed-fleet nuance** — the FIPS provider excludes pure X25519, so
  ring clients land on P-256 with one extra HelloRetryRequest
  round trip (the non-FIPS image accepts X25519 directly).

[`fips-probe/`](fips-probe/) is the verification gate: it runs inside the
image build (loopback: module-in-FIPS-mode, X25519MLKEM768 offered
fips=true + preferred, ring client down-negotiates to P-256) and again in
CI smoke as `--connect` against the booted router, asserting the live
negotiated group. A build that cannot prove the PQ-in-FIPS property does
not publish.

## Bench harness

[`bench/`](bench/) is the M-081 driver: a zenoh-free rustls client that
drives full mTLS handshakes against a router's TLS listener with the
provider selectable per run (`pq` = aws-lc-rs with X25519MLKEM768
preferred, `classical` = ring), counting every
socket byte and reporting the negotiated group + latency percentiles.
Session resumption is disabled so every iteration is a full handshake.

## Maintenance cost (the fork ledger)

Per zenoh version bump: re-apply the patch to the new tag (expected clean;
`git apply` fails loudly if upstream moves `main()`), bump `ZENOH_TAG` +
`UPSTREAM_IMAGE` digest here, and let CI's smoke step prove plugin-ABI
compatibility (rest + storage_manager from the upstream layer + the
bind-mounted rocksdb backend `.so`). zenohd refuses to start on plugin
mismatch, so a toolchain drift fails the build, not the fleet.
