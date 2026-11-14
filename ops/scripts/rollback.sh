#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# Easy rollback per ADR 0859, generalized to a universal capability per
# ADR 1060 (docs/design/universal-deploy-rollback.md).
#
# Roll a deployed component back to its N-th most recent ref, dispatching
# by artifact type and reading the ON-HOST runtime ledger as the source of
# truth (covering role-default, group_vars, AND env-var pins uniformly —
# the in-repo deploy-history.yml only sees role-default *_image pins).
#
# Usage:
#   ops/scripts/rollback.sh <component> [N]      # roll back N steps (default 1)
#   ops/scripts/rollback.sh <component> --list   # show deploy history
#   ops/scripts/rollback.sh <component> --status # show running ref + artifact type per host
#
# Artifact-type dispatch (read from the on-host ledger; default `image`):
#   image        -> re-pin compose/env + force-recreate (automated).
#   config / acl -> guided: print the git-SHA revert + redeploy; no auto-flip.
#   data         -> REFUSE: print the restore pointer (pgBackRest / Object-Lock).
#   cert         -> REFUSE auto: print the rotate pointer (goat admin rotate).
#   forward-only -> REFUSE (reversible:false / do_not_roll_to).
#
# Source-of-truth precedence:
#   - Connection map (host / ssh_user / container / compose) : in-repo
#     ops/ansible/roles/<component>/deploy-history.yml `hosts:` block.
#   - Artifact type / reversible / restore pointer / ref history : the
#     on-host /var/lib/goat/<component>/deploy-history.jsonl when present
#     (requires jq); falls back to the in-repo `history:` block otherwise
#     (back-compat with components not yet on the shared rollback-ledger
#     role).
#
# Idempotent: re-running is a no-op if already on the target ref.
#
# Refuses on:
#   - artifact type data/cert/forward-only or reversible:false (prints pointer).
#   - target ref flagged `do_not_roll_to: true` in the in-repo ledger.
#   - host's running ref not in any known history (host-vs-record divergence).

set -euo pipefail
# Hold the Mac awake for the whole run (#2122; ops/scripts/lib/keep-awake.sh).
_keep_awake="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/keep-awake.sh"
# shellcheck source=ops/scripts/lib/keep-awake.sh
if [ -f "$_keep_awake" ]; then . "$_keep_awake"; goat_keep_awake; fi

SCRIPT="${0##*/}"
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

die() { echo "$SCRIPT: $*" >&2; exit 1; }
note() { echo "$SCRIPT: $*" >&2; }

# Distinct exit codes so wrappers can tell refusal from error.
EX_REFUSE_UNSAFE=3   # data/cert/forward-only/reversible:false
EX_GUIDED=4          # config/acl — manual revert path printed

usage() {
  cat <<EOF >&2
usage:
  $SCRIPT <component> [N]            roll back N steps (default 1; max 7)
  $SCRIPT <component> --list         show deploy history (host ledger if present, else in-repo)
  $SCRIPT <component> --status       show running ref + artifact type per deployed host
  ... --site <site>                  only the hosts: entries with this site: (zagros-dev, zagros, ...)
  ... --offline                      an islanded site: judge the checkout against the cached trunk
EOF
  exit 2
}

[ $# -ge 1 ] || usage
SERVICE="$1"; shift
MODE="1"; SITE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --site) [ $# -ge 2 ] || usage; SITE="$2"; shift 2 ;;
    --offline) export GOAT_DEPLOY_OFFLINE=1; shift ;;
    *) MODE="$1"; shift ;;
  esac
done

ROLE_DIR="${REPO_ROOT}/ops/ansible/roles/${SERVICE}"
LEDGER="${ROLE_DIR}/deploy-history.yml"

[ -d "$ROLE_DIR" ] || die "no Ansible role at $ROLE_DIR — is '$SERVICE' a deploying component?"
[ -f "$LEDGER" ] || die "no deploy-history.yml at $LEDGER — '$SERVICE' not yet adopted (ADR 0859/1060)"

