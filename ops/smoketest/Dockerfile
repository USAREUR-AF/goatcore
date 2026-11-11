# Stage 1 — Go build
FROM golang:1.26.11-bookworm@sha256:dc40a13b61304647ed7e5d581d246b86413dfc29d63c6d018310feb93cd98fc0 AS go-build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY cmd/ cmd/
COPY internal/ internal/
RUN CGO_ENABLED=0 go build -o /smoketest ./cmd/smoketest

# Stage 2 — Runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 python3-pip python3-venv ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=go-build /smoketest /usr/local/bin/smoketest
# The dashboard is plain JavaScript with no build step, copied as is to the
# directory every compose file passes to --dashboard-dir.
COPY dashboard/ /app/dashboard/dist/
COPY apptests/ /app/apptests/
COPY services.yaml /app/services.yaml
# -r apptests/requirements.txt, not bare `pip install pytest requests`: the bare
# form resolved both from PyPI at build time, so the pinned versions in that file
# never reached the image and its CVE pins were decorative.
RUN python3 -m venv /app/venv \
 && /app/venv/bin/pip install --no-cache-dir -r /app/apptests/requirements.txt
WORKDIR /app
EXPOSE 8080
CMD ["smoketest", "serve", "--addr", ":8080", "--results-file", "/app/results/results.jsonl", "--catalogue", "/app/services.yaml", "--dashboard-dir", "/app/dashboard/dist"]
