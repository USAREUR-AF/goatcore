# syntax=docker/dockerfile:1.7
#
# Multi-arch Docker build for signal-server.
# Built by .github/workflows/signal-server.yml for linux/amd64 + linux/arm64.
# Buildx sets BUILDPLATFORM (native builder arch) and TARGETPLATFORM/OS/ARCH
# (the arch we're producing). We run Go on the native builder and
# cross-compile — much faster than emulating the whole build under QEMU.

FROM --platform=$BUILDPLATFORM golang:1.26.11-bookworm@sha256:dc40a13b61304647ed7e5d581d246b86413dfc29d63c6d018310feb93cd98fc0 AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Cache go modules before copying source.
COPY go.mod go.sum ./
RUN go mod download

# Protoc plugins (pinned).
RUN go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.34.2 \
    && go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.5.1
ENV PATH="/go/bin:${PATH}"

COPY proto ./proto
COPY cmd ./cmd
COPY internal ./internal

# Generate Go bindings from the proto contracts.
RUN protoc -I proto \
        --go_out=. --go_opt=module=github.com/usareur-af/goatcore/signal-server \
        --go-grpc_out=. --go-grpc_opt=module=github.com/usareur-af/goatcore/signal-server \
        proto/signalexchange.proto proto/federation.proto

ARG TARGETOS
ARG TARGETARCH
RUN CGO_ENABLED=0 GOOS=$TARGETOS GOARCH=$TARGETARCH \
    go build -trimpath -ldflags='-s -w' -o /out/signal-server ./cmd/signal-server

FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=builder /out/signal-server /signal-server
USER nonroot:nonroot
EXPOSE 10000 9090
ENTRYPOINT ["/signal-server"]
