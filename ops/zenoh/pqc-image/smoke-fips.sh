#!/bin/sh
# smoke-fips.sh — boot test of a locally loaded zenoh-pqc-fips runtime image.
#
#   sh ops/zenoh/pqc-image/smoke-fips.sh <runtime-image>
#
# Boots <runtime-image> with the full baked plugin set over a real TLS
# listener on the host network and fails unless:
#   - no plugin-ABI or boot failure is logged (F-122/F-136 class),
#   - the "goat FIPS: AWS-LC FIPS module active" boot marker is logged,
#   - the rest plugin answers on :8000 (zenohd and its plugins are live),
#   - fips-probe --connect negotiates X25519MLKEM768 with the running zenohd
#     under the FIPS provider.
#
# The probe is the `probe` stage of Dockerfile.fips beside this script. Set
# PROBE_IMAGE to a loaded build of that stage to use it; otherwise the script
# builds it with `docker buildx build --target probe --load` in the current
# builder, which already holds the builder stage from the runtime build.
#
# Called by .github/workflows/zenoh-pqc-fips-image.yml before any push, and by
# any pipeline that builds this image from the same tree. Needs docker with
# buildx, openssl and curl; uses ports 7447 and 8000 on the host.
set -eu

IMAGE=${1:?usage: smoke-fips.sh <runtime-image>}
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
NAME=zenoh-pqc-fips-smoke-$$

cleanup() {
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

if [ -z "${PROBE_IMAGE:-}" ]; then
  PROBE_IMAGE=zenoh-pqc-fips-probe:smoke-$$
  docker buildx build --load --target probe \
    -f "$HERE/Dockerfile.fips" -t "$PROBE_IMAGE" "$HERE"
fi

# Throwaway P-256 server cert (same curve as the goat CA chain).
# basicConstraints CA:FALSE — `req -x509` defaults to CA:TRUE and webpki
# refuses a CA cert as end-entity (CaUsedAsEndEntity).
mkdir -p "$WORK/certs"
openssl ecparam -name prime256v1 -genkey -noout -out "$WORK/certs/key.pem"
openssl req -new -x509 -key "$WORK/certs/key.pem" -out "$WORK/certs/cert.pem" \
  -days 2 -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
  -addext "basicConstraints=critical,CA:FALSE" \
  -addext "keyUsage=digitalSignature" \
  -addext "extendedKeyUsage=serverAuth"
chmod 755 "$WORK" "$WORK/certs"
chmod 644 "$WORK"/certs/*

cat > "$WORK/smoke.json5" <<'EOF'
{
  mode: "router",
  listen: { endpoints: ["tls/0.0.0.0:7447"] },
  transport: { link: { tls: {
    listen_certificate: "/certs/cert.pem",
    listen_private_key: "/certs/key.pem",
  } } },
  plugins_loading: { enabled: true, search_dirs: ["/"] },
  plugins: {
    rest: { http_port: 8000 },
    storage_manager: {
      backend_search_dirs: ["/opt/zenoh/plugins-baked"],
      volumes: { rocksdb: {} },
      storages: { smoke: { key_expr: "smoke/**", volume: { id: "rocksdb", dir: "smoke", create_db: true } } },
    },
  },
}
EOF
chmod 644 "$WORK/smoke.json5"

docker run -d --name "$NAME" --network host \
  -v "$WORK/smoke.json5:/smoke.json5:ro" \
  -v "$WORK/certs:/certs:ro" \
  "$IMAGE" -c /smoke.json5
sleep 8
docker logs "$NAME" > "$WORK/smoke.log" 2>&1
cat "$WORK/smoke.log"

if grep -qi 'compatibility mismatch\|Cannot spawn\|panic' "$WORK/smoke.log"; then
  echo "::error::plugin ABI or boot failure"; exit 1
fi
grep -q 'goat FIPS: AWS-LC FIPS module active' "$WORK/smoke.log" \
  || { echo "::error::FIPS boot marker missing"; exit 1; }

# rest plugin answering == zenohd + plugins all live
curl -sSf -o "$WORK/router.json" http://127.0.0.1:8000/@/local/router
head -c 300 "$WORK/router.json"; echo

# THE assertion: a live handshake against the booted zenohd negotiates the
# hybrid PQ group under the FIPS provider.
docker run --rm --network host -v "$WORK/certs:/certs:ro" \
  "$PROBE_IMAGE" --connect 127.0.0.1:7447 localhost /certs/cert.pem

echo "smoke-fips: $IMAGE passed (FIPS marker, plugin ABI, rest, X25519MLKEM768)"
