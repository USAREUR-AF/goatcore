# Role: runtime-shell (build argument BASE_RUNTIME_SHELL). A runtime with a shell and a package manager (apk or apt-get) for an image that runs scripts; glibc 2.39 or newer, so the host's goat binary runs in it.
# Public default: Chainguard's free image, pulled anonymously and pinned by
# digest (the free catalogue tags only latest; Dependabot moves the digest).
# An overlay replaces this file to build on other bases, such as a private
# Chainguard catalogue: goatcore docs/operations/build-on-your-own-base-images.md.
FROM cgr.dev/chainguard/wolfi-base:latest@sha256:43f53fe2a02ba1a9484cc560ccf188da32f4a4b38055752514d78a3923ff05c8
