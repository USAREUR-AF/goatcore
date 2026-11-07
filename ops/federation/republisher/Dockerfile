# syntax=docker/dockerfile:1.7
# SPDX-License-Identifier: Apache-2.0
#
# goat-fed-republisher container image.
#
# Same two-stage shape as ops/zenoh-trial-peers/audit-subscriber/Dockerfile:
#   1. rust-builder: cargo build --release --locked
#   2. Runtime: the runtime-cc base (ops/base-images/, ADR 1220).
#
# No embedded identity material. The deployment (roles/goat-fed-republisher)
# bind-mounts one directory: the composed agreement, which is the configuration,
# one leaf cert + key (CN fed.<us>.<them>.republisher.01) and our CA. Both
# sessions dial OUR core router with that leaf; the core's fed-rp-<them> subject
# lets it read the offered release sets and write from/<us>/** and nothing wider.
# The binary still takes the two sessions' endpoints and identities separately.

# Base images are build arguments named for their role; the defaults are
# the public images in ops/base-images/ (docs/design/base-images.md).
ARG BASE_RUST_BUILD=cgr.dev/chainguard/rust:latest-dev@sha256:54120f0b57f28063bd9f23e29113a28826ae08d731bb0bff11d6a6fa53bf74cf
ARG BASE_RUNTIME_CC=cgr.dev/chainguard/glibc-dynamic:latest@sha256:671a621016fb33812136dea6182b1db957996d3c8c618187aaf86abd9b14af53

FROM ${BASE_RUST_BUILD} AS rust-builder
USER root
WORKDIR /build
COPY Cargo.toml Cargo.lock* ./
RUN mkdir -p src && echo "fn main() {}" > src/main.rs && cargo fetch && rm -rf src
COPY src/ ./src/
RUN cargo build --release --locked

FROM ${BASE_RUNTIME_CC}
WORKDIR /
COPY --from=rust-builder /build/target/release/goat-fed-republisher /usr/local/bin/goat-fed-republisher
EXPOSE 9310
ENTRYPOINT ["/usr/local/bin/goat-fed-republisher"]
