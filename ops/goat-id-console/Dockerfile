# Multi-stage build for goat-id-console.
#
# Stage 1: build a static binary (no CGO, embed.FS for assets).
# Stage 2: the runtime-static base (ops/base-images/) — no shell, no package manager.
#
# Per ADR 0860: this is the admin tenant of the IdP plane and runs behind
# an oauth2-proxy sidecar with --allowed-group=ADMINS (same role pattern
# as oauth2-proxy-dashboard).

# Base images are build arguments named for their role; the defaults are
# the public images in ops/base-images/ (docs/design/base-images.md).
ARG BASE_RUNTIME_STATIC=gcr.io/distroless/static-debian13:nonroot@sha256:25ac8f0c3b2334a9eb05452266bd4697864a8c8ef0ef71e5e6480d7f92868329

FROM golang:1.26-bookworm@sha256:23ce71337b726608afb1bc00986e630a05034a87a87ea1fc2cd1ae5cf33387eb AS build
WORKDIR /src

# Cache go.mod first for layer reuse.
COPY go.mod go.sum ./
RUN go mod download

# Sources.
COPY cmd/ ./cmd/
COPY internal/ ./internal/
# Only what web.go embeds; web/e2e (browser tests) never reaches the binary.
COPY web/web.go ./web/
COPY web/templates/ ./web/templates/
COPY web/static/ ./web/static/

ARG VERSION=0.1.0
ENV CGO_ENABLED=0
RUN go build \
    -trimpath \
    -ldflags="-s -w -X main.version=${VERSION}" \
    -o /goat-id-console \
    ./cmd/console
# store-migrate ships beside the server: the SQLite -> Postgres cutover and the
# Postgres -> SQLite rollback run it from this image (ADR 1165 step A,
# ops/ansible/id-console-postgres-cutover.yml), so the copy is always made by
# the same build that owns the schema.
RUN go build \
    -trimpath \
    -ldflags="-s -w" \
    -o /store-migrate \
    ./cmd/store-migrate

# Default canonical permissions YAML — the runtime image bundles a baseline
# copy under /etc/goat-id-console/group-permissions.yml so the binary boots
# without a separate volume mount. Operators override via env or by bind-
# mounting a newer copy at the same path.
FROM ${BASE_RUNTIME_STATIC}
# The store this image's server opens. The id-admin-console role reads it with
# `docker image inspect` and refuses to deploy an image whose store differs
# from the goatnet's declared id_admin_console_store. An image without the label
# predates the Postgres server and is read as "sqlite".
# goat.id-console.store carries the same fact under a neutral key; the role
# reads whichever key id_admin_console_store_label names.
LABEL org.usareur-af.goat-id-console.store="postgres"
LABEL goat.id-console.store="postgres"
USER 65532:65532
COPY --from=build /goat-id-console /usr/local/bin/goat-id-console
COPY --from=build /store-migrate /usr/local/bin/store-migrate
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/goat-id-console"]
