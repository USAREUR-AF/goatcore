#!/usr/bin/env bash
# units/ipsec.sh — the IPsec transport profile (scenario ipsec-profile). Sourced by lib/unit.sh.
#
# Two sovereigns {A} and {B}. strongSwan is installed on both edge hosts when the unit
# starts (apt, charon-systemd and swanctl): the host image the other units and CI use is
# not changed. Then, in order:
#
#  1. The substrate. With a throwaway CA, an XFRM interface on each edge and a swanctl
#     connection with the suite goat-peering-edge configures (AES-256-GCM, SHA-384, P-384),
#     IKE runs between the two edges' boundary addresses and a ping crosses. This shows
#     the machine and the host containers can carry IPsec, so that a BLOCKED below names
#     the ref, not the machine. Torn down again before the driver runs.
#  2. Phases 1-10 through the avenue, both sides. Phase 3 (`goat federate init`) measures
#     capabilities.transports from the running host: ipsec-fips only where swanctl answers
#     and /proc/sys/crypto/fips_enabled reads 1. A declaration may not name a transport.
#  3. Each side's package: does it offer ipsec-fips? If both do, the agreement's transport
#     is checked. Otherwise every check that needs the ipsec-fips agreement is BLOCKED, with
#     what init measured as the reason.
#  4. Evidence while the ipsec-fips agreement is unavailable: the play phase 11 runs for an
#     ipsec-fips agreement (peering-edge.yml), invoked exactly as the driver invokes it, with
#     peering_edge_fips_waived_reason set. An optional check.
#  5. Only when the agreement selected ipsec-fips: phases 11-12 with a recorded waiver, the
#     IKE SA and CHILD SA (swanctl --list-sas), ESP counters both ways, phase 13 and strict
#     acceptance from both sides. Not a FIPS claim.

IPSEC_WAIVER="federation e2e on a stock kernel without FIPS mode; no FIPS claim is made"
IPSEC_PKGS="strongswan-swanctl charon-systemd libstrongswan-standard-plugins"

ipsec_install() { # $1 sovereign: strongSwan on its edge, charon running
  local s="$1"
  rec "ipsec-$s-strongswan-installed" --expect-in "strongSwan swanctl" -- docker exec "$HP-$s-edge" sh -c \
    "export DEBIAN_FRONTEND=noninteractive; for i in 1 2 3; do apt-get update -qq >/dev/null 2>&1 && apt-get install -y -qq --no-install-recommends $IPSEC_PKGS >/tmp/ipsec-apt.log 2>&1 && break; sleep 5; done; tail -2 /tmp/ipsec-apt.log; systemctl start strongswan && systemctl is-active strongswan && swanctl --version 2>/dev/null"
}