command -v yq >/dev/null 2>&1 || die "yq required (brew install yq); ledger parsing won't work without it"

HAVE_JQ=0
command -v jq >/dev/null 2>&1 && HAVE_JQ=1
[ "$HAVE_JQ" -eq 1 ] || note "jq not found — host-truth ledger path disabled; using in-repo deploy-history.yml only (install jq to cover group_vars/env-pinned components)"

# BatchMode: a host that would prompt (no key, a passphrase the agent lacks)
# fails at once and reads as unreachable, the same in both avenues (ADR 1201).
SSH_OPTS=(-o StrictHostKeyChecking=no -o ConnectTimeout=4 -o BatchMode=yes)

# rssh <user> <host> <command...> — run a command on a ledger host. Direct SSH
# first. A host this laptop cannot reach directly (a goatnet reached through its
# env door, docs/design/env-doors.md) is reached through the --site goatnet's
# door when that goatnet's map is one host: a door lands on one host, so a
# multi-host map never guesses which. ops/scripts/door-open.sh opens or reuses
# the door and proves ssh lands before the port is used.
DIRECT_OK=" "; DIRECT_NO=" "; DOOR_PORT=""
door_port() {
  [ -n "$SITE" ] && [ "$HOST_COUNT" -eq 1 ] || return 1
  [ -n "$DOOR_PORT" ] && return 0
  local door; door="$(dirname "$0")/door-open.sh"
  [ -x "$door" ] || [ -f "$door" ] || return 1
  bash "$door" "$SITE" >/dev/null 2>&1 || return 1
  DOOR_PORT="$(bash "$door" "$SITE" --status 2>/dev/null | sed -n 's/.*(127\.0\.0\.1:\([0-9]*\)).*/\1/p' | head -1)"
  [ -n "$DOOR_PORT" ]
}
rssh() {
  local user="$1" host="$2"; shift 2
  case "$DIRECT_NO" in *" $host "*) ;; *)
    case "$DIRECT_OK" in *" $host "*) ssh "${SSH_OPTS[@]}" "${user}@${host}" "$@"; return ;; esac
    if ssh "${SSH_OPTS[@]}" "${user}@${host}" true >/dev/null 2>&1; then
      DIRECT_OK="$DIRECT_OK$host "; ssh "${SSH_OPTS[@]}" "${user}@${host}" "$@"; return
    fi
    DIRECT_NO="$DIRECT_NO$host " ;;
  esac
  if door_port; then
    note "$host is not reachable directly; using the $SITE door (127.0.0.1:$DOOR_PORT)"
    ssh "${SSH_OPTS[@]}" -p "$DOOR_PORT" "${user}@127.0.0.1" "$@"; return
  fi
  ssh "${SSH_OPTS[@]}" "${user}@${host}" "$@"
}

# The in-repo ledger as JSON, whichever yq is installed: mikefarah's yq (brew)
# and the jq-wrapping Python yq (pip) take different flags, and their @tsv
# differ — mikefarah's leaves a newline inside a field raw (and quotes the
# field), which split one multi-line summary into several numbered rows.
# Through jq's @tsv a newline, tab or backslash is always escaped.
ledger_json() {
  if yq --version 2>&1 | grep -qi mikefarah; then yq -o=json '.' "$1"; else yq '.' "$1"; fi
}
# A role's explicit restore-only policy applies even when every host is offline
# or carries an older ledger row. Read-only modes exit before this guard is called.
refuse_declared_irreversible() {
  local meta="$ROLE_DIR/meta/main.yml" declared
  [ -f "$meta" ] || return 0
  command -v jq >/dev/null 2>&1 || die "jq is required to verify declared rollback safety"
  if ! declared="$(ledger_json "$meta" | jq -sce '
    if length != 1 or (.[0] | type) != "object" then error("invalid role metadata") else .[0] end
    | [.dependencies[]? | select(type == "object" and .role == "rollback-ledger")
       | .vars | select(.rollback_reversible == false)]
  ')"; then
    die "cannot verify declared rollback safety for $SERVICE"
  fi
  if [ "$(printf '%s' "$declared" | jq length)" -gt 0 ]; then
    note "REFUSING: '$SERVICE' declares reversible:false in its role metadata."
    printf '%s' "$declared" | jq -r '.[] | .rollback_restore_pointer // "Consistent-state restoration is required."' >&2
    exit "$EX_REFUSE_UNSAFE"
  fi
}

