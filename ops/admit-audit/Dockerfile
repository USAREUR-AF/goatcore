# admit-audit — read-only sidecar for the mgmt-host stack.
#
# Build arg `GOPROXY` lets CI cache module fetches; defaults to the
# public proxy.

FROM golang:1.27.2-alpine@sha256:f92b6ef800e499660581efdabdf25d9d817a9d124eaf900924f0504e7e27e12d AS builder
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
