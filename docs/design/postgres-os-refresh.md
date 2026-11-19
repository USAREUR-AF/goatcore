# PostgreSQL OS package refresh

Status: built.

The PostgreSQL image keeps its pinned base, PostgreSQL 16.15, Spock, contrib and
pgBackRest versions. A separate build layer installs exact same-release fixes for
affected OS packages. The transaction excludes database, extension, backup and
OpenSSL provider packages and disables weak dependencies. A required dependency
that conflicts with those exclusions fails the build. An exact installed-package
query also fails the build if any requested version is missing.

Curl fixes require a newer OpenSSL provider package, so curl and libcurl remain
unchanged in this refresh. Their findings remain deferred with the provider update.

The layer uses the existing package install-time normalization so reproducible
build checks still apply. Package inventories and database/extension binary hashes
must remain unchanged outside the selected OS packages and their required
dependencies. Initialization, replication, seed/revocation, backup/restore and
reproducibility checks precede publication. This change does not upgrade a running
database or advance a deployment pin.

## Packages that age off upstream

Status: built. Measured on one local amd64 build; arm64 not measured.

Both upstream sources drop packages the image pins, so a fresh build of an unchanged
Dockerfile can fail at `dnf install`:

- PGDG publishes pgBackRest under the RHEL minor it was built for and removes older
  builds. pgBackRest 2.58.0 under `rhel9.8` is gone; the image now pins 2.60.1 under
  `rhel9.9`. From 2.60, PGDG links pgBackRest against its own libssh2, so
  `pgdg-libssh2` 1.11.1 is installed by URL in the same transaction.
- Once Rocky Linux ships a newer point release, its live mirror serves only that
  release's packages. The same-release fixes therefore come from Rocky's archive of
  the base's own release (`ROCKY_RELEASE`, 9.8), with signature checks against the
  Rocky key the base already holds. The pinned versions are unchanged.

Before a pgBackRest bump, read the release notes of every version in between for the
commands the roles run. From 2.58.0 to 2.60.1 two changes touch this deployment, and
neither changes its behaviour:

- 2.59.0: only `restore` may run as root unless `allow-root` is set. Every
  `docker exec` of `pgbackrest` runs as the image's `USER postgres`, or with
  `-u postgres`.
- 2.60.0: `restore` errors, and `expire` refuses, on an invalid page checksum. These
  clusters are created without data checksums (`data_checksums` is `off`), so no
  page checksum is read.

The `pgbackrest-runner` sidecar keeps its own image and version (2.53.1). Running it
against a repository the 2.60.1 binary writes is not measured.

A rebuilt image has a new digest. Deployment pins move to it in a separate change.
