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
#   - the retire step removes the set-aside key and keeps its certificate, only once
#     the new leaf is live; it never switches the live pair or replaces our CA, and an
#     unknown id, a retire before the switch or with the SA down is refused;
#   - a resume or retire after a rollback reports GB-302 again, the old leaf live;
#   - swanctl.conf renders byte for byte as before without the partner's next CA,
#     and lists the current CA and then the next one with it;
#   - the rotation reads the strongSwan version from the daemon and refuses a series
#     it was not tested with, or a version it cannot read (GB-341), before anything
#     moves; peering_edge_rotation_strongswan_tested widens the list;
#   - once the new SA is up, the credentials are reloaded from the files alone
#     (`--load-creds --clear`), and the confirm passes only when charon holds the live
#     leaf's serial with its private key and no other certificate of our id with one;
#     otherwise the old pair is put back (GB-302). The record names the serial and
#     the strongSwan version.
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

# The stand-in swanctl: records each call. While $T/sa-up exists, --list-sas reports
# the IKE SAs in $T/sas (one "<unique id> <established epoch>" per line; at first one
# SA established an hour ago), each ESTABLISHED, with its established age and an
# INSTALLED CHILD SA, as `swanctl --list-sas` prints them. `--rekey --reauth` makes a
# new SA with the next id and drops the old one (make-before-break done), unless
# $T/reauth-stalls exists: then the old SA stays and no new one comes. Each SA shows
# "rekeying in <s>s": an hour, or, with $T/rekey-soon, one second from the first
# listing. When that time comes, the next listing shows a scheduled IKE rekey: a new
# SA id with a fresh established time (and, as in strongSwan, the old authentication).
# A plain `--rekey` (no --reauth) does the same at once and restarts the timer.
# $T/partner-rekey-on holds N: the Nth listing shows a rekey the partner started (a
# new SA id, old authentication) while our own "rekeying in" stays an hour away. A
# completed rekey or re-authentication of ours resets the partner's timer too, so it
# cancels a partner rekey still pending.
mkdir -p "$T/bin"
cat > "$T/bin/swanctl" <<'SH'
#!/bin/sh
T=@T@
echo "$*" >> "$T/swanctl.log"
now=$(date +%s)
[ -e "$T/sas" ] || echo "1 $((now - 3600))" > "$T/sas"
newsa() { # a new IKE SA in place of the old ones, its rekey an hour away
  n=$(($(cut -d' ' -f1 "$T/sas" | sort -n | tail -1) + 1))
  echo "$n $now" > "$T/sas"; echo $((now + 3600)) > "$T/rekey-at"
}
# The credentials charon holds, as `swanctl --list-certs` shows them (measured on
# strongSwan 5.9.8 and 6.0.4): $T/creds-certs holds "<serial> <CN> <public key digest>"
# per end-entity certificate, $T/creds-keys the digest of each private key. The first
# call loads what the swanctl directory held before anything was staged (the live
# pair: charon was started before the staging). `--load-creds` adds the certificates on
# disk and keeps those whose files are gone; it holds only the keys on disk. With
# --clear it holds only what is on disk. $T/creds-stale: --load-creds changes nothing
# (charon keeps the old pair). --flush-certs touches neither (it drops only the cache of
# certificates received). `--version --daemon --raw` reports $T/sw-version (5.9.8), or
# fails while $T/no-daemon exists.
S="$T/swanctl"
certinfo() {
  s=$(openssl x509 -noout -serial -in "$1" 2>/dev/null | cut -d= -f2)
  [ -n "$s" ] || return 0
  cn=$(openssl x509 -noout -subject -nameopt multiline -in "$1" | sed -n 's/^ *commonName *= *//p')
  d=$(openssl x509 -noout -pubkey -in "$1" | openssl sha256 | awk '{print $NF}')
  echo "$s $cn $d"
}
ondisk_certs() { for f in "$S"/x509/*.pem; do [ -e "$f" ] || continue; [ "${1:-}" = live ] && case "$f" in *.next.pem) continue ;; esac; certinfo "$f"; done; }
ondisk_keys() {
  for f in "$S"/private/*.pem; do
    [ -e "$f" ] || continue
    [ "${1:-}" = live ] && case "$f" in *.next.pem) continue ;; esac
    o=$(openssl pkey -pubout -in "$f" 2>/dev/null) && printf '%s\n' "$o" | openssl sha256 | awk '{print $NF}'
  done
}
if [ ! -e "$T/creds-certs" ]; then ondisk_certs live > "$T/creds-certs"; ondisk_keys live > "$T/creds-keys"; fi
case "$1" in
  --version)
    case "$*" in *--daemon*)
      [ -e "$T/no-daemon" ] && { echo "connecting to 'unix:///var/run/charon.vici' failed: Connection refused" >&2; exit 1; }
      echo "version reply {daemon=charon version=$(cat "$T/sw-version" 2>/dev/null || echo 5.9.8) sysname=Linux release=6.1.0 machine=x86_64}" ;;
    *) echo "strongSwan swanctl $(cat "$T/sw-version" 2>/dev/null || echo 5.9.8)" ;;
    esac
    exit 0 ;;
  --load-creds)
    [ -e "$T/creds-stale" ] && exit 0
    case "$*" in
      *--clear*) ondisk_certs > "$T/creds-certs.new" ;;
      *) { cat "$T/creds-certs"; ondisk_certs; } | sort -u > "$T/creds-certs.new" ;;
    esac
    /bin/mv -f "$T/creds-certs.new" "$T/creds-certs"; ondisk_keys > "$T/creds-keys"
    exit 0 ;;
  --list-certs)
    echo; echo "List of X.509 End Entity Certificates"; echo
    while read -r ser cn d; do
      [ -n "$ser" ] || continue
      h=$(printf '%s' "$ser" | tr 'A-F' 'a-f')
      [ $((${#h} % 2)) -eq 1 ] && h="0$h"
      case "$h" in [89a-f]*) h="00$h" ;; esac
      h=$(printf '%s' "$h" | sed 's/../&:/g; s/:$//')
      k=""; grep -qx "$d" "$T/creds-keys" 2>/dev/null && k=", has private key"
      printf '  subject:  "CN=%s"\n  issuer:   "CN=ca1"\n  serial:    %s\n  pubkey:    ECDSA 384 bits%s\n\n' "$cn" "$h" "$k"
    done < "$T/creds-certs"
    printf '  subject:  "OU=partner, CN=fed.xab.xaa.01"\n  serial:    0b:01\n  pubkey:    ECDSA 384 bits\n\n'
    printf 'List of X.509 CA Certificates\n\n  subject:  "CN=fed.xaa.xab.01 test CA"\n  serial:    01\n  pubkey:    ECDSA 384 bits, has private key\n'
    exit 0 ;;
  --list-sas)
    [ -e "$T/sa-up" ] || exit 0
    if [ -e "$T/partner-rekey-on" ]; then
      c=$(($(cat "$T/listings" 2>/dev/null || echo 0) + 1)); echo "$c" > "$T/listings"
      if [ "$c" -ge "$(cat "$T/partner-rekey-on")" ]; then
        rm -f "$T/partner-rekey-on"
        n=$(($(cut -d' ' -f1 "$T/sas" | sort -n | tail -1) + 1))
        r=$(cat "$T/rekey-at" 2>/dev/null || echo $((now + 3600)))
        echo "$n $now" > "$T/sas"; echo "$r" > "$T/rekey-at"
      fi
    fi
    if [ ! -e "$T/rekey-at" ]; then
      if [ -e "$T/rekey-soon" ]; then echo $((now + 1)) > "$T/rekey-at"; else echo $((now + 3600)) > "$T/rekey-at"; fi
    fi
    [ "$now" -ge "$(cat "$T/rekey-at")" ] && newsa
    at=$(cat "$T/rekey-at")
    while read -r id ep; do
      printf 'xab: #%s, ESTABLISHED, IKEv2\n  established %ss ago, rekeying in %ss\n  fabric: #%s, reqid 1, INSTALLED, TUNNEL\n' \
        "$id" "$((now - ep))" "$((at - now))" "$id"
    done < "$T/sas"
    exit 0 ;;
  --rekey)
    [ -e "$T/sa-up" ] || exit 0
    case "$*" in
      *--reauth*) [ -e "$T/reauth-stalls" ] && exit 0; rm -f "$T/partner-rekey-on"
        if [ -e "$T/old-lingers" ]; then # make-before-break not completed: the old SA stays listed
          n=$(($(cut -d' ' -f1 "$T/sas" | sort -n | tail -1) + 1)); echo "$n $now" >> "$T/sas"
        else newsa; fi ;;
      *) rm -f "$T/partner-rekey-on"; newsa
         # A run killed right after the rekey, before the re-authentication.
         [ -e "$T/die-after-rekey" ] && exit 1 ;;
    esac
    exit 0 ;;
  *) exit 0 ;;
esac
SH
sed -i "s|@T@|$T|" "$T/bin/swanctl"
chmod 755 "$T/bin/swanctl"
# A cp that fails copying out of a set-aside directory while $T/cp-fails exists: a
# run killed during the roll-back.
cat > "$T/bin/cp" <<'SH'
#!/bin/sh
T=@T@
for a in "$@"; do last=$a; done
if [ -e "$T/cp-fails" ]; then
  case "$*" in *.previous-*) case "$last" in *.previous-*) ;; *) exit 1 ;; esac ;; esac
fi
# A run killed during the set-aside, after the certificate and before the key.
if [ -e "$T/cp-dies-on-key" ]; then
  case "$last" in *.previous-*) case "$*" in *xab-key.pem*) exit 1 ;; esac ;; esac
fi
exec /bin/cp "$@"
SH
sed -i "s|@T@|$T|" "$T/bin/cp"
chmod 755 "$T/bin/cp"
# An mv that dies moving the staged certificate while $T/mv-dies exists: a run killed
# between the switch's two moves (the key moved in, the certificate not).
cat > "$T/bin/mv" <<'SH'
#!/bin/sh
T=@T@
if [ -e "$T/mv-dies" ]; then
  case "$*" in *cert.next*) exit 1 ;; esac
fi
exec /bin/mv "$@"
SH
sed -i "s|@T@|$T|" "$T/bin/mv"
chmod 755 "$T/bin/mv"

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
  rm -rf "$S" "$T/swanctl.log" "$T/sa-up" "$T/sas" "$T/reauth-stalls" "$T/cp-fails" "$T/rekey-at" "$T/rekey-soon" "$T/partner-rekey-on" "$T/listings" "$T/die-after-rekey" "$T/old-lingers" "$T/mv-dies" "$T/cp-dies-on-key" \
    "$T/creds-certs" "$T/creds-keys" "$T/creds-stale" "$T/sw-version" "$T/no-daemon"
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
callx() { grep -cx -- "$1" "$T/swanctl.log" 2>/dev/null || true; } # whole call lines
# A serial as the confirm writes it: lower-case hex, no colons, no leading zeros.
serial_of() { openssl x509 -noout -serial -in "$1" | cut -d= -f2 | tr 'A-F' 'a-f' | sed 's/^0*//'; }
rec() { sed -n "s/^$1=//p" "$S/.previous-$2/confirmed" 2>/dev/null; }
snap() { find "$S" -type f -exec openssl sha256 -r {} + 2>/dev/null | sort; }
# For a successful re-run with the same id only: every file but the confirm's record,
# which each confirmed run rewrites with its own SA id; the record is compared by
# existence there.
snap_but_record() { find "$S" -type f ! -name confirmed -exec openssl sha256 -r {} + 2>/dev/null | sort; }

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
   && [ ! -e "$S/x509/xab-cert.next.pem" ] && [ "$(callx --load-creds)" = 1 ] && [ "$(calls '--reauth')" = 1 ]; then
  pass "a good pair is set aside (0700) and switched in, one load and one re-authentication"
else
  fail "the good pair was not switched as designed"; tail -15 "$LOG"; cat "$T/swanctl.log" 2>/dev/null || true
fi
aside=$(snap_but_record)
if rot -e peering_edge_rotation=r1 && live_is good && [ "$(snap_but_record)" = "$aside" ] && [ -e "$S/.previous-r1/confirmed" ]; then
  pass "a rerun with the same id moves nothing"
else
  fail "a rerun with the same id changed files"; tail -12 "$LOG"
fi

# The confirm proves the leaf charon can sign with, and the old one is dropped.
fresh; stage good; touch "$T/sa-up"
if rot -e peering_edge_rotation=r1 && live_is good && [ "$(rec serial r1)" = "$(serial_of "$T/pki/good.pem")" ] \
   && [ "$(rec strongswan r1)" = 5.9.8 ] && [ "$(callx '--load-creds --clear --noprompt')" = 1 ] \
   && [ "$(grep -n -- '--reauth' "$T/swanctl.log" | tail -1 | cut -d: -f1)" -lt "$(grep -n -- '--clear' "$T/swanctl.log" | tail -1 | cut -d: -f1)" ] \
   && ! grep -q "^$(openssl x509 -noout -serial -in "$T/pki/live.pem" | cut -d= -f2) " "$T/creds-certs"; then
  pass "after the new SA, the credentials are reloaded from the files alone; the old leaf is gone, the record names the serial and strongSwan 5.9.8"
else
  fail "the confirm did not reload the credentials, or its record lacks the serial or the version"; tail -15 "$LOG"; cat "$T/swanctl.log" 2>/dev/null || true; cat "$S/.previous-r1/confirmed" 2>/dev/null || true
fi

# charon kept the old pair (the credentials never reloaded): the SA came up, but not provably on the new leaf.
fresh; stage good; touch "$T/sa-up" "$T/creds-stale"
if ! rot -e peering_edge_rotation=r1 && grep -q "GB-302" "$LOG" && grep -q "serial $(serial_of "$T/pki/good.pem")" "$LOG" \
   && live_is live && [ ! -e "$S/.previous-r1/confirmed" ] && [ -e "$S/.previous-r1/rolled-back" ]; then
  pass "charon does not hold the new leaf's key: GB-302 names its serial, the old pair is put back"
else
  fail "a new SA was confirmed while charon could only sign with the old leaf"; tail -15 "$LOG"
fi

# The old pair is still loadable under other names: charon could sign with either leaf.
fresh; stage good; touch "$T/sa-up"
cp "$T/pki/live.pem" "$S/x509/xab-old-cert.pem"; cp "$T/pki/live.key" "$S/private/xab-old-key.pem"
if ! rot -e peering_edge_rotation=r1 && grep -q "GB-302" "$LOG" && grep -q "serial $(serial_of "$T/pki/live.pem")" "$LOG" \
   && live_is live && [ ! -e "$S/.previous-r1/confirmed" ]; then
  pass "another certificate of our id with its key still loaded: GB-302 names its serial, the old pair is put back"
else
  fail "the confirm passed while charon held a second leaf of our id with its key"; tail -15 "$LOG"
fi

# The strongSwan version: read from the daemon, refused outside the tested series (GB-341), before anything moves.
for v in 5.8.4 6.1.0 unreadable; do
  fresh; stage good; touch "$T/sa-up"
  if [ "$v" = unreadable ]; then touch "$T/no-daemon"; else echo "$v" > "$T/sw-version"; fi
  before=$(snap)
  if ! rot -e peering_edge_rotation=r1 && grep -q "GB-341" "$LOG" && [ "$(snap)" = "$before" ] && [ ! -e "$S/.previous-r1" ] \
     && ! grep -q -- '--rekey\|--load-creds' "$T/swanctl.log"; then
    pass "strongSwan $v: refused with GB-341 before anything moves"
  else
    fail "strongSwan $v was not refused before the switch"; tail -12 "$LOG"; cat "$T/swanctl.log" 2>/dev/null || true
  fi
done
fresh; stage good; touch "$T/sa-up"; echo 6.0.4 > "$T/sw-version"
if rot -e peering_edge_rotation=r1 && live_is good && [ "$(rec strongswan r1)" = 6.0.4 ]; then
  pass "strongSwan 6.0.4 (a tested series): the rotation runs and records the version"
else
  fail "strongSwan 6.0.4 did not rotate"; tail -12 "$LOG"
fi
fresh; stage good; touch "$T/sa-up"; echo 5.8.4 > "$T/sw-version"
if rot -e peering_edge_rotation=r1 -e '{"peering_edge_rotation_strongswan_tested": ["5.8"]}' && live_is good && [ "$(rec strongswan r1)" = 5.8.4 ]; then
  pass "peering_edge_rotation_strongswan_tested widened to 5.8: strongSwan 5.8.4 rotates"
else
  fail "a widened tested list did not let 5.8.4 rotate"; tail -12 "$LOG"
fi

# Stopped after the set-aside (aside present, staged still there): finishes, no second set-aside.
fresh; stage good; touch "$T/sa-up"
mkdir -m 700 "$S/.previous-r2"; cp -p "$S/x509/xab-cert.pem" "$S/private/xab-key.pem" "$S/.previous-r2/"
if rot -e peering_edge_rotation=r2 && live_is good && [ "$(sha "$S/.previous-r2/xab-cert.pem")" = "$(sha "$T/pki/live.pem")" ] \
   && [ "$(callx --load-creds)" = 1 ]; then
  pass "stopped after the set-aside: finishes without setting aside the new pair"
else
  fail "a run stopped after the set-aside did not finish cleanly"; tail -12 "$LOG"
fi

# Stopped after the switch (files switched, staged gone, aside present): loads and re-authenticates, moves nothing.
fresh; touch "$T/sa-up"
mkdir -m 700 "$S/.previous-r3"; cp -p "$S/x509/xab-cert.pem" "$S/private/xab-key.pem" "$S/.previous-r3/"
cp "$T/pki/good.pem" "$S/x509/xab-cert.pem"; cp "$T/pki/good.key" "$S/private/xab-key.pem"
before=$(snap_but_record)
if rot -e peering_edge_rotation=r3 && live_is good && [ "$(snap_but_record)" = "$before" ] && [ -e "$S/.previous-r3/confirmed" ] \
   && [ "$(callx --load-creds)" = 1 ] && [ "$(calls '--reauth')" = 1 ]; then
  pass "stopped after the switch: loads and re-authenticates once, moves nothing"
else
  fail "a run stopped after the switch did not finish cleanly"; tail -12 "$LOG"
fi

# The SA never comes up: the old pair is put back, GB-302.
fresh; stage good
if ! rot -e peering_edge_rotation=r4 && grep -q "GB-302" "$LOG" && live_is live && [ "$(calls '--reauth')" = 2 ] \
   && [ ! -e "$S/.previous-r4/confirmed" ]; then
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

# The retire step never switches the live pair, and removes the set-aside key only
# once the new leaf is confirmed live (the review's cases A and B, the owner's rules).
RETIRE_CODE=GB-309

# Case A: r1 passed, then the next leaf is staged, then --retire r1.
fresh; stage good; touch "$T/sa-up"
rot -e peering_edge_rotation=r1 || { fail "setup: r1 did not pass"; tail -12 "$LOG"; }
stage good2
reauth_before=$(calls '--reauth')
if rot -e peering_edge_rotation=r1 -e peering_edge_rotation_retire=true && live_is good \
   && [ -f "$S/x509/xab-cert.next.pem" ] && [ -f "$S/private/xab-key.next.pem" ] \
   && [ ! -e "$S/.previous-r1/xab-key.pem" ] && [ -f "$S/.previous-r1/xab-cert.pem" ] \
   && [ "$(calls '--reauth')" = "$reauth_before" ]; then
  pass "retire with the next leaf staged: the live pair and the staged pair stay, only the set-aside key goes"
else
  fail "retire with something staged switched or removed the wrong file"; tail -12 "$LOG"
fi

# Case B: a mistyped id with something staged: refused, nothing moves.
fresh; stage good; touch "$T/sa-up"
before=$(snap)
if ! rot -e peering_edge_rotation=r9 -e peering_edge_rotation_retire=true && grep -q "$RETIRE_CODE" "$LOG" \
   && [ "$(snap)" = "$before" ] && [ ! -e "$S/.previous-r9" ] && ! grep -q -- --load-creds "$T/swanctl.log" 2>/dev/null; then
  pass "retire of an id with no set-aside here: $RETIRE_CODE, nothing moved"
else
  fail "retire of an unknown id was not refused, or something moved"; tail -12 "$LOG"
fi

# Not confirmed live: the SA is down after the switch; the set-aside key stays.
fresh; stage good; touch "$T/sa-up"
rot -e peering_edge_rotation=r1 || { fail "setup: r1 did not pass"; tail -12 "$LOG"; }
rm -f "$T/sa-up"
if ! rot -e peering_edge_rotation=r1 -e peering_edge_rotation_retire=true && grep -q "$RETIRE_CODE" "$LOG" \
   && [ -f "$S/.previous-r1/xab-key.pem" ] && live_is good; then
  pass "retire while the SA on the new leaf is not up: $RETIRE_CODE, the set-aside key stays"
else
  fail "retire removed the set-aside key without the new leaf confirmed live"; tail -12 "$LOG"
fi

# Stopped after the set-aside, before the switch: the set-aside copy is the live key's only
# other copy and the live pair is still the old one, so retire removes nothing.
fresh; stage good; touch "$T/sa-up"
mkdir -m 700 "$S/.previous-r2"; cp -p "$S/x509/xab-cert.pem" "$S/private/xab-key.pem" "$S/.previous-r2/"
before=$(snap)
if ! rot -e peering_edge_rotation=r2 -e peering_edge_rotation_retire=true && grep -q "$RETIRE_CODE" "$LOG" \
   && [ "$(snap)" = "$before" ]; then
  pass "retire before the switch happened: $RETIRE_CODE, nothing moved or removed"
else
  fail "retire before the switch moved or removed something"; tail -12 "$LOG"
fi

# A CA rotation's "new CA becomes the CA" never runs on retire, read-back or not.
fresh; stage good; touch "$T/sa-up"
rot -e peering_edge_rotation=r6 || { fail "setup: r6 did not pass"; tail -12 "$LOG"; }
cp "$T/pki/ca2.pem" "$S/x509ca/xab-local-ca.next.pem"
rot -e peering_edge_rotation=r6 -e peering_edge_rotation_kind=ca -e peering_edge_rotation_retire=true || true
if [ "$(sha "$T/local-ca.pem")" = "$(sha "$T/pki/ca1.pem")" ] && [ -f "$S/x509ca/xab-local-ca.next.pem" ]; then
  pass "retire of an ipsec-ca rotation without the read-back: our CA is not replaced"
else
  fail "retire replaced our CA without the partner's read-back"; tail -12 "$LOG"
fi

# A resume after a GB-302 rollback reports the rollback, never success, and the old leaf live.
fresh; stage good
rot -e peering_edge_rotation=r4 || true
touch "$T/sa-up"
reauth_before=$(calls '--reauth')
if ! rot -e peering_edge_rotation=r4 && grep -q "GB-302" "$LOG" && grep -q "the old leaf is live" "$LOG" \
   && live_is live && [ "$(calls '--reauth')" = "$reauth_before" ]; then
  pass "a resume after a rollback: GB-302 again, the old leaf live, nothing re-authenticated"
else
  fail "a resume after a rollback reported success or changed the live pair"; tail -12 "$LOG"
fi
if ! rot -e peering_edge_rotation=r4 -e peering_edge_rotation_retire=true && grep -q "GB-302" "$LOG" \
   && [ -f "$S/.previous-r4/xab-key.pem" ]; then
  pass "retire after a rollback: GB-302, the set-aside key stays"
else
  fail "retire after a rollback removed the set-aside key"; tail -12 "$LOG"
fi

# The same for an ipsec-ca rotation: our CA is not replaced by the resume.
fresh; stage ca2leaf; cp "$T/pki/ca2.pem" "$S/x509ca/xab-local-ca.next.pem"
RB=(-e peering_edge_rotation=r7 -e peering_edge_rotation_kind=ca -e peering_edge_rotation_ca_readback="sha256:$(sha "$T/pki/ca2.pem")")
rot "${RB[@]}" || true
touch "$T/sa-up"
if ! rot "${RB[@]}" && grep -q "GB-302" "$LOG" && live_is live \
   && [ "$(sha "$T/local-ca.pem")" = "$(sha "$T/pki/ca1.pem")" ]; then
  pass "a resume after an ipsec-ca rollback: GB-302, the old leaf live, our CA not replaced"
else
  fail "a resume after an ipsec-ca rollback replaced our CA or reported success"; tail -12 "$LOG"
fi

# Make-before-break: while the new SA negotiates the old one stays ESTABLISHED. The
# confirm counts only an SA other than the ones up before the re-authentication.
fresh; stage good; touch "$T/sa-up" "$T/reauth-stalls"
if ! rot -e peering_edge_rotation=r8 && grep -q "GB-302" "$LOG" && live_is live; then
  pass "the old SA still up and no new one: the confirm does not pass, GB-302, the old pair back"
else
  fail "the confirm passed on the SA that was up before the re-authentication"; tail -12 "$LOG"
fi

# A scheduled IKE rekey makes a new SA id, but keeps the old authentication. One due
# within the confirm window is done first, before the ids are noted, so it cannot pass
# the confirm on the old leaf.
fresh; stage good; touch "$T/sa-up" "$T/reauth-stalls" "$T/rekey-soon"
if ! rot -e peering_edge_rotation=r11 && grep -q "GB-302" "$LOG" && live_is live && [ ! -e "$S/.previous-r11/confirmed" ]; then
  pass "a rekey due in the confirm window, the reauth stalled: no pass on the rekeyed SA, GB-302"
else
  fail "a scheduled rekey in the confirm window passed the confirm on the old leaf"; tail -12 "$LOG"
fi
fresh; stage good; touch "$T/sa-up" "$T/rekey-soon"
if rot -e peering_edge_rotation=r12 && live_is good \
   && awk '/^--rekey --ike xab$/ { p = NR } /--reauth/ && !r { r = NR } END { exit (p && r && p < r) ? 0 : 1 }' "$T/swanctl.log"; then
  pass "a rekey due in the confirm window is done first (a plain --rekey before the --reauth)"
else
  fail "no rekey before the re-authentication"; tail -12 "$LOG"; cat "$T/swanctl.log" 2>/dev/null || true
fi

# A rekey the partner starts follows its own timer, so our "rekeying in" does not
# warn of it: the rotation always rekeys first, which resets both sides' timers.
fresh; stage good; touch "$T/sa-up" "$T/reauth-stalls"; echo 3 > "$T/partner-rekey-on"
if ! rot -e peering_edge_rotation=r13 && grep -q "GB-302" "$LOG" && live_is live && [ ! -e "$S/.previous-r13/confirmed" ]; then
  pass "a rekey the partner starts in the confirm window, the reauth stalled: no pass, GB-302"
else
  fail "a partner-started rekey in the confirm window passed the confirm on the old leaf"; tail -12 "$LOG"
fi

# The IKE SA's rekey interval must leave room for the confirm: checked before anything moves.
fresh; stage good; touch "$T/sa-up"
before=$(snap)
if ! rot -e peering_edge_rotation=r14 -e peering_edge_ike_rekey_time=30s -e peering_edge_ike_rand_time=10s \
   && grep -q "peering_edge_ike_rekey_time" "$LOG" && [ "$(snap)" = "$before" ]; then
  pass "rekey_time minus rand_time shorter than the confirm window and margin: refused, nothing moved"
else
  fail "a rekey interval with no room for the confirm was not refused before anything moved"; tail -12 "$LOG"
fi

# The confirm leaves its proof: .previous-<id>/confirmed holds the new SA's id and
# time, and retire requires it. An SA established after the set-aside proves nothing
# on its own: the rekey before the re-authentication makes one, still on the old leaf.
fresh; stage good; touch "$T/sa-up"
if rot -e peering_edge_rotation=r15 && grep -Eq '^id=[0-9]+$' "$S/.previous-r15/confirmed" 2>/dev/null \
   && grep -Eq '^at=[0-9]+$' "$S/.previous-r15/confirmed" \
   && rot -e peering_edge_rotation=r15 -e peering_edge_rotation_retire=true && [ ! -e "$S/.previous-r15/xab-key.pem" ]; then
  pass "the confirm writes confirmed (the new SA's id and time), and retire then removes the set-aside key"
else
  fail "no confirmed marker, or retire did not pass on it"; tail -12 "$LOG"
fi
# Killed after the rekey, before the re-authentication: retire refuses; a re-run confirms.
fresh; stage good; touch "$T/sa-up" "$T/die-after-rekey"
rot -e peering_edge_rotation=r16 || true
rm -f "$T/die-after-rekey"
if [ -e "$S/.previous-r16" ] && [ ! -e "$S/.previous-r16/confirmed" ] \
   && ! rot -e peering_edge_rotation=r16 -e peering_edge_rotation_retire=true && grep -q "$RETIRE_CODE" "$LOG" \
   && grep -q "was not confirmed" "$LOG" && [ -f "$S/.previous-r16/xab-key.pem" ]; then
  pass "killed after the rekey, before the re-authentication: retire refuses ($RETIRE_CODE, not confirmed)"
else
  fail "retire passed on the rekeyed SA of a rotation that never confirmed"; tail -12 "$LOG"
fi
if rot -e peering_edge_rotation=r16 && [ -e "$S/.previous-r16/confirmed" ] && live_is good \
   && [ "$(calls '--reauth')" -ge 1 ]; then
  pass "the re-run with the same id re-authenticates, confirms and writes the marker"
else
  fail "the re-run after a kill did not confirm"; tail -12 "$LOG"
fi
# Killed after the confirm, before the marker: the same; the re-run writes it.
fresh; stage good; touch "$T/sa-up"
rot -e peering_edge_rotation=r17 || { fail "setup: r17 did not pass"; tail -12 "$LOG"; }
rm -f "$S/.previous-r17/confirmed"
if ! rot -e peering_edge_rotation=r17 -e peering_edge_rotation_retire=true && grep -q "$RETIRE_CODE" "$LOG" \
   && [ -f "$S/.previous-r17/xab-key.pem" ] \
   && rot -e peering_edge_rotation=r17 && [ -e "$S/.previous-r17/confirmed" ] \
   && rot -e peering_edge_rotation=r17 -e peering_edge_rotation_retire=true && [ ! -e "$S/.previous-r17/xab-key.pem" ]; then
  pass "killed after the confirm, before the marker: retire refuses; the re-run confirms; retire then passes"
else
  fail "a rotation without its confirmed marker was retired, or the re-run did not write it"; tail -12 "$LOG"
fi

# A malformed confirm record fails closed: retire needs both id= and at=.
fresh; stage good; touch "$T/sa-up"
rot -e peering_edge_rotation=r18 || { fail "setup: r18 did not pass"; tail -12 "$LOG"; }
for body in "" "id=5"; do
  printf '%s\n' "$body" > "$S/.previous-r18/confirmed"
  if ! rot -e peering_edge_rotation=r18 -e peering_edge_rotation_retire=true && grep -q "$RETIRE_CODE" "$LOG" \
     && grep -q "unreadable" "$LOG" && [ -f "$S/.previous-r18/xab-key.pem" ]; then
    pass "a confirm record without id= and at= (\"$body\"): retire refuses, $RETIRE_CODE unreadable"
  else
    fail "retire passed on a malformed confirm record (\"$body\")"; tail -12 "$LOG"
  fi
done

# The old-leaf SA must be gone before the confirm is recorded (make-before-break done):
# a lingering one could later rekey into an SA established after the record.
fresh; stage good; touch "$T/sa-up" "$T/old-lingers"
if ! rot -e peering_edge_rotation=r19 && grep -q "GB-302" "$LOG" && live_is live \
   && [ ! -e "$S/.previous-r19/confirmed" ]; then
  pass "the old SA still listed after the new one came up: no record, rolled back, GB-302"
else
  fail "the confirm was recorded while the old-leaf SA was still up"; tail -12 "$LOG"
fi

# A roll-back removes an earlier confirm record, so a rolled-back re-run leaves none.
fresh; stage good; touch "$T/sa-up"
rot -e peering_edge_rotation=r20 || { fail "setup: r20 did not pass"; tail -12 "$LOG"; }
rm -f "$T/sa-up"
if ! rot -e peering_edge_rotation=r20 && grep -q "GB-302" "$LOG" && [ ! -e "$S/.previous-r20/confirmed" ]; then
  pass "a re-run that rolls back removes the earlier confirm record"
else
  fail "a rolled-back re-run left a confirm record"; tail -12 "$LOG"
fi

# K1: killed between the switch's two moves (the key moved in, the certificate not).
# The re-run finishes the switch; it never leaves the live pair mixed.
fresh; stage good; touch "$T/sa-up" "$T/mv-dies"
rot -e peering_edge_rotation=r21 || true
rm -f "$T/mv-dies"
mixed=no
[ "$(sha "$S/private/xab-key.pem")" = "$(sha "$T/pki/good.key")" ] && [ "$(sha "$S/x509/xab-cert.pem")" = "$(sha "$T/pki/live.pem")" ] && mixed=yes
if [ "$mixed" = yes ] && rot -e peering_edge_rotation=r21 && live_is good && [ ! -e "$S/x509/xab-cert.next.pem" ] \
   && [ -e "$S/.previous-r21/confirmed" ]; then
  pass "killed between the switch's moves: the re-run finishes the switch and confirms"
else
  fail "a run killed between the switch's moves did not finish (mixed setup: $mixed)"; tail -12 "$LOG"
fi
# The same, but what is left staged is not the moved key's certificate: refused, nothing moved.
fresh; stage good; touch "$T/sa-up" "$T/mv-dies"
rot -e peering_edge_rotation=r22 || true
rm -f "$T/mv-dies"
cp "$T/pki/good2.pem" "$S/x509/xab-cert.next.pem"
before=$(snap)
if ! rot -e peering_edge_rotation=r22 && grep -q "GB-311" "$LOG" && [ "$(snap)" = "$before" ]; then
  pass "a half-done switch whose staged remainder does not match: GB-311, nothing moved"
else
  fail "a half-done switch with a mismatched remainder was not refused, or something moved"; tail -12 "$LOG"
fi

# A half-done switch whose files do not parse: an unreadable key and an unreadable staged
# certificate must not "match" (both would hash empty input). Refused, nothing moved.
fresh; stage good; touch "$T/sa-up" "$T/mv-dies"
rot -e peering_edge_rotation=r24 || true
rm -f "$T/mv-dies"
echo "not a certificate" > "$S/x509/xab-cert.next.pem"; rm -f "$S/private/xab-key.pem"
before=$(snap)
if ! rot -e peering_edge_rotation=r24 && grep -q "GB-311" "$LOG" && [ "$(snap)" = "$before" ]; then
  pass "a half-done switch with an unreadable key and staged certificate: GB-311, nothing moved"
else
  fail "a half-done switch finished on files that do not parse"; tail -12 "$LOG"
fi

# K2: killed during the set-aside (certificate copied, key not). The set-aside exists
# only when complete, so the re-run redoes it, and a roll-back restores both files.
fresh; stage good; touch "$T/cp-dies-on-key"
rot -e peering_edge_rotation=r23 || true
rm -f "$T/cp-dies-on-key"
if [ ! -e "$S/.previous-r23" ] && live_is live; then
  pass "killed during the set-aside: no set-aside directory, the live pair untouched"
else
  fail "a set-aside killed half way left a set-aside directory or touched the live pair"; ls -la "$S" 2>/dev/null
fi
if ! rot -e peering_edge_rotation=r23 && grep -q "GB-302" "$LOG" && live_is live \
   && [ -f "$S/.previous-r23/xab-key.pem" ] && [ -f "$S/.previous-r23/xab-cert.pem" ] && [ ! -e "$S/.previous-r23.tmp" ]; then
  pass "the re-run redoes the set-aside, and its roll-back restores both files"
else
  fail "after a set-aside killed half way, the roll-back did not restore both files"; tail -12 "$LOG"
fi

# Retire: the SA must be established after the rotation set the old pair aside.
fresh; stage good; touch "$T/sa-up"
rot -e peering_edge_rotation=r1 || { fail "setup: r1 did not pass"; tail -12 "$LOG"; }
echo "1 $(( $(date +%s) - 3600 ))" > "$T/sas"
if ! rot -e peering_edge_rotation=r1 -e peering_edge_rotation_retire=true && grep -q "$RETIRE_CODE" "$LOG" \
   && [ -f "$S/.previous-r1/xab-key.pem" ]; then
  pass "retire while the only SA predates the rotation: $RETIRE_CODE, the set-aside key stays"
else
  fail "retire removed the set-aside key on an SA older than the rotation"; tail -12 "$LOG"
fi

# A run killed during the roll-back is reported rolled back on its resume, never done.
fresh; stage good; touch "$T/cp-fails"
rot -e peering_edge_rotation=r10 || true
rm -f "$T/cp-fails"; touch "$T/sa-up"
if ! rot -e peering_edge_rotation=r10 && grep -q "GB-302" "$LOG"; then
  pass "killed during the roll-back: the resume reports GB-302, not a switch done"
else
  fail "a run killed during the roll-back resumed as switched"; tail -12 "$LOG"
fi

# make_before_break is set explicitly, not left to the packaged strongSwan's default.
render() { # render <src> <out> [extra]
  ANSIBLE_NOCOLOR=1 ANSIBLE_RETRY_FILES_ENABLED=0 ANSIBLE_LOCALHOST_WARNING=0 ANSIBLE_INVENTORY_UNPARSED_WARNING=0 \
    ansible-playbook -i localhost, "$HERE/render-swanctl.yml" -e "src=$1" -e "out=$2" "${@:3}" > "$LOG" 2>&1
}
if render "$HERE/../templates/goat-peering-edge.strongswan.conf.j2" "$T/charon.conf" \
   && grep -Eq '^ *make_before_break = yes$' "$T/charon.conf"; then
  pass "the charon drop-in sets make_before_break = yes"
else
  fail "the charon drop-in does not set make_before_break"; cat "$T/charon.conf" 2>/dev/null | tail -5 || tail -8 "$LOG"
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
