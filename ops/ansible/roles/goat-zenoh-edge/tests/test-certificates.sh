#!/usr/bin/env bash
# test-certificates.sh — goat-zenoh-edge installs the peering certificates from
# the run's working directory, and refuses bad material before writing anything.
#
# Mints disposable CAs and leaves with openssl in a temporary directory, then runs
# tests/certificates.yml (the role's tasks/certificates.yml) on localhost:
#   - good material installs with the right modes, and a re-run changes nothing
#     and restarts nothing; a --check run changes nothing;
#   - a replaced leaf is installed and restarts the edge once;
#   - a mismatched key, a leaf from another CA, a partner CA with the wrong
#     fingerprint, a partner CA file with two certificates, a key readable by
#     others, a directory another peering holds and a path set by hand each
#     fail with their own message, and leave the certificate directory unwritten.
#
# Usage:  ops/ansible/roles/goat-zenoh-edge/tests/test-certificates.sh
# Exit:   0 = pass (or SKIP when ansible-playbook or openssl is missing); 1 = fail.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
for tool in ansible-playbook openssl; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "goat-zenoh-edge test-certificates: SKIP ($tool not on PATH)"
    exit 0
  fi
done

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
FAILS=0
pass() { echo "  PASS  $1"; }
fail() { echo "  FAIL  $1"; FAILS=$((FAILS + 1)); }

# --- material ------------------------------------------------------------------
ca() { # $1 name
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 30 \
    -subj "/CN=test-$1-ca" -keyout "$T/$1-ca-key.pem" -out "$T/$1-ca.pem" \
    -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign,cRLSign 2>/dev/null
}
leaf() { # $1 out dir, $2 name, $3 cn, $4 signing CA name
  openssl ecparam -name prime256v1 -genkey -noout -out "$1/$2-key.pem"
  chmod 0600 "$1/$2-key.pem"
  openssl req -new -key "$1/$2-key.pem" -subj "/CN=$3" -out "$T/$2.csr"
  printf 'subjectAltName=DNS:%s\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth,clientAuth\n' "$3" > "$T/$2.ext"
  openssl x509 -req -in "$T/$2.csr" -CA "$T/$4-ca.pem" -CAkey "$T/$4-ca-key.pem" -set_serial "0x$(openssl rand -hex 8)" \
    -days 10 -sha256 -extfile "$T/$2.ext" -out "$1/$2.pem" 2>/dev/null
}
fp() { echo "sha256:$(openssl x509 -in "$1" -outform DER | openssl dgst -sha256 -r | cut -d' ' -f1)"; }

ca ours; ca theirs; ca rogue; ca root
# An intermediate under a shared root, as an operator bundle can name our CA.
openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -subj /CN=test-mid-ca \
  -keyout "$T/mid-ca-key.pem" -out "$T/mid.csr" 2>/dev/null
printf 'basicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n' > "$T/mid.ext"
openssl x509 -req -in "$T/mid.csr" -CA "$T/root-ca.pem" -CAkey "$T/root-ca-key.pem" -set_serial 0x1001 \
  -days 30 -sha256 -extfile "$T/mid.ext" -out "$T/mid-ca.pem" 2>/dev/null
GOOD="$T/good"; mkdir -p "$GOOD/pki"
leaf "$GOOD/pki" peering-outward fed.xaa.xab.01 ours
leaf "$GOOD/pki" peering-inward fed.xab.xaa.01 ours
cp "$T/theirs-ca.pem" "$GOOD/theirs-ca.pem"
printf '{"parties":[{"sovereign_id":"xaa","ca_fingerprint":"%s"},{"sovereign_id":"xab","ca_fingerprint":"%s"}]}\n' \
  "$(fp "$T/ours-ca.pem")" "$(fp "$T/theirs-ca.pem")" > "$GOOD/agreement.json"
mkdir -p "$T/edge"; cp "$T/ours-ca.pem" "$T/edge/our-ca.pem"

case_work() { # $1 case name -> a copy of the good working directory
  rm -rf "$T/case-$1"; cp -Rp "$GOOD" "$T/case-$1"; echo "$T/case-$1"
}

