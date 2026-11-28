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
#                       [--group <member group> [--node <machine>]] [--dry-run] [--verify-only] [--preflight] [--offline]
#                       [--skip-rung --reason "<why>" --ack "<operator>"]
#
# A release component (fleet-pins artifact_type: release, gate J23) ships as a
# file: it is fetched and verified here before any host (GB-381, by goatbuilder's
# `release-file` verb, the code goatbuilder deploy runs; GOATBUILDER_BIN names the
# binary, else goatbuilder on PATH), the release-file role installs it, and verify
# holds each host's file to the pin. rollback.sh calls this with
# --release-rollback, --release-list or --release-status for one.
#
# Release ladder (ADR 1120, GB-373): on a rung above the lowest, the digest being
# deployed must already be what the rung below pins, and that rung's hosts must
# have converged on it (read live with this script's --verify-only on that rung).
# ops/release/ladder_gate.py decides; an open hotfix record for the ref, or
# --skip-rung with a reason and an operator ack (written into the ledger row as
# ladder_skip), are the only ways past it. Checked before any host is touched.
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
SKIP_RUNG=0; SKIP_REASON=""; SKIP_ACK=""; RELEASE_MODE=""; NODE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --goatnet) GOATNET="$2"; shift 2 ;;
    --group)   ONLY_GROUP="$2"; shift 2 ;;
    --node)    NODE="$2"; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    --verify-only) VERIFY_ONLY=1; shift ;;
    --preflight) PREFLIGHT_ONLY=1; shift ;;
    --offline) OFFLINE=1; shift ;;
    --release-rollback|--release-list|--release-status) RELEASE_MODE="${1#--release-}"; shift ;;
    --skip-rung) SKIP_RUNG=1; shift ;;
    --reason)  SKIP_REASON="$2"; shift 2 ;;
    --ack)     SKIP_ACK="$2"; shift 2 ;;
    -h|--help) sed -n '2,40p' "$0"; exit 0 ;;
    -*) die "unknown flag $1" ;;
    *) [ -z "$COMP" ] && COMP="$1" || die "unexpected argument $1"; shift ;;
  esac
done
[ -n "$COMP" ] && [ -n "$GOATNET" ] || die "usage: deploy-component.sh <component> --goatnet <key> [--group g] [--dry-run] [--verify-only] [--preflight]"
if [ -n "$NODE" ]; then
  # It is an Ansible pattern below: one host's name, and nothing wider.
  printf '%s' "$NODE" | grep -qE '^[A-Za-z0-9][A-Za-z0-9._-]*$' && [ -n "$ONLY_GROUP" ] && [ "$NODE" != all ] && [ "$NODE" != ungrouped ] \
    || die "--node \"$NODE\" is one machine's inventory name, with the --group it is in"
  # And a host of that group in the base inventory, before any door opens or play
  # runs: a group's name would play on all its hosts. Child groups' hosts count, a
  # name that is a group is refused. goatbuilder reads it the same way
  # (push::node_refusal).
  _node_why="$(python3 - "$REPO/ops/ansible/inventory/mgmt-hosts.ini" "$ONLY_GROUP" "$NODE" <<'PY'
import sys
path, group, node = sys.argv[1:4]
base = "ops/ansible/inventory/mgmt-hosts.ini"
try:
    text = open(path, encoding="utf-8").read()
except OSError as e:
    print(f"--node {node} cannot be checked: {base} cannot be read ({e.strerror})")
    sys.exit()
hosts, children, section = {}, {}, None
for line in (l.strip() for l in text.splitlines()):
    if not line or line[0] in "#;":
        continue
    if line.startswith("[") and line.endswith("]"):
        g, _, kind = line[1:-1].partition(":")
        hosts.setdefault(g, [])
        section = (g, "children" if kind == "children" else "hosts" if not kind else "vars")
        continue
    word = line.split()[0]
    if section and section[1] == "hosts":
        hosts.setdefault(section[0], []).append(word)
    elif section and section[1] == "children":
        children.setdefault(section[0], []).append(word)
if node in hosts or node in children:
    print(f"--node {node} is a group in {base}, not one machine: name a host of {group}")
    sys.exit()
seen, todo, members = set(), [group], set()
while todo:
    g = todo.pop()
    if g not in seen:
        seen.add(g)
        members.update(hosts.get(g, []))
        todo.extend(children.get(g, []))
if node not in members:
    print(f"--node {node} is not a host of {group} in {base}: name one machine of the group")
PY
)"
  [ -z "$_node_why" ] || die "$_node_why"
fi
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
if [ -z "$RELEASE_MODE" ]; then
  echo "prep — rollout preflight ($COMP):"
  "$PY" "$HERE/rollout-preflight.py" "$COMP" --repo "$REPO" 2>&1 | sed 's/^/    /' || true
fi
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
# An opt-in component runs only on a goatnet whose row lists it under opt_in:
# (resolve.py opted_out; push.rs declared).
if comp.get("opt_in") is True and sys.argv[2] not in (g.get("opt_in") or []):
    sys.exit(f"goatnet {sys.argv[3]!r} does not run component {sys.argv[2]!r}: it is opt-in, and the goatnet does not list it under opt_in:")
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
                  "play": comp.get("deploy_playbook") or "", "source": "source" in comp, "carried": bool((comp.get("release") or {}).get("play")),
                  # push::placement: a router_role letter, or host:<var> for a
                  # component on the one host a group variable names.
                  "write_role": ("host:" + comp["one_host_var"]) if comp.get("one_host_var") else (comp.get("single_homed_role") or ""),
                  "release": (comp.get("artifact_type") or "image") == "release",
                  "members": members, "skipped": skipped}))
