# syntax=docker/dockerfile:1.7
#
# Spock-enabled PostgreSQL 16 for the goat multi-site mesh.
#
# Upstream base: ghcr.io/pgedge/pgedge-postgres:16-spock5-minimal
#   - multi-arch (linux/amd64 + linux/arm64)
#   - ships PG 16.15 + Spock 5.0.11
#   - PostgreSQL License, free for self-hosted use
#
# PINNED BY DIGEST, and it was not before. `16-spock5-minimal` is a FLOATING tag
# that pgEdge re-points on every release — it moved to 16.14/spock-5.0.11 on
# and to 16.15/spock-5.0.11 . A floating base means the
# image, and every test of it, changes with no commit here: this workflow was
# green and red with nothing in between touching
# ops/postgres/spock or the workflow. Same class as a vulnerability database
# turning a green check red overnight — the fix is to make the input a fact we
# committed. Digest pins are the F-245 discipline used for every other image in
# this repo; the base had escaped it.
#
# Bump procedure: list the published versions and take the one currently tagged
# 16-spock5-minimal, then update BOTH the digest and the version comment above.
#   gh api --paginate /orgs/pgedge/packages/container/pgedge-postgres/versions \
#     -q '.[]|select((.metadata.container.tags//[])|join(",")|test("16.*spock5.*minimal"))
#         |"\(.updated_at)\t\(.name)\t\((.metadata.container.tags//[])|join(","))"'
#
# We overlay:
#   - postgresql.conf tuned for Spock logical replication
#   - pg_hba.conf template allowing mesh-overlay IPs (site-specific entries
#     are rendered in by Ansible at deploy time; the image ships a safe default)
#   - SQL init scripts that create the Netbird databases and load the Spock
#     extension in each replicated DB. Role creation + Spock subscriptions
#     are driven by Ansible at runtime, not baked into the image.

FROM ghcr.io/pgedge/pgedge-postgres:16-spock5-minimal@sha256:67890298cfd8a4f8e1f0f53132a7a1eda06db7a70591cff60db4ef99c1f2a034

# Baked config lives at /etc/postgresql/. The upstream entrypoint doesn't use
# this path by default — docker-compose passes `-c config_file=...` so this
# file is picked up instead of the one created by initdb inside PGDATA.
COPY conf/postgresql.conf /etc/postgresql/postgresql.conf
COPY conf/pg_hba.conf     /etc/postgresql/pg_hba.conf

# Runs once on first boot via the upstream postgres entrypoint.
COPY init/ /docker-entrypoint-initdb.d/

