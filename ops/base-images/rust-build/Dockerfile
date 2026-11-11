# Role: rust-build (build argument BASE_RUST_BUILD). The Rust build stage. Pair it with runtime-cc: a binary runs only on a runtime whose glibc is at least the builder's.
# Public default: Chainguard's free image, pulled anonymously and pinned by
# digest (the free catalogue tags only latest; Dependabot moves the digest).
# An overlay replaces this file to build on other bases, such as a private
# Chainguard catalogue: goatcore docs/operations/build-on-your-own-base-images.md.
FROM cgr.dev/chainguard/rust:latest-dev@sha256:b2095bdcb670ddeb5f5919f35e281c44326e53c5a77b895663997aff67b02cff
