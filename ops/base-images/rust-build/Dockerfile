# Role: rust-build (build argument BASE_RUST_BUILD). The Rust build stage. Pair it with runtime-cc: a binary runs only on a runtime whose glibc is at least the builder's.
# Public default: Chainguard's free image, pulled anonymously and pinned by
# digest (the free catalogue tags only latest; Dependabot moves the digest).
# An overlay replaces this file to build on other bases, such as a private
# Chainguard catalogue: goatcore docs/operations/build-on-your-own-base-images.md.
FROM cgr.dev/chainguard/rust:latest-dev@sha256:7cc4f990b3bf5167bb8560307702238db95b6bd8eb43e80a7d7fe6908abd8682