LOG="$T/play.log"; RESTARTS="$T/restarts"; : > "$RESTARTS"
play() { # $1 work dir, $2 cert dir, rest: extra ansible-playbook arguments
  local work="$1" dir="$2"; shift 2
  ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -i localhost, "$HERE/certificates.yml" \
    -e goat_zenoh_edge_partner_id=xab -e goat_zenoh_edge_install_certs=true \
    -e "fed_work=$work" -e "goat_zenoh_edge_cert_dir=$dir" \
    -e "goat_zenoh_edge_our_ca_path=$T/edge/our-ca.pem" \
    -e "goat_zenoh_edge_cert_owner=$(id -un)" -e "goat_zenoh_edge_cert_group=$(id -gn)" \
    -e "restart_log=$RESTARTS" "$@" > "$LOG" 2>&1
}
mode_of() { if stat -c %a "$1" >/dev/null 2>&1; then stat -c %a "$1"; else stat -f %Lp "$1"; fi; }
changed() { sed -n 's/.*changed=\([0-9]*\).*/\1/p' "$LOG" | head -1; }
restarts() { wc -l < "$RESTARTS" | tr -d ' '; }

expect_refusal() { # $1 label, $2 work, $3 cert dir, $4 message fragment, rest: extra args
  local label="$1" work="$2" dir="$3" want="$4"; shift 4
  if play "$work" "$dir" "$@"; then
    fail "$label: the play passed"; return
  fi
  if ! grep -qF -- "$want" "$LOG"; then
    fail "$label: no message containing '$want'"; grep -E -A3 'FAILED|fatal' "$LOG" | head -8; return
  fi
  if [ -e "$dir/peering-outward.pem" ] || [ -e "$dir/partner-ca.pem" ] || [ -e "$dir/peering-outward-key.pem" ]; then
    fail "$label: certificate files were written before the refusal"; return
  fi
  pass "$label"
}

# --- good material ---------------------------------------------------------------
echo "goat-zenoh-edge test-certificates"
DIR="$T/edge/etc/goat-zenoh-edge-xab/tls"
if play "$GOOD" "$DIR"; then
  ok=1
  for f in peering-outward.pem peering-inward.pem partner-ca.pem; do [ "$(mode_of "$DIR/$f")" = 644 ] || ok=0; done
  for f in peering-outward-key.pem peering-inward-key.pem; do [ "$(mode_of "$DIR/$f")" = 600 ] || ok=0; done
  [ "$(mode_of "$DIR")" = 750 ] || ok=0
  [ "$(cat "$DIR/.peering")" = xab ] || ok=0
  cmp -s "$DIR/partner-ca.pem" "$T/theirs-ca.pem" || ok=0
  cmp -s "$DIR/peering-inward-key.pem" "$GOOD/pki/peering-inward-key.pem" || ok=0
  [ "$(restarts)" = 1 ] || ok=0
  if [ "$ok" = 1 ]; then pass "good material installs, modes 0644/0600/0750, one restart"; else fail "good material: wrong files, modes or restart count ($(restarts))"; ls -la "$DIR"; fi
else
  fail "good material: the play failed"; tail -20 "$LOG"
fi

if play "$GOOD" "$DIR" && [ "$(changed)" = 0 ] && [ "$(restarts)" = 1 ]; then
  pass "re-run changes nothing and restarts nothing"
else
  fail "re-run: changed=$(changed) restarts=$(restarts)"
fi

if play "$GOOD" "$DIR" --check && [ "$(changed)" = 0 ]; then
  pass "--check run passes the checks and changes nothing"
else
  fail "--check run: changed=$(changed)"; tail -10 "$LOG"
fi

W="$(case_work rotated)"
leaf "$W/pki" peering-outward fed.xaa.xab.01 ours
if play "$W" "$DIR" && [ "$(restarts)" = 2 ] && cmp -s "$DIR/peering-outward.pem" "$W/pki/peering-outward.pem"; then
  pass "a replaced leaf is installed and restarts the edge once"
else
  fail "replaced leaf: restarts=$(restarts)"
fi

W2="$(case_work second)"
if play "$W2" "$T/edge/etc/goat-zenoh-edge-xac/tls" \
   && [ -s "$DIR/peering-outward.pem" ] && [ -s "$T/edge/etc/goat-zenoh-edge-xac/tls/peering-outward.pem" ]; then
  pass "a second certificate directory leaves the first one's files in place"
