#!/usr/bin/env bash
# deploy-component.sh — put the pin main declares onto one goatnet, and prove it.
#
# The leg fastlane.sh used to print and rc.sh needs twice (dev, then preprod):
# resolve the component's deploy playbook and the goatnet's member groups from
# fleet-pins, pick the transport that reaches that goatnet from this laptop
# without touching the NetBird profile (docs/design/env-doors.md), run the
# playbook through ansible-preflight.sh, then read the deploy ledger back from
# every host and refuse to call it done unless the ledger's newest row names
# the ref the rendered pin carries.
#
# usage:
#   deploy-component.sh <component> --goatnet <fleet-pins goatnet key> \
#                       [--group <member group>] [--dry-run] [--verify-only] [--preflight] [--offline]
#
# Before the first host is touched, ops/scripts/fresh-source.sh refuses a
# checkout behind the trunk in ops/ansible or ops/pins (contract C3, GB-064) or
# one it cannot fetch the trunk for (GB-065); --offline (an islanded site)
# judges against the cached trunk and the ledger row records it.
#
# Runs from a clone of the trunk or from an ASSEMBLED tree (goatcore at the
# commit in goatcore.ref, an environment overlay copied over it, `.goat-assembled`
# at the root). The assembled tree is judged against that pair of commits, not a
# trunk remote, and each ledger row names the pair (`source_tree`).
#
# Transport, decided per goatnet:
#   - the laptop's NetBird profile IS this goatnet and Management is Connected
#       -> -i mgmt-hosts.ini -i mgmt-hosts-mesh.ini   (mesh SSH)
#   - env-door.sh registers a door for it
#       -> door-open.sh <name>; -i mgmt-hosts.ini -i doors-groupvars.ini
#   - otherwise
#       -> -i mgmt-hosts.ini                          (direct)
# mgmt-hosts.ini always loads first: a service deploy needs the site group_vars
# and router_role, and doors.ini alone fails the ADR 1084 §D3 guard by design.
set -euo pipefail
# Hold the Mac awake for the whole run (#2122; ops/scripts/lib/keep-awake.sh).
_keep_awake="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/keep-awake.sh"
# shellcheck source=ops/scripts/lib/keep-awake.sh
if [ -f "$_keep_awake" ]; then . "$_keep_awake"; goat_keep_awake; fi
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
PINS="$REPO/ops/pins/fleet-pins.yaml"
die() { echo "deploy-component: $*" >&2; exit 1; }
note() { printf '    %s\n' "$*"; }
COMP=""; GOATNET=""; ONLY_GROUP=""; DRY=0; VERIFY_ONLY=0; OFFLINE=0; PREFLIGHT_ONLY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --goatnet) GOATNET="$2"; shift 2 ;;
    --group)   ONLY_GROUP="$2"; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    --verify-only) VERIFY_ONLY=1; shift ;;
    --preflight) PREFLIGHT_ONLY=1; shift ;;
    --offline) OFFLINE=1; shift ;;
    -h|--help) sed -n '2,31p' "$0"; exit 0 ;;
    -*) die "unknown flag $1" ;;
    *) [ -z "$COMP" ] && COMP="$1" || die "unexpected argument $1"; shift ;;
  esac
done
[ -n "$COMP" ] && [ -n "$GOATNET" ] || die "usage: deploy-component.sh <component> --goatnet <key> [--group g] [--dry-run] [--verify-only] [--preflight]"
# The component and goatnet resolution below reads fleet-pins.yaml with PyYAML.
# Check it up front with the install command, as rc.sh does, rather than die in
# a traceback with an empty reason.
# The Python with PyYAML: python3 when it has it, else Ansible's own interpreter,
# which always does (#2120) — the same choice goatbuilder makes, so neither avenue
# stops where the other runs.
PY="${GOATBUILDER_PYTHON:-python3}"
if ! "$PY" -c 'import yaml' 2>/dev/null; then
  _ap="$(head -1 "$(command -v ansible-playbook 2>/dev/null || echo /dev/null)" 2>/dev/null | sed 's/^#!//' | awk '{print $NF}' || true)"
  if [ -n "$_ap" ] && "$_ap" -c 'import yaml' 2>/dev/null; then PY="$_ap"; fi
