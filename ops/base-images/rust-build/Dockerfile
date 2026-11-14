# Role: rust-build (build argument BASE_RUST_BUILD). The Rust build stage. Pair it with runtime-cc: a binary runs only on a runtime whose glibc is at least the builder's.
# Public default: Chainguard's free image, pulled anonymously and pinned by
# digest (the free catalogue tags only latest; Dependabot moves the digest).
# An overlay replaces this file to build on other bases, such as a private
# Chainguard catalogue: goatcore docs/operations/build-on-your-own-base-images.md.
FROM cgr.dev/chainguard/rust:latest-dev@sha256:a189bb0b5bc4c22a86636023d2f23b0526ba8f7b280e339e04d0bf559c2f094c
