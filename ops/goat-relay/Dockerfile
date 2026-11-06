# syntax=docker/dockerfile:1.7
#
# goat-relay container image — Block 33B Rust relay-server data plane.
#
# Architecture:
#   - Build stage: rust:1.92-bookworm (Debian-slim with rustc + gcc +
#     glibc — required by ktls-sys's build script which generates C
#     bindings to the kernel TLS ULP at compile time).
#   - Runtime stage: the runtime-cc base (ops/base-images/, ADR 1220)
#     — minimal, no shell; an overlay may substitute a hardened
#     catalogue image for the runtime surface (the security-relevant
#     layer). The
#     builder is a build-time-only dependency that does not ship in
#     the runtime image.
#
# References:
#   - ADR 0709 (Block 33 architecture; D7 tokio + D8 rustls/ktls + D9
#     quinn + D10 tungstenite)
#   - ADR 0700 + ADR 1220 (hardened runtime surface; bases by role)
#   - ops/zenoh/image/Dockerfile (precedent for Rust-from-source builds
#     using a stock Debian builder when the alternative requires
#     extensive feature backports)
#
# Build context: repository root (this Dockerfile copies the entire
# Cargo workspace). Invoke as:
#
#   docker build -f ops/goat-relay/Dockerfile -t goat-relay:dev .
#
# Run with TLS cert + key mounted (paths inside the container):
#
#   docker run --rm \
#     -v $(pwd)/cert.pem:/etc/goat-relay/cert.pem:ro \
#     -v $(pwd)/key.pem:/etc/goat-relay/key.pem:ro \
#     -p 33073:33073/tcp -p 33073:33073/udp \
#     goat-relay:dev \
#     --tls-cert /etc/goat-relay/cert.pem \
#     --tls-key /etc/goat-relay/key.pem \
#     --bind 0.0.0.0:33073
#
# Test stage — builds with the full workspace + cargo, runs the
# integration tests inside Linux. Exercises the cfg(target_os = "linux")
# branch in goat-relay/src/listener/ws_ktls.rs at runtime (the test
# clients connect over real TCP sockets, which on Linux triggers
# `ktls::config_ktls_server` via the cork-stream wrapper). To run:
#
#   docker build -f ops/goat-relay/Dockerfile --target test -t goat-relay:test .
#   docker run --rm goat-relay:test                     # runs all unit + integration tests
#   docker run --rm goat-relay:test -- --ignored        # runs the multi-peer-pair sweep
#   docker run --rm goat-relay:test -- --include-ignored  # runs both
#
# Used by `ops/goat-relay/docker-compose.yml`'s `tester` service.

ARG RUST_VERSION=1.92

# Base images are build arguments named for their role; the defaults are
# the public images in ops/base-images/ (docs/design/base-images.md).
ARG BASE_RUNTIME_CC=gcr.io/distroless/cc-debian13:nonroot@sha256:08476c244888bfb425bae1ae4dd7c17f093e972aab8426883d97f3c6f280f7c1

# --- Stage 1: build the binary ---
FROM rust:${RUST_VERSION}-bookworm AS builder

WORKDIR /build

# Workspace manifest first, then per-crate manifests, so cargo can
# resolve the workspace before the source files arrive.
COPY Cargo.toml Cargo.lock clippy.toml ./
COPY goat-relay-protocol/ ./goat-relay-protocol/
COPY goat-relay/ ./goat-relay/

# Build release binary, locked deps for reproducibility.
#
# Note: a previous Block 51F deps-cache-shim split was reverted
# during M-040 re-bench prep — the shim's empty-bench/empty-bin stubs
# interacted badly with cargo's per-crate fingerprint cache and caused
# 11 unresolved-imports compile errors on the second-stage real-source
# rebuild despite `cargo check` succeeding outside docker. Single-step
# build is slower (~17 min) but reliable; restore the shim only with a
# discipline that exercises it cleanly across both `[[bin]]` and
# `[[bench]]` target rewrites in both crates.
RUN cargo build --release --bin goat-relay --locked

# --- Stage 2: test runner (Linux runtime validation) ---
# Inherits the builder's source + target/, so cargo doesn't recompile
# on first run. ENTRYPOINT defaults to the workspace test runner;
# extra args (e.g. `-- --ignored`) flow through to the test binary.
FROM builder AS test
WORKDIR /build
ENTRYPOINT ["cargo", "test", "--workspace", "--release", "--locked"]

# --- Stage 3: minimal runtime ---
FROM ${BASE_RUNTIME_CC} AS runtime

COPY --from=builder /build/target/release/goat-relay /usr/local/bin/goat-relay

# Standard relay port (TCP for WS, UDP for QUIC — both bound to the
# same port number per ADR 0709 D9/D10; Linux kernel routes by L4
# protocol).
EXPOSE 33073/tcp 33073/udp

ENTRYPOINT ["/usr/local/bin/goat-relay"]