# The substrate: IKE between the two edges' boundary addresses, outside the driver.
ipsec_substrate() {
  local d="$WORK/ipsec-substrate" s them id rid lip rip dev
  mkdir -p "$d"
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-384 -nodes -keyout "$d/ca-key.pem" -out "$d/ca.pem" -days 2 -subj "/O=substrate/CN=substrate CA" 2>/dev/null
  for s in "$A" "$B"; do
    them="$(other "$s")"; id="sub.$s.$them.01"
    openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-384 -nodes -keyout "$d/$s-key.pem" -out "$d/$s.csr" -subj "/CN=$id" 2>/dev/null
    printf 'subjectAltName=DNS:%s\n' "$id" > "$d/$s.ext"
    openssl x509 -req -in "$d/$s.csr" -CA "$d/ca.pem" -CAkey "$d/ca-key.pem" -CAcreateserial -out "$d/$s.pem" -days 2 -extfile "$d/$s.ext" 2>/dev/null
  done
  for s in "$A" "$B"; do
    them="$(other "$s")"; id="sub.$s.$them.01"; rid="sub.$them.$s.01"
    if [ "$s" = "$A" ]; then lip=169.254.90.0; rip=169.254.90.1; else lip=169.254.90.1; rip=169.254.90.0; fi
    docker cp -q "$d/$s.pem" "$HP-$s-edge:/etc/swanctl/x509/substrate-cert.pem"
    docker cp -q "$d/$s-key.pem" "$HP-$s-edge:/etc/swanctl/private/substrate-key.pem"
    docker cp -q "$d/ca.pem" "$HP-$s-edge:/etc/swanctl/x509ca/substrate-ca.pem"
    dev="$(docker exec "$HP-$s-edge" ip -o route get "$(boundary_ip "$them")" | sed -n 's/.* dev \([^ ]*\).*/\1/p')"
    cat > "$d/$s.conf" <<CONF
connections {
  substrate {
    version = 2
    local_addrs = $(boundary_ip "$s")
    remote_addrs = $(boundary_ip "$them")
    proposals = aes256gcm16-prfsha384-ecp384
    if_id_in = 999
    if_id_out = 999
    local {
      auth = pubkey
      certs = substrate-cert.pem
      id = $id
    }
    remote {
      auth = pubkey
      id = $rid
      cacerts = substrate-ca.pem
    }
    children {
      substrate {
        local_ts = $lip/32
        remote_ts = $rip/32
        esp_proposals = aes256gcm16-ecp384
        start_action = start
      }
    }
  }
}
CONF
    docker cp -q "$d/$s.conf" "$HP-$s-edge:/etc/swanctl/conf.d/substrate.conf"
    rec "ipsec-substrate-$s-xfrm-interface" -- docker exec "$HP-$s-edge" sh -c \
      "ip link del ipsec-subst 2>/dev/null; ip link add ipsec-subst type xfrm dev $dev if_id 0x3e7 && ip addr add $lip/31 dev ipsec-subst && ip link set ipsec-subst mtu 1380 up && ip -d link show ipsec-subst"
  done
  for s in "$A" "$B"; do docker exec "$HP-$s-edge" swanctl --load-all >/dev/null 2>&1 || true; done
  rec "ipsec-substrate-ike-sa-established" --expect-in "ESTABLISHED" --expect-in "AES_GCM_16-256/PRF_HMAC_SHA2_384/ECP_384" -- bash -c \
    "for _ in \$(seq 1 20); do docker exec $HP-$A-edge swanctl --list-sas --ike substrate 2>/dev/null | grep -q INSTALLED && break; sleep 1; done; docker exec $HP-$A-edge swanctl --list-sas --ike substrate"
  rec "ipsec-substrate-ping-both-ways" -- bash -c \
    "docker exec $HP-$A-edge ping -c3 -W2 169.254.90.1 && docker exec $HP-$B-edge ping -c3 -W2 169.254.90.0"
  rec "ipsec-substrate-child-sa-counters-both-ways" -- bash -c \
    "docker exec $HP-$A-edge swanctl --list-sas --ike substrate | tee /dev/stderr | awk '/ in  /{if (match(\$0, /[0-9]+ bytes/)) i+=substr(\$0, RSTART, RLENGTH-6)} / out /{if (match(\$0, /[0-9]+ bytes/)) o+=substr(\$0, RSTART, RLENGTH-6)} END{print \"in=\" i+0, \"out=\" o+0; exit !(i > 0 && o > 0)}'"
  docker exec "$HP-$A-edge" swanctl --list-sas > "$RECORD/ipsec-substrate-sas.txt" 2>&1 || true
  # Gone again before the driver measures anything.
  for s in "$A" "$B"; do
    docker exec "$HP-$s-edge" sh -c 'swanctl --terminate --ike substrate >/dev/null 2>&1; rm -f /etc/swanctl/conf.d/substrate.conf /etc/swanctl/x509/substrate-cert.pem /etc/swanctl/private/substrate-key.pem /etc/swanctl/x509ca/substrate-ca.pem; swanctl --load-all >/dev/null 2>&1; ip link del ipsec-subst 2>/dev/null; true'
  done
}

# Phases 1-10 through the avenue, both sides; A listens and goes first.
ipsec_phases_1_to_10() {
  local first="$A" second="$B" s them
  prepare_edge "$first" "$second" true
  prepare_edge "$second" "$first" false
  for s in "$first" "$second"; do
    them="$(other "$s")"
    rec "ipsec-$s-$them-phases-1-to-5" --expect-in "exchange through the directory" -- bash -c "drive '$s' '$them' '$AVENUE' 1 5"
  done
  exchange "$first" "$second" "$AVENUE" 1
  for s in "$first" "$second"; do
    them="$(other "$s")"
    rec "ipsec-$s-$them-phases-6-to-10" --expect-in "review what apply rendered" -- bash -c "drive '$s' '$them' '$AVENUE' 6 10"
  done
}

