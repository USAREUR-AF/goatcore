#!/usr/bin/env bash
# test-certificates.sh — goat-zenoh-edge installs the peering certificates from
# the run's working directory, and refuses bad material before writing anything.
#
# Mints disposable CAs and leaves with openssl in a temporary directory, then runs
# tests/certificates.yml (the role's two certificate task files) on localhost:
#   - good material installs with the right modes, and a re-run changes nothing
#     and restarts nothing; a --check run changes nothing;
#   - a replaced leaf is installed and restarts the edge once;
#   - bad material (a mismatched key, a leaf from another CA or for another
#     peering, a CA used as a leaf, a partner CA that is wrong, not a CA or
#     short-lived, a malformed agreement, and more) and a directory another
#     peering holds each fail with their own message and write nothing, also
#     against a directory an earlier install populated;
#   - two peerings each get their own directory and marker;
#   - a failed restart or a half-done install leaves .restart-pending, and the
#     next run restarts the edge once;
# then tests/main.yml (the whole role) for where the check and the install sit in
# tasks/main.yml, with the switch on and off.
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
leaf() { # $1 out dir, $2 name, $3 cn, $4 signing CA name, [$5 SAN DNS name, $6 basicConstraints]
  openssl ecparam -name prime256v1 -genkey -noout -out "$1/$2-key.pem"
  chmod 0600 "$1/$2-key.pem"
  openssl req -new -key "$1/$2-key.pem" -subj "/CN=$3" -out "$T/$2.csr"
  printf 'subjectAltName=DNS:%s\nbasicConstraints=%s\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth,clientAuth\n' \
    "${5:-$3}" "${6:-CA:FALSE}" > "$T/$2.ext"
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

# --- our CA, from the working directory -------------------------------------------
# When goat_zenoh_edge_our_ca_path is the role's own <cert_dir>/our-ca.pem (its
# default with the switch on), the role installs our CA itself, from
# <fed_work>/pki/our-ca.pem where phase 8 places it, so a driver that is not the
# edge host leaves the edge everything it mounts. Only the CA's certificate: never
# its key.
play_ca() { # $1 work dir, $2 cert dir, rest: extra arguments (no our_ca_path: derived)
  local work="$1" dir="$2"; shift 2
  ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -i localhost, "$HERE/certificates.yml" \
    -e goat_zenoh_edge_partner_id=xab -e goat_zenoh_edge_install_certs=true \
    -e "fed_work=$work" -e "goat_zenoh_edge_cert_dir=$dir" \
    -e "goat_zenoh_edge_cert_owner=$(id -un)" -e "goat_zenoh_edge_cert_group=$(id -gn)" \
    -e "restart_log=$RESTARTS" "$@" > "$LOG" 2>&1
}
no_ca_key() { # $1 cert dir: no file there is our CA's key, and only the two leaf keys hold a key
  local f
  for f in "$1"/* "$1"/.[!.]*; do
    [ -f "$f" ] || continue
    cmp -s "$f" "$T/ours-ca-key.pem" && return 1
    if grep -q 'PRIVATE KEY' "$f"; then
      case "$(basename "$f")" in peering-outward-key.pem|peering-inward-key.pem) ;; *) return 1 ;; esac
    fi
  done
  return 0
}

W="$(case_work our-ca)"; cp "$T/ours-ca.pem" "$W/pki/our-ca.pem"
D="$T/edge/ca/tls"
if play_ca "$W" "$D" && cmp -s "$D/our-ca.pem" "$T/ours-ca.pem" && [ "$(mode_of "$D/our-ca.pem")" = 644 ] \
   && [ "$(mode_of "$D/peering-outward-key.pem")" = 600 ] && [ "$(mode_of "$D/peering-inward-key.pem")" = 600 ] && no_ca_key "$D"; then
  pass "our CA installed from the working directory: the certificate alone, 0644, keys 0600, no CA key on the edge"
else
  fail "our CA from the working directory: not installed, wrong mode, or a CA key on the edge"; ls -la "$D" 2>/dev/null || true; grep -E 'fatal|FAILED' -A3 "$LOG" | head -8 || true
fi
if play_ca "$W" "$D" && [ "$(changed)" = 0 ]; then
  pass "our CA: a re-run changes nothing"
else
  fail "our CA re-run: changed=$(changed)"
fi

W="$(case_work our-ca-with-key)"; cat "$T/ours-ca.pem" "$T/ours-ca-key.pem" > "$W/pki/our-ca.pem"
D="$T/edge/ca-key/tls"
if play_ca "$W" "$D"; then
  fail "our CA source holding a private key: the play passed"
elif grep -qF "holds a private key" "$LOG" && [ ! -e "$D/our-ca.pem" ] && [ ! -e "$D/peering-outward.pem" ] && no_ca_key "$D"; then
  pass "our CA source holding a private key: refused, nothing written"
else
  fail "our CA source holding a private key: wrong message or something written"; grep -E 'fatal' -A3 "$LOG" | head -6 || true
fi

W="$(case_work our-ca-two-certs)"; cat "$T/ours-ca.pem" "$T/root-ca.pem" > "$W/pki/our-ca.pem"
D="$T/edge/ca-two/tls"
if play_ca "$W" "$D"; then
  fail "our CA source holding two certificates: the play passed"
elif grep -qF "is not exactly one certificate" "$LOG" && [ ! -e "$D/our-ca.pem" ] && [ ! -e "$D/peering-outward.pem" ]; then
  pass "our CA source holding two certificates: refused, nothing written"
else
  fail "our CA source holding two certificates: wrong message or something written"; grep -E 'fatal' -A3 "$LOG" | head -6 || true
fi

W="$(case_work our-ca-not-the-cert)"; cp "$T/ours-ca.pem" "$W/pki/our-ca.pem"
D="$T/edge/ca-name/tls"
if play_ca "$W" "$D" -e "goat_zenoh_edge_our_ca_src=$T/ours-ca-key.pem"; then
  fail "our CA source that is not our-ca.pem: the play passed"
elif grep -qF "is not our CA's certificate file" "$LOG" && [ ! -e "$D/our-ca.pem" ] && no_ca_key "$D"; then
  pass "our CA source that is not our-ca.pem: refused, nothing written"
else
  fail "our CA source that is not our-ca.pem: wrong message or something written"; grep -E 'fatal' -A3 "$LOG" | head -6 || true
fi

# The compose file mounts our CA where the role installed it, in fragment mode.
cat > "$T/render.yml" <<'YML'
- hosts: localhost
  connection: local
  gather_facts: false
  vars_files: ["{{ role_dir }}/defaults/main.yml"]
  tasks:
    - ansible.builtin.template:
        src: "{{ role_dir }}/templates/docker-compose.yml.j2"
        dest: "{{ out }}"
YML
if ANSIBLE_NOCOLOR=1 ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 ansible-playbook -i localhost, "$T/render.yml" \
     -e "role_dir=$HERE/.." -e "out=$T/compose.yml" -e goat_zenoh_edge_partner_id=xab -e goat_zenoh_edge_install_certs=true \
     -e "goat_zenoh_edge_cert_dir=$T/edge/ca/tls" -e "goat_zenoh_edge_image=example.org/zenoh@sha256:$(printf '0%.0s' $(seq 64))" \
     > "$T/render.log" 2>&1 && grep -qF -- "- \"$T/edge/ca/tls/our-ca.pem:$T/edge/ca/tls/our-ca.pem:ro\"" "$T/compose.yml"; then
  pass "the compose file mounts the installed our-ca.pem"
else
  fail "the compose file does not mount <cert_dir>/our-ca.pem"; grep -n 'our-ca\|fatal' "$T/compose.yml" "$T/render.log" | head -6 || true
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
expect_refusal "unmarked directory holding other files" "$GOOD" "$T/edge/x11/tls" "not this peering's checked material: somebody-else.pem"

expect_refusal "a certificate path set by hand" "$GOOD" "$T/edge/x9/tls" \
  "Remove goat_zenoh_edge_outward_cert" -e "goat_zenoh_edge_outward_cert=$T/elsewhere.pem"

expect_refusal "a relative certificate directory" "$GOOD" "edge/tls" "must be an absolute"

# --- identity: the leaves the agreement derives, and nothing else ------------------
W="$(case_work wrong-cn)"
leaf "$W/pki" peering-outward fed.xaa.xac.01 ours
expect_refusal "leaf minted for another peering (wrong CN)" "$W" "$T/edge/i1/tls" "but the agreement derives 'CN=fed.xaa.xab.01'"

W="$(case_work swapped)"
leaf "$W/pki" peering-outward fed.xab.xaa.01 ours
expect_refusal "inward identity in the outward slot" "$W" "$T/edge/i2/tls" "has subject 'CN=fed.xab.xaa.01'"

W="$(case_work wrong-san)"
leaf "$W/pki" peering-inward fed.xab.xaa.01 ours fed.xab.xac.01
expect_refusal "leaf whose DNS SAN is not its CN" "$W" "$T/edge/i3/tls" "has no DNS SAN 'fed.xab.xaa.01'"

W="$(case_work ca-true-leaf)"
leaf "$W/pki" peering-outward fed.xaa.xab.01 ours "" CA:TRUE
expect_refusal "leaf with basicConstraints CA:TRUE" "$W" "$T/edge/i4/tls" "must carry basicConstraints CA:FALSE"

W="$(case_work ca-as-leaf)"
cp "$T/ours-ca.pem" "$W/pki/peering-outward.pem"
cp "$T/ours-ca-key.pem" "$W/pki/peering-outward-key.pem"; chmod 0600 "$W/pki/peering-outward-key.pem"
expect_refusal "our CA certificate and its key used as a leaf" "$W" "$T/edge/i5/tls" "is a CA certificate itself"

# --- lifetimes ---------------------------------------------------------------------
# The leaves live 10 days and the CAs 30. An expired file fails the same
# `openssl x509 -checkend` with 0.
expect_refusal "leaf with less lifetime than min_valid" "$GOOD" "$T/edge/l1/tls" \
  "peering-outward: $GOOD/pki/peering-outward.pem expires within 1728000 seconds" \
  -e goat_zenoh_edge_cert_min_valid_seconds=1728000
expect_refusal "partner CA with less lifetime than min_valid" "$GOOD" "$T/edge/l2/tls" \
  "partner CA $GOOD/theirs-ca.pem expires within 3456000 seconds" \
  -e goat_zenoh_edge_cert_min_valid_seconds=3456000

W="$(case_work partner-ca-not-ca)"
leaf "$T" theirs-leaf fed.xab.zz.01 theirs
cp "$T/theirs-leaf.pem" "$W/theirs-ca.pem"
printf '{"parties":[{"sovereign_id":"xaa","ca_fingerprint":"%s"},{"sovereign_id":"xab","ca_fingerprint":"%s"}]}\n' \
  "$(fp "$T/ours-ca.pem")" "$(fp "$T/theirs-leaf.pem")" > "$W/agreement.json"
expect_refusal "partner 'CA' that is not a CA" "$W" "$T/edge/l3/tls" "is not a CA certificate (basicConstraints CA:FALSE)"

# --- a malformed agreement is a refusal, never a traceback --------------------------
bad_agreement() { # $1 label, $2 agreement text, $3 message fragment
  local w; w="$(case_work "ag-$4")"
  printf '%s\n' "$2" > "$w/agreement.json"
  expect_refusal "malformed agreement: $1" "$w" "$T/edge/a-$4/tls" "$3"
  if grep -q Traceback "$LOG"; then fail "malformed agreement: $1: a Python traceback"; fi
}
bad_agreement "not JSON" '{"parties": [' "is not JSON" 1
bad_agreement "a list, not an object" '[1, 2]' "must carry \`parties\` as a list of exactly two objects" 2
bad_agreement "parties as strings" '{"parties":["xaa","xab"]}' "must carry \`parties\` as a list of exactly two objects" 3
bad_agreement "the same id twice" '{"parties":[{"sovereign_id":"xab"},{"sovereign_id":"xab"}]}' "names the same sovereign_id twice" 4
bad_agreement "a party without an id" '{"parties":[{"ca_fingerprint":"x"},{"sovereign_id":"xab"}]}' "every party needs a sovereign_id" 5
bad_agreement "a fingerprint of the wrong shape" '{"parties":[{"sovereign_id":"xaa","ca_fingerprint":"md5:00"},{"sovereign_id":"xab","ca_fingerprint":"md5:00"}]}' "is not sha256:<64 lowercase hex>" 6

expect_refusal "a source directory with a trailing slash" "$GOOD" "$T/edge/s1/tls" "without a trailing '/'" \
  -e "goat_zenoh_edge_cert_src_dir=$GOOD/pki/"

# --- two peerings on one edge, each with its own directory and marker -------------
XAC="$T/xac"; mkdir -p "$XAC/pki"
leaf "$XAC/pki" peering-outward fed.xaa.xac.01 ours
leaf "$XAC/pki" peering-inward fed.xac.xaa.01 ours
cp "$T/rogue-ca.pem" "$XAC/theirs-ca.pem"
printf '{"parties":[{"sovereign_id":"xaa","ca_fingerprint":"%s"},{"sovereign_id":"xac","ca_fingerprint":"%s"}]}\n' \
  "$(fp "$T/ours-ca.pem")" "$(fp "$T/rogue-ca.pem")" > "$XAC/agreement.json"
DIR_XAC="$T/edge/etc/goat-zenoh-edge-xac/tls"
cp "$DIR/peering-outward.pem" "$T/xab-outward-before.pem"
if play "$XAC" "$DIR_XAC" -e goat_zenoh_edge_partner_id=xac \
   && [ "$(cat "$DIR/.peering")" = xab ] && [ "$(cat "$DIR_XAC/.peering")" = xac ] \
   && cmp -s "$DIR/peering-outward.pem" "$T/xab-outward-before.pem" \
   && cmp -s "$DIR_XAC/peering-outward.pem" "$XAC/pki/peering-outward.pem"; then
  pass "a second peering installs in its own directory with its own marker; the first is untouched"
else
  fail "second peering"; grep -E 'refused|fatal' "$LOG" | head -4
fi
if play "$XAC" "$DIR" -e goat_zenoh_edge_partner_id=xac; then
  fail "the second peering was installed over the first one's directory"
elif grep -qF "holds the certificates of the peering with 'xab'" "$LOG" && cmp -s "$DIR/peering-outward.pem" "$T/xab-outward-before.pem"; then
  pass "the second peering is refused the first one's directory, which keeps its files"
else
  fail "second peering over the first one's directory: wrong message or files changed"
fi
if play "$GOOD" "$DIR_XAC"; then
  fail "the first peering's material was installed in the second one's directory"
else
  pass "the first peering is refused the second one's directory"
fi

# --- refusals leave an installed directory exactly as it was ------------------------
snapshot() { (cd "$1" && for f in .peering peering-outward.pem peering-outward-key.pem peering-inward.pem peering-inward-key.pem partner-ca.pem; do
  printf '%s %s ' "$f" "$(mode_of "$f")"; openssl dgst -sha256 -r "$f" | cut -d' ' -f1; done; ls -A); }
BEFORE="$(snapshot "$DIR")"; R0="$(restarts)"
refuse_populated() { # $1 label, $2 work, $3 message fragment
  if play "$2" "$DIR"; then fail "populated directory, $1: the play passed"; return; fi
  if ! grep -qF -- "$3" "$LOG"; then fail "populated directory, $1: no message containing '$3'"; return; fi
  if [ "$(snapshot "$DIR")" != "$BEFORE" ] || [ "$(restarts)" != "$R0" ]; then
    fail "populated directory, $1: the installed files or the restart count changed"; return
  fi
  pass "populated directory, $1: refused, files and restarts unchanged"
}
refuse_populated "mismatched key" "$T/case-mismatched-key" "does not belong to"
refuse_populated "leaf from another CA" "$T/case-wrong-ca" "does not chain to our CA"
refuse_populated "wrong CN" "$T/case-wrong-cn" "but the agreement derives"
refuse_populated "CA used as a leaf" "$T/case-ca-as-leaf" "is a CA certificate itself"
refuse_populated "partner CA with the wrong fingerprint" "$T/case-wrong-fingerprint" "but the agreement names"

# --- adopting a directory with no .peering file -------------------------------------
A1="$T/edge/adopt-same/tls"; mkdir -p "$A1"
cp -p "$T/case-rotated/pki/peering-outward.pem" "$T/case-rotated/pki/peering-outward-key.pem" "$A1/"
cp -p "$GOOD/pki/peering-inward.pem" "$GOOD/pki/peering-inward-key.pem" "$A1/"
cp -p "$T/theirs-ca.pem" "$A1/partner-ca.pem"
if play "$T/case-rotated" "$A1" && [ "$(cat "$A1/.peering")" = xab ]; then
  pass "an unmarked directory holding exactly the checked files is adopted"
else
  fail "adopting byte-identical files"; grep -E 'refused|fatal|msg' "$LOG" | head -4
fi
A2="$T/edge/adopt-older/tls"; mkdir -p "$A2"; cp -p "$GOOD/pki/peering-outward.pem" "$A2/"
expect_refusal_keeps() { # an unmarked directory with an older leaf: refused, untouched
  if play "$T/case-rotated" "$A2"; then fail "unmarked directory with an older leaf: adopted"; return; fi
  if grep -qF "not this peering's checked material: peering-outward.pem" "$LOG" && [ ! -e "$A2/.peering" ] \
     && cmp -s "$A2/peering-outward.pem" "$GOOD/pki/peering-outward.pem"; then
    pass "an unmarked directory holding an older leaf is refused and left as it was"
  else
    fail "unmarked directory with an older leaf: wrong message or files changed"
  fi
}
expect_refusal_keeps

# --- a restart is never lost ---------------------------------------------------------
W="$(case_work restart-fails)"
leaf "$W/pki" peering-outward fed.xaa.xab.01 ours
R0="$(restarts)"
if play "$W" "$DIR" -e restart_fails=true; then
  fail "failing restart: the play passed"
elif [ -e "$DIR/.restart-pending" ] && cmp -s "$DIR/peering-outward.pem" "$W/pki/peering-outward.pem"; then
  if play "$W" "$DIR" && [ "$(restarts)" = $((R0 + 1)) ] && [ ! -e "$DIR/.restart-pending" ]; then
    pass "a failed restart leaves .restart-pending, and the next run (no file changing) restarts and clears it"
  else
    fail "failed restart: the next run restarts=$(restarts) (want $((R0 + 1))), pending=$([ -e "$DIR/.restart-pending" ] && echo yes || echo no)"
  fi
else
  fail "failed restart: no .restart-pending after a changed file"
fi

W="$(case_work half-done)"
leaf "$W/pki" peering-outward fed.xaa.xab.01 ours
leaf "$W/pki" peering-inward fed.xab.xaa.01 ours
R0="$(restarts)"
# A directory where the key goes, holding a directory of the same name: copy
# descends into the first and cannot replace the second.
rm -f "$DIR/peering-inward-key.pem"; mkdir -p "$DIR/peering-inward-key.pem/peering-inward-key.pem"
if play "$W" "$DIR"; then
  fail "half-done install: the play passed"
elif grep -qF "could not install every peering certificate file" "$LOG" && [ -e "$DIR/.restart-pending" ] \
     && [ "$(restarts)" = "$R0" ] && cmp -s "$DIR/peering-outward.pem" "$W/pki/peering-outward.pem"; then
  rm -rf "$DIR/peering-inward-key.pem"
  if play "$W" "$DIR" && [ "$(restarts)" = $((R0 + 1)) ] && [ ! -e "$DIR/.restart-pending" ] \
     && cmp -s "$DIR/peering-inward-key.pem" "$W/pki/peering-inward-key.pem"; then
    pass "a half-done install leaves .restart-pending; the next run completes it and restarts once"
  else
    fail "half-done install: the next run restarts=$(restarts) (want $((R0 + 1)))"
  fi
else
  fail "half-done install: wrong message, no .restart-pending, or a restart"; grep -E 'fatal|msg' "$LOG" | head -4
fi

if play "$W" "$DIR" && [ "$(changed)" = 0 ] && [ "$(restarts)" = $((R0 + 1)) ] && [ ! -e "$DIR/.restart-pending" ]; then
  pass "after the restart, a re-run changes nothing and restarts nothing"
else
  fail "re-run after recovery: changed=$(changed) restarts=$(restarts)"
fi

# --- the whole role: where the check and the install sit in tasks/main.yml ---------
ROLES="$(cd "$HERE/../.." && pwd)"
FILTERS="$(cd "$HERE/../../../filter_plugins" && pwd)"
STACK="..$T/stack"   # /etc/..<T>/stack and /opt/..<T>/stack: inside the test directory
role_play() { # $1 work dir ('' for none), $2 cert dir, rest: extra arguments
  local work="$1" dir="$2"; shift 2
  local fw=(); [ -n "$work" ] && fw=(-e "fed_work=$work")
  ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
  ANSIBLE_ROLES_PATH="$ROLES" ANSIBLE_FILTER_PLUGINS="$FILTERS" \
    ansible-playbook -i localhost, "$HERE/main.yml" ${fw[@]+"${fw[@]}"} \
    -e ansible_become=false -e "rollback_ledger_dir=$T/ledger" \
    -e "rollback_ledger_owner=$(id -un)" -e "rollback_ledger_group=$(id -gn)" \
    -e goat_zenoh_edge_enabled=true -e goat_zenoh_edge_partner_id=xab \
    -e goat_zenoh_edge_partner_cn=fed.xab.xaa.01 -e goat_zenoh_edge_network_mode=bridge \
    -e "goat_zenoh_edge_image=example.org/zenoh@sha256:$(printf '0%.0s' $(seq 64))" \
    -e '{"goat_zenoh_edge_ingress_key_exprs": ["from/xab/**"]}' -e goat_zenoh_edge_ingress_scope_check=false \
    -e "{\"goat_zenoh_edge_core_endpoints\": [\"tls/core.example.org:7447#root_ca_certificate_file=$T/not-mounted.pem\"]}" \
    -e "goat_zenoh_edge_our_ca_path=$T/edge/our-ca.pem" -e "goat_zenoh_edge_stack=$STACK" \
    -e "goat_zenoh_edge_cert_dir=$dir" \
    -e "goat_zenoh_edge_cert_owner=$(id -un)" -e "goat_zenoh_edge_cert_group=$(id -gn)" "$@" > "$LOG" 2>&1
}
LATE="A core endpoint names a CA file this container will not have"

D="$T/edge/role-a/tls"
if role_play "$GOOD" "$D" -e goat_zenoh_edge_install_certs=true; then
  fail "whole role, install on: the deliberate later refusal did not happen"
elif grep -qF "$LATE" "$LOG" && grep -qF "edge certs | check the leaves" "$LOG" && [ ! -e "$D" ] \
     && [ -e "/etc/$STACK/config.json5" ]; then
  pass "whole role, install on: checked first, a later refusal leaves no certificate installed"
else
  fail "whole role, install on: wrong stop, no check, or certificates installed"; grep -E 'fatal|FAILED' -A2 "$LOG" | head -6
fi
rm -rf "$T/stack"

D="$T/edge/role-b/tls"
if role_play "$T/case-mismatched-key" "$D" -e goat_zenoh_edge_install_certs=true; then
  fail "whole role, bad material: the play passed"
elif grep -qF "does not belong to" "$LOG" && [ ! -e "$D" ] && [ ! -e "$T/stack" ]; then
  pass "whole role, bad material: refused before the role's first write"
else
  fail "whole role, bad material: wrong message, or something was written"; grep -E 'fatal' -A2 "$LOG" | head -4
fi

D="$T/edge/role-c/tls"
if role_play "" "$D" -e goat_zenoh_edge_install_certs=false \
     -e "goat_zenoh_edge_outward_cert=$T/hand/o.pem" -e "goat_zenoh_edge_outward_key=$T/hand/o-key.pem" \
     -e "goat_zenoh_edge_inward_cert=$T/hand/i.pem" -e "goat_zenoh_edge_inward_key=$T/hand/i-key.pem" \
     -e "goat_zenoh_edge_partner_ca_path=$T/hand/partner-ca.pem"; then
  fail "whole role, install off: the deliberate later refusal did not happen"
elif grep -qF "$LATE" "$LOG" && ! grep -qF "edge certs |" "$LOG" && [ ! -e "$D" ]; then
  pass "whole role, install off: no certificate task runs, no working directory needed, nothing installed"
else
  fail "whole role, install off: a certificate task ran or the stop was elsewhere"; grep -E 'fatal|edge certs' "$LOG" | head -6
fi
rm -rf "$T/stack"

if [ "$FAILS" -eq 0 ]; then
  echo "goat-zenoh-edge test-certificates: PASS"
else
  echo "goat-zenoh-edge test-certificates: $FAILS FAILED"
  exit 1
fi
