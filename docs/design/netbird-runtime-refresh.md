# NetBird runtime refresh

Status: built.

The management image keeps the pinned NetBird v0.71.4 source, its seven patches,
CGO, module dependencies and entrypoint. The builder uses Go 1.26.11. The pinned
Ubuntu 26.04 runtime installs exact same-release fixes for APT, libapt and PCRE2.
The image still takes deployment certificates and migration policy at runtime.

This refresh addresses runtime packages and the Go toolchain. It does not update
the application dependencies; findings in those dependencies remain separate work.
The native build, package inventory and component controls must be verified before
publishing a replacement image. A deployment advances its image digest separately.
