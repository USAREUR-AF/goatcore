#!/usr/bin/env bash
# test-rotation.sh — the IPsec leaf of a peering is rotated from material staged
# beside the live files (K6, docs/design/federation-key-rotation.md §8). Runs
# tasks/rotate.yml of goat-peering-edge on localhost against a stand-in swanctl,
# with certificates made at test time (nothing here is real key material).
#
#   - a staged key that is not the certificate's, a wrong CN, a certificate from
#     another CA, an RSA key, and one that expires within the minimum days are each
#     refused (GB-290), and nothing on disk changes; missing material is GB-289;
#   - a good pair is set aside once per id and switched in, with one load of the
#     credentials and one re-authentication; a rerun with the same id moves nothing;
#   - a run stopped after the set-aside, and one stopped after the switch, finish
#     without a second set-aside or move;
#   - when the SA never comes up the old pair is put back, and the run stops (GB-302);
#   - an ipsec-ca rotation is refused without the partner's read-back of the new CA
#     (GB-303) and switches with it; the new CA replaces the one the chain is
#     checked against;
#   - the retire step removes the set-aside key and keeps its certificate;
#   - swanctl.conf renders byte for byte as before without the partner's next CA,
#     and lists the current CA and then the next one with it.
#
# Usage:  ops/ansible/roles/goat-peering-edge/tests/test-rotation.sh
# Exit:   0 = pass (or SKIP when ansible-playbook or openssl is missing); 1 = fail.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
for c in ansible-playbook openssl; do
  if ! command -v "$c" >/dev/null 2>&1; then
    echo "goat-peering-edge test-rotation: SKIP ($c not on PATH)"
    exit 0
  fi
done

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
FAILS=0
pass() { echo "  PASS  $1"; }
fail() { echo "  FAIL  $1"; FAILS=$((FAILS + 1)); }
sha() { openssl sha256 -r < "$1" | cut -d' ' -f1; }

# The stand-in swanctl: records each call; --list-sas reports an established SA
# with an installed CHILD SA only while $T/sa-up exists.
mkdir -p "$T/bin"
cat > "$T/bin/swanctl" <<SH
#!/bin/sh
echo "\$*" >> "$T/swanctl.log"
case "\$1" in
  --list-sas) [ -e "$T/sa-up" ] && printf 'xab: #1, ESTABLISHED\n  fabric: #1, INSTALLED\n'; exit 0 ;;
  *) exit 0 ;;
esac
SH
chmod 755 "$T/bin/swanctl"

# Certificates made at test time: two CAs, then leaves.
P384=(-newkey ec -pkeyopt ec_paramgen_curve:secp384r1 -nodes)
ca() { openssl req -x509 "${P384[@]}" -keyout "$T/pki/$1.key" -out "$T/pki/$1.pem" -days 30 -subj "/CN=$1" 2>/dev/null; }
leaf() { # leaf <name> <ca> <cn> <days> [rsa]
  local kopt=("${P384[@]}")
  [ "${5:-}" = rsa ] && kopt=(-newkey rsa:2048 -nodes)
  openssl req "${kopt[@]}" -keyout "$T/pki/$1.key" -out "$T/pki/$1.csr" -subj "/CN=$3" 2>/dev/null
  openssl x509 -req -in "$T/pki/$1.csr" -CA "$T/pki/$2.pem" -CAkey "$T/pki/$2.key" -CAcreateserial \
    -out "$T/pki/$1.pem" -days "$4" 2>/dev/null
}
mkdir -p "$T/pki"
ca ca1; ca ca2
leaf live ca1 fed.xaa.xab.01 90
leaf good ca1 fed.xaa.xab.01 90
leaf good2 ca1 fed.xaa.xab.01 90
leaf ca2leaf ca2 fed.xaa.xab.01 90
leaf wrongcn ca1 fed.xaa.xzz.01 90
leaf otherca ca2 fed.xaa.xab.01 90
leaf rsa ca1 fed.xaa.xab.01 90 rsa
leaf short ca1 fed.xaa.xab.01 10

