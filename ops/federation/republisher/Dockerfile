# syntax=docker/dockerfile:1.7
# SPDX-License-Identifier: Apache-2.0
#
# goat-fed-republisher container image.
#
# Same two-stage shape as ops/zenoh-trial-peers/audit-subscriber/Dockerfile:
#   1. rust-builder: cargo build --release --locked
#   2. Runtime: the runtime-cc base (ops/base-images/, ADR 1220).
#
# No embedded identity material. FOUR files are bind-mounted at deploy time —
# a leaf cert + key for the LOCAL session and another pair for the PEERING EDGE
# session, plus each side's CA root. They are deliberately different identities:
# the local one is an ordinary fabric client, the edge one is what the edge's
# access rules grant `from/<sovereign>/**` and nothing wider.
#
# The composed agreement is mounted read-only too. It is the configuration.

# Base images are build arguments named for their role; the defaults are
# the public images in ops/base-images/ (docs/design/base-images.md).
ARG BASE_RUST_BUILD=docker.io/library/rust:1.92-bookworm@sha256:e90e846de4124376164ddfbaab4b0774c7bdeef5e738866295e5a90a34a307a2
ARG BASE_RUNTIME_CC=gcr.io/distroless/cc-debian13:nonroot@sha256:08476c244888bfb425bae1ae4dd7c17f093e972aab8426883d97f3c6f280f7c1

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
