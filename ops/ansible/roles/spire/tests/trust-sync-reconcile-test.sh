#!/usr/bin/env bash
# Behavioural test for spire-trust-sync trust RECONCILIATION and peering
# RETIREMENT. It renders the real template, runs the rendered script against a
# fake `docker` and real self-signed certificates, and inspects the trust files
# afterwards.
#
# WHY A BEHAVIOURAL TEST AND NOT MORE STRING ASSERTIONS. Trust removal is the
# operation most likely to be quietly wrong, because it fails SAFE in testing and
# DANGEROUS in production: a test that forgets to remove something still passes,
# while a host that forgets to remove something still trusts a partner you
# stopped peering with. render-check.py asserts the emitted shell says the right
# things. This asserts the emitted shell DOES the right things — that a
# certificate present before the run is absent after it.
#
# The five cases, each of which has a way of being wrong that the other four
# would not catch:
#
#   1. two active peerings          -> each edge holds our own bundle ⊕ its own
#                                      partner, and NOT the other partner.
#   2. one peering retired          -> the retired edge's partner certificate is
#                                      GONE, our own bundle survives, the other
#                                      edge is untouched, the router restarted.
#   3. partner bundle fetch fails   -> the edge KEEPS that partner's certificate.
#                                      A transient failure must never reduce
#                                      trust. This is the case that would look
#                                      like a pass if reconciliation were wrong.
#   4. a stale foreign authority    -> an authority no declaration claims (what
#                                      the pre-isolation all-partners fold left
#                                      behind) is removed on the next tick.
#   5. --dry-run                    -> reports the removal and performs none of
#                                      it; the file on disk is byte-identical.
#
#   ops/ansible/roles/spire/tests/trust-sync-reconcile-test.sh
#
# Exit 0 = pass, 1 = fail, 2 = a prerequisite (python3+jinja2, openssl) missing.
set -uo pipefail

command -v openssl >/dev/null 2>&1 || { echo "ERROR: openssl required" >&2; exit 2; }
python3 -c 'import jinja2' 2>/dev/null || { echo "ERROR: jinja2 required (pip install jinja2)" >&2; exit 2; }

HERE="$(cd "$(dirname "$0")" && pwd)"
TPL_DIR="$HERE/../templates"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fails=0
check() {  # $1 = condition already evaluated as 0/1 via caller, $2 = message
  if [ "$1" = 0 ]; then printf '  PASS: %s\n' "$2"; else printf '  FAIL: %s\n' "$2"; fails=$((fails + 1)); fi
}
contains() {  # $1 = trust file, $2 = single-cert pem — compares base64 bodies
  local hay nee
  hay="$(grep -v -- '-----' < "$1" | tr -d ' \n')"
  nee="$(grep -v -- '-----' < "$2" | tr -d ' \n')"
  [ -n "$nee" ] && case "$hay" in *"$nee"*) return 0 ;; *) return 1 ;; esac
}
# The four assertions, named so a NEGATIVE reads as plainly as a positive. The
# negatives are the load-bearing ones here: "this certificate is absent" is the
# whole point of the change, and it is the assertion that passes for free if the
# removing code never runs.
have()     { if contains "$1" "$2"; then check 0 "$3"; else check 1 "$3"; fi; }
lack()     { if contains "$1" "$2"; then check 1 "$3"; else check 0 "$3"; fi; }
logged()   { if grep -q "$1" "$WORK/out.log"; then check 0 "$2"; else check 1 "$2"; fi; }
unlogged() { if grep -q "$1" "$WORK/out.log"; then check 1 "$2"; else check 0 "$2"; fi; }

# --- fixtures ----------------------------------------------------------------
mkdir -p "$WORK/bundles" "$WORK/bin" "$WORK/edge" "$WORK/core"
mkcert() {  # $1 = name -> $WORK/<name>.pem
  openssl req -x509 -newkey rsa:2048 -nodes -keyout /dev/null \
    -out "$WORK/$1.pem" -days 2 -subj "/CN=$1-ca" >/dev/null 2>&1
}
mkcert own; mkcert partner-a; mkcert partner-b
cp "$WORK/partner-a.pem" "$WORK/bundles/aaa.partner.example.pem"
cp "$WORK/partner-b.pem" "$WORK/bundles/bbb.partner.example.pem"

CORE="$WORK/core/ca-root-cert.pem"
EDGE_A="$WORK/edge/partner-a-ca.pem"
EDGE_B="$WORK/edge/partner-b-ca.pem"
: > "$CORE"; : > "$EDGE_A"; : > "$EDGE_B"