fi
"$PY" -c 'import yaml' 2>/dev/null || die "python dep missing — this needs PyYAML. Install once:  pip install pyyaml  (or pipx/venv), then re-run."
# The AWS profile and the image organisation are the project's, not this
# script's: GOAT_AWS_PROFILE / GHCR_ORG for one command, then what the checkout
# declares in ops/goatbuilder/goatbuilder.yaml (deploy.aws_profile and
# repo.image_namespace, read by project-setting.sh), then `default`, the name
# the AWS tools fall back to themselves. The builder reads the same three in
# the same order (crate::settings), so both avenues run under one profile.
AWS_PROFILE_DEPLOY="${GOAT_AWS_PROFILE:-$("$HERE/project-setting.sh" --repo "$REPO" deploy aws_profile 2>/dev/null || true)}"
[ -n "$AWS_PROFILE_DEPLOY" ] || AWS_PROFILE_DEPLOY=default
IMAGE_ORG="${GHCR_ORG:-}"
if [ -z "$IMAGE_ORG" ]; then
  _ns="$("$HERE/project-setting.sh" --repo "$REPO" repo image_namespace 2>/dev/null || true)"
  case "$_ns" in ghcr.io/*) IMAGE_ORG="${_ns#ghcr.io/}" ;; esac
fi

# --- prep phase: the rollout readiness/reversibility readout (ADR 1211) --------
# The three declared facts, printed first, before anything is cut — the script
# avenue of `goatbuilder preflight`'s prep phase (ADR 1201; the CLI is the richer,
# supported path that also measures readiness and validates the rollback fact).
# Informational here: it never fails the deploy. `--preflight` prints it and stops.
echo "prep — rollout preflight ($COMP):"
"$PY" "$HERE/rollout-preflight.py" "$COMP" --repo "$REPO" 2>&1 | sed 's/^/    /' || true
if [ "$PREFLIGHT_ONLY" = 1 ]; then
  "$PY" "$HERE/rollout-preflight.py" "$COMP" --repo "$REPO" >/dev/null 2>&1; exit $?
fi

# --- resolve component + goatnet facts from fleet-pins -------------------------
J="$("$PY" - "$PINS" "$COMP" "$GOATNET" "$ONLY_GROUP" <<'PY'
import sys, yaml, json
pins = yaml.safe_load(open(sys.argv[1])); comp = (pins.get("components") or {}).get(sys.argv[2])
if not comp: sys.exit(f"unknown component {sys.argv[2]!r}")
g = (pins.get("goatnets") or {}).get(sys.argv[3])
if not g: sys.exit(f"unknown goatnet {sys.argv[3]!r}; known: {sorted(pins.get('goatnets') or {})}")
deviations = comp.get("deviations") or {}
absent = {k for k, v in deviations.items() if isinstance(v, dict) and v.get("absent")}
declared = list(g.get("members") or [])
requested = sys.argv[4]
if requested:
    if requested not in declared:
        sys.exit(f"--group {requested!r} is not a member of goatnet {sys.argv[3]!r}")
    effective = next((deviations[scope] for scope in (requested, sys.argv[3]) if scope in deviations), {})
    if isinstance(effective, dict) and effective.get("absent"):
        sys.exit(f"--group {requested!r} does not run component {sys.argv[2]!r} (absent: true)")
    members, skipped = [requested], []
else:
    members = [m for m in declared if m not in absent]
    skipped = [m for m in declared if m in absent]
print(json.dumps({"var": comp["var"], "ledger": comp.get("ledger_component") or sys.argv[2],
                  "play": comp.get("deploy_playbook") or "",
                  "write_role": comp.get("single_homed_role") or "",
                  "members": members, "skipped": skipped}))
PY
)" || die "$J"
VAR="$(printf '%s' "$J" | python3 -c 'import json,sys; print(json.load(sys.stdin)["var"])')"
LEDGER="$(printf '%s' "$J" | python3 -c 'import json,sys; print(json.load(sys.stdin)["ledger"])')"
DECLARED_PLAY="$(printf '%s' "$J" | python3 -c 'import json,sys; print(json.load(sys.stdin)["play"])')"
# A single-homed surface (ADR 1084 §D3) lives on ONE host per goatnet — the one
# whose router_role matches. The deploy plays enforce this via
# tasks/assert-admin-write-host.yml; verification did not, so a correctly
# configured two-node goatnet reported NOT converged forever and a permanent
# red became indistinguishable from a real one.
WRITE_ROLE="$(printf '%s' "$J" | python3 -c 'import json,sys; print(json.load(sys.stdin)["write_role"])')"
MEMBERS="$(printf '%s' "$J" | python3 -c 'import json,sys; print(" ".join(json.load(sys.stdin)["members"]))')"
SKIPPED="$(printf '%s' "$J" | python3 -c 'import json,sys; print(" ".join(json.load(sys.stdin)["skipped"]))')"
# A component whose playbook is named after neither its ledger nor itself
# declares it: zenoh-router is deployed by zenoh-deploy.yml, while its ledger is
# <site>-zenoh and its key is zenoh-router, so neither convention finds it.
if [ -n "$DECLARED_PLAY" ]; then
  PLAY="$REPO/ops/ansible/$DECLARED_PLAY"
  [ -f "$PLAY" ] || die "$COMP declares deploy_playbook: $DECLARED_PLAY, which is not in ops/ansible/"
else
  PLAY="$REPO/ops/ansible/${LEDGER}-deploy.yml"; [ -f "$PLAY" ] || PLAY="$REPO/ops/ansible/${COMP}-deploy.yml"
  [ -f "$PLAY" ] || die "no deploy playbook for $COMP (looked for ${LEDGER}-deploy.yml, ${COMP}-deploy.yml; declare deploy_playbook in fleet-pins.yaml if it is named otherwise)"
fi
# The target mode is decided by the play that actually declares `hosts:`. A
# fleet-standard <component>-deploy.yml may be an alias that only imports the
# real play (microscope-deploy.yml -> microscope.yml), so follow
# import_playbook first; deciding on the alias
# read it as a plain `-l` play, and the imported `hosts: "{{ target | mandatory }}"`
# refused the run before any task ("Mandatory variable 'target' not defined",
# F-587). A play whose hosts is a mandatory variable gets `-e <that var>=<group>`,
# whichever variable it names; a play with literal hosts gets `-l <group>`.
# Portable regex ([[:space:]], not \s): this runs on operators' macOS (BSD grep
# and sed) as well as on Linux.
HOSTS_PLAY="$PLAY"
for _hop in 1 2 3; do
  grep -qE '^[[:space:]]*hosts:' "$HOSTS_PLAY" && break
  _imp="$(grep -oE 'import_playbook:[[:space:]]*[^[:space:]]+' "$HOSTS_PLAY" | head -1 | awk '{print $2}')"
  [ -n "$_imp" ] || break
  HOSTS_PLAY="$(dirname "$HOSTS_PLAY")/$_imp"
  [ -f "$HOSTS_PLAY" ] || die "$(basename "$PLAY") imports $_imp, which is not in ops/ansible/"
done
grep -qE '^[[:space:]]*hosts:' "$HOSTS_PLAY" || die "$(basename "$PLAY") resolves to no play with a hosts: line (followed import_playbook to $(basename "$HOSTS_PLAY"))"
# `|| true`: a play with LITERAL hosts has no `{{` to match, grep returns 1, and
# under set -e the assignment would end the run silently with exit 1.
HOSTS_VAR="$(grep -m1 -E '^[[:space:]]*hosts:[[:space:]]*"?\{\{[[:space:]]*[a-z_]+' "$HOSTS_PLAY" | sed -E 's/.*\{\{[[:space:]]*([a-z_]+).*/\1/' || true)"
if [ -n "$HOSTS_VAR" ]; then TARGET_FLAG="-e ${HOSTS_VAR}="; else TARGET_FLAG="-l "; fi
NAME="${GOATNET//_/-}"
[ -n "$SKIPPED" ] && note "skipping (absent: true, topology not skew): $SKIPPED"

