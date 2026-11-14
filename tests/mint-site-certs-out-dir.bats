#!/usr/bin/env bats
# Behavior tests for where ops/scripts/mint-site-certs.sh leaves its output.
#
# Plays read a leaf from the manifest's out_dir through the goat_leaf filter
# (#2165), so the mint writes the pair there and copies nothing into the
# checkout: a fresh worktree finds the leaf, and no private key sits in a
# working tree. tests/ansible_filter_plugins/test_goat_leaf.py covers the reader.
#
# The script resolves REPO_ROOT from its own location, so each test copies it
# into a throwaway tree and swaps cert-sign for a stub.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
SCRIPT="$REPO_ROOT/ops/scripts/mint-site-certs.sh"

setup() {
  command -v yq >/dev/null || skip "yq not installed"
  FAKE="$BATS_TEST_TMPDIR/repo"
  OUT="$BATS_TEST_TMPDIR/leaves"
  mkdir -p "$FAKE/ops/scripts" "$FAKE/ops/enrollment/manifests" "$BATS_TEST_TMPDIR/bin"
  cp "$SCRIPT" "$FAKE/ops/scripts/mint-site-certs.sh"
  for f in root-cert.pem root-key.age age.txt; do : > "$BATS_TEST_TMPDIR/$f"; done

  # cert-sign stub: write a cert and a 0400 key at the requested paths.
  cat > "$BATS_TEST_TMPDIR/bin/cert-sign" <<'EOF'
#!/usr/bin/env bash
while [ $# -gt 0 ]; do
  case "$1" in
    --out-cert) cert="$2"; shift 2 ;;
    --out-key)  key="$2"; shift 2 ;;
    *) shift ;;
  esac
done
echo CERT > "$cert"
echo KEY > "$key"; chmod 0400 "$key"
EOF
  chmod +x "$BATS_TEST_TMPDIR/bin/cert-sign"
  export CERT_SIGN="$BATS_TEST_TMPDIR/bin/cert-sign"
}

manifest() {
  {
    cat <<EOF
site_id: testsite
out_dir: $OUT
ca:
  root_cert: $BATS_TEST_TMPDIR/root-cert.pem
  root_key: $BATS_TEST_TMPDIR/root-key.age
  age_identity: $BATS_TEST_TMPDIR/age.txt
  ca_id: dev-testsite-ca
leaves:
EOF
    for n in "$@"; do
      printf '  - name: %s\n    cn: %s.testsite\n    sans: [%s.testsite]\n' "$n" "$n" "$n"
    done
  } > "$FAKE/ops/enrollment/manifests/testsite-certs.yaml"
}

@test "mint-site-certs: leaves land in out_dir, nothing is copied into the checkout" {
  manifest router mgmt-identity dashboard-a fabric-health
  run "$FAKE/ops/scripts/mint-site-certs.sh" testsite
  [ "$status" -eq 0 ]
  for n in router mgmt-identity dashboard-a fabric-health; do
    [ -f "$OUT/$n-cert.pem" ]
    [ -f "$OUT/$n-key.pem" ]
  done
  [ ! -e "$FAKE/ops/enrollment/testsite" ]
  [[ "$output" == *"out=$OUT"* ]]
  [[ "$output" != *"staged"* ]]
}

@test "mint-site-certs: --dry-run writes nothing" {
  manifest router mgmt-identity
  run "$FAKE/ops/scripts/mint-site-certs.sh" testsite --dry-run
  [ "$status" -eq 0 ]
  [ ! -e "$FAKE/ops/enrollment/testsite" ]
  [ ! -e "$OUT/router-key.pem" ]
}
