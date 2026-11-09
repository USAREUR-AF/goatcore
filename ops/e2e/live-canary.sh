#!/usr/bin/env bash
# live-canary.sh — does a deploy work on a real goatnet today? One command, from an operator laptop.
#
# docs/design/deployment-e2e.md §Live canary. Deploys the e2e-canary component
# to the dev goatnet through goatbuilder (the push converger over the real
# transport), proves it from the host's ledger, reads the rollback history back,
# and records the run (ops/e2e/report.py). The canary templates no secret, so no
# `op run`.
#
# usage: ops/e2e/live-canary.sh [--env GOATNET] [--host USER@HOST]
#                               [--goatbuilder BIN] [--record DIR] [--bump]
#   --env    the goatnet (its group_vars name, site_a_dev). Default: the goatnet
#            of the release ladder's `dev` rung (ops/release/ladder.yaml).
#   --host   user@host whose deploy ledger the rollback steps read. Default: the
#            goatnet's first management host in ops/ansible/inventory/doors.yaml,
#            reached through the goatnet's door (ssh://user@127.0.0.1:<port>)
#            when its mesh address does not answer.
#   --bump   first move the goatnet's canary deviation to the newest image CI
#            built from main (ghcr.io/<ghcr_org>/e2e-canary:sha-<main>, ghcr_org
#            from group_vars), regenerate and re-sign its intent (the operator's
#            CA key for it), land the pin through pr-land.sh, then deploy.
#            Without --bump the pin on main is deployed as declared. A
#            single-repository checkout only: from an assembled tree the pins
#            belong to the overlay, and --bump is refused.
#
# No GitHub runner reaches a goatnet, so this runs where the operator is:
# a laptop on the goatnet's mesh or holding its door (docs/design/env-doors.md).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
GB=""; RECORD=""; BUMP=0; ENV=""; HOST=""; COMP=e2e-canary
while [ $# -gt 0 ]; do
  case "$1" in
    --env) ENV="${2:?--env needs a goatnet}"; shift 2 ;;
    --host) HOST="${2:?--host needs user@host}"; shift 2 ;;
    --goatbuilder) GB="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2 ;;
    --record) RECORD="$2"; shift 2 ;;
    --bump) BUMP=1; shift ;;
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
    *) echo "live-canary: unknown argument $1" >&2; exit 2 ;;
  esac
done
[ -n "$ENV" ] || ENV="$(python3 "$REPO/ops/scripts/goatnet-facts.py" dev-goatnet)" \
  || { echo "live-canary: name the goatnet with --env (the release ladder has no dev rung)" >&2; exit 2; }
ENV="$(printf '%s' "$ENV" | tr '-' '_')"
case "$ENV" in ""|*[!a-z0-9_]*) echo "live-canary: '$ENV' is not a goatnet name" >&2; exit 2 ;; esac
GOATNET="$(printf '%s' "$ENV" | tr '_' '-')"
# The ledger reads below and both rollback avenues reach the goatnet the same
# way: rollback.sh and goatbuilder rollback take --site, which decides the mesh
# or the door as deploy does. The canary's own ssh goes to the management
# host's mesh address; off the mesh it jumps through the door to that same
# address (onboard-intake-smoke.sh via_ssh, ops/validation/probes.py _ssh_jump):
# only the door's tunnel hop skips the host-key check, the host's key is checked.
SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=10)
if [ -z "$HOST" ]; then
  HOST="$(python3 "$REPO/ops/scripts/goatnet-facts.py" mgmt-ssh "$ENV")" \
    || { echo "live-canary: no management host for $ENV in ops/ansible/inventory/doors.yaml; pass --host user@host" >&2; exit 2; }
  if ! ssh "${SSH_OPTS[@]}" "$HOST" true >/dev/null 2>&1 \
     && bash "$REPO/ops/scripts/door-open.sh" "$GOATNET" >/dev/null 2>&1 \
     && DOOR="$(bash "$REPO/ops/scripts/env-door.sh" "$GOATNET" --ssh-target)"; then
    echo "live-canary: $HOST does not answer; reading its ledger by jumping through the $GOATNET door ($DOOR)"
    SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new
              -o "ProxyCommand=ssh -o BatchMode=yes -o ConnectTimeout=6 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -p ${DOOR##*:} -W %h:%p ${DOOR%:*}")
  fi