PY
)" || die "$J"
VAR="$(printf '%s' "$J" | python3 -c 'import json,sys; print(json.load(sys.stdin)["var"])')"
# The value of $VAR in a member group's pins.yml as YAML reads it, which is what
# Ansible installs: quotes removed and a trailing `# comment` dropped. A value
# that is not a YAML string prints as written, and GB-382 refuses it. goatbuilder
# reads the line the same way (push::pin_value).
pin_ref() {   # <group>
  [ -f "$REPO/ops/ansible/group_vars/$1/pins.yml" ] || return 0
  "$PY" - "$REPO/ops/ansible/group_vars/$1/pins.yml" "$VAR" <<'PY'
import sys, yaml
var = sys.argv[2]
for line in open(sys.argv[1], encoding="utf-8"):
    if line.startswith(var + ":"):
        raw = line[len(var) + 1:].strip()
        try:
            value = yaml.safe_load(raw)
        except yaml.YAMLError:
            value = None
        print(value if isinstance(value, str) else raw)
        break
PY
}
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
RELEASE="$(printf '%s' "$J" | python3 -c 'import json,sys; print(1 if json.load(sys.stdin)["release"] else 0)')"
if [ -n "$RELEASE_MODE" ] && [ "$RELEASE" = 0 ]; then die "$COMP is not a release component; use rollback.sh"; fi
# A rollback or a read of a release component deploys nothing: no ladder, no secrets.
if [ -n "$RELEASE_MODE" ]; then VERIFY_ONLY=1; fi
# A component whose playbook is named after neither its ledger nor itself
# declares it: zenoh-router is deployed by zenoh-deploy.yml, while its ledger is
# <site>-zenoh and its key is zenoh-router, so neither convention finds it.
SOURCE="$(printf '%s' "$J" | python3 -c 'import json,sys; print(1 if json.load(sys.stdin)["source"] else 0)')"
PLAY_WORD=""
# Ansible reads group_vars and host_vars beside the play or an inventory file: a play
# kept elsewhere is run with one that names no host and is beside this checkout's own.
PLAY_INV=""
CARRIED="$(printf '%s' "$J" | python3 -c 'import json,sys; print(1 if json.load(sys.stdin)["carried"] else 0)')"
if [ "$SOURCE" = 1 ]; then
  # Its play is in another repository, at a pinned commit (gate J26): goatbuilder's
  # source-play verb fetches it, the code goatbuilder deploy runs (GB-385).
  GOATBUILDER_BIN="${GOATBUILDER_BIN:-goatbuilder}"
  # A tag taken is recorded only by a run that goes on to run the play.
  _rec=""; if [ "$DRY" != 1 ] && [ "$VERIFY_ONLY" != 1 ] && [ "$PREFLIGHT_ONLY" != 1 ]; then _rec="--record"; fi
  # shellcheck disable=SC2086 # _rec is one word or none
  _src="$("$GOATBUILDER_BIN" -C "$REPO" source-play "$COMP" $_rec 2>/dev/null || true)"
  case "$_src" in "{"*) ;; *) die "GB-385 refused before any host: $COMP's play is in its source repository, and $GOATBUILDER_BIN is not here or is too old to fetch it; install goatbuilder, or set GOATBUILDER_BIN (check: goatbuilder)" ;; esac
  _why="$(printf '%s' "$_src" | "$PY" -c 'import json,sys; print(json.load(sys.stdin).get("refused") or "")')"
  [ -z "$_why" ] || die "$_why"
  PLAY="$(printf '%s' "$_src" | "$PY" -c 'import json,sys; print(json.load(sys.stdin)["play"])')"
  ANSIBLE_ROLES_PATH="$(printf '%s' "$_src" | "$PY" -c 'import json,sys; print(json.load(sys.stdin)["roles_path"])')"; export ANSIBLE_ROLES_PATH
  PLAY_WORD="$PLAY"; PLAY_INV=" -i vars-here.ini"
elif [ "$CARRIED" = 1 ]; then
  # Its release's archive carries its play (release.play): release-file fetch unpacks it
  # and names it in the variables it writes.
  PLAY="$REPO/ops/ansible/release-archive-play.yml"
  [ -f "$PLAY" ] || die "$COMP's play is in its release, and ops/ansible/release-archive-play.yml is not here"
elif [ -n "$DECLARED_PLAY" ]; then
  PLAY="$REPO/ops/ansible/$DECLARED_PLAY"
  [ -f "$PLAY" ] || die "$COMP declares deploy_playbook: $DECLARED_PLAY, which is not in ops/ansible/"
else
  PLAY="$REPO/ops/ansible/${LEDGER}-deploy.yml"; [ -f "$PLAY" ] || PLAY="$REPO/ops/ansible/${COMP}-deploy.yml"
  # A release component with no play of its own runs the one that installs a release file.
  if [ "$RELEASE" = 1 ] && [ ! -f "$PLAY" ]; then PLAY="$REPO/ops/ansible/release-file-deploy.yml"; fi
  [ -f "$PLAY" ] || die "no deploy playbook for $COMP (looked for ${LEDGER}-deploy.yml, ${COMP}-deploy.yml; declare deploy_playbook in fleet-pins.yaml if it is named otherwise)"