runtime_ledger_path() { echo "/var/lib/goat/${1}/deploy-history.jsonl"; }

# Pull the host map once (connection details only live in the YAML).
HOST_COUNT="$(yq -r '.hosts | length' "$LEDGER")"
[ "$HOST_COUNT" -ge 1 ] || die "deploy-history.yml has no hosts entries"

# --site: the same ledger with its hosts: narrowed to one site, so a component
# that runs on several goatnets is listed, read and rolled back on one. JSON is
# YAML, so every read below works on the narrowed copy unchanged.
if [ -n "$SITE" ]; then
  command -v jq >/dev/null 2>&1 || die "--site needs jq"
  NARROWED="$(mktemp "${TMPDIR:-/tmp}/rollback-site.XXXXXX")"
  trap 'rm -f "$NARROWED"' EXIT
  ledger_json "$LEDGER" | jq --arg s "$SITE" '.hosts |= map(select(.site == $s))' > "$NARROWED"
  LEDGER="$NARROWED"
  HOST_COUNT="$(yq -r '.hosts | length' "$LEDGER")"
  [ "$HOST_COUNT" -ge 1 ] || die "deploy-history.yml has no hosts entry with site: $SITE"
fi

# --- host-truth helpers (jq-gated) ----------------------------------------

# Echo the latest jsonl row (one JSON object) from a host, or empty.
host_latest_row() {
  local user="$1" host="$2" comp="$3"
  rssh "${user}" "${host}" \
    "sudo tail -n 1 $(runtime_ledger_path "$comp") 2>/dev/null" 2>/dev/null || true
}

# Echo distinct refs newest-first (one per line) from a host's jsonl.
host_refs_newest_first() {
  local user="$1" host="$2" comp="$3"
  rssh "${user}" "${host}" \
    "sudo tac $(runtime_ledger_path "$comp") 2>/dev/null" 2>/dev/null \
    | jq -r '(.ref // .image) // empty' 2>/dev/null \
    | awk '!seen[$0]++'
}

# Determine the component's artifact type / reversible / restore pointer from
# the first reachable host's latest jsonl row. Sets AT_TYPE, AT_REVERSIBLE,
# AT_POINTER, AT_SOURCE.
resolve_artifact_meta() {
  AT_TYPE="image"; AT_REVERSIBLE="true"; AT_POINTER=""; AT_SOURCE="default(image)"
  [ "$HAVE_JQ" -eq 1 ] || return 0
  local i user host row
  for i in $(seq 0 $((HOST_COUNT - 1))); do
    user="$(yq -r ".hosts[$i].ssh_user" "$LEDGER")"
    host="$(yq -r ".hosts[$i].host" "$LEDGER")"
    row="$(host_latest_row "$user" "$host" "$SERVICE")"
    [ -n "$row" ] || continue
    AT_TYPE="$(echo "$row" | jq -r '.artifact_type // "image"' 2>/dev/null || echo image)"
    AT_REVERSIBLE="$(echo "$row" | jq -r '.reversible // true' 2>/dev/null || echo true)"
    AT_POINTER="$(echo "$row" | jq -r '.restore_pointer // ""' 2>/dev/null || echo "")"
    AT_SOURCE="host-ledger($host)"
    return 0
  done
  return 0
}

