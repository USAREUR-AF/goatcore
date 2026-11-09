#!/usr/bin/env bash
# mint-peering-leaves.sh — mint this side's peering certificates from the
# agreement, so their names are DERIVED rather than typed.
#
# THE DEFECT THIS CLOSES. these were minted by hand, three times,
# and two of the three failed silently:
#
#   - a core certificate carried no SAN for the address its edge dials, so the
#     edge refused the dial with nothing logged at INFO and no error anywhere;
#   - an inward certificate carried a CN the core's ACL did not name, so the
#     link established and matched no subject.
#
# Both present as "it is up and nothing is happening", which is the most
# expensive failure shape this system has. Every name below comes
# out of the agreement, and every address that will be dialled goes in as a SAN.
#
# FOUR CERTIFICATES, and they are not interchangeable:
#
#   outward   presented TO the partner.   CN = fed.<us>.<them>.NN   (ours, derived)
#   inward    presented to OUR core.      CN = fed.<them>.<us>.NN   (the agreed
#             per-partner name our core's ACL matches; signed by OUR CA, so no
#             foreign authority ever reaches a core router — F-424)
#   core      presented BY our core.      CN = the core's own name, with a SAN
#             for every address the edge is configured to dial
#   republisher  presented to OUR core by the outbound republisher.
#             CN = fed.<us>.<them>.republisher.01 (ours, derived; the subject the
#             core fragment `apply` renders admits it to read the offered release
#             sets and write our wire name, nothing else). Never shown to them.
#
# Usage:
#   mint-peering-leaves.sh <agreement.json> <out-dir> [--as <sovereign>]
#                          [--ca-cert <pem>] [--ca-key <pem>]
#                          [--ca-key-age <file.age> --age-identity <keys.txt>]
#                          [--core-cn <name>] [--core-san <san>[,<san>...]]
#                          [--link-ip <ip>] [--rotate] [--min-valid-seconds <n>]
#
# Environment: FED_CA_CERT, FED_CA_KEY, FED_CA_KEY_AGE, FED_AGE_IDENTITY,
#              FED_CORE_CN, FED_CORE_SAN, FED_LINK_IP
#
# THE CA KEY. An explicit key file (--ca-key, FED_CA_KEY) always wins. Without
# one, the deployment's age-encrypted key (--ca-key-age, FED_CA_KEY_AGE; the
# driver derives it from <deploy.state_dir>/<site>-ca/ca.key.age) is opened with
# its age identity into a file this run owns: mode 0600 in a fresh private
# directory, removed when the script exits, whatever the exit. The key is
# needed only to issue a missing leaf or to rotate.
set -uo pipefail

AGREEMENT=""; OUT=""; AS=""
CA_CERT="${FED_CA_CERT:-}"; CA_KEY="${FED_CA_KEY:-}"
CA_KEY_AGE="${FED_CA_KEY_AGE:-}"; AGE_IDENTITY="${FED_AGE_IDENTITY:-}"
CORE_CN="${FED_CORE_CN:-goat-zenoh}"; CORE_SAN="${FED_CORE_SAN:-}"; LINK_IP="${FED_LINK_IP:-}"
DAYS="${FED_LEAF_DAYS:-90}"
ROTATE=0; MIN_VALID_SECONDS="${FED_LEAF_MIN_VALID_SECONDS:-86400}"
HELPER="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/peering_leaf_material.py"

while [ $# -gt 0 ]; do
  case "$1" in
    --rotate)   ROTATE=1; shift ;;
    --min-valid-seconds) MIN_VALID_SECONDS="$2"; shift 2 ;;
    --as)       AS="$2"; shift 2 ;;
    --ca-cert)  CA_CERT="$2"; shift 2 ;;
    --ca-key)   CA_KEY="$2"; shift 2 ;;
    --ca-key-age)   CA_KEY_AGE="$2"; shift 2 ;;
    --age-identity) AGE_IDENTITY="$2"; shift 2 ;;
    --core-cn)  CORE_CN="$2"; shift 2 ;;
    --core-san) CORE_SAN="$2"; shift 2 ;;
    --link-ip)  LINK_IP="$2"; shift 2 ;;
    -h|--help)  sed -n '2,44p' "$0"; exit 0 ;;
    -*)         echo "unknown flag $1" >&2; exit 2 ;;
    *)          if [ -z "$AGREEMENT" ]; then AGREEMENT="$1"; else OUT="$1"; fi; shift ;;
  esac
