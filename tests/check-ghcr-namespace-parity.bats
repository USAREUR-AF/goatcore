#!/usr/bin/env bats
# Behavior tests for ops/ci/check-ghcr-namespace-parity.sh with ghcr stubbed.
#
# A curl stub on PATH plays ghcr: the token endpoint and the manifest endpoint
# answer per namespace from $STUB_<ORG>_{TOKEN,MANIFEST} codes. These tests pin
# that a 401/403 reads as DENIED (private to the credential), never as MISSING,
# and exits 75 rather than 1 so ansible-preflight does not send the operator to
# --backfill or GHCR_ORG=<other> (#386).

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
SCRIPT="$REPO_ROOT/ops/ci/check-ghcr-namespace-parity.sh"
DIGEST="sha256:$(printf '0%.0s' $(seq 1 64))"

# This tree may not carry the code a test reads (a public goatcore checkout
# does not carry a goatnet's own scripts); such a test is skipped by name,
# and runs wherever that code is present.
setup() {
  [ -f "$SCRIPT" ] || skip "ops/ci/check-ghcr-namespace-parity.sh is not in this tree"
  command -v python3 >/dev/null || skip "python3 not installed"
  FAKE="$BATS_TEST_TMPDIR/repo"
  mkdir -p "$FAKE/ops/ci" "$FAKE/ops/ansible" "$FAKE/ops/pins" "$BATS_TEST_TMPDIR/bin"
  cp "$SCRIPT" "$FAKE/ops/ci/"
  printf 'img: ghcr.io/{{ ghcr_org }}/zenoh-pqc@%s\n' "$DIGEST" > "$FAKE/ops/ansible/vars.yml"
  : > "$FAKE/ops/pins/fleet-pins.yaml"
  git -C "$FAKE" init -q

  cat > "$BATS_TEST_TMPDIR/bin/curl" <<'EOF'
#!/usr/bin/env bash
# Minimal ghcr stand-in: honours -o <file> and -w '%{http_code}'.
out=/dev/stdout; url=""; want_code=0
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -w) want_code=1; shift 2 ;;
    -u|-H) shift 2 ;;
    http*) url="$1"; shift ;;
    *) shift ;;
  esac
done
case "$url" in
  *repository:usareur-af/*) org=USAREUR-AF ;;
  *repository:goat-maintainer/*)    org=DLF ;;
  */v2/usareur-af/*)        org=USAREUR-AF ;;
  */v2/goat-maintainer/*)           org=DLF ;;
esac
case "$url" in
  */token*) var="STUB_${org}_TOKEN"; code="${!var:-200}"
            [ "$code" = 200 ] && echo '{"token":"t"}' > "$out" \
              || echo '{"errors":[{"code":"DENIED","message":"permission_denied: read_package"}]}' > "$out" ;;
  *)        var="STUB_${org}_MANIFEST"; code="${!var:-200}" ;;
esac
[ "$want_code" = 1 ] && printf '%s' "$code"
case "$code" in 2??) exit 0 ;; *) exit 22 ;; esac   # curl -f exit code
EOF
  chmod +x "$BATS_TEST_TMPDIR/bin/curl"
  export PATH="$BATS_TEST_TMPDIR/bin:$PATH" GHCR_TOKEN=stub
}

run_parity() { cd "$FAKE" && run ./ops/ci/check-ghcr-namespace-parity.sh "$@"; }

@test "ghcr-parity: both namespaces 200 passes" {
  run_parity
  [ "$status" -eq 0 ]
  [[ "$output" == *"ok: all 1 pinned reference(s) resolve"* ]]
}

@test "ghcr-parity: a 403 manifest under the mirror is DENIED, not MISSING, and exits 75" {
  export STUB_DLF_MANIFEST=403
  run_parity
  [ "$status" -eq 75 ]
  [[ "$output" == *"DENIED under goat-maintainer"*"zenoh-pqc"* ]]
  [[ "$output" != *"MISSING under"* ]]
  [[ "$output" == *"denied:usareur-af=0,goat-maintainer=1"* ]]
}

@test "ghcr-parity: a 403 from the token endpoint is DENIED too" {
  export STUB_GOAT_TOKEN=403
  run_parity --require usareur-af
  [ "$status" -eq 75 ]
  [[ "$output" == *"DENIED under usareur-af"* ]]
  [[ "$output" != *"FAIL:"* ]]
}

@test "ghcr-parity: --require on the readable side passes despite DENIED on the other" {
  export STUB_DLF_MANIFEST=403
  run_parity --require usareur-af
  [ "$status" -eq 0 ]
  [[ "$output" == *"ok: every pinned reference resolves under usareur-af"* ]]
}

@test "ghcr-parity: a 404 is still MISSING and exits 1" {
  export STUB_DLF_MANIFEST=404
  run_parity
  [ "$status" -eq 1 ]
  [[ "$output" == *"MISSING under goat-maintainer"* ]]
  [[ "$output" != *"DENIED under"* ]]
}
