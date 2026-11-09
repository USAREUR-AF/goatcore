#!/usr/bin/env bash
# peering-standup.sh — THE single entrypoint for standing up a cross-sovereign peering.
#
# THE DEFECT THIS CLOSES. Federation had every hard part solved — capabilities
# measured rather than typed, one signed agreement as the source of truth, one
# writer, refusals that must fire — and no driver. The ordering lived in a
# six-step runbook a human followed, so a peering stood up needed
# roughly a dozen improvised interventions, and four defects were
# invisible until the GENERATED artifacts were the thing deployed. This script IS
# the ordering, executable and inspectable.
#
# Same shape as ops/scripts/goat-standup.sh on purpose: numbered phases, a kind
# per phase, --plan and --from/--to for controlled resumption. Someone who has run one
# has effectively run the other. See docs/design/staged-delivery-pattern.md.
#
# Usage:
#   ops/scripts/peering-standup.sh <peering-name> [--plan] [--from N] [--to N]
#                                                 [--as <sovereign>] [--edge-group <group>]
#     --plan        print the ordered phase plan and the exact command per phase;
#                   execute NOTHING. Offline-safe; start here, always.
#     --from/--to   run a sub-range — resume after a manual step, or after a
#                   failed phase once its cause is fixed. Every phase is
#                   resumable; inspect its inputs and end state before re-running.
#     --as          which party we are (default: read from the declaration)
#     --edge-group  the ansible group the edge runs in (default: <peering>_edge)
#
#   The outbound republisher runs on our core host, the host the edge dials, in
#   the inventory group <peering>_core (a child of [peering_cores]).
#
#   Environment:
#     GOAT_BIN            the goat binary (default: whatever is on PATH)
#     FED_VERBS           who runs the federation verbs of phases 3, 4, 6, 7 and 9:
#                         `goatbuilder` (the default: `$GOATBUILDER_BIN federate <verb>`)
#                         or `goat` (`$GOAT_BIN federate <verb>`, the same library,
#                         flags and exit codes)
#     GOATBUILDER_BIN     the goatbuilder binary (default: whatever is on PATH)
#     FED_WORK            working directory (default: /var/lib/goat-federation/<peering>)
#     FED_COUNTERPARTY    path to the counterparty's package (phase 6 needs it)
#     FED_SIGNING_KEY     our Ed25519 signing key (64 hex or 44 base64)
#     FED_ROUTER_DOCKER_HOST  DOCKER_HOST for phase 3 when the core router is not on
#                         this host, e.g. ssh://<user>@<core>. `init` reads the
#                         federation interface here and the running router there; the
#                         edge must not share a machine with the core.
#     FED_ROUTER_CONTAINER  optional exported core container name or ID for phase 3;
#                         unset, init discovers a running core by role label or
#                         standard/legacy name. Multiple candidates refuse.
#     GOAT_AWS_PROFILE    aws-vault profile the ansible phases run under
#     FED_DIRECTORY_URL   the peering directory (saltlick), required unless
#                         FED_NO_DIRECTORY=1. Phase 5 uploads our package and CA,
#                         publishes our pointer, fetches the counterparty's package
#                         and CA, and prints both keys and CA fingerprints and a
#                         compatibility report before it stops. It grants nothing:
#                         compose checks their package against the key read aloud.
#     FED_NO_DIRECTORY    1 only for a counterparty that can reach no directory;
#                         phase 5 then names the files to hand over directly
#     FED_PACKAGE_URL     optional: where our own public host serves our package
#                         (https, a static file). Unset, phase 5 uploads our
#                         package to the directory and the pointer names that, so
#                         a party behind NAT needs no public host
#     FED_CA_CERT         our own CA certificate (PEM), the one phase 8 signs the
#                         peering leaves with; phase 5 uploads it to the directory
#                         (unset there, the profile's ca-roots.pem). Only the
#                         certificate whose SHA-256 is our identity.ca_fingerprint
#                         is sent
#     FED_COUNTERPARTY_ID the counterparty's sovereign_id, to look them up; phase 5
#                         fetches their package and CA (to $FED_WORK/theirs-ca.pem)
#
# WHAT THIS DOES NOT DO. It never trusts anything from the counterparty on its
# own. The two exchanges — our package and CA out, theirs in — are phase 5, through
# the directory, and phase 5 stops for the one act a machine cannot do: reading
# the verifying key and CA fingerprint aloud, and recording the key once read back.
set -uo pipefail
# Hold the Mac awake for the whole run (#2122; ops/scripts/lib/keep-awake.sh).
_keep_awake="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/keep-awake.sh"
# shellcheck source=ops/scripts/lib/keep-awake.sh
if [ -f "$_keep_awake" ]; then . "$_keep_awake"; goat_keep_awake; fi

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PEERING=""; PLAN=0; FROM=1; TO=99; AS=""; EDGE_GROUP=""