fi
[ -n "$GB" ] || GB="$REPO/crates/target/debug/goatbuilder"
[ -x "$GB" ] || { echo "live-canary: no goatbuilder at $GB — build it: (cd crates && cargo build -p goatbuilder) or pass --goatbuilder" >&2; exit 2; }
[ -n "$RECORD" ] || RECORD="$(mktemp -d "${TMPDIR:-/tmp}/goat-live-canary.XXXXXX")"
mkdir -p "$RECORD"; export E2E_RECORD_DIR="$RECORD"
FAILS=0
rec() { python3 "$HERE/rec.py" "$@" || FAILS=$((FAILS + 1)); }
cd "$REPO"
if [ -f "$REPO/.goat-assembled" ]; then
  # An assembled tree is detached by construction; its currency is the overlay
  # commit it was built from, which the deploy itself checks (fresh-source.sh).
  [ "$BUMP" = 0 ] || { echo "live-canary: --bump edits the pins and the intent, which an assembled tree takes from its overlay; land the canary pin on the overlay (its assembled-build pins branch), re-assemble, then run without --bump" >&2; exit 2; }
else
  BR="$(git -C "$REPO" symbolic-ref --quiet --short HEAD 2>/dev/null || echo DETACHED)"
  case "$BR" in main|master) ;; *) echo "live-canary: run from a checkout on main (this one is on '$BR')" >&2; exit 1 ;; esac
fi

if [ "$BUMP" = 1 ]; then
  command -v crane >/dev/null || { echo "live-canary: --bump needs crane" >&2; exit 2; }
  MAIN="$(git -C "$REPO" rev-parse --short=7 origin/main)"
  # The namespace the goatnet pulls from: its group_vars' ghcr_org, else the
  # fleet-wide one (a templated value with a quoted default reads as that default).
  ORG="$(python3 "$REPO/ops/scripts/goatnet-facts.py" var "$ENV" ghcr_org 2>/dev/null || true)"
  [ -n "$ORG" ] || ORG="$(sed -nE "s/^ghcr_org:.*[\"' ]([A-Za-z0-9][A-Za-z0-9._-]*)[\"']?[ }\"]*\$/\\1/p" "$REPO/ops/ansible/group_vars/all.yml" 2>/dev/null | head -1 || true)"
  case "$ORG" in ""|*"{{"*) echo "live-canary: no plain ghcr_org for $ENV in its group_vars or group_vars/all.yml" >&2; exit 2 ;; esac
  DIG="$(crane digest "ghcr.io/$ORG/e2e-canary:sha-$MAIN" 2>&1)" || { echo "live-canary: no image for main@$MAIN yet — crane said: $DIG (e2e-canary-image.yml builds it on every push to main that touches ops/e2e/canary/**)" >&2; exit 1; }
  REF="ghcr.io/{{ ghcr_org }}/e2e-canary:sha-$MAIN@$DIG"
  WT="$REPO/../$(basename "$REPO")-wt-canary-$(date -u +%H%M%S)"
  git -C "$REPO" worktree add -q "$WT" -b "chore/e2e-canary-$(date -u +%Y%m%d-%H%M%S)" origin/main
  python3 - "$WT/ops/pins/fleet-pins.yaml" "$COMP" "$ENV" "$REF" <<'PY'
import re, sys
p, comp, g, ref = sys.argv[1:5]
s = open(p).read()
m = re.search(rf'(?m)^  {re.escape(comp)}:\n', s); start = m.start()
nxt = re.search(r'(?m)^  [A-Za-z0-9_-]+:\n', s[m.end():]); end = m.end() + (nxt.start() if nxt else len(s) - m.end())
block = s[start:end]
entry = (f'      {g}:\n        value: "{ref}"\n        reason: >-\n          The live canary (docs/design/deployment-e2e.md): the newest image CI built\n'
         f'          from main, deployed nightly by ops/e2e/live-canary.sh to prove the push\n          converger on a real goatnet. Never a user-facing service.\n')
block = re.sub(rf'(?ms)^      {re.escape(g)}:\n(?:        .*\n)+?(?=      [A-Za-z0-9_-]+:|\Z)', '', block)
block = re.sub(r'(?m)^    deviations:\n', '    deviations:\n' + entry, block, count=1)
open(p, "w").write(s[:start] + block + s[end:])
PY
  python3 "$WT/ops/pins/generate-pins.py" >/dev/null
  I="$WT/ops/consistency/intent/$ENV/intent.json"
  CA="$(python3 -c "import json; print(json.load(open('$I')).get('ca_id',''))")"; N="$(printf '%s' "$CA" | sed -e 's/^dev-//' -e 's/-ecdsa-[0-9-]*$//')"
  # The CA key and the audit log are where the checkout declares its state
  # directory (deploy.state_dir, else ~/.goat/state), as rc.sh reads them.
  S="$("$REPO/ops/scripts/project-setting.sh" --repo "$REPO" deploy state_dir 2>/dev/null || true)"; S="${S:-$HOME/.goat/state}"
  # shellcheck disable=SC2088  # matching the literal "~/" of a declared path
  case "$S" in "~/"*) S="$HOME/${S#\~/}" ;; esac
  K="$S/$N/ca.key.age"; [ -f "$K" ] || { echo "live-canary: CA key for $ENV not at $K" >&2; exit 1; }
  (cd "$WT/ops/enrollment" && go run ./cmd/intent-sign --intent "$I" --root-key "$K" --age-identity "$HOME/.config/age/keys.txt" --audit-log "$(dirname "$S")/ca-audit.jsonl" >/dev/null)
  python3 "$WT/ops/pins/generate-pins.py" --check >/dev/null
  python3 "$WT/ops/release/graduate.py" check >/dev/null
  B="$(mktemp)"; printf 'The live canary moves to the image CI built from main@%s (docs/design/deployment-e2e.md §Live canary). %s only.\n' "$MAIN" "$GOATNET" > "$B"
  "$REPO/ops/scripts/pr-land.sh" --worktree "$WT" --branch "$(git -C "$WT" rev-parse --abbrev-ref HEAD)" --title "chore(e2e): the live canary runs main@$MAIN on $GOATNET" --body-file "$B"
  git -C "$REPO" fetch -q origin main && GOAT_ALLOW_MASTER_GIT_MUTATE=1 git -C "$REPO" merge --ff-only origin/main >/dev/null
  git -C "$REPO" worktree remove "$WT" 2>/dev/null || true
