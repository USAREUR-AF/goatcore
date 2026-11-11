# admit-audit — read-only sidecar for the mgmt-host stack.
#
# Build arg `GOPROXY` lets CI cache module fetches; defaults to the
# public proxy.

FROM golang:1.27.3-alpine@sha256:fa3d596898d9f126af7c01d14ee7597bab3f39cccf392ef36b69fe77cd681f18 AS builder
ARG GOPROXY=https://proxy.golang.org,direct
WORKDIR /src
COPY go.mod go.sum* ./
RUN go mod download || true
COPY . .
RUN CGO_ENABLED=0 GOOS=linux go build -ldflags="-s -w" -o /out/admit-audit .

FROM gcr.io/distroless/static:nonroot
COPY --from=builder /out/admit-audit /admit-audit
USER nonroot:nonroot
EXPOSE 9124
ENTRYPOINT ["/admit-audit"]
