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
