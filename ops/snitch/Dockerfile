# Stage 1 — Go build (server + cross-compiled probe binaries for download)
#
# Block 61H — multi-platform probe builds. Cross-compiles snitch for six
# targets so any peer (Linux Orin / mgmt-host, macOS operator laptop,
# Windows operator laptop) can download a working probe binary from the
# collector at /bin/snitch?os=<goos>&arch=<goarch>. CGO_ENABLED=0 across
# all targets — pure Go, no native deps.
FROM golang:1.26.11-bookworm@sha256:dc40a13b61304647ed7e5d581d246b86413dfc29d63c6d018310feb93cd98fc0 AS build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY cmd/ cmd/
COPY internal/ internal/
RUN CGO_ENABLED=0 go build -o /snitch ./cmd/snitch
RUN set -eux; \
    for target in linux/amd64 linux/arm64 darwin/amd64 darwin/arm64 windows/amd64 windows/arm64; do \
        os="${target%/*}"; arch="${target#*/}"; \
        out="/bin-${os}-${arch}/snitch"; \
        if [ "$os" = "windows" ]; then out="$out.exe"; fi; \
        echo "+ building $os/$arch -> $out"; \
        CGO_ENABLED=0 GOOS="$os" GOARCH="$arch" go build -o "$out" ./cmd/snitch; \
    done

# Stage 2 — Runtime (needs ping, arp, ip, avahi-browse for probing on Linux)
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    iputils-ping \
    iproute2 \
    net-tools \
    avahi-utils \
    curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /snitch /usr/local/bin/snitch
COPY --from=build /bin-linux-amd64/snitch /app/bin/linux-amd64/snitch
COPY --from=build /bin-linux-arm64/snitch /app/bin/linux-arm64/snitch
COPY --from=build /bin-darwin-amd64/snitch /app/bin/darwin-amd64/snitch
COPY --from=build /bin-darwin-arm64/snitch /app/bin/darwin-arm64/snitch
COPY --from=build /bin-windows-amd64/snitch.exe /app/bin/windows-amd64/snitch.exe
COPY --from=build /bin-windows-arm64/snitch.exe /app/bin/windows-arm64/snitch.exe
WORKDIR /app
EXPOSE 8080
CMD ["snitch", "serve", "--addr", ":8080", "--key-file", "/app/data/identity.key"]
