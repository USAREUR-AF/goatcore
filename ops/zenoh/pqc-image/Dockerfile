# zenoh-pqc router image — upstream eclipse/zenoh:1.9.0 with /zenohd rebuilt
# from the same 1.9.0 source tag with three patches:
#  1. patches/zenohd-pqc-aws-lc-rs.patch — installs the aws-lc-rs rustls
#     CryptoProvider (X25519MLKEM768 hybrid key exchange, preferred) as the
#     process default before any TLS/QUIC link initializes; zenoh's link crates
#     tolerate this by construction (their ring install_default() calls ignore
#     the already-installed error). Confined to the zenohd binary crate.
#  2. patches/zenohd-f253-accept-throttle.patch — throttles the TLS accept loop
#     only on genuine listener-level (fd) errors, not per-connection handshake
#     failures (goat F-253). This one touches the zenoh-link-tls LIBRARY crate.
#
# Plugin-ABI discipline: the runtime stage IS the
# digest-pinned upstream image, so libzenoh_plugin_rest.so and
# libzenoh_plugin_storage_manager.so are upstream's own artifacts, and the
# ansible role's bind-mounted libzenoh_backend_rocksdb.so is unchanged. Our
# zenohd is built from the same source tag with the toolchain the tag pins
# (rust-toolchain.toml → rustc 1.93.0) as a dynamic musl binary, mirroring the
# upstream build. zenohd validates plugin compatibility at load and refuses to
# start on mismatch — fail-closed, which the CI smoke test exercises.
#
# Rollback = revert the zenoh_image pin to the upstream digest (artifact_type
# image, universal deploy-rollback ledger).

ARG UPSTREAM_IMAGE=eclipse/zenoh:1.9.0@sha256:157965d71e0bfd0a044d76a985ff0e5c306ad3968929168fb9678cd2a7fec23f

FROM rust:1.93.0-alpine3.22 AS builder

RUN apk add --no-cache musl-dev cmake make gcc g++ git perl linux-headers

ARG ZENOH_TAG=1.9.0
RUN git clone --depth 1 --branch ${ZENOH_TAG} \
    https://github.com/eclipse-zenoh/zenoh.git /src/zenoh

COPY patches/zenohd-pqc-aws-lc-rs.patch /tmp/
RUN cd /src/zenoh && git apply /tmp/zenohd-pqc-aws-lc-rs.patch

# goat F-253: the TLS accept loop throttled 100ms on ANY accept() error,
# including per-connection handshake failures, serializing all admission to
# ~10 opens/s under a reconnect storm and stalling new sessions past the 10s
# open_timeout. This patch throttles only genuine listener-level (fd) errors.
COPY patches/zenohd-f253-accept-throttle.patch /tmp/
RUN cd /src/zenoh && git apply /tmp/zenohd-f253-accept-throttle.patch

# NOTE: fabric-wedge fix #1 (adminspace_getter storage-manager deadlock,
# patches/zenohd-adminspace-getter-deadlock.patch, upstream #1094) is NOT
# applied here on purpose. This non-FIPS image reuses the UPSTREAM
# libzenoh_plugin_storage_manager.so (only zenohd is rebuilt below), so a source
# patch would not reach the shipped plugin. That fix ships in Dockerfile.fips,
# which builds `-p zenoh-plugin-storage-manager` from source — and the ZAGROS
# router runs that FIPS variant. To fix this image too, build+bake the plugin.

# goat fabric-wedge fix #2 — upstream eclipse-zenoh#2635 (merged
# post-1.9.0, no release carries it). Session::undeclare_querier_inner dropped a
# pending query's callback while holding the session `state` write lock; a
# completion callback re-entering the Session self-deadlocks on the non-reentrant
# lock and the transport/admin threads pile up behind it (a distinct lock cycle
# from #1094). Fix drops the removed queries after releasing the lock. Touches
# the zenohd binary crate (zenoh/src/api/session.rs), which IS rebuilt here.
COPY patches/zenohd-2635-querier-session-deadlock.patch /tmp/
RUN cd /src/zenoh && git apply /tmp/zenohd-2635-querier-session-deadlock.patch

# goat fabric-wedge fix #3 — admin-space reply under the tables lock
# (see Dockerfile.fips for the full account). Touches the zenoh crate
# (zenoh/src/net/runtime/adminspace.rs), which is compiled into zenohd and
# therefore IS rebuilt here.
COPY patches/zenohd-adminspace-reply-under-tables-lock.patch /tmp/
RUN cd /src/zenoh && git apply /tmp/zenohd-adminspace-reply-under-tables-lock.patch
# Finding F-614: zenoh-link-tls shares one TLS stream between its read and
# write tasks through an UnsafeCell. This keeps the stream behind a per-poll
# Mutex instead, the same change goatcore's vendor/zenoh-link-tls carries for
# the client workspaces. Drop it when a zenoh release ships the fix.
COPY patches/zenohd-link-tls-stream-mutex.patch /tmp/
RUN cd /src/zenoh && git apply /tmp/zenohd-link-tls-stream-mutex.patch

# Dynamic musl (upstream ships a dynamically-linked musl zenohd — a static
# binary could not dlopen the plugin .so's).
ENV RUSTFLAGS="-C target-feature=-crt-static"
RUN cd /src/zenoh && cargo build --release -p zenohd \
    && target/release/zenohd --version

FROM ${UPSTREAM_IMAGE} AS runtime

# The single delta vs upstream.
COPY --from=builder /src/zenoh/target/release/zenohd /zenohd
