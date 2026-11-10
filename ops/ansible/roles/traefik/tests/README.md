# traefik role tests

Parse-only smoke. End-to-end TLS-termination behavior is covered by the
mgmt-host bringup runbook
and the per-site `site-bootstrap.yml` Phase 2.5 step that mints + delivers
the offline-CA leaf cert and verifies HTTPS reachability.

Run: `./syntax-check.sh`.