while [ $# -gt 0 ]; do
  case "$1" in
    --plan)       PLAN=1; shift ;;
    --from)       FROM="$2"; shift 2 ;;
    --to)         TO="$2"; shift 2 ;;
    --as)         AS="$2"; shift 2 ;;
    --edge-group) EDGE_GROUP="$2"; shift 2 ;;
    -h|--help)    sed -n '2,67p' "$0"; exit 0 ;;
    -*)           echo "unknown flag $1" >&2; exit 2 ;;
    *)            [ -z "$PEERING" ] && PEERING="$1"; shift ;;
  esac
done
[ -n "$PEERING" ] || { echo "usage: peering-standup.sh <peering-name> [--plan] [--from N] [--to N]" >&2; exit 2; }

: "${GOAT_BIN:=$(command -v goat || echo goat)}"
: "${GOATBUILDER_BIN:=$(command -v goatbuilder || echo goatbuilder)}"
case "${FED_VERBS:-goatbuilder}" in
  goat)        FED_VERB="$GOAT_BIN federate" ;;
  goatbuilder) FED_VERB="$GOATBUILDER_BIN federate" ;;
  *) echo "peering-standup.sh: FED_VERBS is goat or goatbuilder, not '$FED_VERBS'" >&2; exit 2 ;;
esac
: "${FED_WORK:=/var/lib/goat-federation/$PEERING}"
: "${FED_COUNTERPARTY:=$FED_WORK/theirs.canonical.json}"
: "${FED_SIGNING_KEY:=}"
# GOAT_AWS_PROFILE, then the project's own deploy.aws_profile
# (ops/goatbuilder/goatbuilder.yaml), then the name the AWS tools fall back to
# themselves: the three goat-standup.sh and the builder read, in that order.
if [ -z "${GOAT_AWS_PROFILE:-}" ] && [ -f "$REPO/ops/goatbuilder/goatbuilder.yaml" ]; then
  GOAT_AWS_PROFILE="$(python3 -c 'import sys, yaml; print(str(((yaml.safe_load(open(sys.argv[1])) or {}).get("deploy") or {}).get("aws_profile") or "").strip())' "$REPO/ops/goatbuilder/goatbuilder.yaml" 2>/dev/null || true)"
fi
: "${GOAT_AWS_PROFILE:=default}"

# THE PROFILE DIRECTORY, RESOLVED ONCE, HERE. Every phase that reads the profile
# resolves it from $HOME, and `sudo -E` PRESERVES the invoking user's HOME — so a
# driver run under sudo reads /home/<user>/.goat while the fabric's profile is
# root's. The symptom is phase 1 passing and phase 2 refusing on a declaration
# that plainly exists, which is the most confusing shape a fail-closed gate can
# take. Resolve it once and export it, so preflight and every `goat` call below
# agree by construction rather than by coincidence.
if [ -z "${GOAT_PROFILE_DIR:-}" ]; then
  if [ "$(id -u)" = "0" ]; then GOAT_PROFILE_DIR=/root/.goat; else GOAT_PROFILE_DIR="$HOME/.goat"; fi
fi
export GOAT_PROFILE_DIR
[ -n "$EDGE_GROUP" ] || EDGE_GROUP="${PEERING}_edge"
# The core host the edge dials: the outbound republisher runs there, beside our
# core router, and both of its sessions dial that router (phase 12).
CORE_GROUP="${PEERING}_core"