# --- pgBackRest ------------------------------------------------------------
# Baked in so the binary survives container recreates (e.g. image pin bumps)
# and so postgres-a does not need PGDG network reachability at runtime — a
# hard requirement for DDIL/disconnected deployments (Orin sites).
#
# pgBackRest is NOT in EPEL 9; PGDG is the canonical source. The role at
# ops/ansible/roles/pgbackrest still ships the dnf-install tasks as a
# safety-net — they are gated on `command -v pgbackrest` and become no-ops
# when the binary is already present.
USER root
# Direct RPM install from PGDG — pinned version, HTTPS-authenticated transport,
# no repo config needed. Dependencies (libyaml, libxml2 etc.) resolve from the
# already-enabled Rocky baseos/appstream repos which have their own GPG chain.
# Multi-arch: TARGETARCH is set by buildx (amd64/arm64) — map to dnf arch name.
#
# Bump procedure: list https://download.postgresql.org/pub/repos/yum/common/redhat/rhel-9-x86_64/
# and pick the latest `pgbackrest-X.Y.Z-1PGDG.rhel9.<minor>.<arch>.rpm` entry.
# Two moving parts: PGBACKREST_VERSION (semver) AND PGBACKREST_RHEL_SUFFIX
# (PGDG's downstream rhel-minor tag — they used to publish under bare
# `rhel9` and as of mid-2026 switched to `rhel9.8` once Rocky 9.8 landed
# upstream; older RPMs aged off the active stream, so a bare `rhel9` URL
# now 404s for every version. The same happened again with `rhel9.9`:
# 2.58.0 under `rhel9.8` was removed and only `rhel9.9` builds are listed.
# If a future PGDG bump moves to `rhel9.10` or similar, change the suffix
# here).
#
# Before a bump, read pgBackRest's release notes for every version in
# between for changes to the commands the roles run. 2.59.0: only restore
# may run as root by default; every docker exec of pgbackrest runs as the
# image's USER postgres (set at the end of this file) or with -u postgres.
# 2.60.0: restore errors and expire refuses on an invalid page checksum;
# these clusters are created without data checksums, so neither applies.
#
# Both dnf RUNs below build to the same bytes from the same tree. dnf writes
# its logs to /tmp/dnf-log, which is removed; countme is off, so dnf neither
# reports to the Rocky mirror's weekly user count nor rewrites the per-repo
# countme window file; the ldconfig aux-cache (inode
# and ctime of every library) is removed and rebuilt by the next ldconfig;
# and build/pin-install-time.py, mounted for the RUN only, sets the install
# time rpm and dnf record for the new packages to SOURCE_DATE_EPOCH (or, when
# a build passes none, to the base image's newest install time).
ARG TARGETARCH
ARG PGBACKREST_VERSION=2.60.1
ARG PGBACKREST_RHEL_SUFFIX=rhel9.9
# From 2.60 PGDG builds pgBackRest against its own libssh2 (pgdg-libssh2),
# published next to it in the same directory and not in any enabled repo,
# so it is installed from the same directory in the same transaction.
ARG PGDG_LIBSSH2_VERSION=1.11.1
# Both RPMs are fetched, then checked before dnf sees them, and a mismatch
# fails the build:
#   1. sha256sum -c against the digests pinned here, one per architecture;
#   2. rpm --checksig against PGDG's signing key for that architecture,
#      itself fetched and checked against its pinned digest, and imported
#      into a throwaway rpm database (the image's keyring is not changed).
# A bump updates the version ARGs above AND these digests: download each
# RPM for both architectures and take its sha256sum. PGDG signs x86_64 with
# key 08b40d20 (PGDG-RPM-GPG-KEY-RHEL) and aarch64 with key b9738825
# (PGDG-RPM-GPG-KEY-AARCH64-RHEL).
ARG PGBACKREST_SHA256_X86_64=dc3e6ef54c02f1bb8ceb0eacc4c3fa04312a000baf884fca585ef3a76955b500
ARG PGBACKREST_SHA256_AARCH64=3d0350bee21b9b0149bac0f7cf82f0f40c66a842b46604cedfba0440ab3406f3
ARG PGDG_LIBSSH2_SHA256_X86_64=14e53249dfd8f8fe82b9bd86695f78e707cc26a40b3d4926cb55f3e010db639d
ARG PGDG_LIBSSH2_SHA256_AARCH64=1086f9514fd9257a57e96efd51f203b76e3f9981c6bdee330e16515688c3662a
ARG PGDG_KEY_SHA256_X86_64=a70c9527426017d00fa4e6f9d2941d515357a27a7be82e155248ece53bbe5453
ARG PGDG_KEY_SHA256_AARCH64=cc506fa92aa97e8e58f88551a2ec99a61d9d603f7f2c1ae0c06191f58c29979f
ARG SOURCE_DATE_EPOCH
RUN --mount=type=bind,source=build/pin-install-time.py,target=/tmp/pin-install-time.py \
    set -eu \
 && case "${TARGETARCH:-amd64}" in \
      amd64) RPM_ARCH=x86_64; KEY=PGDG-RPM-GPG-KEY-RHEL; \
             KEY_SHA=$PGDG_KEY_SHA256_X86_64; BR_SHA=$PGBACKREST_SHA256_X86_64; SSH_SHA=$PGDG_LIBSSH2_SHA256_X86_64 ;; \
      arm64) RPM_ARCH=aarch64; KEY=PGDG-RPM-GPG-KEY-AARCH64-RHEL; \
             KEY_SHA=$PGDG_KEY_SHA256_AARCH64; BR_SHA=$PGBACKREST_SHA256_AARCH64; SSH_SHA=$PGDG_LIBSSH2_SHA256_AARCH64 ;; \
      *) echo "Unsupported TARGETARCH: $TARGETARCH"; exit 1 ;; \
    esac \
 && base="https://download.postgresql.org/pub/repos/yum/common/redhat/rhel-9-${RPM_ARCH}" \
 && ssh_rpm="pgdg-libssh2-${PGDG_LIBSSH2_VERSION}-1PGDG.${PGBACKREST_RHEL_SUFFIX}.${RPM_ARCH}.rpm" \
 && br_rpm="pgbackrest-${PGBACKREST_VERSION}-1PGDG.${PGBACKREST_RHEL_SUFFIX}.${RPM_ARCH}.rpm" \
 && mkdir /tmp/pgdg && cd /tmp/pgdg \
 && curl -fsSL --proto '=https' -o "$KEY" "https://download.postgresql.org/pub/repos/yum/keys/$KEY" \
 && curl -fsSL --proto '=https' -o "$ssh_rpm" "$base/$ssh_rpm" \
 && curl -fsSL --proto '=https' -o "$br_rpm" "$base/$br_rpm" \
 && printf '%s  %s\n' "$KEY_SHA" "$KEY" "$SSH_SHA" "$ssh_rpm" "$BR_SHA" "$br_rpm" | sha256sum -c - \
 && rpm --dbpath /tmp/pgdg/db --initdb \
 && rpm --dbpath /tmp/pgdg/db --import "$KEY" \
 && rpm --dbpath /tmp/pgdg/db --checksig "$ssh_rpm" "$br_rpm" \
 && cd / \
 && mark=$(python3 /tmp/pin-install-time.py mark) \
 && dnf install -y --setopt=logdir=/tmp/dnf-log --setopt=*.countme=0 "/tmp/pgdg/$ssh_rpm" "/tmp/pgdg/$br_rpm" \
 && rm -rf /tmp/pgdg \
 && mkdir -p /etc/pgbackrest /var/log/pgbackrest /var/spool/pgbackrest \
 && chown -R postgres:postgres /var/log/pgbackrest /var/spool/pgbackrest /etc/pgbackrest \
 && dnf clean all --setopt=logdir=/tmp/dnf-log \
 && rm -rf /var/cache/dnf /tmp/dnf-log /var/cache/ldconfig/aux-cache \
 && python3 /tmp/pin-install-time.py pin "$mark" \
 && rpm -q pgbackrest pgdg-libssh2

