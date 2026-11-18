# Role: runtime-shell (build argument BASE_RUNTIME_SHELL). A runtime with a shell and a package manager (apk or apt-get) for an image that runs scripts; glibc 2.39 or newer, so the host's goat binary runs in it.
# Public default: Chainguard's free image, pulled anonymously and pinned by
# digest (the free catalogue tags only latest; Dependabot moves the digest).
# An overlay replaces this file to build on other bases, such as a private
# Chainguard catalogue: goatcore docs/operations/build-on-your-own-base-images.md.
FROM cgr.dev/chainguard/wolfi-base:latest@sha256:7135c6a5e9f29b2542aaefee63d474973312bfdea95555963aef0e648210e4d2