# A fake `docker` covering exactly the four calls the script makes. `bundle list
# -id` reads $WORK/bundles/<domain>.pem and EXITS NONZERO when it is absent —
# which is both the before-first-exchange state and the transient-failure state,
# and is what case 3 exercises.
cat > "$WORK/bin/docker" <<'SHIM'
#!/usr/bin/env bash
case "$1" in
  exec)
    shift
    [ "$1" = "-i" ] && shift
    shift 2                      # container name, spire-server binary path
    case "$1" in
      healthcheck) exit 0 ;;
      bundle)
        case "$2" in
          show) cat "$FAKE_OWN"; exit 0 ;;
          list)
            id=""
            while [ $# -gt 0 ]; do [ "$1" = "-id" ] && id="$2"; shift; done
            [ -n "$id" ] || { echo "shim: refusing an unqualified bundle list" >&2; exit 1; }
            f="$FAKE_BUNDLES/${id#spiffe://}.pem"
            [ -f "$f" ] || exit 1
            printf '****************************************\n* %s\n****************************************\n' "${id#spiffe://}"
            cat "$f"; exit 0 ;;
        esac
        exit 1 ;;
    esac
    exit 1 ;;
  restart) echo "$2" >> "$FAKE_RESTARTS"; exit 0 ;;
  ps|logs) exit 0 ;;
esac
exit 1
SHIM
chmod +x "$WORK/bin/docker"

export FAKE_OWN="$WORK/own.pem" FAKE_BUNDLES="$WORK/bundles" FAKE_RESTARTS="$WORK/restarts"
: > "$FAKE_RESTARTS"

# --- render + run ------------------------------------------------------------
render() {  # $1 = json edges, $2 = json retired, [$3 = prune true/false]
  TPL_DIR="$TPL_DIR" EDGES="$1" RETIRED="$2" PRUNE="${3:-true}" \
  DATA_DIR="$WORK" CORE_TRUST="$CORE" BIN_DIR="/opt/spire/bin" python3 - > "$WORK/spire-trust-sync.sh" <<'PY'
import json, os
from jinja2 import Environment, FileSystemLoader
env = Environment(loader=FileSystemLoader(os.environ["TPL_DIR"]), trim_blocks=True)
print(env.get_template("spire-trust-sync.sh.j2").render(
    ansible_managed="TEST RENDER",
    spire_data_dir=os.environ["DATA_DIR"],
    spire_compose_dir=os.environ["DATA_DIR"],
    spire_bin_dir=os.environ["BIN_DIR"],
    spire_trust_domain="zagros.goat.example",
    spire_agent_node_path="agent/thor",
    spire_agent_rescue_enabled=False,
    spire_trust_sync_interval="15min",
    spire_trust_sync_router_trust_file=os.environ["CORE_TRUST"],
    spire_trust_sync_restart_containers=["edge-router"],
    spire_trust_sync_federated_edges=json.loads(os.environ["EDGES"]),
    spire_trust_sync_federated_edges_retired=json.loads(os.environ["RETIRED"]),
    spire_trust_sync_prune_enabled=os.environ["PRUNE"] == "true",
    spire_federation_enabled=True,
))
PY
  chmod +x "$WORK/spire-trust-sync.sh"
}
run() { PATH="$WORK/bin:$PATH" "$WORK/spire-trust-sync.sh" "$@" > "$WORK/out.log" 2>&1; }

mkdir -p "$WORK/server"
ACTIVE_BOTH="[{\"trust_file\":\"$EDGE_A\",\"trust_domain\":\"aaa.partner.example\"},{\"trust_file\":\"$EDGE_B\",\"trust_domain\":\"spiffe://bbb.partner.example\"}]"
ACTIVE_B_ONLY="[{\"trust_file\":\"$EDGE_B\",\"trust_domain\":\"bbb.partner.example\"}]"
RETIRED_A="[{\"trust_file\":\"$EDGE_A\",\"trust_domain\":\"aaa.partner.example\"}]"