fi
[ -n "$PLAY_WORD" ] || PLAY_WORD="$(basename "$PLAY")"
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

# What a dry run found that a real run refuses before any host. A dry run runs
# every check that touches no host, notes each refusal, goes on to print the
# whole plan, and exits 1 at the end; goatbuilder deploy --dry-run does the same.
DRY_REFUSED=""
dry_refuse() { DRY_REFUSED="${DRY_REFUSED:+$DRY_REFUSED }$1"; }

# --- release ladder (ADR 1120, GB-373) ----------------------------------------------
# Before any transport or host: the digest each member group is about to receive
# must be what the rung below pins (ops/release/ladder_gate.py), and that rung must
# have converged on it, read live with --verify-only. A verify-only run deploys
# nothing and is not gated.
if [ "$SKIP_RUNG" = 0 ] && { [ -n "$SKIP_REASON" ] || [ -n "$SKIP_ACK" ]; }; then
  die "--reason/--ack are for --skip-rung; pass --skip-rung with both, or neither"
fi
if [ "$SKIP_RUNG" = 1 ]; then GATE_SKIP=(--skip-reason "${SKIP_REASON:- }" --skip-ack "${SKIP_ACK:- }"); else GATE_SKIP=(); fi
LADDER_GATE="$REPO/ops/release/ladder_gate.py"
LADDER_LOWER=""
if [ "$VERIFY_ONLY" = 0 ]; then
  ladder_refuse() {
    [ "$DRY" = 1 ] || die "$1"
    note "ladder: REFUSE — a real run stops here, before any host: $1"; dry_refuse ladder
  }
  _ladder_groups="$MEMBERS"
  if [ ! -f "$LADDER_GATE" ]; then
    ladder_refuse "$LADDER_GATE is missing: the release-ladder gate cannot run, refusing (fail-closed)"; _ladder_groups=""
  fi
  for G in $_ladder_groups; do
    _want="$(pin_ref "$G")"
    [ -n "$_want" ] || continue
    _gate="$("$PY" "$LADDER_GATE" component --repo "$REPO" --component "$COMP" --goatnet "$GOATNET" --ref "$_want" ${GATE_SKIP[@]+"${GATE_SKIP[@]}"})" && _grc=0 || _grc=$?
    _msg="$(printf '%s' "$_gate" | "$PY" -c 'import json,sys; print(json.load(sys.stdin)["message"])' 2>/dev/null || printf '%s' "$_gate")"
    case "$_grc" in
      0) note "$G: $_msg"
         if printf '%s' "$_gate" | grep -q '"exception": "skip-rung"'; then
           GOAT_DEPLOY_LADDER_SKIP="$("$PY" -c 'import json,sys; print(json.dumps({"reason": sys.argv[1], "ack": sys.argv[2]}))' "$SKIP_REASON" "$SKIP_ACK")"
           export GOAT_DEPLOY_LADDER_SKIP
         fi ;;
      3) note "$G: $_msg"
         _lg="$(printf '%s' "$_gate" | "$PY" -c 'import json,sys; print(json.load(sys.stdin)["lower_goatnet"])')"
         case " $LADDER_LOWER " in *" $_lg "*) ;; *) LADDER_LOWER="$LADDER_LOWER $_lg" ;; esac ;;
      *) ladder_refuse "$G: $_msg"; break ;;
    esac
  done
  for _lg in $LADDER_LOWER; do
    if [ "$DRY" = 1 ]; then note "DRY: would confirm $COMP converged on $_lg (deploy-component.sh $COMP --goatnet $_lg --verify-only)"; continue; fi
    [ "$OFFLINE" = 0 ] || die "GB-373 ladder: an --offline run cannot read $_lg's ledgers to confirm $COMP converged there; pass --skip-rung --reason \"<why>\" --ack \"<operator>\""
    note "ladder: confirming $COMP converged on $_lg before touching $GOATNET"
    if ! GOAT_DEPLOY_LADDER_NESTED=1 "$0" "$COMP" --goatnet "$_lg" --verify-only; then
      die "GB-373 ladder: $COMP has not converged on $_lg (the rung below $GOATNET): its hosts' ledgers do not name the pinned ref. Run ops/scripts/deploy-component.sh $COMP --goatnet $_lg first, or pass --skip-rung --reason \"<why>\" --ack \"<operator>\""
    fi
  done
  export GOAT_LADDER_CHECKED=1
fi

# --- fresh source (contract C3) ------------------------------------------------
# One check before the first host, pinned for every play of this run (each play
# goes through ansible-preflight.sh, which judges against the same commit). A dry
# run runs it too: it reads the checkout and the trunk, never a host.
# A release rollback (VERIFY_ONLY=1, RELEASE_MODE=rollback) runs it too, before
# its door and play; it is never a dry run here.
if [ "$VERIFY_ONLY" = 0 ] || { [ "$RELEASE_MODE" = rollback ] && [ "$DRY" = 0 ]; }; then
  if [ "$OFFLINE" = 1 ]; then export GOAT_DEPLOY_OFFLINE=1; fi
  if [ "$DRY" = 1 ]; then
    if ! "$HERE/fresh-source.sh" --repo "$REPO" >/dev/null; then
      note "fresh-source: REFUSE — a real run stops here, before any host: this checkout is not current with the source of record (above)"
      dry_refuse fresh-source
    fi
  else
    GOAT_FRESH_SOURCE_TRUNK="$("$HERE/fresh-source.sh" --repo "$REPO")" \
      || die "refusing before any host is touched: this checkout is not current with the source of record (above)"
    export GOAT_FRESH_SOURCE_TRUNK
  fi