fi

rec doctor -- "$GB" doctor --env "$ENV"
rec deploy-dry-run --expect-in "DRY: would read /var/lib/goat/$COMP/deploy-history.jsonl" -- "$GB" deploy --env "$ENV" --component "$COMP" --dry-run --yes
rec deploy-live --expect-in "every host's ledger names the declared ref" -- "$GB" deploy --env "$ENV" --component "$COMP" --ack "${USER:-operator} (live canary)" --yes
rec verify-only --expect-in "every host's ledger names the declared ref" -- "$GB" deploy --env "$ENV" --component "$COMP" --verify-only --yes
rec rollback-list --expect-in "$COMP" -- "$GB" rollback "$COMP" --list --site "$GOATNET"

# A real rollback, then the roll forward — the canary exists only for this.
# Both avenues (ADR 1201), alternating by day: even days goatbuilder, odd days
# rollback.sh; --avenue overrides. Needs two refs in the host's ledger, which the
# second --bump provides; with one, the step says so and the run stays green.
AVENUE="${LIVE_CANARY_AVENUE:-$( [ $(( $(date -u +%j | sed 's/^0*//') % 2 )) = 0 ] && echo cli || echo scripts )}"
LEDGER_ROW() { ssh "${SSH_OPTS[@]}" "$HOST" "sudo tail -n 1 /var/lib/goat/$COMP/deploy-history.jsonl"; }
REFS="$(ssh "${SSH_OPTS[@]}" "$HOST" "sudo grep -o '\"ref\": *\"[^\"]*\"' /var/lib/goat/$COMP/deploy-history.jsonl | sort -u | wc -l" 2>/dev/null || echo 0)"
if [ "${REFS:-0}" -ge 2 ]; then
  DECLARED="$(LEDGER_ROW | python3 -c 'import json,sys; print(json.loads(sys.stdin.read())["ref"])')"
  if [ "$AVENUE" = cli ]; then
    rec rollback-one-step-cli -- "$GB" rollback "$COMP" 1 --site "$GOATNET" --yes
  else
    rec rollback-one-step-scripts --expect-in "rolled back" -- "$REPO/ops/scripts/rollback.sh" "$COMP" 1 --site "$GOATNET"
  fi
  # The ssh options travel as arguments, so a ProxyCommand with spaces stays one word.
  # shellcheck disable=SC2016  # $1.. expand in the inner shell
  rec ledger-names-the-rollback -- bash -c 'h="$1" c="$2" d="$3"; shift 3
    row="$(ssh "$@" "$h" "sudo tail -n 1 /var/lib/goat/$c/deploy-history.jsonl")"; echo "$row"
    echo "$row" | grep -q "\"action\": *\"rollback\"" && ! echo "$row" | grep -q "\"ref\": *\"$d\""' _ "$HOST" "$COMP" "$DECLARED" "${SSH_OPTS[@]}"
  rec roll-forward --expect-in "every host's ledger names the declared ref" -- "$GB" deploy --env "$ENV" --component "$COMP" --ack "${USER:-operator} (live canary, roll forward)" --yes
  rec verify-after-roll-forward --expect-in "every host's ledger names the declared ref" -- "$GB" deploy --env "$ENV" --component "$COMP" --verify-only --yes
else
  rec rollback-needs-two-refs --note "the host's ledger holds ${REFS:-0} ref(s); the next --bump gives the rollback a target" -- true
fi
python3 "$HERE/report.py" "$RECORD" --check --title "live canary ($ENV)" || true
echo "record: $RECORD"
[ "$FAILS" = 0 ] && exit 0 || exit 1
