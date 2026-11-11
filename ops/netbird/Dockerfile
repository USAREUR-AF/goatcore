# syntax=docker/dockerfile:1.7
#
# Patched Netbird management server build for multi-master Spock deployments.
#
# Matches upstream's management/Dockerfile.multistage pattern exactly. The
# key thing this buys us is building with CGO=1, which is required: the
# netbirdio/dex fork (pulled in via go.mod `replace` directive) has
# CGO-gated paths in its SQLite storage that won't resolve with CGO=0 —
# that's why every CGO=0 buildx attempt before this failed with
# "unknown field File in struct literal of type
#  github.com/dexidp/dex/storage/sql.SQLite3".
#
# Architecture:
#   - Native per-arch build (no buildx cross-compile). The workflow runs
#     this Dockerfile twice — on ubuntu-24.04 (amd64) and ubuntu-24.04-arm
#     (arm64) — and manifest-combines the resulting images into a single
#     multi-arch tag. See .github/workflows/netbird-mgmt.yml.
#   - Final stage is ubuntu:24.04 (glibc) rather than alpine (musl),
#     because CGO=1 produces a glibc-linked binary.
#
# See docs/design/ha-multi-site.md §5.8.1 for the skip-migrations patch context.

ARG NETBIRD_REF=0358be23136da50e829ba99a83e54ef555071a7f  # upstream v0.71.4

# --- Stage 1: build the patched binary natively ---
FROM golang:1.27-bookworm@sha256:dc0963f32053e4bc451d24d87e3892d42d01021b61f093e98eb95c0352bb84b0 AS builder
ARG NETBIRD_REF

RUN apt-get update \
    && apt-get install -y --no-install-recommends git gcc libc6-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# git init + fetch works for tags, branches, and SHAs alike
# (git clone --branch rejects commit SHAs).
WORKDIR /build/netbird
RUN git init -q . \
    && git fetch --depth 1 https://github.com/netbirdio/netbird.git ${NETBIRD_REF} \
    && git checkout -q FETCH_HEAD

COPY patches/ /build/patches/
# The last two patches (goat ADR 1081 L1) are CLIENT-scoped — they change the
# NetBird agent's signal retry/breaker behavior, not the management server, so
# `go build ./management` below does not compile them. They are applied here
# anyway so the whole patch set stays verified-appliable against the pinned
# NETBIRD_REF in one place (this build is the only apply-point in-repo); the
# client build path consumes the same patch files. Both apply cleanly after
# the management patches (disjoint files).
RUN git apply --verbose /build/patches/skip-migrations.patch \
    && git apply --verbose /build/patches/auto-peer-name.patch \
    && git apply --verbose /build/patches/peer-id-bound-relay-tokens.patch \
    && git apply --verbose /build/patches/setupkey-user-id-stamp.patch \
    && git apply --verbose /build/patches/client-signal-backoff-jitter.patch \
    && git apply --verbose /build/patches/client-offer-circuit-breaker.patch \
    && git apply --verbose /build/patches/service-user-token-bootstrap.patch

# Resolve module dependencies (including the netbirdio/dex replace
# directive) before compilation. Upstream's Dockerfile.multistage does the
# same.
RUN go mod download

# CGO=1 is mandatory — see header comment.
RUN CGO_ENABLED=1 GOOS=linux \
    go build -trimpath -ldflags "-s -w" -o /build/netbird-mgmt ./management

# --- Stage 2: minimal runtime (matches upstream management/Dockerfile) ---
FROM ubuntu:26.04@sha256:6a06111c3b0a8600feadb270aec771dc239bf407b25625f93f44ab00e11f4d3c
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# No deployment CA is baked into this image: each goatnet supplies its own
# internal CA at run time. ops/compose/mgmt-host mounts the files listed in
# netbird_mgmt_trust_certs at /etc/netbird/trust and adds it to SSL_CERT_DIR.
RUN update-ca-certificates

COPY --from=builder /build/netbird-mgmt /usr/local/bin/netbird-mgmt
COPY --chmod=0755 entrypoint.sh /usr/local/bin/entrypoint.sh

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["management", "--log-file", "console"]