fi

# --- a digest-pinned ref (contract C1, GB-382) ------------------------------------
# Deploy installs only a ref pinned by its digest, whatever fresh-source said of
# the tree: a pins.yml value with no @sha256:<64 hex> part is refused before any
# host. goatbuilder reads the same value and the same part of it.
if [ "$VERIFY_ONLY" = 0 ]; then
  _digest_refused=0
  for G in $MEMBERS; do
    _want="$(pin_ref "$G")"
    [ -n "$_want" ] || continue
    # Digest-pinned: the text after the last @ is sha256: and exactly 64
    # lowercase hex characters, and the value holds no # (a comment kept
    # because the value was not a YAML string pins nothing). push::digest_pinned.
    case "$_want" in *'#'*) _last="" ;; *@*) _last="${_want##*@}" ;; *) _last="" ;; esac
    _hex="${_last#sha256:}"
    if [ "$_hex" != "$_last" ] && [ "${#_hex}" = 64 ]; then
      case "$_hex" in *[!0123456789abcdef]*) ;; *) continue ;; esac
    fi
    _line="GB-382 REFUSE — $VAR in group_vars/$G/pins.yml is $_want, with no @sha256: digest; deploy installs only a digest-pinned ref (contract C1)"
    [ "$DRY" = 1 ] || die "$G: $_line"
    note "$G: $_line"
    _digest_refused=1
  done
  [ "$_digest_refused" = 0 ] || dry_refuse digest
fi

# --- the goatnet's signed intent (GB-383) -----------------------------------------
# As generate-pins.py --check verifies it, for this goatnet: offline, from the
# checkout, before any host. What is deployed is then what the goatnet's CA
# signed. goatbuilder deploy makes the same checks in its own code.
if [ "$VERIFY_ONLY" = 0 ]; then
  if _intent="$("$PY" "$REPO/ops/pins/generate-pins.py" --check --goatnet "$GOATNET" 2>&1)"; then
    note "signed intent verified: ${_intent#ok: }"
  else
    _why="$(printf '%s\n' "$_intent" | sed -E 's/^(FAIL|ERROR): //' | awk 'NF{printf "%s%s", s, $0; s="; "}')"
    _line="GB-383 REFUSE — the signed intent of goatnet $GOATNET does not verify; nothing is deployed that its CA did not sign: $_why"
    [ "$DRY" = 1 ] || die "$_line"
    note "$_line"
    dry_refuse intent
  fi
fi

# --- a release component's files (GB-381) --------------------------------------------
# Fetched and verified on this machine before any host: goatbuilder's release-file
# verb, the code goatbuilder deploy runs, so both avenues refuse in the same words.
# A dry run does the same and says what a real run would refuse; a verify-only run
# compares each host's file with them, and its dry run reads nothing.
GOATBUILDER_BIN="${GOATBUILDER_BIN:-goatbuilder}"
# release_fetch <pin>: one JSON object, {line, vars} or {refused, not_compared}.
release_fetch() {
  local out why
  if ! command -v "$GOATBUILDER_BIN" >/dev/null 2>&1; then
    why="$COMP's release $1: $GOATBUILDER_BIN is not here to fetch and verify it; install goatbuilder, or set GOATBUILDER_BIN (check: goatbuilder)"
  else
    out="$("$GOATBUILDER_BIN" -C "$REPO" release-file fetch "$COMP" --pin "$1" 2>/dev/null || true)"
    case "$out" in "{"*) printf '%s' "$out"; return 0 ;; esac
    why="$COMP's release $1: $GOATBUILDER_BIN answered nothing to release-file; a newer goatbuilder is needed (check: goatbuilder)"
  fi
  "$PY" -c 'import json,sys; print(json.dumps({"refused": "GB-381 refused before any host: " + sys.argv[1], "not_compared": "GB-381 " + sys.argv[1]}))' "$why"
}
release_field() { printf '%s' "$1" | "$PY" -c 'import json,sys; print(json.load(sys.stdin).get(sys.argv[1]) or "")' "$2"; }
# RELEASE_VARS holds "<group> <vars file>" lines, RELEASE_WHY "<group> <reason>" lines.
RELEASE_VARS=""; RELEASE_WHY=""
release_vars() { printf '%s' "$RELEASE_VARS" | sed -n "s/^$1 //p" | head -1; }
if [ "$RELEASE" = 1 ] && [ -z "$RELEASE_MODE" ] && ! { [ "$DRY" = 1 ] && [ "$VERIFY_ONLY" = 1 ]; }; then
  _release_refused=0
  for G in $MEMBERS; do
    _want="$(pin_ref "$G")"
    [ -n "$_want" ] || continue
    _got="$(release_fetch "$_want")"
    if [ -n "$(release_field "$_got" vars)" ]; then
      note "$G: $(release_field "$_got" line)"
      RELEASE_VARS="$RELEASE_VARS$G $(release_field "$_got" vars)"$'\n'
    elif [ "$VERIFY_ONLY" = 1 ]; then
      RELEASE_WHY="$RELEASE_WHY$G $(release_field "$_got" not_compared)"$'\n'
    else
      [ "$DRY" = 1 ] || die "$G: $(release_field "$_got" refused)"
      note "$G: $(release_field "$_got" refused)"; _release_refused=1
    fi
  done
  [ "$_release_refused" = 0 ] || dry_refuse release
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
  note "secrets: REFUSE — a real run stops here, before any host: $COMP needs $MISSING in the environment"
  dry_refuse secrets
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
# open_door — open (or re-open) this goatnet's door; a door idles out after
# about 20 minutes, so a long run re-opens it before each group and before
# reading the ledgers. Two attempts, a short pause between them.
open_door() {
  local n
  for n in 1 2; do
    if "$HERE/door-open.sh" "$NAME" >/dev/null 2>&1; then return 0; fi
    [ "$n" = 2 ] || sleep "${GOAT_DOOR_RETRY_PAUSE:-5}"
  done
  return 1
}
# transport_failed <log> — the play lost its connection (a host UNREACHABLE, a
# door or SSM session that dropped), as opposed to a task that failed.
transport_failed() {
  grep -q -E 'UNREACHABLE!|unreachable=[1-9]|Connection (refused|reset|closed)|Session (terminated|closed)|TargetNotConnected' "$1"
}
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
  if [ "$DRY" = 0 ]; then open_door || die "door to $NAME did not open (tried twice)"; fi
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

