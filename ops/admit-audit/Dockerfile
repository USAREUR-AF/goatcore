# admit-audit — read-only sidecar for the mgmt-host stack.
#
# Build arg `GOPROXY` lets CI cache module fetches; defaults to the
# public proxy.

FROM golang:1.27.4-alpine@sha256:0a5f7fe796599ff487605987432e7619ae45beeb2fba4c2ce0b907879932aebf AS builder
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
