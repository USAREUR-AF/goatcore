# Multi-stage build for goat-zenoh-portal.
#
# Stage 1: build static binaries (CGO disabled). The server keeps its registry
#          in Postgres (pgx); store-migrate also links the pure-Go SQLite driver
#          to copy the registry between the two engines (ADR 1165 step A).
# Stage 2: Chainguard glibc-dynamic runtime per ADR 0700.
#
# The server embeds all static assets via embed.FS, so the runtime image
# carries two files: /usr/local/bin/goat-zenoh-portal and
# /usr/local/bin/store-migrate. Cosign-signing
# lands as a follow-on when Block 40 / 50B.5 substrate fires (no per-
# image change required — registry-side signature).

FROM golang:1.26-bookworm@sha256:23ce71337b726608afb1bc00986e630a05034a87a87ea1fc2cd1ae5cf33387eb AS build
WORKDIR /src

# Cache go.mod first for layer reuse.
COPY go.mod go.sum ./
RUN go mod download

# Sources.
COPY cmd/ ./cmd/
COPY internal/ ./internal/
COPY web/ ./web/

# Strip + reproducible build flags.
ARG VERSION=0.1.0
ENV CGO_ENABLED=0
RUN go build \
    -trimpath \
    -ldflags="-s -w -X main.version=${VERSION}" \
    -o /goat-zenoh-portal \
    ./cmd/portal
# store-migrate ships beside the server: the SQLite -> Postgres cutover and the
# Postgres -> SQLite rollback run it from this image (ADR 1165 step A,
# ops/ansible/portal-postgres-cutover.yml), so the copy is always made by the
# same build that owns the schema.
RUN go build \
    -trimpath \
    -ldflags="-s -w" \
    -o /store-migrate \
    ./cmd/store-migrate

# Runtime base pinned by digest (#1251): the :latest tag floats, so two builds a
# day apart could differ in their base. No renovate in this repo, so refresh it
# manually: `docker buildx imagetools inspect cgr.dev/chainguard/glibc-dynamic:latest`
# and update the multi-arch index digest below.
FROM cgr.dev/chainguard/glibc-dynamic:latest@sha256:671a621016fb33812136dea6182b1db957996d3c8c618187aaf86abd9b14af53
# The store this image's server opens. The goat-zenoh-portal-store role reads it
# with `docker image inspect` and refuses to deploy an image whose store differs
# from the goatnet's declared portal_store. An image without the label predates
# the Postgres server and is read as "sqlite". The second label carries the
# same fact under a neutral key, so the role can read either.
LABEL org.usareur-af.goat-zenoh-portal.store="postgres"
LABEL io.goat.zenoh-portal.store="postgres"
# Static binary, but Chainguard glibc-dynamic gives ca-certificates +
# /etc/passwd for a non-root run. Use chainguard's nonroot user.
USER 65532:65532
COPY --from=build /goat-zenoh-portal /usr/local/bin/goat-zenoh-portal
COPY --from=build /store-migrate /usr/local/bin/store-migrate
EXPOSE 8090
ENTRYPOINT ["/usr/local/bin/goat-zenoh-portal"]
CMD ["serve"]
