# Kratos UI runtime refresh

Status: built.

The self-service UI keeps the pinned upstream v1.3.1 application, Alpine 3.17,
runtime user and local consent, claim-forwarding and double-submit patches.
The overlay installs exact same-release fixes for OpenSSL and musl before
applying those unchanged patches. It does not rebuild Node or application packages.

This is partial remediation. Alpine 3.17 is past end of support and the application
dependencies remain separate upgrade work. Package scans, the existing behavior
tests, built-image patch verification and application startup must pass before
publishing a replacement image. Deployment image pins advance separately.