# --- a release component: rollback.sh's three modes ------------------------------------
ansible_read() { # <group> <shell words> [ansible flag ...]
  local g="$1" words="$2"; shift 2
  ( cd "$REPO/ops/ansible" && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES GOAT_AWS_MODE="$READONLY_AWS_MODE" ../scripts/with-aws "$AWS_PROFILE_DEPLOY" -- \
      ansible $INV "$g" -m shell -a "$words" "$@" )
}
RELEASE_CURRENT="/opt/goat/releases/$COMP/current"
if [ "$RELEASE_MODE" = list ] || [ "$RELEASE_MODE" = status ]; then
  RC=0
  if [ "$RELEASE_MODE" = list ]; then
    note "deploy history for $COMP (newest first):"; _words="tac /var/lib/goat/$LEDGER/deploy-history.jsonl 2>/dev/null || true"
  else
    note "$COMP — artifact_type=release reversible=true (one step: the file before the one in use)"
    _words="cat \$(dirname \$(readlink -e $RELEASE_CURRENT) 2>/dev/null)/.ref 2>/dev/null || echo none"
  fi
  for G in $MEMBERS; do
    ansible_read "$G" "$_words" 2>/dev/null | rr_mode="$RELEASE_MODE" rr_net="$NAME" rr_comp="$COMP" "$PY" -c '
import json, os, re, sys
mode, name, comp, bad = os.environ["rr_mode"], os.environ["rr_net"], os.environ["rr_comp"], 0
for l in sys.stdin.read().splitlines():
    m = re.match(r"(\S+) \| ([A-Z!]+)", l)
    if m:
        if m.group(2) in ("CHANGED", "SUCCESS"): print("      (on-host @ %s, goatnet %s)" % (m.group(1), name))
        else: print("      %s: <unreachable>" % m.group(1)); bad = 1
    elif mode == "status":
        if l.strip(): print("      %s: %s" % (comp, l.strip()))
    else:
        try: r = json.loads(l)
        except ValueError: continue
        if not isinstance(r, dict): continue
        print("      %s  [%s]  %s%s%s  action=%s" % (r.get("ts") or "?", r.get("artifact_type") or "image", r.get("ref") or r.get("image") or "—",
              "  (reversible:false)" if r.get("reversible") is False else "", "  (finished %s)" % r["promoted_at"] if r.get("promoted_at") else "", r.get("action") or "deploy"))
sys.exit(bad)' || RC=1
  done
  exit $RC
fi