# --- fresh source (contract C3) ------------------------------------------------
# One check before the first host, pinned for every play of this run (each play
# goes through ansible-preflight.sh, which judges against the same commit).
if [ "$DRY" = 0 ] && [ "$VERIFY_ONLY" = 0 ]; then
  if [ "$OFFLINE" = 1 ]; then export GOAT_DEPLOY_OFFLINE=1; fi
  GOAT_FRESH_SOURCE_TRUNK="$("$HERE/fresh-source.sh" --repo "$REPO")" \
    || die "refusing before any host is touched: this checkout is not current with the source of record (above)"
  export GOAT_FRESH_SOURCE_TRUNK
fi

# --- secrets --------------------------------------------------------------------
# The play templates this component's secrets out of the deployer's environment
# (group_vars binds `<component>_…: lookup('env', NAME)`). A fresh clone has none,
# and previously the first anyone heard of it was an assert on the host,
# one variable per run. Derive the whole set now and refuse before any host is
# touched, naming every variable and the one command that resolves them.
SECRETS_REPORT="$(python3 "$HERE/deploy-secrets-needed.py" --repo "$REPO" --component "$COMP" --ledger "$LEDGER" --play "$HOSTS_PLAY" $MEMBERS 2>&1)" && SECRETS_RC=0 || SECRETS_RC=$?
if [ "$SECRETS_RC" = 1 ]; then
  MISSING="$(printf '%s\n' "$SECRETS_REPORT" | awk '$3=="MISSING"{print $2}' | tr '\n' ' ')"
  note "secrets: $COMP on $NAME templates $(printf '%s\n' "$SECRETS_REPORT" | grep -c .) variable(s) from your environment; NOT SET: $MISSING"
  # How to supply them depends on the team's declared secrets posture (ADR 1204).
  while IFS= read -r _hint; do note "         $_hint"; done < <(python3 "$REPO/ops/secrets/posture.py" hint $MISSING 2>/dev/null)
  [ "$DRY" = 1 ] || die "refusing before any host is touched: $COMP needs $MISSING in the environment (see above)"