S="$T/swanctl"
fresh() { # a fresh edge: live leaf of ca1, our CA ca1, nothing staged
  rm -rf "$S" "$T/swanctl.log" "$T/sa-up"
  mkdir -p "$S/x509" "$S/private" "$S/x509ca"
  cp "$T/pki/live.pem" "$S/x509/xab-cert.pem"; cp "$T/pki/live.key" "$S/private/xab-key.pem"
  chmod 600 "$S/private/xab-key.pem"
  cp "$T/pki/ca1.pem" "$T/local-ca.pem"
}
stage() { # stage <leaf name>
  cp "$T/pki/$1.pem" "$S/x509/xab-cert.next.pem"; cp "$T/pki/$1.key" "$S/private/xab-key.next.pem"
  chmod 600 "$S/private/xab-key.next.pem"
}
LOG="$T/play.log"
rot() { # rest: extra arguments
  ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -i localhost, "$HERE/rotation.yml" -e "stand_in_bin=$T/bin" \
    -e peering_edge_name=xab -e peering_edge_local_id=fed.xaa.xab.01 -e "peering_edge_swanctl_dir=$S" \
    -e "peering_edge_local_ca_path=$T/local-ca.pem" -e peering_edge_rotation_confirm_seconds=2 "$@" > "$LOG" 2>&1
}
live_is() { [ "$(sha "$S/x509/xab-cert.pem")" = "$(sha "$T/pki/$1.pem")" ] && [ "$(sha "$S/private/xab-key.pem")" = "$(sha "$T/pki/$1.key")" ]; }
calls() { grep -c -- "$1" "$T/swanctl.log" 2>/dev/null || true; }
snap() { find "$S" -type f -exec openssl sha256 -r {} + 2>/dev/null | sort; }

echo "goat-peering-edge test-rotation"

# Refusals: nothing on disk changes.
for bad in good-mismatched wrongcn otherca rsa short; do
  fresh
  if [ "$bad" = good-mismatched ]; then stage good; cp "$T/pki/good2.key" "$S/private/xab-key.next.pem"; else stage "$bad"; fi
  before=$(snap)
  if ! rot -e peering_edge_rotation=r1 && grep -q "GB-290" "$LOG" && [ "$(snap)" = "$before" ] && [ ! -e "$S/.previous-r1" ]; then
    pass "refused ($bad): GB-290, nothing moved"
  else
    fail "not refused, or something moved ($bad)"; tail -12 "$LOG"
  fi
done

fresh
before=$(snap)
if ! rot -e peering_edge_rotation=r1 && grep -q "GB-289" "$LOG" && [ "$(snap)" = "$before" ]; then
  pass "refused: nothing staged is GB-289, nothing moved"
else
  fail "nothing staged was not refused with GB-289"; tail -12 "$LOG"
fi

fresh; stage good; chmod 644 "$S/private/xab-key.next.pem"
if ! rot -e peering_edge_rotation=r1 && grep -q "GB-289" "$LOG"; then
  pass "refused: a staged key not at mode 0600 is GB-289"
else
  fail "a staged key at 0644 was not refused"; tail -12 "$LOG"
fi

if ! rot -e '{"peering_edge_rotation": "bad id"}' && grep -q "is not a rotation id" "$LOG"; then
  pass "refused: a rotation id that is not a plain word"
else
  fail "a bad rotation id was not refused"; tail -12 "$LOG"
fi

# A good pair: set aside once, switched, one load and one re-authentication.
fresh; stage good; touch "$T/sa-up"
if rot -e peering_edge_rotation=r1 && live_is good && [ -f "$S/.previous-r1/xab-key.pem" ] \
   && [ "$(sha "$S/.previous-r1/xab-cert.pem")" = "$(sha "$T/pki/live.pem")" ] \
   && [ "$(stat -c %a "$S/.previous-r1" 2>/dev/null || stat -f %Lp "$S/.previous-r1")" = 700 ] \
   && [ ! -e "$S/x509/xab-cert.next.pem" ] && [ "$(calls --load-creds)" = 1 ] && [ "$(calls '--reauth')" = 1 ]; then
  pass "a good pair is set aside (0700) and switched in, one load and one re-authentication"
else
  fail "the good pair was not switched as designed"; tail -15 "$LOG"; cat "$T/swanctl.log" 2>/dev/null || true
fi
aside=$(snap)
if rot -e peering_edge_rotation=r1 && live_is good && [ "$(snap)" = "$aside" ]; then
  pass "a rerun with the same id moves nothing"
else
  fail "a rerun with the same id changed files"; tail -12 "$LOG"
fi

# Stopped after the set-aside (aside present, staged still there): finishes, no second set-aside.
fresh; stage good; touch "$T/sa-up"
mkdir -m 700 "$S/.previous-r2"; cp -p "$S/x509/xab-cert.pem" "$S/private/xab-key.pem" "$S/.previous-r2/"
if rot -e peering_edge_rotation=r2 && live_is good && [ "$(sha "$S/.previous-r2/xab-cert.pem")" = "$(sha "$T/pki/live.pem")" ] \
   && [ "$(calls --load-creds)" = 1 ]; then
  pass "stopped after the set-aside: finishes without setting aside the new pair"
else
  fail "a run stopped after the set-aside did not finish cleanly"; tail -12 "$LOG"
fi