# --- mode: --list ----------------------------------------------------------
if [ "$MODE" = "--list" ]; then
  echo "deploy history for $SERVICE (newest first):"
  if [ "$HAVE_JQ" -eq 1 ]; then
    # Try each host until one returns a ledger (a component may run on
    # several goatnets; the first host can be down — fall through to the
    # next reachable one before giving up to the in-repo history).
    for i in $(seq 0 $((HOST_COUNT - 1))); do
      USERi="$(yq -r ".hosts[$i].ssh_user" "$LEDGER")"
      HOSTi="$(yq -r ".hosts[$i].host" "$LEDGER")"
      SITEi="$(yq -r ".hosts[$i].site // \"\"" "$LEDGER")"
      HROWS="$(rssh "${USERi}" "${HOSTi}" \
        "sudo tac $(runtime_ledger_path "$SERVICE") 2>/dev/null" 2>/dev/null || true)"
      if [ -n "$HROWS" ]; then
        echo "  (on-host ledger @ ${HOSTi}${SITEi:+, goatnet ${SITEi}} — source of truth)"
        if [ -z "$SITE" ] && [ "$HOST_COUNT" -gt 1 ]; then
          note "this is the first reachable of $HOST_COUNT hosts; pass --site <goatnet> for one goatnet's history"
        fi
        echo "$HROWS" | jq -r '"  " + (.ts // "?") + "  [" + (.artifact_type // "image") + "]  " + ((.ref // .image) // "—") + (if (.reversible // true)==false then "  (reversible:false)" else "" end) + (if .promoted_at then "  (finished " + .promoted_at + ")" else "" end)'
        exit 0
      fi
    done
    note "no reachable on-host ledger — falling back to in-repo deploy-history.yml"
  fi
  if [ "$HAVE_JQ" -eq 1 ]; then
    ledger_json "$LEDGER" | jq -r '.history[] | [.sha, .merged_at, "PR#" + (.pr | tostring), .summary] | @tsv'
  else
    yq -r '.history[] | [.sha, .merged_at, "PR#" + (.pr | tostring), .summary] | @tsv' "$LEDGER"
  fi \
    | awk -F'\t' '{ printf "  [%d] %-14s %s  %s\n    %s\n", NR-1, $1, $2, $3, $4 }'
  exit 0
fi

# --- mode: --status --------------------------------------------------------
if [ "$MODE" = "--status" ]; then
  resolve_artifact_meta
  echo "$SERVICE — artifact_type=${AT_TYPE} reversible=${AT_REVERSIBLE} (from ${AT_SOURCE})"
  for i in $(seq 0 $((HOST_COUNT - 1))); do
    HOST="$(yq -r ".hosts[$i].host" "$LEDGER")"
    SSH_USER="$(yq -r ".hosts[$i].ssh_user" "$LEDGER")"
    CONTAINER="$(yq -r ".hosts[$i].container_name" "$LEDGER")"
    CURRENT="$(rssh "${SSH_USER}" "${HOST}" \
      "sudo docker inspect ${CONTAINER} --format '{{.Config.Image}}'" 2>/dev/null \
      || echo "<unreachable>")"
    printf "  %s @ %s: %s\n" "$CONTAINER" "$HOST" "$CURRENT"
  done
  exit 0
fi

# --- mode: roll back N steps ----------------------------------------------
N="$MODE"
[[ "$N" =~ ^[1-7]$ ]] || die "N must be 1..7 (got: $N)"

# Fresh source (contract C3): the guards below read this checkout's
# deploy-history.yml (do_not_roll_to, the hosts map), so a checkout behind the
# trunk in ops/ansible or ops/pins is refused before any host is touched.
"$(dirname "$0")/fresh-source.sh" --repo "$REPO_ROOT" >/dev/null \
  || die "refusing before any host is touched: this checkout is not current with the source of record (above)"