# --- Postgres contrib extensions (Block 21C — added) -----------
# Ory Kratos's migrations require pg_trgm + btree_gin (full-text indexes on
# identity_credential_identifiers). The pgedge-postgres-minimal base ships
# with PG16 binaries but does NOT bundle the contrib extensions. Adding the
# pgEdge-built contrib RPM here ensures Kratos + Hydra can run on the same
# postgres-spock cluster as netbird-mgmt — preserving the architectural
# pattern (Spock multi-master, pgBackRest paired-stanza, active-active over
# Router-A/B) for identity state without forking a separate Postgres image.
#
# pgEdge ships the contrib package (pgedge-postgresql16-contrib, which
# provides postgresql16-contrib) in the pgedge repo the base already enables.
# It is named by exact version-release so the layer does not depend on what
# the repo serves that day; its dependencies (perl and friends from Rocky
# appstream/baseos) resolve from the repos as they stand. Resolution order:
#   1. dnf install pgedge-postgresql16-contrib-<version>-<release>
#   2. fall back to PGDG community RPM if that pgEdge package is missing
# Bump both ARGs with the base (see header): the contrib must match the
# server's minor.
ARG POSTGRES_CONTRIB_VERSION=16.15
ARG PGEDGE_CONTRIB_RELEASE=1.el9
RUN --mount=type=bind,source=build/pin-install-time.py,target=/tmp/pin-install-time.py \
    set -eu \
 && case "${TARGETARCH:-amd64}" in \
      amd64) RPM_ARCH=x86_64 ;; \
      arm64) RPM_ARCH=aarch64 ;; \
    esac \
 && mark=$(python3 /tmp/pin-install-time.py mark) \
 && (dnf install -y --setopt=logdir=/tmp/dnf-log --setopt=*.countme=0 "pgedge-postgresql16-contrib-${POSTGRES_CONTRIB_VERSION}-${PGEDGE_CONTRIB_RELEASE}" 2>/dev/null \
     || dnf install -y --setopt=logdir=/tmp/dnf-log --setopt=*.countme=0 "https://download.postgresql.org/pub/repos/yum/common/redhat/rhel-9-${RPM_ARCH}/postgresql16-contrib-${POSTGRES_CONTRIB_VERSION}-2PGDG.rhel9.${RPM_ARCH}.rpm") \
 && dnf clean all --setopt=logdir=/tmp/dnf-log \
 && rm -rf /var/cache/dnf /tmp/dnf-log /var/cache/ldconfig/aux-cache \
 && python3 /tmp/pin-install-time.py pin "$mark" \
 && ls /usr/pgsql-16/share/extension/ | grep -E "^(pg_trgm|btree_gin)\.control$" \
   || (echo "FATAL: contrib extensions not present after install" && exit 1)