else
  fail "second directory"
fi

W="$(case_work intermediate)"
leaf "$W/pki" peering-outward fed.xaa.xab.01 mid
leaf "$W/pki" peering-inward fed.xab.xaa.01 mid
printf '{"parties":[{"sovereign_id":"xaa","ca_fingerprint":"%s"},{"sovereign_id":"xab","ca_fingerprint":"%s"}]}\n' \
  "$(fp "$T/mid-ca.pem")" "$(fp "$T/theirs-ca.pem")" > "$W/agreement.json"
cat "$T/mid-ca.pem" "$T/root-ca.pem" > "$T/edge/mid-bundle.pem"
if play "$W" "$T/edge/mid/tls" -e "goat_zenoh_edge_our_ca_path=$T/edge/mid-bundle.pem"; then
  pass "our CA named as an intermediate inside a bundle"
else
  fail "intermediate CA: the play failed"; grep -E 'refused' "$LOG" | head -4
fi

# --- refusals ---------------------------------------------------------------------
W="$(case_work mismatched-key)"
openssl ecparam -name prime256v1 -genkey -noout -out "$W/pki/peering-outward-key.pem"; chmod 0600 "$W/pki/peering-outward-key.pem"
expect_refusal "mismatched key" "$W" "$T/edge/x1/tls" "does not belong to"

W="$(case_work wrong-ca)"
leaf "$W/pki" peering-inward fed.xab.xaa.01 rogue
expect_refusal "leaf from another CA" "$W" "$T/edge/x2/tls" "does not chain to our CA"

W="$(case_work wrong-fingerprint)"
cp "$T/rogue-ca.pem" "$W/theirs-ca.pem"
expect_refusal "partner CA with the wrong fingerprint" "$W" "$T/edge/x3/tls" "but the agreement names"

W="$(case_work two-certs)"
cat "$T/theirs-ca.pem" "$T/rogue-ca.pem" > "$W/theirs-ca.pem"
expect_refusal "partner CA file with two certificates" "$W" "$T/edge/x4/tls" "holds 2 certificates"

W="$(case_work pinned-fingerprint)"
expect_refusal "pinned fingerprint that differs from the agreement" "$W" "$T/edge/x5/tls" \
  "goat_zenoh_edge_partner_ca_fingerprint is" -e "goat_zenoh_edge_partner_ca_fingerprint=$(fp "$T/rogue-ca.pem")"

W="$(case_work key-mode)"
chmod 0644 "$W/pki/peering-inward-key.pem"
expect_refusal "key readable by others" "$W" "$T/edge/x6/tls" "a private key must be 0600"

W="$(case_work missing)"
rm "$W/pki/peering-inward.pem"
expect_refusal "missing leaf" "$W" "$T/edge/x7/tls" "Run phase 8"

mkdir -p "$T/edge/x8/tls"; echo xac > "$T/edge/x8/tls/.peering"
expect_refusal "directory another peering holds" "$GOOD" "$T/edge/x8/tls" "holds the certificates of the peering with 'xac'"

W="$(case_work two-leaves)"
cat "$T/rogue-ca.pem" >> "$W/pki/peering-outward.pem"
expect_refusal "leaf file with two certificates" "$W" "$T/edge/x10/tls" "holds 2 certificates; it must hold exactly the one leaf"

mkdir -p "$T/edge/x11/tls"; echo other > "$T/edge/x11/tls/somebody-else.pem"
expect_refusal "unmarked directory holding other files" "$GOOD" "$T/edge/x11/tls" "holds files this role does not install: somebody-else.pem"

expect_refusal "a certificate path set by hand" "$GOOD" "$T/edge/x9/tls" \
  "Remove goat_zenoh_edge_outward_cert" -e "goat_zenoh_edge_outward_cert=$T/elsewhere.pem"

expect_refusal "a relative certificate directory" "$GOOD" "edge/tls" "must be an absolute"

if [ "$FAILS" -eq 0 ]; then
  echo "goat-zenoh-edge test-certificates: PASS"
else
  echo "goat-zenoh-edge test-certificates: $FAILS FAILED"
  exit 1
fi