# Fail-closed (GB-055): a rollback with no --site that would flip more than one
# goatnet is refused, so `rollback.sh <c> <N>` can never flip prod as a side
# effect of touching dev. A single-goatnet component still rolls back with no
# --site. Mirror of goatbuilder rollback's require_site_when_multi_goatnet; the
# site→goatnet fact is each host's `site:` (generate-pins.py's host→goatnet
# resolution, recorded in the ledger). jq-gated like the rest of the host path;
# the CLI avenue applies the guard without jq.
if [ -z "$SITE" ] && [ "$HAVE_JQ" -eq 1 ]; then
  NETS="$(ledger_json "$LEDGER" | jq -r '[.hosts[]? | (.site // "") | select(. != "")] | unique | .[]')"
  NCOUNT="$(printf '%s\n' "$NETS" | grep -c . || true)"
  [ -n "$NCOUNT" ] || NCOUNT=0
  if [ "$NCOUNT" -gt 1 ]; then
    LIST="$(printf '%s\n' "$NETS" | paste -sd',' - | sed 's/,/, /g')"
    PIPES="$(printf '%s\n' "$NETS" | paste -sd'|' -)"
    die "'$SERVICE' is deployed on $NCOUNT goatnets ($LIST); a rollback with no --site would flip every one, prod included. Re-run with --site <$PIPES>."
  fi
fi

# Enforce the current source policy before consulting any host ledger.
refuse_declared_irreversible

# Artifact-type gate — refuse / guide where a pin swap is not the mechanism.
resolve_artifact_meta
case "$AT_TYPE" in
  data)
    note "REFUSING: '$SERVICE' is artifact_type=data (${AT_SOURCE}). A ref swap cannot undo committed data."
    note "  Restore via backup: ${AT_POINTER:-<no restore_pointer recorded; see pgbackrest.md / audit Object-Lock>}"
    exit "$EX_REFUSE_UNSAFE" ;;
  cert)
    note "REFUSING auto-rollback: '$SERVICE' is artifact_type=cert (${AT_SOURCE}). Certs are re-minted, not reverted (ADR 1044)."
    note "  Rotate: ${AT_POINTER:-goat admin rotate <anchor|alias>}"
    exit "$EX_REFUSE_UNSAFE" ;;
  forward-only)
    note "REFUSING: '$SERVICE' is artifact_type=forward-only (${AT_SOURCE}) — not reversible."
    note "  ${AT_POINTER:-Forward-fix only; there is no safe rollback target.}"
    exit "$EX_REFUSE_UNSAFE" ;;
esac
if [ "$AT_REVERSIBLE" = "false" ]; then
  note "REFUSING: '$SERVICE' latest deploy is flagged reversible:false (${AT_SOURCE})."
  note "  ${AT_POINTER:-Forward-fix only.}"
  exit "$EX_REFUSE_UNSAFE"
fi