# --- 1: two active peerings, each edge gets its own partner and no other ------
echo "1. two active peerings"
render "$ACTIVE_BOTH" "[]"; run
have "$EDGE_A" "$WORK/own.pem" "edge A trusts our own bundle"
have "$EDGE_A" "$WORK/partner-a.pem" "edge A trusts partner A"
lack "$EDGE_A" "$WORK/partner-b.pem" "edge A does NOT trust partner B"
have "$EDGE_B" "$WORK/partner-b.pem" "edge B trusts partner B"
lack "$EDGE_B" "$WORK/partner-a.pem" "edge B does NOT trust partner A"
have "$CORE" "$WORK/own.pem" "the core trusts our own bundle"
lack "$CORE" "$WORK/partner-a.pem" "the core trusts NO partner"

# --- 2: retire peering A -----------------------------------------------------
# THE PROPERTY. Declaring A retired must make A's authority ABSENT from edge A,
# not merely stop it being re-added.
echo "2. peering A retired"
: > "$FAKE_RESTARTS"
render "$ACTIVE_B_ONLY" "$RETIRED_A"; run
lack "$EDGE_A" "$WORK/partner-a.pem" "partner A's authority is GONE from edge A"
have "$EDGE_A" "$WORK/own.pem" "edge A still trusts our own bundle (not emptied)"
have "$EDGE_B" "$WORK/partner-b.pem" "edge B is untouched by A's retirement"
logged 'REMOVE trust from' "the removal is named in the log"
if grep -q 'edge-router' "$FAKE_RESTARTS"; then check 0 "the edge router was restarted so it re-reads the file"
else check 1 "the edge router was restarted so it re-reads the file"; fi

# --- 3: the partner bundle cannot be fetched ---------------------------------
# A transient `bundle list` failure must never be the reason a partner stops
# being trusted. This is the case that passes trivially if removal is broken, so
# it is the one worth stating loudest.
echo "3. partner B's bundle is unfetchable"
render "$ACTIVE_BOTH" "[]"; run                      # re-establish A on edge A
have "$EDGE_A" "$WORK/partner-a.pem" "edge A trusts partner A again after re-declaring"
mv "$WORK/bundles/bbb.partner.example.pem" "$WORK/bbb.hidden.pem"
run
have "$EDGE_B" "$WORK/partner-b.pem" "edge B STILL trusts partner B after a failed fetch"
have "$EDGE_B" "$WORK/own.pem" "edge B still trusts our own bundle"
logged 'appended, NOT reconciled' "the log says the file was not reconciled and why"
unlogged 'REMOVE trust from' "nothing was removed on the failed-fetch tick"
mv "$WORK/bbb.hidden.pem" "$WORK/bundles/bbb.partner.example.pem"

# --- 4: a stale authority nothing declares -----------------------------------
# What the pre-isolation all-partners fold left on a host: partner B's authority
# sitting in partner A's edge file, which the isolation fix alone cannot undo.
echo "4. a stale authority no declaration claims"
cat "$WORK/partner-b.pem" >> "$EDGE_A"
have "$EDGE_A" "$WORK/partner-b.pem" "(setup) edge A carries the stale partner B authority"
render "$ACTIVE_BOTH" "[]"; run
lack "$EDGE_A" "$WORK/partner-b.pem" "the stale partner B authority is removed from edge A"
have "$EDGE_A" "$WORK/partner-a.pem" "edge A keeps the partner it is actually peered with"

# --- 5: --dry-run reports and changes nothing --------------------------------
echo "5. --dry-run"
cat "$WORK/partner-b.pem" >> "$EDGE_A"
before="$(cksum < "$EDGE_A")"
: > "$FAKE_RESTARTS"
render "$ACTIVE_BOTH" "[]"; run --dry-run
logged 'would REMOVE trust from' "the dry run names the certificate it would remove"
logged 'partner-b-ca' "and identifies it by subject, not just by file"
if [ "$(cksum < "$EDGE_A")" = "$before" ]; then check 0 "the trust file is byte-identical after the dry run"
else check 1 "the trust file is byte-identical after the dry run"; fi
if [ ! -s "$FAKE_RESTARTS" ]; then check 0 "no container was restarted during the dry run"
else check 1 "no container was restarted during the dry run"; fi

# --- 6: reconciliation disabled ----------------------------------------------
# The escape hatch must be honest about what it costs.
echo "6. reconciliation disabled with a retirement declared"
render "$ACTIVE_B_ONLY" "$RETIRED_A" false; run
have "$EDGE_A" "$WORK/partner-a.pem" "with pruning off, A's authority survives (as documented)"
logged 'Retirement is not' "and every tick says the retirement is not in effect"

if [ "$fails" -ne 0 ]; then echo "FAIL ($fails)"; exit 1; fi
echo "PASS"
