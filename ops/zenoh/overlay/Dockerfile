# Upstream zenoh runtime + rocksdb backend, layered. NOT the canonical
# deploy path (see below).
#
# Two paths exist for ADR 0858 step 5. The CANONICAL path is the
# Ansible role downloading the backend .so at deploy time + bind-mounting
# it into the host plugin dir (see ops/ansible/roles/{zenoh,goat-zenoh}/
# tasks/main.yml). No image rebuild on backend SHA256 bumps.
#
# This Dockerfile is the ALTERNATIVE: a single self-contained layered
# image `ghcr.io/usareur-af/zenoh-with-rocksdb:<version>` that bakes the
# upstream `libzenoh_backend_rocksdb.so` into upstream `eclipse/zenoh`.
# Reach for it if a site prefers one immutable image artifact over the
# image+.so deploy shape, or if Block 75's per-site OCI cache wants to
# pre-pull the substrate without a separate fetch step. It is NOT
# wired into CI today — there is no zenoh-with-rocksdb-image.yml; build
# it on demand.
#
# Eclipse doesn't ship an official `zenoh-backend-rocksdb` Docker image,
# so this fetch+COPY pattern is the only way to get a single image.
# The "standalone" variant statically links rocksdb so we don't need
# to worry about distro libstdc++ in the runtime image.
#
# Build with:
#   docker buildx build --platform linux/arm64 \
#     -t ghcr.io/usareur-af/zenoh-with-rocksdb:1.9.0 --push .
FROM debian:bookworm-slim AS fetch
RUN apt-get update && apt-get install -y --no-install-recommends curl unzip ca-certificates && rm -rf /var/lib/apt/lists/*
ARG TARGETARCH
ARG ZENOH_VERSION=1.10.2
RUN set -eux; \
    case "$TARGETARCH" in \
      arm64) RUST_ARCH=aarch64 ;; \
      amd64) RUST_ARCH=x86_64 ;; \
      *) echo "unsupported arch $TARGETARCH"; exit 1 ;; \
    esac; \
    URL="https://github.com/eclipse-zenoh/zenoh-backend-rocksdb/releases/download/${ZENOH_VERSION}/zenoh-backend-rocksdb-${ZENOH_VERSION}-${RUST_ARCH}-unknown-linux-musl-standalone.zip"; \
    curl -fsSL -o /tmp/rocksdb.zip "$URL"; \
    unzip -j /tmp/rocksdb.zip -d /tmp/; \
    ls -la /tmp/libzenoh_backend_rocksdb.so

FROM eclipse/zenoh:1.10.2@sha256:1885c59e7df9f38f73cc3ec0ba5b8efccb566b059a9ae3dd9c3095dcf760b86f
COPY --from=fetch /tmp/libzenoh_backend_rocksdb.so /libzenoh_backend_rocksdb.so
