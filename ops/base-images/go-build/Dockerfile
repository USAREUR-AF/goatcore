# Role: go-build (build argument BASE_GO_BUILD). The Go build stage; goat's Go binaries build with CGO_ENABLED=0.
# Public default: Chainguard's free image, pulled anonymously and pinned by
# digest (the free catalogue tags only latest; Dependabot moves the digest).
# An overlay replaces this file to build on other bases, such as a private
# Chainguard catalogue: goatcore docs/operations/build-on-your-own-base-images.md.
FROM cgr.dev/chainguard/go:latest@sha256:d5eb532c64db43c555c9fbf644628aac2690dcbfd07ac8b2bae97a7c66d085fb