elif [ "$SECRETS_RC" != 0 ]; then
  note "secrets: could not derive what $COMP needs ($SECRETS_REPORT) — continuing; the play will refuse on the host if a secret is empty"
fi

# --- transport ------------------------------------------------------------------
# `|| true`: with no netbird on PATH the pipeline fails and, under set -e, the
# assignment ended the run silently (a CI runner, or a laptop without the
# client) — the fourth silent exit goatbuilder's parity gate found in this file.
PROFILE="$(netbird status 2>/dev/null | awk -F': ' '/^Profile:/{print $2}' || true)"
MGMT="$(netbird status 2>/dev/null | awk -F': ' '/^Management:/{print $2}' || true)"
# The mesh is decided by REACHING it, not by the profile's name: the first
# mesh-inventory host of the first member group, tcp/22, three seconds. A
# laptop on the goatnet whose profile is called something else (or whose
# netbird prints no Profile line) deploys over the mesh like any other; the
# profile match stays as the fast path, so a deployer on the mesh with a
# differently named profile does not fall through to the base inventory.
MESH_HOST=""
for G in $MEMBERS; do
  MESH_HOST="$(awk -v g="[$G]" '$0==g{f=1;next} /^\[/{f=0} f && /ansible_host=/{sub(/.*ansible_host=/,""); sub(/[[:space:]].*/,""); print; exit}' "$REPO/ops/ansible/inventory/mgmt-hosts-mesh.ini" 2>/dev/null || true)"
  [ -n "$MESH_HOST" ] && break