transports_of() { # $1 sovereign: the transports its own package offers, comma separated
  docker exec "$HP-$1-edge" cat "/var/lib/goat-federation/$(other "$1")/ours.canonical.json" 2>/dev/null |
    python3 -c 'import json,sys
try: print(",".join(json.load(sys.stdin)["capabilities"]["transports"]))
except Exception: print("")' || true
}
fips_reading() { # $1 sovereign: what its edge host reads for the kernel FIPS flag
  docker exec "$HP-$1-edge" sh -c 'cat /proc/sys/crypto/fips_enabled 2>&1; true' | head -1 || true
}
init_said() { # $1 sovereign: phase 3's own line for capabilities.transports, from its log
  cat "$RECORD"/[0-9]*-ipsec-"$1"-"$(other "$1")"-phases-1-to-5.log 2>/dev/null |
    sed -n 's/^.*\(capabilities\.transports — .*\)$/\1/p' | head -1 || true
}

unit_main() {
  local s them offered=0 kind="" why="" short="" r tr c
  set_partners "$A" "$B"; set_partners "$B" "$A"; set_partners "$C" ""; export NODIR=""
  export_helpers
  setup_hosts 1
  for s in "$A" "$B"; do ipsec_install "$s"; done
  ipsec_substrate
  ipsec_phases_1_to_10

  # What init measured. Evidence steps: the package's transports and the host's FIPS flag.
  for s in "$A" "$B"; do
    them="$(other "$s")"
    rec "ipsec-$s-init-measured-transports" -- bash -c \
      "echo transports=$(transports_of "$s"); echo fips_enabled=\"$(fips_reading "$s")\"; docker exec $HP-$s-edge swanctl --version"
  done
  for s in "$A" "$B"; do
    tr="$(transports_of "$s")"
    case ",$tr," in
      *,ipsec-fips,*) rec "ipsec-$s-offers-ipsec-fips" -- echo "$s offers $tr"; offered=$((offered + 1)) ;;
      *) short="$short${short:+, }$s offers [$tr]"
         r="$s's package offers [$tr]; init said: $(init_said "$s"). goat federate init offers ipsec-fips only where swanctl answers and /proc/sys/crypto/fips_enabled reads 1 (this host: $(fips_reading "$s")); a declaration may not name a transport (measured-not-claimed) and init has no waiver"
         why="$why${why:+; }$r"
         mark "ipsec-profile.$([ "$s" = "$A" ] && echo first || echo second)-offers-ipsec-fips" BLOCKED "$r" ;;
    esac
  done

  if [ "$offered" = 2 ]; then
    kind="$(docker exec "$HP-$A-edge" python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("transport",{}).get("kind",""))' "/var/lib/goat-federation/$B/agreement.json" 2>/dev/null || true)"
    rec "ipsec-$A-$B-the-agreement-selected-ipsec-fips" --expect-in '"kind":"ipsec-fips"' -- docker exec "$HP-$A-edge" cat "/var/lib/goat-federation/$B/agreement.json"
    [ "$kind" = ipsec-fips ] || why="the agreement selected '$kind', not ipsec-fips"
  else
    why="no ipsec-fips agreement can be composed ($short; see the offers-ipsec-fips checks)"
    mark ipsec-profile.agreement-selected-ipsec-fips BLOCKED "$why"
  fi

  if [ "$kind" != ipsec-fips ]; then
    # Phase 11's ipsec-fips play, as the driver invokes it, with the waiver given.
    for s in "$A" "$B"; do
      them="$(other "$s")"
      rec "ipsec-$s-peering-edge-play-with-a-waiver" -- docker exec "$HP-$s-edge" bash -c \
        "cd /opt/goat/ops/ansible && GOAT_AWS_MODE=none GOAT_ANSIBLE_TRUNK_REMOTE=origin ../scripts/with-aws none -- ../scripts/ansible-preflight.sh -i inventory/peering-edges.ini --limit ${them}_edge -e fed_edge_group=${them}_edge -e fed_work=/var/lib/goat-federation/$them -e fed_counterparty=/var/lib/goat-federation/$them/theirs.canonical.json -e 'peering_edge_fips_waived_reason=\"$IPSEC_WAIVER\"' peering-edge.yml"
    done
    for c in first-phases-11-to-12 second-phases-11-to-12 ike-sa-established child-sa-installed first-counters second-counters acceptance-first acceptance-second; do
      mark "ipsec-profile.$c" BLOCKED "needs an agreement that selects ipsec-fips: $why"
    done
    return 0
  fi

  # The agreement selected ipsec-fips. The waiver is this sovereign's declaration, beside
  # the overlay's own group_vars for the peering.
  review_and_admit "$A" "$B"; review_and_admit "$B" "$A"
  for s in "$A" "$B"; do
    them="$(other "$s")"
    docker exec "$HP-$s-edge" sh -c "printf 'peering_edge_fips_waived_reason: \"%s\"\n' '$IPSEC_WAIVER' > /opt/goat/ops/ansible/group_vars/${them}_edge/ipsec-waiver.yml"
    rec "ipsec-$s-$them-phases-11-to-12" --expect-in "open the ipsec-fips transport" --expect-in "goat-fed-republisher-$them is running" -- bash -c "drive '$s' '$them' '$AVENUE' 11 12"
  done
  rec "ipsec-$A-$B-ike-sa-established" --expect-in "ESTABLISHED" -- bash -c \
    "for _ in \$(seq 1 30); do docker exec $HP-$A-edge swanctl --list-sas | grep -q ESTABLISHED && break; sleep 2; done; docker exec $HP-$A-edge swanctl --list-sas"
  rec "ipsec-$A-$B-child-sa-installed" --expect-in "INSTALLED" -- docker exec "$HP-$A-edge" swanctl --list-sas
  for s in "$A" "$B"; do
    them="$(other "$s")"
    rec "ipsec-$s-$them-phase-13" --expect-in "federate the SPIRE trust domains" -- bash -c "drive '$s' '$them' '$AVENUE' 13 13"
  done
  for s in "$A" "$B"; do
    them="$(other "$s")"
    host_exec "$HP-$s-core" sh -c 'cat > /usr/local/bin/fed-responder.py' < "$SRC/ops/federation/roundtrip/responder.py"
    docker exec -d "$HP-$s-core" python3 /usr/local/bin/fed-responder.py "from/$them" "release/all"
    start_probe "$s" "$them" agreed "/var/lib/goat-federation/$them/pki/peering-outward.pem" "/var/lib/goat-federation/$them/pki/peering-outward-key.pem" "$(probe_port "$s" "$them" 8101)"
    start_probe "$s" "$them" forged /etc/fed-e2e/forged.pem /etc/fed-e2e/forged-key.pem "$(probe_port "$s" "$them" 8102)"
    start_probe "$s" "$them" impersonate "/etc/fed-e2e/impersonate-$them.pem" "/etc/fed-e2e/impersonate-$them-key.pem" "$(probe_port "$s" "$them" 8103)"
  done
  for s in "$A" "$B"; do
    them="$(other "$s")"
    rec "ipsec-acceptance-as-$s-with-$them" --expect-in "acceptance-counts:" --expect-in "skip=0" --expect-in "fail=0" --expect-not-in "ACCEPTANCE FAILED" --expect-not-in "ACCEPTANCE INCOMPLETE" \
      -- bash -c "accept_as '$s' '$them' '$AVENUE'"
  done
  # After acceptance's own traffic: ESP bytes in and out on each side.
  for s in "$A" "$B"; do
    rec "ipsec-$s-counters-move-both-ways" -- bash -c \
      "docker exec $HP-$s-edge swanctl --list-sas | tee /dev/stderr | awk '/ in  /{if (match(\$0, /[0-9]+ bytes/)) i+=substr(\$0, RSTART, RLENGTH-6)} / out /{if (match(\$0, /[0-9]+ bytes/)) o+=substr(\$0, RSTART, RLENGTH-6)} END{print \"in=\" i+0, \"out=\" o+0; exit !(i > 0 && o > 0)}'"
  done
}
