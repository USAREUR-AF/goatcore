# Kratos UI runtime refresh

Status: built.

The self-service UI keeps the pinned upstream v26.2.0 application (still Alpine 3.17.0, musl 1.2.3-r4, OpenSSL 3.0.7 in the upstream image),
runtime user and local consent, claim-forwarding and double-submit patches.
The overlay installs exact same-release fixes for OpenSSL and musl before
applying those unchanged patches. It does not rebuild Node or application packages.

This is partial remediation. Alpine 3.17 is past end of support and the application
dependencies remain separate upgrade work. Package scans, the existing behavior
tests, built-image patch verification and application startup must pass before
publishing a replacement image. Deployment image pins advance separately.
