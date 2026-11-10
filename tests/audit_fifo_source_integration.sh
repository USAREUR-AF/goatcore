#!/usr/bin/env bash
# Build exact historical source into disposable test images, never publish them.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch=$(mktemp -d)
identity=$(python3 -c 'import uuid; print(uuid.uuid4().hex)')
server_image="goat-fifo-fixture:server-$identity"
client_image="goat-fifo-fixture:client-$identity"
cleanup() {
  docker image rm "$server_image" "$client_image" >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT
case "$(docker info --format '{{.Architecture}}')" in
  aarch64|arm64) fixture_arch=arm64 ;;
  x86_64|amd64) fixture_arch=amd64 ;;
  *) echo 'Unsupported Docker architecture for the integration fixture' >&2; exit 1 ;;
esac
export GOTOOLCHAIN=go1.26.10 GOMAXPROCS=2
git clone -q --depth 1 --branch RELEASE.2024-10-29T16-01-48Z https://github.com/minio/minio.git "$scratch/server"
git clone -q --depth 1 --branch RELEASE.2024-10-29T15-34-59Z https://github.com/minio/mc.git "$scratch/client"
test "$(git -C "$scratch/server" rev-parse HEAD)" = c4239ced225b9fead5f6b44e3665c5ccd7eacc89
test "$(git -C "$scratch/client" rev-parse HEAD)" = 9f4659884dd45dca726ba38ee6bfacb2bf776eb8
(
  cd "$scratch/server"
  CGO_ENABLED=0 GOOS=linux GOARCH="$fixture_arch" go build -mod=readonly -p 2 -o "$scratch/minio" .
)
(
  cd "$scratch/client"
  CGO_ENABLED=0 GOOS=linux GOARCH="$fixture_arch" go build -mod=readonly -p 2 -o "$scratch/mc" .
)
cat > "$scratch/Dockerfile.server" <<'DOCKER'
FROM scratch
COPY minio /minio
ENTRYPOINT ["/minio"]
DOCKER
cat > "$scratch/Dockerfile.client" <<'DOCKER'
FROM scratch
ENV HOME=/root
ENV MC_CONFIG_DIR=/tmp/mc
COPY mc /usr/bin/mc
ENTRYPOINT ["/usr/bin/mc"]
DOCKER
docker build -q -f "$scratch/Dockerfile.server" -t "$server_image" "$scratch"
docker build -q -f "$scratch/Dockerfile.client" -t "$client_image" "$scratch"
python3 "$ROOT/tests/audit_fifo_minio_integration.py" --server-image "$server_image" --mc-image "$client_image"
