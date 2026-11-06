# Build + run image for signal-federation-probe.
# Multi-arch via the go-build base + the runtime-static base (ADR 1220):
# no shell, no package manager in the runtime image.
#
# Base images are build arguments named for their role; the defaults are
# the public images in ops/base-images/ (docs/design/base-images.md).
ARG BASE_GO_BUILD=docker.io/library/golang:1.26-bookworm@sha256:23ce71337b726608afb1bc00986e630a05034a87a87ea1fc2cd1ae5cf33387eb
ARG BASE_RUNTIME_STATIC=gcr.io/distroless/static-debian13:nonroot@sha256:25ac8f0c3b2334a9eb05452266bd4697864a8c8ef0ef71e5e6480d7f92868329

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