# Stopped after the switch (files switched, staged gone, aside present): loads and re-authenticates, moves nothing.
fresh; touch "$T/sa-up"
mkdir -m 700 "$S/.previous-r3"; cp -p "$S/x509/xab-cert.pem" "$S/private/xab-key.pem" "$S/.previous-r3/"
cp "$T/pki/good.pem" "$S/x509/xab-cert.pem"; cp "$T/pki/good.key" "$S/private/xab-key.pem"
before=$(snap)
if rot -e peering_edge_rotation=r3 && live_is good && [ "$(snap)" = "$before" ] \
   && [ "$(calls --load-creds)" = 1 ] && [ "$(calls '--reauth')" = 1 ]; then
  pass "stopped after the switch: loads and re-authenticates once, moves nothing"
else
  fail "a run stopped after the switch did not finish cleanly"; tail -12 "$LOG"
fi

# The SA never comes up: the old pair is put back, GB-302.
fresh; stage good
if ! rot -e peering_edge_rotation=r4 && grep -q "GB-302" "$LOG" && live_is live && [ "$(calls '--reauth')" = 2 ]; then
  pass "SA not up on the new leaf: the old pair is put back and loaded, GB-302"
else
  fail "the roll-back did not restore the old pair"; tail -15 "$LOG"
fi

# ipsec-ca: refused without the read-back, then with a wrong one; switches with the right one.
fresh; stage ca2leaf; cp "$T/pki/ca2.pem" "$S/x509ca/xab-local-ca.next.pem"; touch "$T/sa-up"
before=$(snap)
if ! rot -e peering_edge_rotation=r5 -e peering_edge_rotation_kind=ca && grep -q "GB-303" "$LOG" && [ "$(snap)" = "$before" ]; then
  pass "ipsec-ca without the partner's read-back: GB-303, nothing moved"
else
  fail "ipsec-ca without a read-back was not refused"; tail -12 "$LOG"
fi
if ! rot -e peering_edge_rotation=r5 -e peering_edge_rotation_kind=ca -e peering_edge_rotation_ca_readback="sha256:$(sha "$T/pki/ca1.pem")" \
   && grep -q "GB-303" "$LOG" && [ "$(snap)" = "$before" ]; then
  pass "ipsec-ca with a read-back of another CA: GB-303, nothing moved"
else
  fail "ipsec-ca with a wrong read-back was not refused"; tail -12 "$LOG"
fi
if rot -e peering_edge_rotation=r5 -e peering_edge_rotation_kind=ca -e peering_edge_rotation_ca_readback="sha256:$(sha "$T/pki/ca2.pem")" \
   && live_is ca2leaf && [ "$(sha "$T/local-ca.pem")" = "$(sha "$T/pki/ca2.pem")" ] && [ ! -e "$S/x509ca/xab-local-ca.next.pem" ]; then
  pass "ipsec-ca with the read-back: switched; the new CA is the one the chain is checked against"
else
  fail "ipsec-ca with the right read-back did not switch"; tail -15 "$LOG"
fi

# Retire: the set-aside key goes, its certificate stays, nothing else is touched.
reauth_before=$(calls '--reauth')
if rot -e peering_edge_rotation=r5 -e peering_edge_rotation_kind=ca -e peering_edge_rotation_retire=true \
   && [ ! -e "$S/.previous-r5/xab-key.pem" ] && [ -f "$S/.previous-r5/xab-cert.pem" ] && [ "$(calls '--reauth')" = "$reauth_before" ]; then
  pass "retire removes the set-aside key, keeps its certificate, and re-authenticates nothing"
else
  fail "retire did not remove only the set-aside key"; tail -12 "$LOG"
fi

# swanctl.conf: unchanged without the partner's next CA; current then next with it.
render() { # render <src> <out> [extra]
  ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -i localhost, "$HERE/render-swanctl.yml" -e "src=$1" -e "out=$2" "${@:3}" > "$LOG" 2>&1
}
# swanctl.golden.conf is the render of the template before this change, with render-swanctl.yml's peering.
if render "$HERE/../templates/swanctl.conf.j2" "$T/new.conf" && cmp -s "$HERE/swanctl.golden.conf" "$T/new.conf"; then
  pass "swanctl.conf without the partner's next CA is byte for byte as before (swanctl.golden.conf)"
else
  fail "swanctl.conf changed without the next CA"; diff "$HERE/swanctl.golden.conf" "$T/new.conf" | head -10
fi
if render "$HERE/../templates/swanctl.conf.j2" "$T/next.conf" -e peering_edge_remote_ca_next_path=/etc/swanctl/x509ca/xab-partner-ca.next.pem \
   && grep -q 'cacerts = xab-partner-ca.pem,xab-partner-ca.next.pem$' "$T/next.conf"; then
  pass "with the partner's next CA, remote.cacerts lists the current CA and then the next"
else
  fail "remote.cacerts does not list both CAs"; grep cacerts "$T/next.conf" || tail -8 "$LOG"
fi

if [ "$FAILS" -eq 0 ]; then echo "goat-peering-edge test-rotation: PASS"; exit 0; fi
echo "goat-peering-edge test-rotation: FAIL ($FAILS)"; exit 1