# Same-release OS fixes. Keep the database, extensions, backup tool and
# OpenSSL provider packages unchanged; a dependency requiring them fails here.
# The fixes come from Rocky's archive of the base's own release (9.8), not
# from the live mirror: once a newer point release ships, the live mirror
# serves only that release's packages, so these exact versions are no longer
# found there. The archive keeps every release's packages, signed with the
# same Rocky key the base already holds, and gpgcheck stays on.
ARG ROCKY_RELEASE=9.8
RUN --mount=type=bind,source=build/pin-install-time.py,target=/tmp/pin-install-time.py \
    set -eu \
 && set -- \
      ca-certificates-2026.2.92_v9.0.319-90.3.el9_8 \
      expat-2.5.0-6.el9_8.7 \
      gdb-gdbserver-16.3-3.1.el9_8 \
      gzip-1.12-2.el9_8 \
      libevent-2.1.13-1.el9_8 \
      libxml2-2.9.13-14.el9_8.5 \
      pcre2-10.40-7.el9_8 \
      pcre2-syntax-10.40-7.el9_8 \
      python-unversioned-command-3.9.25-7.el9_8.6 \
      python3-3.9.25-7.el9_8.6 \
      python3-libs-3.9.25-7.el9_8.6 \
      sqlite-libs-3.34.1-11.el9_8 \
      vim-minimal-2:8.2.2637-26.el9_8.23 \
 && mark=$(python3 /tmp/pin-install-time.py mark) \
 && dnf upgrade -y --setopt=logdir=/tmp/dnf-log --setopt='*.countme=0' \
      --setopt=install_weak_deps=False --disablerepo='*' \
      --repofrompath="rocky-vault-baseos,https://dl.rockylinux.org/vault/rocky/${ROCKY_RELEASE}/BaseOS/\$basearch/os/" \
      --repofrompath="rocky-vault-appstream,https://dl.rockylinux.org/vault/rocky/${ROCKY_RELEASE}/AppStream/\$basearch/os/" \
      --enablerepo='rocky-vault-*' \
      --setopt='rocky-vault-*.gpgcheck=1' \
      --setopt='rocky-vault-*.gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky-9' \
      --exclude='pgedge*' --exclude='postgresql*' --exclude='spock*' \
      --exclude='pgbackrest*' --exclude='openssl*' "$@" \
 && rpm -q -- "$@" \
 && dnf clean all --setopt=logdir=/tmp/dnf-log \
 && rm -rf /var/cache/dnf /tmp/dnf-log /var/cache/ldconfig/aux-cache \
 && python3 /tmp/pin-install-time.py pin "$mark"

# --- F-044 — scrub baked PGDATA ------------------------------------------
# The pgedge-postgres base image ships /var/lib/pgsql/16/data already
# populated from build-time setup. Docker named-volume first-mount
# auto-copies image contents into the new volume; the upstream postgres
# entrypoint then sees a non-empty PGDATA, prints "Database directory
# appears to contain a database; Skipping initialization", skips initdb
# entirely, and POSTGRES_PASSWORD env is silently ignored. Every consumer
# of postgres-spock then crash-loops in management-a / kratos / hydra
# with `password authentication failed for user "postgres"` until the
# operator manually ALTER USERs the password.
#
# Scrubbing the data dir at build time forces upstream initdb to run on
# first volume mount, which applies POSTGRES_PASSWORD properly and
# executes /docker-entrypoint-initdb.d/ scripts (our init/ overlay).
#
# See docs/findings/F-044-pgedge-postgres-baked-data-dir.md for the full
# mechanism + reproducer + the F-006 distinction (post-init restart timing
# vs initdb being skipped entirely).
RUN set -eu \
 && rm -rf /var/lib/pgsql/16/data \
 && mkdir -p /var/lib/pgsql/16/data \
 && chown postgres:postgres /var/lib/pgsql/16/data \
 && chmod 0700 /var/lib/pgsql/16/data

USER postgres