done
INV="-i inventory/mgmt-hosts.ini"; TRANSPORT="direct (mgmt-hosts.ini)"; TRANSPORT_KIND="direct"
if [ "$PROFILE" = "$NAME" ] && [ "$MGMT" = "Connected" ]; then
  INV="-i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini"; TRANSPORT="mesh (this laptop is on the $NAME profile)"; TRANSPORT_KIND="mesh"
elif [ "${GOAT_MESH_PROBE:-1}" != 0 ] && [ -n "$MESH_HOST" ] && nc -z -w 3 "$MESH_HOST" 22 >/dev/null 2>&1; then
  INV="-i inventory/mgmt-hosts.ini -i inventory/mgmt-hosts-mesh.ini"; TRANSPORT="mesh (this laptop reaches $MESH_HOST:22; netbird profile reads '$PROFILE')"; TRANSPORT_KIND="mesh"
elif "$HERE/env-door.sh" "$NAME" --print >/dev/null 2>&1; then
  INV="-i inventory/mgmt-hosts.ini -i inventory/doors-groupvars.ini"; TRANSPORT="door ($HERE/door-open.sh $NAME)"; TRANSPORT_KIND="door"
  if [ "$DRY" = 0 ]; then "$HERE/door-open.sh" "$NAME" >/dev/null 2>&1 || die "door to $NAME did not open"; fi
fi
# direct and mesh reach this goatnet over plain SSH; no AWS account is needed to
# list its inventory or tail its deploy ledger (GOAT_AWS_MODE=none, ADR 1204).
# Forcing those two read-only calls through with-aws anyway made a read-only
# inventory/ledger check on an SSH-only goatnet fail or stall for an operator
# with no AWS profile configured, for an AWS dependency the call never had
# (#1574). Scoped to those two calls only: door/SSM transports, and the
# mutating deploy play below, keep going through with-aws unchanged.
READONLY_AWS_MODE=""
case "$TRANSPORT_KIND" in direct|mesh) READONLY_AWS_MODE="none" ;; esac
# The source of record this run renders from, said once and recorded in every
# ledger row it writes (rollback-ledger `source_tree`): the commit of a clone, or
# goatcore.ref + the overlay commit of an assembled tree (source_of_record.py).
GOAT_DEPLOY_SOURCE_TREE="$(python3 "$HERE/lib/source_of_record.py" describe --repo "$REPO" 2>/dev/null || true)"
export GOAT_DEPLOY_SOURCE_TREE
note "source tree $(python3 "$HERE/lib/source_of_record.py" kind --repo "$REPO" 2>/dev/null || echo unknown) $GOAT_DEPLOY_SOURCE_TREE"
note "goatnet $GOATNET ($NAME): groups [$MEMBERS] via $TRANSPORT"
note "playbook $(basename "$PLAY") ${TARGET_FLAG}<group>; ledger component $LEDGER"