done
[ -n "$AGREEMENT" ] && [ -n "$OUT" ] || { echo "usage: mint-peering-leaves.sh <agreement.json> <out-dir> [--as <sovereign>]" >&2; exit 2; }
[ -f "$AGREEMENT" ] || { echo "no agreement at $AGREEMENT" >&2; exit 2; }

fail() { echo "[✗] $1" >&2; [ -n "${2:-}" ] && printf '     %s\n' "$2" >&2; exit 1; }

# --- the two derived names, read from the agreement, never typed --------------
read -r US THEM OURS_CN THEIRS_CN DERIVED_LINK_IP REPUBLISHER_CN CA_FINGERPRINT <<EOF
$(python3 - "$AGREEMENT" "$AS" <<'PY'
import json, sys
ag = json.load(open(sys.argv[1]))
want = sys.argv[2] or ""
parties = [p["sovereign_id"] for p in ag.get("parties", [])]
if len(parties) != 2:
    print("ERR ERR ERR ERR -"); sys.exit(0)
if want:
    if want not in parties:
        print("ERR ERR ERR ERR -"); sys.exit(0)
    us = want
else:
    us = parties[0]
them = [p for p in parties if p != us][0]
# fed.<issuer>.<counterparty>.<ordinal> — spec section 3.1, derived on both sides
def cn(issuer, counterparty): return f"fed.{issuer}.{counterparty}.01"
# The link address the far side dials: parties[0] holds the first of the agreed
# /31 or /30 and parties[1] the second, the rule `goat federate apply` renders.
import ipaddress
link = "-"
subnet = (ag.get("transport") or {}).get("link_subnet", "")
if subnet:
    net = ipaddress.ip_network(subnet)
    hosts = list(net) if net.prefixlen == 31 else list(net.hosts())
    if len(hosts) == 2:
        link = str(hosts[parties.index(us)])
fingerprint = next(p.get("ca_fingerprint", "-") for p in ag["parties"] if p["sovereign_id"] == us)
print(us, them, cn(us, them), cn(them, us), link, f"fed.{us}.{them}.republisher.01", fingerprint)
PY
)
EOF
[ "$US" = "ERR" ] && fail "could not read both parties from the agreement" \
  "Pass --as <sovereign> when the agreement does not say which side we are."

# An address read from the agreement is not one anybody can type wrong. --link-ip
# stays for an agreement carrying no transport member.
[ -n "$LINK_IP" ] || { [ "$DERIVED_LINK_IP" = "-" ] || LINK_IP="$DERIVED_LINK_IP"; }

echo "  agreement names us '$US', counterparty '$THEM'"
echo "  outward CN (to them):   $OURS_CN"
echo "  inward  CN (to core):   $THEIRS_CN   <- the name our core's ACL matches"
echo "  republisher CN (core):  $REPUBLISHER_CN"

# Validate every requested pair before issuing or replacing anything. The CA
# private key is needed only for missing leaves or explicit rotation.
[ -n "$CA_CERT" ] || fail "no CA certificate" "Set FED_CA_CERT to OUR OWN CA."
LEAVES=()
mint() { LEAVES+=(--leaf "$1" "$2" "$3"); }

# Outward: what the partner dials. The link address must be a SAN or their
# name verification fails, and on this software that failure is quiet.
osan="DNS:$OURS_CN"
[ -n "$LINK_IP" ] && osan="$osan,IP:$LINK_IP"
mint peering-outward "$OURS_CN" "$osan"

# Inward: an identity from OUR domain carrying the agreed per-partner CN, which
# is what our core's generated ACL fragment names as its subject.
mint peering-inward "$THEIRS_CN" "DNS:$THEIRS_CN"

# Republisher: our own client of our own core. Both of its sessions dial the core,
# so it needs no SAN beyond its name; the core fragment's `fed-rp-<them>` subject
# matches this CN.
mint republisher "$REPUBLISHER_CN" "DNS:$REPUBLISHER_CN"

# Core: presented BY our core TO this edge. Every address the edge may dial has
# to appear here — a missing SAN is the silent failure this script exists for.
if [ -n "$CORE_SAN" ]; then
  csan="DNS:$CORE_CN"
  IFS=','; for s in $CORE_SAN; do
    case "$s" in
      *:*)           entry="$s" ;;
      *[0-9].[0-9]*) entry="IP:$s" ;;
      *)             entry="DNS:$s" ;;
    esac
    # The CN is already in the list; adding it again from --core-san produces a
    # duplicate SAN, which is legal and reads as carelessness in a certificate
    # someone else has to review.
    case ",$csan," in *",$entry,"*) continue ;; esac
    csan="$csan,$entry"
  done; unset IFS
  mint core "$CORE_CN" "$csan"
