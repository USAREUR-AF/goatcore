# Multi-stage build for the mgmt-host-bootstrap CLI.
# Per ADR 0410. Bases are the go-build and runtime-static roles (ADR 1220).
#
# Multi-arch via docker buildx: TARGETARCH/TARGETOS are populated by the
# buildx driver per `--platform=linux/amd64` or `--platform=linux/arm64`.
# Default to amd64 when neither is set (plain `docker build` on host arch).

# Base images are build arguments named for their role; the defaults are
# the public images in ops/base-images/ (docs/design/base-images.md).
ARG BASE_GO_BUILD=cgr.dev/chainguard/go:latest@sha256:65e8b42e2c06a951df80725171337d9bdc8d3a2181d33cd2ef411a8a3d8873cc
ARG BASE_RUNTIME_STATIC=cgr.dev/chainguard/static:latest@sha256:fc42af33e643f20d08baf03cddcc1c02ba57052dfb02edb0d79f3fc47bcea8d9

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