# --- run + verify per group ------------------------------------------------------
LOGDIR="${GOAT_DEPLOY_LOG_DIR:-${TMPDIR:-/tmp}}"; TS="$(date -u +%Y%m%dT%H%M%SZ)"; RC=0; VERIFIED=0
for G in $MEMBERS; do
  PINFILE="$REPO/ops/ansible/group_vars/$G/pins.yml"
  # `|| true`: a component with no pin line in this group (declared absent in
  # the pins, or never rendered) is "nothing declared", not a silent exit 1 —
  # under set -e a failed grep here ended the run with no message at all
  # (found by goatbuilder's parity gate).
  WANT="$(grep -E "^${VAR}:" "$PINFILE" 2>/dev/null | sed -E 's/^[^:]+:[[:space:]]*"?//; s/"?[[:space:]]*$//' || true)"
  [ -n "$WANT" ] || { note "$G: no $VAR in $PINFILE — nothing declared, skipping"; continue; }
  WANT_DIGEST="${WANT##*@}"
  LOG="$LOGDIR/goat-deploy-$COMP-$G-$TS.log"
  if [ "$VERIFY_ONLY" = 0 ]; then
    # Show the ref with ghcr_org resolved (GHCR_ORG, else the organisation the
    # checkout declares), so the note does not read like an unrendered/broken
    # variable. Ansible still renders the real value at apply time from the
    # goatnet's group_vars.
    SHOWN="$WANT"
    [ -z "$IMAGE_ORG" ] || SHOWN="$(printf '%s' "$WANT" | sed "s#{{ ghcr_org }}#${IMAGE_ORG}#g")"
    note "$G: deploying $SHOWN"
    if [ "$DRY" = 1 ]; then
      note "DRY: (cd ops/ansible && ../scripts/with-aws $AWS_PROFILE_DEPLOY -- ../scripts/ansible-preflight.sh $INV $(basename "$PLAY") ${TARGET_FLAG}$G) > $LOG"
    else
      ( cd "$REPO/ops/ansible" && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws "$AWS_PROFILE_DEPLOY" -- \
          ../scripts/ansible-preflight.sh $INV "$(basename "$PLAY")" ${TARGET_FLAG}"$G" ) > "$LOG" 2>&1 \
        && DEPLOY_RC=0 || DEPLOY_RC=$?
      RECAP="$(grep -A8 'PLAY RECAP' "$LOG" | grep -E 'ok=' || true)"
      printf '%s\n' "$RECAP" | sed 's/^/      /'
      # A declared member group with no host in the loaded inventories (a site
      # declared in fleet-pins but not built yet, e.g. one whose
      # instance id is still TBD) runs no play. That is not a failed deploy; say
      # what it is and move on, rather than exiting 1 on a deploy that landed.
      if [ -z "$RECAP" ] && grep -q 'skipping: no hosts matched' "$LOG"; then
        note "$G: no host of this group in the loaded inventories (declared, not built?) — nothing deployed, skipping"; continue
      fi
      # A nonzero exit is a failed deploy even when a clean recap printed (#1545).
      if [ "$DEPLOY_RC" != 0 ] || [ -z "$RECAP" ] || printf '%s' "$RECAP" | grep -qE 'unreachable=[1-9]|failed=[1-9]'; then
        # fatal: lines from the play, and the wrapper's own refusal (a stale
        # checkout is refused by ansible-preflight before any play runs, and
        # that refusal used to stay in the log file, unread).
        note "$G: deploy FAILED — log $LOG"; grep -n -E 'fatal:|REFUSE' "$LOG" | head -3 | sed 's/^/      /'; RC=1; continue
      fi
    fi
  fi
  # verify: the newest ledger row on every host of the group names the digest the pin carries
  # served-payloads.py imports PyYAML: the interpreter chosen once above (#2120).
  served_py="$PY"
  # Served payloads (goat-get, goat-docs): the releases the surface serves are
  # versioned by role vars, not by its image; the ledger row records them and
  # served-payloads.py judges them against main (goatbuilder calls the same module).
  served_want="$("$served_py" "$HERE/served-payloads.py" declared --repo "$REPO" --ledger "$LEDGER" --group "$G" 2>&1)" && served_rc=0 || served_rc=$?
  served_note=""
  if [ "$served_rc" != 0 ]; then served_note=" and served payloads UNRESOLVED ($served_want)"
  elif [ -n "$served_want" ] && [ "$served_want" != "{}" ]; then served_note=" and served payloads $served_want"; fi
  if [ "$DRY" = 1 ]; then note "DRY: would read /var/lib/goat/$LEDGER/deploy-history.jsonl on $G and require ref digest $WANT_DIGEST$served_note"; continue; fi
  # Hosts in this group that are NOT the declared writer carry this surface by
  # design-absence, so a missing ledger there is the correct state, not drift.
  # The inventory read is also what says which hosts MUST answer: a failed read,
  # or a host that never returns a row, is not convergence (#1545).
  if ! INVENTORY="$( cd "$REPO/ops/ansible" && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES GOAT_AWS_MODE="$READONLY_AWS_MODE" ../scripts/with-aws "$AWS_PROFILE_DEPLOY" -- \
      ansible-inventory $INV --list 2> "$LOG.inventory" )"; then
    note "$G: inventory read FAILED — log $LOG.inventory"; RC=1; continue
  fi
  if ! HOSTSPLIT="$(printf '%s' "$INVENTORY" | WR="$WRITE_ROLE" GRP="$G" python3 -c '