# Resolve the target ref. Prefer the on-host ledger (covers group_vars/env
# pins); fall back to the in-repo history block.
TARGET_IMAGE=""
TARGET_SRC=""
CURRENT_IMAGE=""
if [ "$HAVE_JQ" -eq 1 ]; then
  # Use the first REACHABLE host's ledger (hosts[0] may be a down goatnet).
  for i in $(seq 0 $((HOST_COUNT - 1))); do
    USERi="$(yq -r ".hosts[$i].ssh_user" "$LEDGER")"
    HOSTi="$(yq -r ".hosts[$i].host" "$LEDGER")"
    HREFS=()
    while IFS= read -r ref; do
      HREFS+=("$ref")
    done < <(host_refs_newest_first "$USERi" "$HOSTi" "$SERVICE")
    if (( ${#HREFS[@]} == 0 )); then
      continue
    fi
    if [ "${#HREFS[@]}" -gt "$N" ]; then
      TARGET_IMAGE="${HREFS[$N]}"
      TARGET_SRC="host-ledger(${HOSTi})"
      CURRENT_IMAGE="${HREFS[0]}"
    fi
    break
  done
fi

if [ -z "$TARGET_IMAGE" ]; then
  HISTORY_COUNT="$(yq -r '.history | length' "$LEDGER")"
  [ "$HISTORY_COUNT" -ge 1 ] || die "no history available (host ledger absent + in-repo history empty)"
  [ "$N" -lt "$HISTORY_COUNT" ] || die "history has only $HISTORY_COUNT entries; cannot roll back $N"
  TARGET_IMAGE="$(yq -r ".history[$N].image" "$LEDGER")"
  TARGET_SRC="in-repo deploy-history.yml"
  CURRENT_IMAGE="$(yq -r ".history[0].image" "$LEDGER")"
fi

# config / acl are reversible but not a pin swap — print the guided path.
if [ "$AT_TYPE" = "config" ] || [ "$AT_TYPE" = "acl" ]; then
  note "'$SERVICE' is artifact_type=${AT_TYPE} (${AT_SOURCE}) — guided rollback (no auto-flip)."
  note "  Target ref (${TARGET_SRC}): ${TARGET_IMAGE:-<none>}"
  note "  Revert in git, then redeploy the role: cd ops/ansible && <env>-refresh.yml -e target=..."
  [ "$AT_TYPE" = "acl" ] && note "  WARNING (acl): a full router re-render clobbers portal-appended rules — make a SURGICAL edit, not a blind re-render."
  exit "$EX_GUIDED"
fi

# --- image: one repository only ---------------------------------------------
# A rollback moves a container between refs of ONE image repository. A ledger
# fed by two image vars (the ory ledger recorded the Kratos server while its
# in-repo history and its container are the consent UI) yields a target that
# would put another program in the container. Tag, digest and registry org
# are dropped before comparing, as check-deploy-history.sh normalizes them.
img_repo() { echo "$1" | sed -E 's/@.*$//; s#:[^:/]*$##; s#^(ghcr\.io/)[^/]+/#\1<ORG>/#'; }
if [ -n "$TARGET_IMAGE" ] && [ -n "$CURRENT_IMAGE" ] && [ "$(img_repo "$TARGET_IMAGE")" != "$(img_repo "$CURRENT_IMAGE")" ]; then
  note "REFUSING: the target ref $TARGET_IMAGE ($TARGET_SRC) is another image than the newest one, $CURRENT_IMAGE."
  note "  The ledger mixes two images; a flip would put the other program in this container. Roll back by hand from the in-repo history."
  exit "$EX_REFUSE_UNSAFE"
fi

# --- image: do_not_roll_to guard against the in-repo ledger ---------------
if yq -e ".history[] | select(.image == \"$TARGET_IMAGE\")" "$LEDGER" >/dev/null 2>&1; then
  DNRT="$(yq -r ".history[] | select(.image == \"$TARGET_IMAGE\") | .do_not_roll_to // false" "$LEDGER" | head -1)"
  TARGET_PR="$(yq -r ".history[] | select(.image == \"$TARGET_IMAGE\") | .pr // \"?\"" "$LEDGER" | head -1)"
  [ "$DNRT" = "true" ] && die "target ref $TARGET_IMAGE flagged do_not_roll_to=true in the in-repo ledger; refusing. See PR #$TARGET_PR."
else
  TARGET_PR="?"
fi

note "rolling $SERVICE (artifact_type=image) to [$N] $TARGET_IMAGE (target source: $TARGET_SRC; PR #$TARGET_PR)"

# Iterate each host.
for i in $(seq 0 $((HOST_COUNT - 1))); do
  HOST="$(yq -r ".hosts[$i].host" "$LEDGER")"
  SSH_USER="$(yq -r ".hosts[$i].ssh_user" "$LEDGER")"
  CONTAINER="$(yq -r ".hosts[$i].container_name" "$LEDGER")"
  COMPOSE="$(yq -r ".hosts[$i].compose_path" "$LEDGER")"
  COMPOSE_DIR="$(dirname "$COMPOSE")"
  ENV_FILE="$(yq -r ".hosts[$i].env_file // \"\"" "$LEDGER")"
  ENV_VAR="$(yq -r ".hosts[$i].env_var // \"\"" "$LEDGER")"
  # Optional multi-file compose (mgmt-host overlay services need every -f
  # file at recreate, and the image: line lives in the overlay not the base).
  # Paths must be space-free (they always are: /opt/.../docker-compose*.yml).
  CF_LIST="$(yq -r '.hosts['"$i"'].compose_files[]? // ""' "$LEDGER" | tr '\n' ' ')"
  RUNTIME_LEDGER="$(runtime_ledger_path "$SERVICE")"

  note "  host $HOST"

  # A component can run on several goatnets; a host in the map may be down.
  # Skip an unreachable host (or one without the container) and continue —
  # don't abort the whole rollback.
  PREV_IMAGE="$(rssh "${SSH_USER}" "${HOST}" \
    "sudo docker inspect ${CONTAINER} --format '{{.Config.Image}}'" 2>/dev/null || true)"
  if [ -z "$PREV_IMAGE" ]; then
    note "    unreachable or no '${CONTAINER}' container — skipping this host."
    continue
  fi
  note "    prev: $PREV_IMAGE"

  if [ "$PREV_IMAGE" = "$TARGET_IMAGE" ]; then
    note "    already on target — no-op."
    continue
  fi

  # Host-vs-record divergence guard: prev image must appear in a known
  # history (host ledger refs OR in-repo history). Drain each producer: grep -q
  # can close the pipe on the first match and turn a known image into a SIGPIPE
  # refusal under pipefail. Producer failures must still refuse the match.
  KNOWN=0
  if [ "$HAVE_JQ" -eq 1 ]; then
    if host_refs_newest_first "$SSH_USER" "$HOST" "$SERVICE" | grep -xF "$PREV_IMAGE" >/dev/null; then KNOWN=1; fi
  fi
  if [ "$KNOWN" -eq 0 ] && yq -r '.history[].image' "$LEDGER" | grep -xF "$PREV_IMAGE" >/dev/null; then KNOWN=1; fi
  if [ "$KNOWN" -eq 0 ]; then
    die "host $HOST is running $PREV_IMAGE which is in NO known history (host ledger + in-repo); refusing — operator must investigate. Use --status."
  fi

  TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

  # Image rollback reuses the existing environment. Running a full deploy
  # play here could start the newer image or migrate data before rolling back.
  note "    .env on $HOST is whatever this host's last real deploy rendered — rollback does not refresh it (#971)."
  note "    Check that this environment is compatible with $TARGET_IMAGE; use the service's documented configuration procedure if a change is needed."

  if [ -n "${CF_LIST// /}" ]; then
    # multi-file compose (overlay services): sed the file that actually
    # carries the prev image, then recreate with every -f file.
    MF_DIR="$(dirname "${CF_LIST%% *}")"
    FF=""; for f in $CF_LIST; do FF="$FF -f $f"; done
    note "    flipping image across compose files [$CF_LIST] (recreate with$FF)"
    rssh "${SSH_USER}" "${HOST}" "
        set -euo pipefail
        hit=''
        for f in ${CF_LIST}; do
          if sudo grep -qF '${PREV_IMAGE}' \"\$f\"; then hit=\"\$f\"; break; fi
        done
        [ -n \"\$hit\" ] || { echo 'rollback.sh: prev image not found in any compose file' >&2; exit 1; }
        sudo sed -i.rollback-bak \"s|^\(\s*image:\s*\\\"\?\)${PREV_IMAGE//\//\\/}\(\\\"\?\)|\1${TARGET_IMAGE//\//\\/}\2|\" \"\$hit\"
        if sudo grep -qF '${PREV_IMAGE}' \"\$hit\"; then echo \"rollback.sh: sed did not rewrite \$hit\" >&2; exit 1; fi
        sudo docker compose --project-directory '${MF_DIR}'${FF} up -d --force-recreate ${CONTAINER}
        sudo mkdir -p '$(dirname "$RUNTIME_LEDGER")'
        echo '{\"ts\":\"${TS}\",\"component\":\"${SERVICE}\",\"artifact_type\":\"image\",\"action\":\"rollback\",\"ref\":\"${TARGET_IMAGE}\",\"prior_ref\":\"${PREV_IMAGE}\",\"image\":\"${TARGET_IMAGE}\",\"reversible\":true,\"operator\":\"${USER:-unknown}@$(hostname)\",\"via\":\"rollback.sh:compose_files\",\"target_pr\":\"${TARGET_PR}\"}' | sudo tee -a '${RUNTIME_LEDGER}' >/dev/null
      "
  elif [ -n "$ENV_FILE" ] && [ -n "$ENV_VAR" ]; then
    note "    flipping ${ENV_VAR} in ${ENV_FILE} (compose-time substitution)"
    rssh "${SSH_USER}" "${HOST}" "
        set -euo pipefail
        sudo sed -i.rollback-bak \"s|^${ENV_VAR}=.*|${ENV_VAR}=${TARGET_IMAGE}|\" '${ENV_FILE}' \
          && sudo grep -q '^${ENV_VAR}=' '${ENV_FILE}' \
          || echo '${ENV_VAR}=${TARGET_IMAGE}' | sudo tee -a '${ENV_FILE}' >/dev/null
        sudo docker compose --project-directory '${COMPOSE_DIR}' -f '${COMPOSE}' up -d --force-recreate ${CONTAINER}
        sudo mkdir -p '$(dirname "$RUNTIME_LEDGER")'
        echo '{\"ts\":\"${TS}\",\"component\":\"${SERVICE}\",\"artifact_type\":\"image\",\"action\":\"rollback\",\"ref\":\"${TARGET_IMAGE}\",\"prior_ref\":\"${PREV_IMAGE}\",\"image\":\"${TARGET_IMAGE}\",\"reversible\":true,\"operator\":\"${USER:-unknown}@$(hostname)\",\"via\":\"rollback.sh:env_file\",\"target_pr\":\"${TARGET_PR}\"}' | sudo tee -a '${RUNTIME_LEDGER}' >/dev/null
      "
  else
    rssh "${SSH_USER}" "${HOST}" "
        set -euo pipefail
        sudo sed -i.rollback-bak \"s|^\(\s*image:\s*\\\"\?\)${PREV_IMAGE//\//\\/}\(\\\"\?\)|\1${TARGET_IMAGE//\//\\/}\2|\" '${COMPOSE}'
        if sudo grep -qF '${PREV_IMAGE}' '${COMPOSE}'; then
          echo 'rollback.sh: sed did not rewrite ${COMPOSE} — prev-image still present (image: line pattern miss). Aborting before docker recreate.' >&2
          exit 1
        fi
        sudo docker compose --project-directory '${COMPOSE_DIR}' -f '${COMPOSE}' up -d --force-recreate ${CONTAINER}
        sudo mkdir -p '$(dirname "$RUNTIME_LEDGER")'
        echo '{\"ts\":\"${TS}\",\"component\":\"${SERVICE}\",\"artifact_type\":\"image\",\"action\":\"rollback\",\"ref\":\"${TARGET_IMAGE}\",\"prior_ref\":\"${PREV_IMAGE}\",\"image\":\"${TARGET_IMAGE}\",\"reversible\":true,\"operator\":\"${USER:-unknown}@$(hostname)\",\"via\":\"rollback.sh\",\"target_pr\":\"${TARGET_PR}\"}' | sudo tee -a '${RUNTIME_LEDGER}' >/dev/null
      "
  fi

  note "    rolled back: $PREV_IMAGE -> $TARGET_IMAGE"
done

note "done — verify with: ${SCRIPT} ${SERVICE} --status"