else
  echo "  [ ] core certificate SKIPPED — no --core-san given."
  echo "      Pass every address the edge is configured to dial. A core"
  echo "      certificate without a SAN for the dialled address fails name"
  echo "      verification, and the edge logs nothing at INFO when it does."
fi

# The CA key from the deployment's declared source, when no key file is given.
RUN_KEY_DIR=""
cleanup_key() { [ -z "$RUN_KEY_DIR" ] || rm -rf "$RUN_KEY_DIR"; }
trap cleanup_key EXIT
trap 'cleanup_key; exit 130' INT
trap 'cleanup_key; exit 143' TERM
trap 'cleanup_key; exit 129' HUP
if [ -z "$CA_KEY" ] && [ -n "$CA_KEY_AGE" ]; then
  why=""
  if [ ! -f "$CA_KEY_AGE" ]; then
    why="$CA_KEY_AGE does not exist"
  elif [ -z "$AGE_IDENTITY" ] || [ ! -f "$AGE_IDENTITY" ]; then
    why="no age identity at ${AGE_IDENTITY:-(unset; set FED_AGE_IDENTITY)}"
  elif ! command -v age >/dev/null 2>&1; then
    why="the age command is not installed"
  else
    # $TMPDIR when set; else memory-backed /dev/shm where there is one, so the
    # plaintext never reaches a disk; else /tmp.
    key_parent="${TMPDIR:-}"
    if [ -z "$key_parent" ]; then
      if [ -d /dev/shm ] && [ -w /dev/shm ]; then key_parent=/dev/shm; else key_parent=/tmp; fi
    fi
    RUN_KEY_DIR="$(umask 077 && mktemp -d "$key_parent/fed-ca-key.XXXXXX")" || why="no private temporary directory"
    if [ -z "$why" ]; then
      if ( umask 077 && age -d -i "$AGE_IDENTITY" -o "$RUN_KEY_DIR/ca-key.pem" -- "$CA_KEY_AGE" ) 2>"$RUN_KEY_DIR/age.err"; then
        CA_KEY="$RUN_KEY_DIR/ca-key.pem"
        echo "  CA key: opened $CA_KEY_AGE for this run; the copy is removed when it ends"
      else
        # age repeats a quoted line of a file it cannot read, which is a
        # secret when an identity file is named by mistake: quoted text and
        # anything shaped like an age secret key are never repeated.
        said="$(head -n 1 "$RUN_KEY_DIR/age.err" | sed -e 's/"[^"]*"/"..."/g' -e 's/AGE-SECRET-KEY-[A-Za-z0-9]*/AGE-SECRET-KEY-.../g' | cut -c1-200)"
        why="$CA_KEY_AGE did not open with $AGE_IDENTITY (age: ${said:-no message})"
      fi
    fi
  fi
  # A key is needed only to issue or rotate; a re-run that reuses every leaf
  # goes on without one, and the helper refuses if it does need it.
  [ -z "$why" ] || echo "  [!] CA key not opened: $why" >&2
fi

ARGS=(--out "$OUT" --ca-cert "$CA_CERT" --ca-fingerprint "$CA_FINGERPRINT" --days "$DAYS"
      --min-valid-seconds "$MIN_VALID_SECONDS")
[ -z "$CA_KEY" ] || ARGS+=(--ca-key "$CA_KEY")
[ "$ROTATE" = 0 ] || ARGS+=(--rotate)
python3 "$HELPER" "${ARGS[@]}" "${LEAVES[@]}" || exit 1

cat <<DONE

  Validated material in $OUT. Wire them to the edge role as:

    goat_zenoh_edge_outward_cert: $OUT/peering-outward.pem
    goat_zenoh_edge_outward_key:  $OUT/peering-outward-key.pem
    goat_zenoh_edge_inward_cert:  $OUT/peering-inward.pem
    goat_zenoh_edge_inward_key:   $OUT/peering-inward-key.pem
    goat_zenoh_edge_partner_cn:   $THEIRS_CN     (set by \`goat federate apply\`)

  And the republisher role, on the core host the edge dials, as:

    goat_fed_republisher_cert:    $OUT/republisher.pem
    goat_fed_republisher_key:     $OUT/republisher-key.pem
    goat_fed_republisher_cn:      $REPUBLISHER_CN     (set by \`goat federate apply\`)

  The partner's CA — and ONLY the partner's CA — goes in
  goat_zenoh_edge_partner_ca_path. Never in a core router's trust store.

  Or set goat_zenoh_edge_install_certs: true and leave the edge paths unset:
  phase 12 checks these files and the partner CA and installs them on the edge.
DONE
