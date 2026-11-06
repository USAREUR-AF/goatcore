# SPDX-License-Identifier: Apache-2.0
#
# goat-consistency-health — the consistency domain of the environment-health
# surface (consistency-foundation.md §5, ADR 1110 / ADR 1036).
#
# THE BUILD CONTEXT IS ops/consistency/, not the crate directory, and the image
# reproduces the repo's directory layout exactly (/build/grammar.yaml +
# /build/rust/…). That is deliberate rather than incidental: the grading library
# compiles the committed grammar in with
# `include_str!("../../../grammar.yaml")`, so a layout that merely "put the file
# somewhere findable" would work until someone moved a crate and then fail at
# compile time with a path nobody could read. Mirroring the tree means the
# relative path means the same thing in both places.
#
# Compiling the grammar in is what makes a deployed meter provably execute the
# committed grammar it was built from: no mount, no runtime file to go missing,
# and no "grammar unavailable" degraded mode for a file that is not optional.
#
# runtime-cc base (ops/base-images/, ADR 1220); non-root uid 65532 (the ansible
# role chowns the cert dir to it).

# Base images are build arguments named for their role; the defaults are
# the public images in ops/base-images/ (docs/design/base-images.md).
ARG BASE_RUST_BUILD=docker.io/library/rust:1.92-bookworm@sha256:e90e846de4124376164ddfbaab4b0774c7bdeef5e738866295e5a90a34a307a2
ARG BASE_RUNTIME_CC=gcr.io/distroless/cc-debian13:nonroot@sha256:08476c244888bfb425bae1ae4dd7c17f093e972aab8426883d97f3c6f280f7c1

FROM ${BASE_RUST_BUILD} AS rust-builder
USER root
WORKDIR /build/rust

# Warm the dependency cache on the manifests alone (the sibling env-health
# crates' pattern), then build for real. The workspace manifest names every
# member, so every member's Cargo.toml must exist for `cargo fetch` — the
# console crate is copied here too even though this image builds the meter.
COPY rust/Cargo.toml rust/Cargo.lock* ./
COPY rust/goat-consistency/Cargo.toml goat-consistency/
COPY rust/goat-consistency-wire/Cargo.toml goat-consistency-wire/
COPY rust/goat-consistency-health/Cargo.toml goat-consistency-health/
COPY rust/goat-consistency-console/Cargo.toml goat-consistency-console/
RUN mkdir -p goat-consistency/src goat-consistency-wire/src goat-consistency-health/src goat-consistency-console/src \
 && : > goat-consistency/src/lib.rs \
 && : > goat-consistency-wire/src/lib.rs \
 && echo "fn main() {}" > goat-consistency-health/src/main.rs \
 && echo "fn main() {}" > goat-consistency-console/src/main.rs \
 && cargo fetch \
 && rm -rf goat-consistency/src goat-consistency-wire/src goat-consistency-health/src goat-consistency-console/src

COPY grammar.yaml /build/grammar.yaml
COPY rust/goat-consistency/ goat-consistency/
COPY rust/goat-consistency-wire/ goat-consistency-wire/
COPY rust/goat-consistency-health/ goat-consistency-health/
COPY rust/goat-consistency-console/ goat-consistency-console/
RUN cargo build --release --locked -p goat-consistency-health

FROM ${BASE_RUNTIME_CC}
WORKDIR /
COPY --from=rust-builder /build/rust/target/release/goat-consistency-health /usr/local/bin/goat-consistency-health
ENTRYPOINT ["/usr/local/bin/goat-consistency-health"]
