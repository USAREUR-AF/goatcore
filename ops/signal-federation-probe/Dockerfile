# Build + run image for signal-federation-probe.
# Multi-arch via the go-build base + the runtime-static base (ADR 1220):
# no shell, no package manager in the runtime image.
#
# Base images are build arguments named for their role; the defaults are
# the public images in ops/base-images/ (docs/design/base-images.md).
ARG BASE_GO_BUILD=cgr.dev/chainguard/go:latest@sha256:65e8b42e2c06a951df80725171337d9bdc8d3a2181d33cd2ef411a8a3d8873cc
ARG BASE_RUNTIME_STATIC=cgr.dev/chainguard/static:latest@sha256:fc42af33e643f20d08baf03cddcc1c02ba57052dfb02edb0d79f3fc47bcea8d9

FROM ${BASE_GO_BUILD} AS build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY main.go ./
RUN CGO_ENABLED=0 GOOS=linux go build -ldflags="-s -w" -o /signal-federation-probe .

FROM ${BASE_RUNTIME_STATIC}
COPY --from=build /signal-federation-probe /signal-federation-probe
EXPOSE 9091
ENTRYPOINT ["/signal-federation-probe"]