# --- run + verify per group ------------------------------------------------------
LOGDIR="${GOAT_DEPLOY_LOG_DIR:-${TMPDIR:-/tmp}}"; TS="$(date -u +%Y%m%dT%H%M%SZ)"; RC=0; VERIFIED=0
if [ "$RELEASE_MODE" = rollback ]; then
  note "rolling $COMP (artifact_type=release) to the previous file on every host of $NAME"
  MOVED=0
  # The ladder judges what is deployed; this goes back to what ran before.
  export GOAT_LADDER_CHECKED=1
  for G in $MEMBERS; do
    RB_VARS="-e release_file_component=$COMP -e release_file_ledger=$LEDGER -e release_file_rollback=true"
    RB_PLAY=()
    if [ "$CARRIED" = 1 ]; then
      # Its play is in the archive, the pin's: verified again first.
      _want="$(grep -E "^${VAR}:" "$REPO/ops/ansible/group_vars/$G/pins.yml" 2>/dev/null | sed -E 's/^[^:]+:[[:space:]]*"?//; s/"?[[:space:]]*$//' || true)"
      [ -n "$_want" ] || die "GB-381 refused before any host: $COMP's play is in its release, and $G has no pin for it (check: pin)"
      _got="$(release_fetch "$_want")"
      [ -n "$(release_field "$_got" play)" ] || die "$G: $(release_field "$_got" refused)"
      # As JSON: a path is one value, whatever it holds.
      RB_PLAY=(-e "$(release_field "$_got" play | "$PY" -c 'import json,sys; print(json.dumps({"release_play": sys.stdin.read().rstrip("\n")}))')")
    fi
    if [ "$DRY" = 1 ]; then
      note "$G: DRY: (cd ops/ansible && ../scripts/with-aws $AWS_PROFILE_DEPLOY -- ../scripts/ansible-preflight.sh $INV$PLAY_INV $PLAY_WORD ${TARGET_FLAG}$G $RB_VARS${RB_PLAY[*]:+ ${RB_PLAY[*]}})"
      note "$G: DRY: would read /var/lib/goat/$LEDGER/deploy-history.jsonl on $G and require a rollback row"
      continue
    fi
    LOG="$LOGDIR/goat-rollback-$COMP-$G-$TS.log"
    [ "$TRANSPORT_KIND" != door ] || open_door || true
    # The rows before the play: a rollback row that was there already is not this run's.
    BEFORE="$(ansible_read "$G" "tail -n 1 /var/lib/goat/$LEDGER/deploy-history.jsonl 2>/dev/null || echo NO_LEDGER" -o 2>/dev/null || true)"
    # shellcheck disable=SC2086 # INV and RB_VARS are words; the names in them hold no space
    ( cd "$REPO/ops/ansible" && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws "$AWS_PROFILE_DEPLOY" -- \
        ../scripts/ansible-preflight.sh $INV$PLAY_INV "$PLAY_WORD" ${TARGET_FLAG}"$G" $RB_VARS ${RB_PLAY[@]+"${RB_PLAY[@]}"} ) > "$LOG" 2>&1 && PLAY_RC=0 || PLAY_RC=$?
    RECAP="$(grep -A8 'PLAY RECAP' "$LOG" | grep -E 'ok=' || true)"
    if [ -z "$RECAP" ] && grep -q 'skipping: no hosts matched' "$LOG"; then
      note "$G: no host of this group in the loaded inventories (declared, not built?) — nothing rolled back, skipping"; continue
    fi
    if [ -n "$RECAP" ]; then printf '%s\n' "$RECAP" | while IFS= read -r l; do note "$G: $l"; done; fi
    if [ "$PLAY_RC" != 0 ] || [ -z "$RECAP" ]; then
      note "$G: rollback FAILED — log $LOG"; grep -n -E 'fatal:|REFUSE' "$LOG" | head -3 | sed 's/^/      /'; RC=1
    fi
    # Each host's own ledger says whether it moved, a failed play included.
    ROWS="$(ansible_read "$G" "tail -n 1 /var/lib/goat/$LEDGER/deploy-history.jsonl 2>/dev/null || echo NO_LEDGER" -o 2>/dev/null)" && LEDGER_RC=0 || LEDGER_RC=$?
    INVENTORY="$( cd "$REPO/ops/ansible" && GOAT_AWS_MODE="$READONLY_AWS_MODE" ../scripts/with-aws "$AWS_PROFILE_DEPLOY" -- ansible-inventory $INV --list 2>/dev/null || echo '{}')"
    _said="$(rb_inv="$INVENTORY" rb_grp="$G" rb_rows="$ROWS" rb_before="$BEFORE" "$PY" -c '
import json, os
def short(r):
    n, sep, h = r.partition("@sha256:")
    n = r.split("@")[0].rsplit("/", 1)[-1]
    return n + "@sha256:" + h[:12] if sep else r
seen, before = set(), set(os.environ["rb_before"].splitlines())
for line in os.environ["rb_rows"].splitlines():
    if not line.strip(): continue
    host = line.split()[0]; seen.add(host)
    try: row = json.loads(line[line.index("{"):line.rindex("}") + 1])
    except ValueError: row = {}
    if row.get("action") == "rollback" and line in before:
        print("NO %s: NOT rolled back — this run wrote no ledger row; its newest is an earlier rollback (%s)" % (host, row.get("ts") or ""))
    elif row.get("action") == "rollback":
        print("OK %s: rolled back: %s -> %s" % (host, short(str(row.get("prior_ref") or "")), short(str(row.get("ref") or ""))))
    else:
        tail = (line.split(">>", 1)[1] if ">>" in line else line).strip()[:140]
        print("NO %s: NOT rolled back — ledger tail: %s" % (host, tail))
try: inv = json.loads(os.environ["rb_inv"])
except ValueError: inv = {}
todo, done = [os.environ["rb_grp"]], set()
while todo:
    g = todo.pop()
    if g in done: continue
    done.add(g); e = inv.get(g) or {}
    todo += e.get("children") or []
    for h in e.get("hosts") or []:
        if h not in seen: seen.add(h); print("NO %s: NOT rolled back — no ledger row came back from this host" % h)
')"
    if [ -n "$_said" ]; then printf '%s\n' "$_said" | while IFS= read -r l; do note "${l#?? }"; done; fi
    if printf '%s\n' "$_said" | grep -q '^NO '; then RC=1; fi
    if [ "$LEDGER_RC" != 0 ]; then note "$G: ledger read FAILED (exit $LEDGER_RC)"; RC=1; fi
    MOVED=$((MOVED + $(printf '%s\n' "$_said" | grep -c '^OK ' || true)))
  done
  if [ "$DRY" = 1 ]; then note "$COMP on $GOATNET: DRY: nothing was read or changed"; exit 0; fi
  if [ "$MOVED" = 0 ] && [ "$RC" = 0 ]; then note "$COMP on $GOATNET: nothing was rolled back — no group had a host"; RC=1; fi
  if [ "$RC" = 0 ]; then note "$COMP on $GOATNET: every host is on the file before; the pin still names the one it left"
  else note "$COMP on $GOATNET: NOT rolled back everywhere (see above)"; fi
  exit $RC