import json, os, sys
inv = json.load(sys.stdin); want = os.environ["WR"].upper(); grp = os.environ["GRP"]
meta = (inv.get("_meta") or {}).get("hostvars") or {}
hosts, seen, todo = [], set(), [grp]
while todo:  # the group and its child groups, as an ansible pattern targets them
    g = todo.pop()
    if g in seen:
        continue
    seen.add(g)
    entry = inv.get(g) or {}
    hosts += [h for h in entry.get("hosts") or [] if h not in hosts]
    todo += entry.get("children") or []
skip = [h for h in hosts if want and str(meta.get(h, {}).get("router_role", "A")).upper() != want]
print(" ".join(h for h in hosts if h not in skip))
print(" ".join(skip))
' 2>/dev/null)"; then
    note "$G: inventory could not be read — log $LOG.inventory"; RC=1; continue
  fi
  EXPECTED_HOSTS="$(printf '%s\n' "$HOSTSPLIT" | sed -n 1p)"
  SKIP_HOSTS="$(printf '%s\n' "$HOSTSPLIT" | sed -n 2p)"
  [ -n "$SKIP_HOSTS" ] && note "$G: single-homed on router_role=$WRITE_ROLE — not expected on: $SKIP_HOSTS"
  if [ -z "$EXPECTED_HOSTS" ]; then
    note "$G: no host of this group is expected to carry $LEDGER in the loaded inventories — nothing to verify"; continue
  fi
  SEENFILE="$LOGDIR/.deploy-$COMP-$G-$TS.seen"; : > "$SEENFILE"
  ROWS="$( cd "$REPO/ops/ansible" && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES GOAT_AWS_MODE="$READONLY_AWS_MODE" ../scripts/with-aws "$AWS_PROFILE_DEPLOY" -- \
      ansible $INV "$G" -m shell -a "tail -n 1 /var/lib/goat/$LEDGER/deploy-history.jsonl 2>/dev/null || echo NO_LEDGER" -o 2> "$LOG.verify" )" \
    && LEDGER_RC=0 || LEDGER_RC=$?
  printf '%s\n' "$ROWS" | while IFS= read -r line; do
    [ -n "$line" ] || continue
    host="${line%% *}"
    case " $SKIP_HOSTS " in *" $host "*)
      note "$host: skipped — single-homed on router_role=$WRITE_ROLE (ADR 1084 §D3)"; continue ;;
    esac
    echo "$host" >> "$SEENFILE"
    # The row must name the digest AND say the deploy finished. rollback-ledger
    # writes "pending" up front and a handler promotes it to "deploy" only for
    # hosts that reached the end of the play — so a refused deploy leaves a row
    # carrying the right digest and the wrong action. Matching the digest alone
    # is what let three false rows read as converged.
    if printf '%s' "$line" | grep -q "$WANT_DIGEST" && printf '%s' "$line" | grep -qE '"action"[[:space:]]*:[[:space:]]*"deploy"'; then
      served_line="$(printf '%s' "$line" | "$served_py" "$HERE/served-payloads.py" check --repo "$REPO" --ledger "$LEDGER" --group "$G" 2>&1)" && served_rc=0 || served_rc=$?
      if [ "$served_rc" = 0 ]; then
        note "$host: converged on $WANT_DIGEST"
        # if, not `&&`: a component that serves nothing prints nothing, and an
        # `&&` list ending false as the loop body's last command ended the
        # script silently under set -e + pipefail (the hermetic tier caught it).
        if [ -n "$served_line" ]; then note "$host: $served_line"; fi
      else
        note "$host: NOT converged — image on $WANT_DIGEST, but $served_line"
        echo FAIL > "$LOGDIR/.deploy-$COMP-$G-$TS.fail"
      fi
    elif printf '%s' "$line" | grep -q "$WANT_DIGEST"; then
      # A rollback row names the declared digest as its prior_ref: the host was
      # moved AWAY from it, not left mid-play. goatbuilder prints the same words.
      note "$host: $(ndr_line="$line" ndr_digest="$WANT_DIGEST" ndr_tail="$(printf '%s' "$line" | sed -E 's/^[^|]*\| (CHANGED|SUCCESS)[^>]*>>\s*//' | cut -c1-140)" python3 -c '