# THE BOUND KEYS. `compose` refuses (exit 20) unless a verifying key is bound to
# each party, and it will never take one from the package it verifies — a package
# carrying its own key authenticates nothing. So the keys live in a file the
# operator writes during phase 5, after reading them aloud, and phase 6 passes them.
: "${FED_KEYRING:=$FED_WORK/keys.txt}"
OURS="$FED_WORK/ours.canonical.json"
AGREEMENT="$FED_WORK/agreement.json"
APPLIED="$FED_WORK/applied"
PKI="$FED_WORK/pki"

step() {   # step <n> <kind> <label> <command...>
  local n="$1" kind="$2" label="$3"; shift 3
  [ "$n" -lt "$FROM" ] && return 0
  [ "$n" -gt "$TO" ] && return 0
  printf '\n=== Phase %s [%s] %s ===\n' "$n" "$kind" "$label"
  printf '    %s\n' "$*"
  [ "$PLAN" = 1 ] && return 0
  case "$kind" in
    MANUAL)
      # The directory step runs inside phase 5, before the stop. It is advisory:
      # compose is the check, so its exit status is never read.
      if [ "$n" = 5 ] && [ -n "$DIRECTORY_STEP" ]; then
        python3 "$REPO/ops/federation/standup/directory.py" --url "$FED_DIRECTORY_URL" --ours "$OURS" \
          ${FED_PACKAGE_URL:+--package-url "$FED_PACKAGE_URL"} ${FED_SIGNING_KEY:+--key "$FED_SIGNING_KEY"} \
          ${FED_CA_CERT:+--ca "$FED_CA_CERT"} \
          --theirs "$FED_COUNTERPARTY" ${FED_COUNTERPARTY_ID:+--their-id "$FED_COUNTERPARTY_ID"} \
          --keyring "$FED_KEYRING" --work "$FED_WORK" || true
      fi
      echo "    ^ OPERATOR STEP — do the above, then re-run with --from $((n+1))"
      exit 0 ;;
    *)
      # Capture the status BEFORE negating it. `if ! cmd; then rc=$?` reads the
      # status of the negation, which is 0 whenever the command failed — so the
      # recovery text confidently reported "exit 0" on every failure.
      eval "$*"; rc=$?
      if [ "$rc" -ne 0 ]; then
        # QUOTED delimiter: this block contains backticks, and an unquoted
        # heredoc runs them. The first draft executed `goat federate` while
        # printing its own error message.
        cat >&2 <<RECOVER

  !! Phase $n ("$label") FAILED — exit $rc.
     No later phase has run; the peering is left partway through phase $n.
     To get unstuck and reach this phase's end state:
       1. Read the error above. \`goat federate\` exit codes are specific:
            20 composition refused   21 lint refused
            22 diff refused          23 a value could not be derived
       2. Open docs/operations/sovereign-federation-runbook.md — the phase
          table names this phase's "worked when" (the end state you are
          aiming for), and Steps 7 and 8 are the acceptance criteria.
       3. Fix the cause, check the inputs and current state, then resume:
            ops/scripts/peering-standup.sh $PEERING --from $n
RECOVER
        exit "$rc"
      fi ;;
  esac
}

echo "peering-standup: peering=$PEERING  work=$FED_WORK  edge-group=$EDGE_GROUP  $( [ "$PLAN" = 1 ] && echo '(PLAN — nothing executes)')"

# A plan executes nothing, and that includes creating its working directory.
[ "$PLAN" = 1 ] || mkdir -p "$FED_WORK" "$PKI" 2>/dev/null || true

# Every ansible phase, one shape. The generated vars and the two exchanged files
# reach the plays through these, so a phase cannot read an older run's copy.
play() {
  printf "%s" "cd '$REPO/ops/ansible' && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws $GOAT_AWS_PROFILE -- ../scripts/ansible-preflight.sh -i inventory/peering-edges.ini --limit '${2:-$EDGE_GROUP}' -e fed_edge_group='$EDGE_GROUP' -e fed_work='$FED_WORK' -e fed_counterparty='$FED_COUNTERPARTY' $1"
}

# ---------------------------------------------------------------------------
# Phase 1 — PREFLIGHT. Fail-closed, before anything is measured, signed or deployed.
#     Every input this run needs, named where it is missing rather than
#     defaulted. A peering that cannot be described should not be half-built.
# ---------------------------------------------------------------------------
step 1 GATE "peering preflight (fail-closed)" \
  "python3 '$REPO/ops/federation/standup/preflight.py' '$PEERING' --work '$FED_WORK' --edge-group '$EDGE_GROUP'"

# ---------------------------------------------------------------------------
# Phase 2 — WHAT THE PACKAGE MEASURES HAS TO EXIST BEFORE IT IS MEASURED. Phase 3
#     reads `transport_offer.public_key` with `wg show`, so the federation interface
#     and its key come first, with no peer on them yet. The SPIRE bundle the parties
#     exchange in phase 5 is exported here too. Neither carries anything until the
#     agreement says so.
# ---------------------------------------------------------------------------
step 2 A "prepare what the package measures: the federation interface and the SPIRE bundle" \
  "$(play peering-prepare.yml)"

# ---------------------------------------------------------------------------
# Phase 3 — OUR HALF, MEASURED. Exit 23 means a value could not be derived and init
#     refused to invent it; the list of what is missing IS the output.
# ---------------------------------------------------------------------------
step 3 T "init — our half, read from the running system" \
  "${FED_ROUTER_DOCKER_HOST:+DOCKER_HOST='$FED_ROUTER_DOCKER_HOST' }$FED_VERB init --with-transport --with-data ${FED_SIGNING_KEY:+--signing-key '$FED_SIGNING_KEY' }--out '$OURS'"

step 4 T "lint our package against the spec" \
  "$FED_VERB lint '$OURS'"

# ---------------------------------------------------------------------------
# Phase 5 — THE EXCHANGE, through the directory (ops/federation/standup/directory.py),
#     then a stop for a person: the counterparty's verifying key is bound by reading
#     it aloud — never taken from the package, never from the directory, which
#     holds pointers, not permissions, and is never an input to compose's check.
# ---------------------------------------------------------------------------
# A value printed into the plan and passed to the helper: anything but plain URL
# characters disables the step rather than reach a command line.
DIRECTORY_STEP=""
if [ -n "${FED_DIRECTORY_URL:-}" ]; then
  case "$FED_DIRECTORY_URL${FED_PACKAGE_URL:-}${FED_COUNTERPARTY_ID:-}${FED_CA_CERT:-}" in
    *[!A-Za-z0-9._:/?=\&%~+@-]*) DIRECTORY_STEP="" ;;
    *) DIRECTORY_STEP=1 ;;
  esac
fi
if [ "${FED_NO_DIRECTORY:-}" = 1 ]; then
  EXCHANGE_LABEL="exchange packages by hand (FED_NO_DIRECTORY=1); read the key and CA fingerprint aloud"
  EXCHANGE_TEXT="No directory: FED_NO_DIRECTORY=1 is for a counterparty that can reach none.
     Send $OURS and our CA certificate to the counterparty, with $FED_WORK/ours.spiffe-bundle if phase 2 wrote one.
     Obtain theirs and place them at: $FED_COUNTERPARTY
       $FED_WORK/theirs-ca.pem
       $FED_WORK/theirs.spiffe-bundle"
elif [ -n "$DIRECTORY_STEP" ]; then
  EXCHANGE_LABEL="exchange through the directory; read the key and CA fingerprint aloud"
  EXCHANGE_TEXT="Directory step, run before this stop (it grants nothing):
       python3 '$REPO/ops/federation/standup/directory.py' --url '$FED_DIRECTORY_URL' --ours '$OURS'${FED_PACKAGE_URL:+ --package-url '$FED_PACKAGE_URL'} --their-id '${FED_COUNTERPARTY_ID:-<unset>}'
     It uploads our package (unless FED_PACKAGE_URL names our own host), CA and
     SPIFFE bundle (when phase 2 wrote one), publishes our pointer, fetches
     theirs to $FED_COUNTERPARTY, $FED_WORK/theirs-ca.pem and
     $FED_WORK/theirs.spiffe-bundle, checks digest, signatures and CA
     fingerprint, and prints both verifying keys and CA fingerprints in groups
     of four and a compatibility report. Their entry not there yet: run phase 5
     again after they run theirs."
elif [ -n "${FED_DIRECTORY_URL:-}" ]; then
  EXCHANGE_LABEL="exchange through the directory; read the key and CA fingerprint aloud"
  EXCHANGE_TEXT="FED_DIRECTORY_URL, FED_PACKAGE_URL, FED_COUNTERPARTY_ID or FED_CA_CERT holds
     characters outside a plain URL; phase 1 refuses this run."
else
  EXCHANGE_LABEL="exchange through the directory; read the key and CA fingerprint aloud"
  EXCHANGE_TEXT="Set FED_DIRECTORY_URL, FED_COUNTERPARTY_ID and FED_SIGNING_KEY; phase 1 refuses
     without them. The directory step then uploads our package and CA, fetches
     theirs, and prints both keys and CA fingerprints and a compatibility report.
     FED_NO_DIRECTORY=1 is for a counterparty that can reach no directory."
fi
step 5 MANUAL "$EXCHANGE_LABEL" \
  "$EXCHANGE_TEXT
     On one call, each side reads its verifying key and CA fingerprint aloud and
     the other reads them back: a key taken from the package it verifies proves
     nothing. Then record BOTH parties' keys, one per line, at:
       $FED_KEYRING
     as   <sovereign_id>=<44 base64 characters>
     Phase 6 passes each line to compose as --key; compose refuses (exit 20)
     for any party whose key is not bound. After compose, read the agreement
     digest to each other the same way."

# Each `sovereign=key` line in the keyring becomes one --key argument. A missing
# or empty keyring is not patched over here: compose's own refusal names the
# party whose key is absent, which is a better error than anything this script
# could invent.
KEYARGS=""
if [ -f "$FED_KEYRING" ]; then
  while IFS= read -r line; do
    case "$line" in ''|'#'*) continue ;; esac
    clean="$(printf '%s' "$line" | tr -d '[:space:]')"
    # These commands run through eval and this line holds a counterparty's
    # value: an unsafe one is left out, so compose refuses for the unbound party.
    case "$clean" in *[!A-Za-z0-9._:+/=-]*) continue ;; esac
    KEYARGS="$KEYARGS --key '$clean'"
  done < "$FED_KEYRING"
fi

step 6 T "compose — one agreement, one digest, no perspective" \
  "$FED_VERB compose '$OURS' '$FED_COUNTERPARTY'$KEYARGS --out '$AGREEMENT'"

step 7 T "verify the agreement from our side" \
  "$FED_VERB verify '$AGREEMENT' ${AS:+--as '$AS'}"

# ---------------------------------------------------------------------------
# Phase 8 — PEERING LEAVES, minted from the agreement's DERIVED names. Hand-minting
#     these cost three rounds and two silent failures: a CN or SAN
#     that does not match what the far side dials fails with no error anywhere.
# ---------------------------------------------------------------------------
step 8 T "mint the peering leaf certificates from the agreement" \
  "'$REPO/ops/federation/standup/mint-peering-leaves.sh' '$AGREEMENT' '$PKI' ${AS:+--as '$AS'}"

# ---------------------------------------------------------------------------
# Phase 9 — THE ONLY WRITER. Everything above reads; this renders. The release set we
#     forward and the prefix the far side admits come from one artifact, so they
#     cannot drift apart by being typed twice.
# ---------------------------------------------------------------------------
step 9 T "apply — render the access rules and configuration" \
  "$FED_VERB apply '$AGREEMENT' ${AS:+--as '$AS'} --edge-group '$EDGE_GROUP' --out '$APPLIED' --confirm"

# ---------------------------------------------------------------------------
# Phase 10 — WHAT ONLY THIS PARTY MAY CHANGE. `apply` renders the core's access
#     rules as a fragment for review: a tool that edited a running router's access
#     control from an agreement would be the central override the brokerless design
#     refuses. So the review and the splice are a person's, and the core admits the
#     edge before phase 12 checks the inward link.
# ---------------------------------------------------------------------------
step 10 MANUAL "review what apply rendered, and admit the edge on our core" \
  "Review $APPLIED against the agreement. The review is the control.
     Check $FED_WORK/theirs-ca.pem against the fingerprint \`apply\` printed for the
     counterparty, then place it where goat_zenoh_edge_partner_ca_path says.
     Splice $APPLIED/ops/federation/acl/<them>-fragment.json5 into our core's
     access control with roles/goat-zenoh/files/acl-assemble.py, and give the core
     $PKI/core.pem and its key. The core has to admit the edge before phase 12's
     inward-link check can pass, and only this party can change this party's core."

# THE TRANSPORT THE AGREEMENT CHOSE, NOT A FIXED ONE. `compose` intersects the
# two parties' transports and picks by TRANSPORT_PREFERENCE, so the agreement is
# what says how this link is carried. Each transport has its own play, and a
# transport with no play refuses rather than substituting one that has.
TRANSPORT=$(python3 - "$AGREEMENT" 2>/dev/null <<'PYEOF'
import json, sys
try:
    print(json.load(open(sys.argv[1])).get("transport", {}).get("kind", "") or "")
except Exception:
    print("")
PYEOF
)
# The value goes into a command run through eval: anything but a plain name
# reads as unknown, so phase 10 refuses and nothing in it runs.
case "$TRANSPORT" in *[!A-Za-z0-9._:+/=-]*) TRANSPORT="" ;; esac
case "$TRANSPORT" in
  wireguard)
    TRANSPORT_PLAY="peering-wg.yml" ;;
  ipsec-fips)
    TRANSPORT_PLAY="peering-edge.yml" ;;
  *)
    TRANSPORT_PLAY="" ;;
esac

if [ -n "$TRANSPORT_PLAY" ]; then
  step 11 A "open the $TRANSPORT transport" \
    "$(play "$TRANSPORT_PLAY")"
else
  step 11 GATE "open the transport — REFUSED, no play for '${TRANSPORT:-unknown}'" \
    "echo \"    The agreement selected transport '${TRANSPORT:-<unreadable>}' and this repository\";
     echo \"    has no play that stands one up: wireguard is peering-wg.yml, ipsec-fips is peering-edge.yml.\";
     echo \"\";
     echo \"    Do NOT substitute the other transport: a tunnel the two parties did not\";
     echo \"    agree is exactly the drift between what was agreed and what got deployed\";
     echo \"    that this flow exists to prevent.\";
     false"
fi

# ---------------------------------------------------------------------------
# Phase 12 — THE EDGE, THEN THE REPUBLISHER. The edge binds its partner-facing port
#      on the link address, which exists once the transport is open, and that is
#      why the transport comes first. The republisher runs on our core host and
#      both of its sessions dial our core: it reads `release/<set>/**` and writes
#      `from/<us>/<set>/**`, the only writer of our wire name, and the edge carries
#      that name out. Worked when the edge reached our core and the republisher
#      reports `local_subscribed 1` and `edge_linked 1` with one `release -> wire`
#      line per offered set.
# ---------------------------------------------------------------------------
step 12 A "deploy the sovereign edge router and the outbound republisher" \
  "$(play peering-zenoh-edge.yml) && $(play peering-republisher.yml "$CORE_GROUP")"

# ---------------------------------------------------------------------------
# Phase 13 — ADR 1157 STEP 2. Each side federates the other's SPIRE trust domain:
#     the first bundle arrived out of band in phase 5, and the bundle endpoint keeps
#     it fresh across the tunnel. A side with no SPIRE server declares so, and the
#     play says the step was not done.
# ---------------------------------------------------------------------------
step 13 A "federate the SPIRE trust domains (ADR 1157 step 2)" \
  "$(play peering-spire.yml)"

# ---------------------------------------------------------------------------
# Phase 14 — ACCEPTANCE, all four tiers. The positive rows and the refusal rows are
#      asserted TOGETHER: a refusal checked against a dead path passes vacuously
#      and proves nothing. The round trip is last because it is the only check
#      that fails when an earlier phase silently half-works.
# ---------------------------------------------------------------------------
step 14 GATE "acceptance — carries, refuses, and round-trips" \
  "'$REPO/ops/federation/standup/acceptance.sh' '$AGREEMENT' ${AS:+--as '$AS'}"

if [ "$PLAN" = 1 ]; then
  cat <<'PLANEND'

  PLAN ONLY — nothing above executed.
  Phases 5 and 10 are MANUAL and stop the run; resume with --from 6 and --from 11.
  Acceptance criteria for every phase: docs/operations/sovereign-federation-runbook.md
  The shape and why: docs/design/staged-delivery-pattern.md
PLANEND
fi