fi
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
  # What is played on and read back: the group, or the one machine of it.
  WHO="$G"; LIMIT=(); if [ -n "$NODE" ]; then WHO="$NODE"; LIMIT=(-l "$NODE"); fi
  RVARS="$(release_vars "$G")"
  if [ -n "$RVARS" ]; then EXTRA_VARS=(-e "@$RVARS"); else EXTRA_VARS=(); fi
  if [ "$VERIFY_ONLY" = 0 ]; then
    # Show the ref with ghcr_org resolved (GHCR_ORG, else the organisation the
    # checkout declares), so the note does not read like an unrendered/broken
    # variable. Ansible still renders the real value at apply time from the
    # goatnet's group_vars.
    SHOWN="$WANT"
    [ -z "$IMAGE_ORG" ] || SHOWN="$(printf '%s' "$WANT" | sed "s#{{ ghcr_org }}#${IMAGE_ORG}#g")"
    note "$G: deploying $SHOWN"
    if [ "$DRY" = 1 ]; then
      note "DRY: (cd ops/ansible && ../scripts/with-aws $AWS_PROFILE_DEPLOY -- ../scripts/ansible-preflight.sh $INV$PLAY_INV $PLAY_WORD ${TARGET_FLAG}$G${EXTRA_VARS[*]:+ ${EXTRA_VARS[*]}}${LIMIT[*]:+ ${LIMIT[*]}}) > $LOG"
    else
      [ "$TRANSPORT_KIND" != door ] || open_door || note "$G: door to $NAME did not re-open; trying the play anyway"
      run_play() {
        # shellcheck disable=SC2086 # INV and PLAY_INV are words; the names in them hold no space
        ( cd "$REPO/ops/ansible" && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/with-aws "$AWS_PROFILE_DEPLOY" -- \
            ../scripts/ansible-preflight.sh $INV$PLAY_INV "$PLAY_WORD" ${TARGET_FLAG}"$G" ${EXTRA_VARS[@]+"${EXTRA_VARS[@]}"} ${LIMIT[@]+"${LIMIT[@]}"} ) > "$1" 2>&1 \
          && DEPLOY_RC=0 || DEPLOY_RC=$?
      }
      run_play "$LOG"
      # A dropped connection, not a failed task: re-open the door (door transport)
      # and run the play once more. The play is idempotent, so a second run after
      # a partial one converges the same way (GOAT_DEPLOY_TRANSPORT_RETRY=0: off).
      if [ "$DEPLOY_RC" != 0 ] && [ "${GOAT_DEPLOY_TRANSPORT_RETRY:-1}" != 0 ] && transport_failed "$LOG"; then
        note "$G: the connection dropped (log $LOG); re-opening the transport and running the play once more"
        [ "$TRANSPORT_KIND" != door ] || open_door || true
        mv "$LOG" "$LOG.first"
        run_play "$LOG"
      fi
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
  if [ "$DRY" = 1 ]; then
    note "DRY: would read /var/lib/goat/$LEDGER/deploy-history.jsonl on $WHO and require ref digest $WANT_DIGEST$served_note"
    if [ "$RELEASE" = 1 ]; then note "DRY: would read the file $COMP runs on each host of $G and require the pin's"; fi
    continue
  fi
  # Hosts in this group that are NOT the declared writer carry this surface by
  # design-absence, so a missing ledger there is the correct state, not drift.
  # The inventory read is also what says which hosts MUST answer: a failed read,
  # or a host that never returns a row, is not convergence (#1545).
  [ "$TRANSPORT_KIND" != door ] || open_door || true
  if ! INVENTORY="$( cd "$REPO/ops/ansible" && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES GOAT_AWS_MODE="$READONLY_AWS_MODE" ../scripts/with-aws "$AWS_PROFILE_DEPLOY" -- \
      ansible-inventory $INV --list 2> "$LOG.inventory" )"; then
    note "$G: inventory read FAILED — log $LOG.inventory"; RC=1; continue
  fi
  if ! HOSTSPLIT="$(printf '%s' "$INVENTORY" | WR="$WRITE_ROLE" GRP="$G" GV="$REPO/ops/ansible/group_vars" "$PY" -c '
import glob, json, os, sys, yaml
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
if os.environ["WR"].startswith("host:"):
    # push::one_host: group_vars/<group>/*.yml in name order (a later file wins),
    # then group_vars/all.yml. Unset, empty or templated cannot be read here.
    var, value = os.environ["WR"][5:], None
    for f in sorted(glob.glob(os.path.join(os.environ["GV"], grp, "*.yml"))):
        d = yaml.safe_load(open(f)) or {}
        if isinstance(d, dict) and var in d:
            value = d[var]
    if value is None and os.path.exists(os.path.join(os.environ["GV"], "all.yml")):
        value = (yaml.safe_load(open(os.path.join(os.environ["GV"], "all.yml"))) or {}).get(var)
    host = value.strip() if isinstance(value, str) else ""
    if not host or "{{" in host:
        sys.stderr.write("%s for %s cannot be read from the checkout (unset, empty or templated)\n" % (var, grp)); sys.exit(3)
    if host not in hosts:
        sys.stderr.write("%s for %s names %s, which is not a host of %s in the loaded inventories\n" % (var, grp, host, grp)); sys.exit(3)
    skip = [h for h in hosts if h != host]
else:
    skip = [h for h in hosts if want and str(meta.get(h, {}).get("router_role", "A")).upper() != want]
