FROM golang:1.26.10-alpine@sha256:87c65a688975c0b1d4585d204874c45ca78354670e3a5f11368c5a492303d696 AS builder
ENV GOTOOLCHAIN=local
WORKDIR /src
COPY go.mod ./
RUN go mod download
COPY *.go ./
RUN CGO_ENABLED=0 go build -trimpath -o /loadgen .

FROM alpine:3.24@sha256:f59abe656b692d901e184be3445e084786602108599d2a51d229c0f5a48c71bf
COPY --from=builder /loadgen /loadgen
ENTRYPOINT ["/loadgen"]
