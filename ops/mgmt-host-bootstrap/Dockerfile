# Multi-stage build for the mgmt-host-bootstrap CLI.
# Per ADR 0410. Bases are the go-build and runtime-static roles (ADR 1220).
#
# Multi-arch via docker buildx: TARGETARCH/TARGETOS are populated by the
# buildx driver per `--platform=linux/amd64` or `--platform=linux/arm64`.
# Default to amd64 when neither is set (plain `docker build` on host arch).

# Base images are build arguments named for their role; the defaults are
# the public images in ops/base-images/ (docs/design/base-images.md).
ARG BASE_GO_BUILD=docker.io/library/golang:1.26-bookworm@sha256:23ce71337b726608afb1bc00986e630a05034a87a87ea1fc2cd1ae5cf33387eb
ARG BASE_RUNTIME_STATIC=gcr.io/distroless/static-debian13:nonroot@sha256:25ac8f0c3b2334a9eb05452266bd4697864a8c8ef0ef71e5e6480d7f92868329

FROM --platform=$BUILDPLATFORM ${BASE_GO_BUILD} AS build
ARG TARGETARCH
ARG TARGETOS=linux
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY *.go ./
RUN CGO_ENABLED=0 GOOS=${TARGETOS} GOARCH=${TARGETARCH:-amd64} \
    go build -trimpath -ldflags="-s -w" \
    -o /out/mgmt-host-bootstrap .

FROM ${BASE_RUNTIME_STATIC}
COPY --from=build /out/mgmt-host-bootstrap /usr/local/bin/mgmt-host-bootstrap
ENTRYPOINT ["/usr/local/bin/mgmt-host-bootstrap"]