print(" ".join(h for h in hosts if h not in skip))
print(" ".join(skip))
' 2>"$LOG.hostsplit")"; then
    if [ -s "$LOG.hostsplit" ]; then note "$G: inventory could not be read — $(tail -1 "$LOG.hostsplit")"; else note "$G: inventory could not be read — log $LOG.inventory"; fi
    RC=1; continue
  fi
  EXPECTED_HOSTS="$(printf '%s\n' "$HOSTSPLIT" | sed -n 1p)"
  SKIP_HOSTS="$(printf '%s\n' "$HOSTSPLIT" | sed -n 2p)"
  if [ -n "$NODE" ]; then
    case " $EXPECTED_HOSTS $SKIP_HOSTS " in *" $NODE "*) ;; *) note "$NODE: NOT converged — $NODE is not a host of $G in the loaded inventories"; RC=1; continue ;; esac
    case " $EXPECTED_HOSTS " in *" $NODE "*) EXPECTED_HOSTS="$NODE" ;; *) EXPECTED_HOSTS="" ;; esac
  fi
  case "$WRITE_ROLE" in
    host:*) PLACEMENT="on one host, ${WRITE_ROLE#host:}=$(printf '%s\n' "$HOSTSPLIT" | sed -n 1p)" ;;
    *) PLACEMENT="single-homed on router_role=$WRITE_ROLE" ;;
  esac
  [ -n "$SKIP_HOSTS" ] && note "$G: $PLACEMENT — not expected on: $SKIP_HOSTS"
  if [ -z "$EXPECTED_HOSTS" ]; then
    note "$G: no host of this group is expected to carry $LEDGER in the loaded inventories — nothing to verify"; continue
  fi
  SEENFILE="$LOGDIR/.deploy-$COMP-$G-$TS.seen"; : > "$SEENFILE"
  ROWS="$( cd "$REPO/ops/ansible" && OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES GOAT_AWS_MODE="$READONLY_AWS_MODE" ../scripts/with-aws "$AWS_PROFILE_DEPLOY" -- \
      ansible $INV "$WHO" -m shell -a "tail -n 1 /var/lib/goat/$LEDGER/deploy-history.jsonl 2>/dev/null || echo NO_LEDGER" -o 2> "$LOG.verify" )" \
    && LEDGER_RC=0 || LEDGER_RC=$?
  # A release component's file, as each host has it installed.
  FILE_ROWS=""
  if [ "$RELEASE" = 1 ]; then
    FILE_ROWS="$(ansible_read "$WHO" "readlink -e $RELEASE_CURRENT >/dev/null && echo GBFILE \$(uname -m) \$(sha256sum \$(readlink -e $RELEASE_CURRENT)) || echo GBFILE none" -o 2>> "$LOG.verify" || true)"
  fi
  printf '%s\n' "$ROWS" | while IFS= read -r line; do
    [ -n "$line" ] || continue
    host="${line%% *}"
    case " $SKIP_HOSTS " in *" $host "*)
      case "$WRITE_ROLE" in
        host:*) note "$host: skipped — runs $PLACEMENT" ;;
        *) note "$host: skipped — single-homed on router_role=$WRITE_ROLE (ADR 1084 §D3)" ;;
      esac
      continue ;;
    esac
    echo "$host" >> "$SEENFILE"
    # The row must name the digest AND say the deploy finished. rollback-ledger
    # writes "pending" up front and a handler promotes it to "deploy" only for
    # hosts that reached the end of the play — so a refused deploy leaves a row
    # carrying the right digest and the wrong action. Matching the digest alone
    # is what let three false rows read as converged.
    if printf '%s' "$line" | grep -q "$WANT_DIGEST" && printf '%s' "$line" | grep -qE '"action"[[:space:]]*:[[:space:]]*"deploy"'; then
      if [ "$RELEASE" = 1 ]; then
        file_rc=3; file_line="$(printf '%s' "$RELEASE_WHY" | sed -n "s/^$G //p" | head -1)"
        if [ -n "$RVARS" ]; then
          file_line="$(printf '%s\n' "$FILE_ROWS" | awk -v h="$host" '$1==h{print; exit}' | "$GOATBUILDER_BIN" -C "$REPO" release-file judge --vars "$RVARS" 2>/dev/null)" && file_rc=0 || file_rc=$?
        fi
        case "$file_rc" in
          0) note "$host: $file_line" ;;
          1) note "$host: NOT converged — ledger on $WANT_DIGEST, but $file_line"; echo FAIL > "$LOGDIR/.deploy-$COMP-$G-$TS.fail"; continue ;;
          *) note "$host: unknown — ledger on $WANT_DIGEST, but its file was not compared: $file_line"; echo FAIL > "$LOGDIR/.deploy-$COMP-$G-$TS.fail"; continue ;;
        esac
      fi
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
if [ "$DRY" = 1 ] && [ -n "$DRY_REFUSED" ]; then
  note "$COMP on $GOATNET: DRY: a real run would refuse before any host ($DRY_REFUSED); nothing was read"; exit 1
elif [ "$DRY" = 1 ]; then note "$COMP on $GOATNET: DRY: nothing was read; a real run reads every host's ledger"
elif [ "$RC" = 0 ]; then note "$COMP on $GOATNET: every host's ledger names the declared ref"
else note "$COMP on $GOATNET: NOT converged everywhere (see above)"; fi
exit $RC