import json, os
line, d, tail = os.environ["ndr_line"], os.environ["ndr_digest"], os.environ["ndr_tail"]
def short(r):
    n, sep, h = r.partition("@sha256:")
    return n + sep + h[:12] if sep else r
try:
    row = json.loads(line[line.index("{"):line.rindex("}") + 1])
except ValueError:
    row = {}
f = lambda k: str(row.get(k) or "")
if f("action") == "rollback":
    if d in f("ref"):
        print("NOT converged — rolled back TO the declared ref at %s (from %s); a rollback row is not a deploy: deploy it to record convergence" % (f("ts"), short(f("prior_ref"))))
    else:
        print("NOT converged — rolled back from %s to %s at %s; the host is not at the declared ref" % (short(f("prior_ref")), short(f("ref")), f("ts")))
else:
    print("NOT converged — ledger names the ref but the row is not action=deploy (the play did not finish on this host): " + tail)
')"
      echo FAIL > "$LOGDIR/.deploy-$COMP-$G-$TS.fail"
    else
      note "$host: NOT converged — ledger tail: $(printf '%s' "$line" | sed -E 's/^[^|]*\| (CHANGED|SUCCESS)[^>]*>>\s*//' | cut -c1-140)"
      echo FAIL > "$LOGDIR/.deploy-$COMP-$G-$TS.fail"
    fi
  done
  GROUP_OK=1
  [ -f "$LOGDIR/.deploy-$COMP-$G-$TS.fail" ] && { rm -f "$LOGDIR/.deploy-$COMP-$G-$TS.fail"; GROUP_OK=0; }
  if [ "$LEDGER_RC" != 0 ]; then
    note "$G: ledger read FAILED (exit $LEDGER_RC) — log $LOG.verify"; GROUP_OK=0
  fi
  # Every expected host must answer; silence is not proof. A host that answered with
  # the wrong row was already reported above.
  for h in $EXPECTED_HOSTS; do
    grep -qxF "$h" "$SEENFILE" || { note "$h: NOT converged — no ledger row came back from this host"; GROUP_OK=0; }
  done
  rm -f "$SEENFILE"
  if [ "$GROUP_OK" = 1 ]; then VERIFIED=$((VERIFIED + 1)); else RC=1; fi
done
if [ "$DRY" = 0 ] && [ "$RC" = 0 ] && [ "$VERIFIED" = 0 ]; then
  note "$COMP on $GOATNET: nothing was verified — no group had a host expected to carry $LEDGER"; RC=1
fi
# A dry run reads no ledger, so it must not say every ledger agrees.
if [ "$DRY" = 1 ]; then note "$COMP on $GOATNET: DRY: nothing was read; a real run reads every host's ledger"
elif [ "$RC" = 0 ]; then note "$COMP on $GOATNET: every host's ledger names the declared ref"
else note "$COMP on $GOATNET: NOT converged everywhere (see above)"; fi
exit $RC
