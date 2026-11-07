# syntax=docker/dockerfile:1.7
#
# grant-keyd container image — the E1 key-distribution daemon (data-access M2.1).
# Build context = `ops/grant-crypto` (the parent), because grant-keyd is its own
# standalone workspace at `keyd/` that path-depends on the SDK core at `sdk/core`,
# so both subtrees must be in the context. The daemon opens a native Zenoh session
# (mTLS to the router) and publishes signed key bundles on keys/<rs>/<gen>.
#
#   docker buildx build --platform linux/arm64 \
#     -f ops/grant-crypto/keyd/Dockerfile -t grant-keyd:dev ops/grant-crypto
#
# Builder: rust:bookworm (gcc/glibc for zenoh's C-linking transitive crates).
# Runtime: the runtime-cc base, nonroot 65532 (ADR 1220) — the same UID the
# spiffe-helper sidecar runs as, so the SVID key it writes 0600 is readable here.

ARG RUST_VERSION=1.92

# Base images are build arguments named for their role; the defaults are
# the public images in ops/base-images/ (docs/design/base-images.md).
ARG BASE_RUNTIME_CC=cgr.dev/chainguard/glibc-dynamic:latest@sha256:671a621016fb33812136dea6182b1db957996d3c8c618187aaf86abd9b14af53

FROM rust:${RUST_VERSION}-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p grant-keyd --locked --manifest-path keyd/Cargo.toml

# keyd/ is its own workspace root, so its target dir is keyd/target, not /build/target.
FROM ${BASE_RUNTIME_CC} AS runtime
COPY --from=builder /build/keyd/target/release/grant-keyd /usr/local/bin/grant-keyd
ENTRYPOINT ["/usr/local/bin/grant-keyd"]
