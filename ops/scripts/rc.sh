#!/usr/bin/env bash
# rc.sh — one command takes a release candidate from a tag on main through the
# dev goatnet's live validation to the preprod rung, the way fastlane.sh takes
# a fix to every goatnet (ADR 1173, docs/design/release-candidate-validation.md).
#
# It stops at the soak on purpose (ADR 1120: min one day on preprod). After the
# window, the same driver performs the prod fold:
#   rc.sh <component> --tag <the candidate tag> --from prod --ack "<name>"
#
# usage:
#   rc.sh <component> --version X.Y.Z [--rc N] --ack "<name>" [--dry-run]
#         [--from tag|dev|rows|preprod|preprod-rows|prod]   # resume a run at a step
#         (preprod-rows: preprod already runs the tag; redo only its deploy check,
#          live rows and evidence, without reopening the record or re-pinning)
#         [--tag <existing tag>]               # skip cutting: validate an existing tag
#
# Steps (each prints what it does; --dry-run prints every command and mutates nothing):
#   1 preflight   main fetched, this account holds admin on the repository
#                 (the rule rc-validate.yml enforces), next rc number
#   2 tag         <image>/vX.Y.Z-rc.N on origin/main, pushed; wait for rc-validate
#                 and the image build; resolve the digest under the image namespace
#   3 dev pin     declared deviation on the dev rung's goatnet, pins + intents
#                 regenerated and re-signed, gates run, PR landed (pr-land.sh)
#   4 dev deploy  deploy-component.sh --goatnet <dev>, ledger-verified
#   5 rows        the live acceptance rows the change affects
#                 (affected-flows.py --component), run by run-acceptance.py with
#                 the goatnet's runner settings (ops/validation/goatnet-runners.yaml);
#                 evidence written to ops/validation/evidence/
#   6 preprod     graduate.py open + evidence.validation, preprod deviation,
#                 pins/intents, gates, PR landed, deploy-component.sh --goatnet
#                 <preprod>, rows again, evidence.preprod_convergence, PR landed
#   7 soak        prints the prod command and the day it becomes runnable
#   8 prod        (--from prod, after the soak) graduate.py graduate --ack; the
#                 candidate becomes the fleet value and every rung's deviation
#                 for it is deleted; the prod hotfix record the fleet value
#                 carried is abandoned as superseded; pins/intents regenerated
#                 and re-signed, gates, PR landed; deploy-component.sh --goatnet
#                 <prod>, ledger-verified; dev and preprod ledgers re-verified so
#                 every rung reads one value
#
# Refusals (none is ceremony): not a nominator; tag commit not on main; image
# missing or split between orgs; rc-validate says "not a member" (the tag is
# gone); a live row FAILs (the candidate is dead: fix on main, cut rc.N+1).
#
# From an assembled tree (a checkout carrying the .goat-assembled marker that an
# overlay's assemble.sh writes): the goatnets' pins, intents and release records
# belong to the overlay repository the marker names (overlay_path), not to this
# tree's origin. rc.sh then runs every step after the tag (--from dev|rows|
# preprod|preprod-rows|prod, with --tag):
#   - its scratch tree is a second assembled tree, built by the overlay's own
#     assemble.sh from a fresh overlay worktree on origin/main, so every tool a
#     step calls (generate-pins.py, graduate.py, intent-sign) is the pinned one;
#   - a step's edits land as a pull request on the overlay repository, carrying
#     exactly the files the step changed in the scratch tree;
#   - after a landing the scratch tree is assembled again from the merged overlay
#     and the deploy runs from it, so fresh-source.sh sees a current overlay.
# The nominator check reads the overlay repository. Cutting the tag (--from tag)
# stays with a single-repository checkout: the tag goes where the component's
# image workflow builds it.
set -euo pipefail
# Hold the Mac awake for the whole run (#2122; ops/scripts/lib/keep-awake.sh).
_keep_awake="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/keep-awake.sh"
# shellcheck source=ops/scripts/lib/keep-awake.sh
if [ -f "$_keep_awake" ]; then . "$_keep_awake"; goat_keep_awake; fi
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
PINS="$REPO/ops/pins/fleet-pins.yaml"; LADDER="$REPO/ops/release/ladder.yaml"
RUNNERS="$REPO/ops/validation/goatnet-runners.yaml"
die() { echo "rc: $*" >&2; exit 1; }
# An assembled tree (see the header): ASSEMBLED=1 and OVERLAY = the overlay
# checkout its marker names. DEPLOY_HERE holds the deploy driver the run uses: this
# one, until a landing re-assembles the scratch tree.
ASSEMBLED=0; OVERLAY=""; DEPLOY_HERE="$HERE"
if [ -f "$REPO/.goat-assembled" ]; then
  ASSEMBLED=1
  OVERLAY="$(sed -n 's/^overlay_path=//p' "$REPO/.goat-assembled" | head -1)"
  [ "$(sed -n 's/^goatcore_ref=//p' "$REPO/.goat-assembled" | head -1)" != local ] \
    || die "$REPO was assembled from local directories (goatcore_ref=local), which no deploy accepts; assemble from the pinned refs"
  [ -n "$OVERLAY" ] && [ -x "$OVERLAY/assemble.sh" ] \
    || die "$REPO/.goat-assembled names no overlay checkout with an assemble.sh (overlay_path=${OVERLAY:-<none>}); re-assemble from the overlay"
fi
# One value this checkout declares about itself in ops/goatbuilder/goatbuilder.yaml
# (<section> <key>), as the builder reads it; empty when it declares none.
declared() { python3 -c 'import sys, yaml; print(str(((yaml.safe_load(open(sys.argv[1])) or {}).get(sys.argv[2]) or {}).get(sys.argv[3]) or "").strip())' "$REPO/ops/goatbuilder/goatbuilder.yaml" "$1" "$2" 2>/dev/null || true; }
# The image namespace this checkout deploys from: the ghcr_org default
# ops/ansible/group_vars/all.yml sets, the value every play renders a pin with.
IMAGE_ORG="$(sed -nE "s/^ghcr_org:.*[\"' ]([A-Za-z0-9][A-Za-z0-9._-]*)[\"']?[ }\"]*\$/\\1/p" "$REPO/ops/ansible/group_vars/all.yml" 2>/dev/null | head -1 || true)"
# Where this machine keeps the project's local state (its CA keys):
# deploy.state_dir, else ~/.goat/state, the builder's rule.
STATE_DIR="$(declared deploy state_dir)"; [ -n "$STATE_DIR" ] || STATE_DIR="$HOME/.goat/state"
# shellcheck disable=SC2088  # a declared path is written with a literal "~/"
case "$STATE_DIR" in "~/"*) STATE_DIR="$HOME/${STATE_DIR#\~/}" ;; esac
# The audit log intent-sign appends to: ca-audit.jsonl beside the state
# directory, named on every call (the builder derives the same path).
AUDIT_LOG="$(dirname "$STATE_DIR")/ca-audit.jsonl"
# Display-only: render {{ ghcr_org }} (the default org, as group_vars/all.yml) so
# a printed ref does not read like an unrendered/broken variable. The stored pin
# keeps the template; Ansible renders the real value per goatnet at deploy time.
render() { if [ -n "${GHCR_ORG:-$IMAGE_ORG}" ]; then printf '%s' "$1" | sed "s#{{ ghcr_org }}#${GHCR_ORG:-$IMAGE_ORG}#g"; else printf '%s' "$1"; fi; }
step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
note() { printf '    %s\n' "$*"; }
run() { if [ "$DRY" = 1 ]; then note "DRY: $*"; else "$@"; fi; }
run_with_dry_flag() {
  if [ "$DRY" = 1 ]; then run "$@" --dry-run; else run "$@"; fi
}

COMP=""; VERSION=""; RCN=""; ACK=""; DRY=0; FROM="tag"; TAG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --rc)      RCN="$2"; shift 2 ;;
    --ack)     ACK="$2"; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    --from)    FROM="$2"; shift 2 ;;
    --tag)     TAG="$2"; shift 2 ;;
    -h|--help) sed -n '2,34p' "$0"; exit 0 ;;
    -*) die "unknown flag $1" ;;
    *) [ -z "$COMP" ] && COMP="$1" || die "unexpected argument $1"; shift ;;
  esac
done
[ -n "$COMP" ] || die "usage: rc.sh <component> --version X.Y.Z --ack NAME"
[ -n "$ACK" ] || die "--ack '<name>' is required — a candidate reaches preprod and is attributed"
case "$ACK" in *'<'*'>'*|*'>'*'<'*) die "\`$ACK\` is still a placeholder: put your own name, for example --ack \"\$(git config user.name)\"" ;; esac
[ -n "$VERSION" ] || [ -n "$TAG" ] || die "--version X.Y.Z (or --tag) is required"
case "$FROM" in tag|dev|rows|preprod|preprod-rows|prod) ;; *) die "--from must be tag|dev|rows|preprod|preprod-rows|prod" ;; esac
case "$FROM" in prod|preprod-rows) [ -n "$TAG" ] || die "--from $FROM needs --tag <the candidate tag on preprod>" ;; esac
REPO_SLUG="$(git -C "$REPO" remote get-url origin | sed -E 's#.*[:/]([^/]+/[^/]+)\.git$#\1#')"

# --- environment preflight (fail fast, before any work, with a followable fix) ---
# Deps: this script parses fleet-pins.yaml with PyYAML immediately below, and the
# pin/intent render (generate-pins.py, intent.py) needs jsonschema. Check both now,
# up front — a missing module used to surface as a traceback mid-run or a stall
# after the tag and image were already built.
python3 -c 'import yaml, jsonschema' 2>/dev/null || die "python deps missing — this needs PyYAML + jsonschema. Install once:  pip install pyyaml jsonschema  (or pipx/venv), then re-run. Re-running the same command is safe (an existing tag is validated as-is)."
# Every binary a full run shells out to, here and in what it calls
# (deploy-component.sh -> ansible-preflight.sh -> ansible-playbook under aws-vault;
# pr-land.sh -> gh; generate-pins.py; intent-sign under go; crane for the digest).
# Named up front with the install, because "had to install a couple random
# things" mid-run is how the panoscope candidate lost its afternoon.
# ops/ci/check-deps-named.py keeps this list equal to what the sources invoke.
RC_DEPS="git gh go python3 ansible-playbook ansible ansible-inventory aws-vault nc ssh jq"
RC_MISSING=""
for _t in $RC_DEPS; do command -v "$_t" >/dev/null 2>&1 || RC_MISSING="$RC_MISSING $_t"; done
# the candidate digest: ops/scripts/image-digest uses whichever of crane, skopeo or docker is here (ADR 1204)
command -v crane >/dev/null 2>&1 || command -v skopeo >/dev/null 2>&1 || command -v docker >/dev/null 2>&1 || RC_MISSING="$RC_MISSING crane|skopeo|docker"
if [ -n "$RC_MISSING" ]; then
  # A dry run reports and continues (CI rehearses on runners without aws-vault);
  # a real run refuses before anything is cut.
  [ "$DRY" = 1 ] && note "deps: missing on PATH:$RC_MISSING — a real run refuses here (macOS: brew install gh go jq ansible aws-vault, and one of crane/skopeo/docker; nc and ssh ship with the OS)" \
    || die "missing on PATH:$RC_MISSING — install them first (macOS: brew install gh go jq ansible aws-vault, and one of crane/skopeo/docker; nc and ssh ship with the OS), then re-run."
fi
# Checkout: rc.sh authors the pin in a scratch worktree, MERGES it to origin/main,
# then DEPLOYS from this checkout ($REPO). Only a main/master checkout is kept
# current with the merged pin (land() fast-forwards it), and ansible-preflight
# enforces its staleness guard only there — a feature-branch checkout skips the
# guard AND need not hold the merged pin, so it can silently deploy a stale ref and
# verify green. Require main/master here, before anything is cut.
RBR0="$(git -C "$REPO" symbolic-ref --quiet --short HEAD 2>/dev/null || echo DETACHED)"
if [ "$ASSEMBLED" = 1 ]; then
  # An assembled tree is detached by construction. Its currency is the overlay
  # commit it was built from, which deploy-component.sh (fresh-source.sh) checks
  # against the overlay's origin/main before any host is touched.
  RBR0="assembled tree, overlay $(sed -n 's/^overlay_commit=//p' "$REPO/.goat-assembled" | cut -c1-12)"
  [ "$FROM" != tag ] || die "--from tag cuts the tag on this tree's origin ($REPO_SLUG); from an assembled tree, cut the tag where the component's image workflow builds it, then re-run here with --tag <that tag> --from dev"
  [ -n "$TAG" ] || die "from an assembled tree, name the candidate with --tag <component>/vX.Y.Z-rc.N"
  OVERLAY_SLUG="$(git -C "$OVERLAY" remote get-url origin | sed -E 's#.*[:/]([^/]+/[^/]+)\.git$#\1#; s#.*[:/]([^/]+/[^/]+)$#\1#')"
else
case "$RBR0" in
  main|master) ;;
  *) die "run rc.sh from a checkout on main — this one ($REPO) is on '$RBR0'. rc.sh deploys from the invoking checkout and only a main checkout is kept current with the pin it merges; a feature-branch run can deploy a stale pin and verify green. Switch to a current main checkout (git checkout main && git pull --ff-only origin main) and re-run." ;;
esac
fi

# --- facts ----------------------------------------------------------------------
step "component $COMP"
J="$(python3 - "$PINS" "$LADDER" "$COMP" <<'PY'
import sys, yaml, json, os
pins = yaml.safe_load(open(sys.argv[1])); lad = yaml.safe_load(open(sys.argv[2]))
comp = (pins.get("components") or {}).get(sys.argv[3]) or sys.exit(f"unknown component {sys.argv[3]!r}")
wf = comp.get("image_workflow") or sys.exit(f"{sys.argv[3]}: no image_workflow in fleet-pins")
image = os.path.basename(wf).replace("-image.yml", "")
rungs = {r["name"]: r for r in lad["rungs"] if r.get("status") == "live"}
for need in ("dev", "preprod", "prod"):
    if need not in rungs or not rungs[need].get("goatnet"): sys.exit(f"ladder: rung {need!r} is not live with a goatnet")
print(json.dumps({"image": image, "var": comp["var"], "ledger": comp.get("ledger_component") or sys.argv[3],
                  "fleet": comp.get("fleet"), "dev": rungs["dev"]["goatnet"], "preprod": rungs["preprod"]["goatnet"], "prod": rungs["prod"]["goatnet"]}))
PY
)" || die "$J"
IMAGE="$(printf '%s' "$J" | python3 -c 'import json,sys; print(json.load(sys.stdin)["image"])')"
DEV="$(printf '%s' "$J" | python3 -c 'import json,sys; print(json.load(sys.stdin)["dev"])')"
PRE="$(printf '%s' "$J" | python3 -c 'import json,sys; print(json.load(sys.stdin)["preprod"])')"
PROD="$(printf '%s' "$J" | python3 -c 'import json,sys; print(json.load(sys.stdin)["prod"])')"
FLEET="$(printf '%s' "$J" | python3 -c 'import json,sys; print(json.load(sys.stdin)["fleet"] or "")')"
note "image $IMAGE; rungs dev=$DEV preprod=$PRE prod=$PROD; fleet ref $(render "${FLEET:-<none>}")"
git -C "$REPO" fetch -q origin main
MAIN="$(git -C "$REPO" rev-parse --short origin/main)"

# --- 1 preflight ------------------------------------------------------------------
step "preflight"
note "deploying from $REPO on $RBR0; deps ok (PyYAML, jsonschema)"
# GOAT_RC_ROLE_CHECK=0 skips the nominator check where there is no person (the
# goatbuilder parity gate in CI); `goatbuilder release` honours the same variable.
if [ "${GOAT_RC_ROLE_CHECK:-1}" != 0 ] && [ "$ASSEMBLED" = 1 ]; then
  # From an assembled tree nothing is tagged here; what this run changes is the
  # overlay repository's pins and records, so its admins are the nominators.
  ME="$(gh api user -q .login)"
  ROLE="$(gh api "repos/$OVERLAY_SLUG/collaborators/$ME/permission" -q .role_name 2>/dev/null || echo unknown)"
  case "$ROLE" in admin) note "$ME holds $ROLE on $OVERLAY_SLUG (the overlay this tree was assembled from)" ;; *) die "$ME holds '$ROLE' on $OVERLAY_SLUG; only an admin of the overlay repository may put a release candidate on its goatnets" ;; esac
elif [ "${GOAT_RC_ROLE_CHECK:-1}" != 0 ]; then
  ME="$(gh api user -q .login)"
  ROLE="$(gh api "repos/$REPO_SLUG/collaborators/$ME/permission" -q .role_name 2>/dev/null || echo unknown)"
  case "$ROLE" in admin) note "$ME holds $ROLE on $REPO_SLUG" ;; *) die "$ME holds '$ROLE' on $REPO_SLUG; only a repository admin may nominate a release candidate (the rule rc-validate.yml enforces)" ;; esac
  # The repository this checkout's origin names must run rc-validate.yml, or the
  # tag is pushed and the wait below dies on a 404 with a tag already cut
  # (2026-11-03: a clone of the retired mirror tagged its own main). GB-077.
  gh api "repos/$REPO_SLUG/actions/workflows/rc-validate.yml" -q .state >/dev/null 2>&1 \
    || die "GB-077: $REPO_SLUG does not run rc-validate.yml, so it is not where release candidates are cut; run from a checkout whose origin is the primary repository (git remote get-url origin)"
fi
if [ -z "$TAG" ]; then
  if [ -z "$RCN" ]; then
    LAST="$(git -C "$REPO" ls-remote --tags --refs origin "refs/tags/$IMAGE/v$VERSION-rc.*" | sed -E 's/.*-rc\.([0-9]+)$/\1/' | sort -n | tail -1)"
    RCN=$(( ${LAST:-0} + 1 ))
  fi
  TAG="$IMAGE/v$VERSION-rc.$RCN"
fi
note "candidate tag $TAG on main $MAIN"
# What the candidate carries: the commits since this component's previous tag
# that touch the paths its image workflow builds from. The tag names the
# component and version only; this list goes into the pin PR body and the
# candidate record, so a reader sees what a candidate is without the git log.
CHANGES=""; CHANGES_LINE=""
rc_changes() { # sets CHANGES (one "- subject" line each) and CHANGES_LINE, once
  [ -n "$CHANGES" ] && return 0
  local wf paths head prev n prs
  wf="$(python3 -c "import yaml,sys; print(yaml.safe_load(open(sys.argv[1]))['components'][sys.argv[2]].get('image_workflow') or '')" "$PINS" "$COMP")"
  paths="$(python3 - "$REPO/$wf" <<'PY'
import sys, yaml
w = yaml.safe_load(open(sys.argv[1]))
on = w.get("on") or w.get(True) or {}
paths = (on.get("push") or {}).get("paths") or []
print(" ".join(p.rstrip("*").rstrip("/") for p in paths))
PY
)" || paths=""
  head="$(git -C "$REPO" rev-parse -q --verify "refs/tags/$TAG^{commit}" 2>/dev/null || echo "$MAIN")"
  prev="$(git -C "$REPO" tag -l "$IMAGE/v*" --sort=-v:refname --merged "$head" | grep -vx "$TAG" | head -1 || true)"
  if [ -z "$prev" ]; then
    CHANGES="- the first release candidate tagged for $IMAGE"
    CHANGES_LINE="the first candidate for $IMAGE"
    return 0
  fi
  # shellcheck disable=SC2086 # paths is a deliberate word list
  CHANGES="$(git -C "$REPO" log --no-merges --format='- %s' "$prev..$head" -- $paths)"
  [ -n "$CHANGES" ] || CHANGES="- no change to $IMAGE's source since $prev"
  n="$(printf '%s\n' "$CHANGES" | grep -c '(#[0-9]*)$' || true)"
  prs="$(printf '%s\n' "$CHANGES" | grep -oE '\(#[0-9]+\)$' | tr -d '()' | paste -sd, - | sed 's/,/, /g' || true)"
  CHANGES_LINE="$n change(s) since $prev${prs:+: $prs}"
}
# Secrets, before the tag: every rung this run will deploy to templates the
# component's secrets from this environment. Refuse here, naming every variable,
# rather than tag and pin first and refuse at the deploy (the
# panoscope run did exactly that; the dry run had not looked). A dry run prints
# the same report and continues, so the operator sees it before the real run.
secrets_preflight() { # $1 goatnet key -> 0 ok, 1 missing (printed)
  local g="$1" play hosts_play members report rc missing owner account
  members="$(python3 - "$PINS" "$COMP" "$g" <<'PY'
import sys, yaml
pins = yaml.safe_load(open(sys.argv[1])); comp = pins["components"][sys.argv[2]]
g = (pins.get("goatnets") or {}).get(sys.argv[3]) or {}
devs = comp.get("deviations") or {}
absent = {k for k, v in devs.items() if isinstance(v, dict) and v.get("absent")}
print(" ".join(m for m in (g.get("members") or []) if m not in absent))
PY
)"
  [ -n "$members" ] || return 0
  play="$REPO/ops/ansible/$(python3 -c "import yaml,sys; c=yaml.safe_load(open('$PINS'))['components']['$COMP']; print(c.get('deploy_playbook') or '')")"
  [ -f "$play" ] || play="$REPO/ops/ansible/$LEDGER-deploy.yml"; [ -f "$play" ] || play="$REPO/ops/ansible/$COMP-deploy.yml"
  [ -f "$play" ] || return 0
  hosts_play="$play"
  for _hop in 1 2 3; do
    grep -qE '^[[:space:]]*hosts:' "$hosts_play" && break
    _imp="$(grep -oE 'import_playbook:[[:space:]]*[^[:space:]]+' "$hosts_play" | head -1 | awk '{print $2}')"; [ -n "$_imp" ] || break
    hosts_play="$(dirname "$hosts_play")/$_imp"; [ -f "$hosts_play" ] || break
  done
  report="$(python3 "$HERE/deploy-secrets-needed.py" --repo "$REPO" --component "$COMP" --ledger "$LEDGER" --play "$hosts_play" $members 2>&1)" && rc=0 || rc=$?
  if [ "$rc" = 1 ]; then
    missing="$(printf '%s\n' "$report" | awk '$3=="MISSING"{print $2}' | tr '\n' ' ')"
    note "secrets: $COMP on $g templates $(printf '%s\n' "$report" | grep -c .) variable(s) from your environment; NOT SET: $missing"
    # Named by this checkout, as the builder's release names them: `repo: source:`
    # and `secrets: op_account:` in ops/goatbuilder/goatbuilder.yaml.
    owner="$(declared repo source)"; owner="${owner%%/*}"; account="$(declared secrets op_account)"
    note "         references (pointers, no values) live in ${owner:-<owner>}/goat-credentials refs/goat.refs; put the lines for those names in .envrc.local.d/<file>.refs, then"
    note "         OP_ACCOUNT=${account:-<your account>} op run --env-file=.envrc.local.d/<file>.refs -- <this same command>   (under direnv: direnv exec . env OP_ACCOUNT=… op run …)"
    return 1
  fi
  [ "$rc" = 0 ] && note "secrets: $COMP on $g — every variable the play templates is set" || note "secrets: could not derive what $COMP needs on $g ($report)"
  return 0
}
LEDGER="$(printf '%s' "$J" | python3 -c 'import json,sys; print(json.load(sys.stdin)["ledger"])')"
SECRETS_MISSING=0
case "$FROM" in prod) _RUNGS="$PROD" ;; preprod|preprod-rows) _RUNGS="$PRE" ;; *) _RUNGS="$DEV $PRE" ;; esac
for _g in $_RUNGS; do
  secrets_preflight "$_g" || SECRETS_MISSING=1
done
if [ "$SECRETS_MISSING" = 1 ]; then
  [ "$DRY" = 1 ] && note "secrets: a real run refuses here, before the tag and the pin" || die "refusing before the tag: $COMP needs the variables named above in the environment"
fi

# --- 2 tag + wait -----------------------------------------------------------------
if [ "$FROM" = tag ]; then
  step "tag $TAG"
  if git -C "$REPO" ls-remote --tags origin "refs/tags/$TAG" | grep -q .; then
    note "tag exists on origin — validating it as-is"
  else
    run git -C "$REPO" tag -a "$TAG" origin/main -m "$IMAGE release candidate $VERSION-rc.$RCN (ADR 1173), nominated by $ACK"
    run git -C "$REPO" push -q origin "$TAG"
  fi
  if [ "$DRY" = 0 ]; then
    note "waiting for rc-validate and the image build on $TAG"
    for wf in rc-validate.yml "$IMAGE-image.yml"; do
      R=""; for _i in $(seq 1 20); do R="$(gh run list -R "$REPO_SLUG" --workflow "$wf" -L 5 --json databaseId,headBranch -q ".[] | select(.headBranch==\"$TAG\") | .databaseId" | head -1)"; [ -n "$R" ] && break; sleep 10; done
      [ -n "$R" ] || die "no $wf run appeared for $TAG"
      gh run watch "$R" -R "$REPO_SLUG" --exit-status >/dev/null 2>&1 || true
      C="$(gh run view "$R" -R "$REPO_SLUG" --json conclusion -q .conclusion)"
      note "$wf: $C"
      if [ "$wf" = rc-validate.yml ] && [ "$C" != success ]; then
        if git -C "$REPO" ls-remote --tags origin "refs/tags/$TAG" | grep -q .; then
          note "rc-validate could not read the pusher's role — the tag was kept; the cheap suites did not run on the tag. Continuing on the live rows."
        else
          die "rc-validate deleted $TAG: the pusher does not hold maintain/admin, or the commit is not on main"
        fi
      fi
      [ "$wf" = "$IMAGE-image.yml" ] && [ "$C" != success ] && die "image build failed for $TAG"
    done
  fi
fi
DIG=""
TMPDIR_RC_DIGEST_ERR="$(mktemp)"
if [ "$DRY" = 0 ]; then
  # Any of crane, skopeo or docker resolves it (ADR 1204); a tool's own error is kept,
  # so a missing tool or a refused login never reads as "image not found".
  [ -n "$IMAGE_ORG" ] || die "no image namespace: ops/ansible/group_vars/all.yml sets no ghcr_org default, so there is nowhere to resolve $IMAGE:${TAG#*/}"
  D1="$("$HERE/image-digest" "ghcr.io/$IMAGE_ORG/$IMAGE:${TAG#*/}" 2>"$TMPDIR_RC_DIGEST_ERR")" || D1=""
  [ -n "$D1" ] || die "no digest for ghcr.io/$IMAGE_ORG/$IMAGE:${TAG#*/}: $(cat "$TMPDIR_RC_DIGEST_ERR" 2>/dev/null)"
  DIG="$D1"
else
  DIG="sha256:<digest of $IMAGE:${TAG#*/}>"
fi
REF="ghcr.io/{{ ghcr_org }}/$IMAGE:${TAG#*/}@$DIG"
note "candidate ref $(render "$REF")"

# --- helpers for pin edits + landing -----------------------------------------------
WT="$REPO/../$(basename "$REPO")-wt-rc-$(date -u +%H%M%S)"
mkwt() {
  [ -d "$WT" ] && return 0
  if [ "$ASSEMBLED" = 1 ]; then
    run git -C "$OVERLAY" fetch -q origin main
    run git -C "$OVERLAY" worktree add -q "$OWT" -b "chore/rc-$IMAGE-$(date -u +%Y%m%d-%H%M%S)" origin/main
    if [ "$DRY" = 1 ]; then note "DRY: $OWT/assemble.sh $WT  (the scratch tree: goatcore + goatstuff + the overlay worktree)"; return 0; fi
    assemble_wt
  else
    run git -C "$REPO" worktree add -q "$WT" -b "chore/rc-$IMAGE-$(date -u +%Y%m%d-%H%M%S)" origin/main
  fi
}
# --- assembled tree only ---------------------------------------------------------
# $OWT: the overlay worktree a step's edits land from. $WT: the scratch tree the
# overlay's assemble.sh builds from $OWT; its HEAD stays the goatcore commit
# (source_of_record.py requires that), so its starting content is kept as a git
# tree object ($WT_SNAP) and a step's edits are the diff against it. Once built,
# the run reads its facts and deploys from $WT, the newest assembly it has.
OWT="$WT-overlay"; WT_SNAP=""; EVID_ROOT="$REPO"
assemble_wt() {
  local log; log="$(mktemp)"
  rm -rf "$WT"
  "$OWT/assemble.sh" "$WT" >"$log" 2>&1 || { cat "$log" >&2; die "$OWT/assemble.sh could not build the scratch tree $WT"; }
  git -C "$WT" add -A --force
  WT_SNAP="$(git -C "$WT" write-tree)"
  git -C "$WT" reset -q
  REPO="$WT"; PINS="$WT/ops/pins/fleet-pins.yaml"; LADDER="$WT/ops/release/ladder.yaml"
  RUNNERS="$WT/ops/validation/goatnet-runners.yaml"; DEPLOY_HERE="$WT/ops/scripts"
  note "scratch tree $WT assembled from overlay $(git -C "$OWT" rev-parse --short HEAD)"
}
land_overlay() { # $1 title, $2 body: the step's edits in $WT, as a PR on the overlay
  local changes st p B
  git -C "$WT" add -A --force
  changes="$(git -C "$WT" diff-index --cached --name-status --no-renames "$WT_SNAP")"
  git -C "$WT" reset -q
  if [ -z "$(printf '%s\n' "$changes" | awk 'NF{print $2}' | grep -v '\.sig$' || true)" ]; then
    note "pins already current on the overlay's main; nothing to land"
    return 0
  fi
  while IFS="$(printf '\t')" read -r st p; do
    [ -n "$p" ] || continue
    case "$st" in
      D) git -C "$OWT" rm -q --ignore-unmatch -- "$p" ;;
      *) mkdir -p "$OWT/$(dirname "$p")"; cp -p "$WT/$p" "$OWT/$p" ;;
    esac
    note "lands $st $p"
  done <<< "$changes"
  B="$(mktemp)"; printf '%s\n' "$2" > "$B"
  "$HERE/pr-land.sh" --worktree "$OWT" --branch "$(git -C "$OWT" rev-parse --abbrev-ref HEAD)" --title "$1" --body-file "$B"
  # The overlay's main now carries the edit: build the next scratch tree from it,
  # so the deploy that follows reads a current overlay.
  git -C "$OWT" fetch -q origin main
  git -C "$OWT" checkout -q -B "chore/rc-$IMAGE-$(date -u +%Y%m%d-%H%M%S)" origin/main
  assemble_wt
}
set_deviation() {  # $1 goatnet key, $2 reason
  run python3 - "$WT/ops/pins/fleet-pins.yaml" "$COMP" "$1" "$REF" "$2" <<'PY'
import sys, yaml, re
p, comp, g, ref, reason = sys.argv[1:6]
s = open(p).read()
d = yaml.safe_load(s); assert comp in d["components"]
# find the component block boundaries in the raw text (preserve comments)
m = re.search(rf'(?m)^  {re.escape(comp)}:\n', s); start = m.start()
nxt = re.search(r'(?m)^  [A-Za-z0-9_-]+:\n', s[m.end():]); end = m.end() + (nxt.start() if nxt else len(s) - m.end())
block = s[start:end]
entry = f'      {g}:\n        value: "{ref}"\n        reason: >-\n          ' + "\n          ".join(reason.split("\n")) + "\n"
# an existing value deviation for this goatnet is replaced WHERE IT STANDS:
# re-declaring the same value must leave the table byte-identical, so a re-run of
# a landed step has nothing to land (moving it to the top reordered the map and
# opened a PR on every re-run; found by the hermetic e2e tier)
old = re.compile(rf'(?ms)^      {re.escape(g)}:\n        value:.*?(?=^      [A-Za-z0-9_-]+:|\Z)')
if old.search(block):
    block = old.sub(lambda _m: entry, block, count=1)
elif re.search(r'(?m)^    deviations:\n', block):
    block = re.sub(r'(?m)^    deviations:\n', '    deviations:\n' + entry, block, count=1)
else:
    block = re.sub(r'(?m)^(    fleet: .*\n)', r'\1    deviations:\n' + entry.replace('\\', '\\\\'), block, count=1)
open(p, "w").write(s[:start] + block + s[end:])
PY
}
reconcile_superseded() { # $1 rung's goatnet — before the check: abandon every open ladder
  # candidate check C6 would refuse (overtaken on preprod, not on dev either),
  # naming the open hotfix record that overtook it, else this candidate's ref.
  # Without this a component carrying such a record could not start a new
  # candidate at dev at all (onboard-runner).
  local RECS_BASE="$WT"; [ -d "$RECS_BASE/ops/release" ] || RECS_BASE="$REPO"
  while IFS=$'\t' read -r OLD BY; do
    [ -n "$OLD" ] || continue
    [ -n "$BY" ] || BY="$REF"
    run python3 "$WT/ops/release/graduate.py" abandon "$OLD" \
      --reason "Superseded on $1: the rung no longer runs this candidate (check C6); overtaken by $BY before release candidate $TAG (ADR 1173), nominated by $ACK" \
      --superseded-by "$BY"
  done < <(python3 - "$RECS_BASE" "$COMP" <<'PY'
import pathlib, sys, yaml
base, comp = sys.argv[1:3]
lad = yaml.safe_load(open(pathlib.Path(base, "ops/release/ladder.yaml")))
live = {r["name"]: r["goatnet"] for r in lad.get("rungs", []) if r.get("status") == "live"}
pins = yaml.safe_load(open(pathlib.Path(base, "ops/pins/fleet-pins.yaml")))
cd = (pins.get("components") or {}).get(comp) or {}
def eff(g):
    dev = (cd.get("deviations") or {}).get(g)
    if dev:
        if dev.get("absent"): return ("absent", None)
        if "value" in dev: return ("value", dev["value"])
    return ("unresolved", None) if cd.get("fleet") is None else ("value", cd.get("fleet"))
recs = []
for p in sorted(pathlib.Path(base, "ops/release/records", comp).glob("*.yaml")):
    try: r = yaml.safe_load(p.read_text()) or {}
    except Exception: continue
    recs.append((p, r))
hot = [p for p, r in recs if r.get("class") == "hotfix" and r.get("state") == "candidate"]
by = hot[-1].name if hot else ""
pre = live.get("preprod"); dev = live.get("dev")
for p, r in recs:
    if r.get("state") != "candidate" or r.get("class") == "hotfix": continue
    st, val = eff(pre)
    if st != "value" or val == r.get("candidate_ref"): continue
    if dev:
        dst, dval = eff(dev)
        if dst == "value" and dval == r.get("candidate_ref"): continue
    print(f"{p.relative_to(base)}\t{by}")
PY
)
}
regen_sign_check() {
  # Not `run … >/dev/null`: that redirect swallowed the DRY line too, so a dry
  # run never showed the regeneration step (the goatbuilder parity gate noticed).
  # Re-sign ONLY an intent whose body the regeneration changed: an ECDSA
  # signature is fresh every time, so re-signing an unchanged body is a diff
  # that lands a PR of nothing but .sig files (five such PRs).
  local BEFORE; BEFORE="$(mktemp -d)"
  for d in "$WT/ops/consistency/intent"/*/; do [ -f "$d/intent.json" ] && cp "$d/intent.json" "$BEFORE/$(basename "$d").json"; done
  if [ "$DRY" = 1 ]; then note "DRY: python3 $WT/ops/pins/generate-pins.py"; else python3 "$WT/ops/pins/generate-pins.py" >/dev/null; fi
  for d in "$REPO/ops/consistency/intent"/*/; do
    [ -d "$d" ] || continue; g="$(basename "$d")"
    I="$WT/ops/consistency/intent/$g/intent.json"; [ -f "$I" ] || [ "$DRY" = 1 ] || continue
    if [ "$DRY" = 0 ] && [ -f "$BEFORE/$g.json" ] && cmp -s "$BEFORE/$g.json" "$I"; then continue; fi
    SRC="$I"; [ -f "$SRC" ] || SRC="$REPO/ops/consistency/intent/$g/intent.json"
    CA="$(python3 -c "import json; print(json.load(open('$SRC')).get('ca_id',''))")"; N="$(printf '%s' "$CA" | sed -e 's/^dev-//' -e 's/-ecdsa-[0-9-]*$//')"
    K="$STATE_DIR/$N/ca.key.age"
    # A dry run signs nothing: say what a real run needs and go on, so the
    # rehearsal of a step runs anywhere (ops/e2e/rehearsal.sh).
    if [ ! -f "$K" ]; then
      [ "$DRY" = 1 ] && { note "DRY: CA key for $g not at $K — a real run signs $g's intent with it and refuses without it"; continue; }
      die "CA key for $g not at $K"
    fi
    run bash -c "cd '$WT/ops/enrollment' && go run ./cmd/intent-sign --intent '$I' --root-key '$K' --age-identity '$HOME/.config/age/keys.txt' --audit-log '$AUDIT_LOG' >/dev/null"
  done
  run python3 "$WT/ops/release/graduate.py" check
  run python3 "$WT/ops/pins/generate-pins.py" --check
}
land() { # $1 title, $2 body
  if [ "$ASSEMBLED" = 1 ]; then
    if [ "$DRY" = 1 ]; then note "DRY: the step's edits in $WT land as a PR on $OVERLAY_SLUG via $HERE/pr-land.sh --worktree $OWT: $1"
    else land_overlay "$1" "$2"; fi
    return 0
  fi
  if [ "$DRY" = 0 ]; then
    # A re-run whose pin is already merged regenerates the same bytes: the tree
    # differs from main in nothing (signatures are no longer re-signed) — there
    # is nothing to land, and no PR of .sig files to open.
    local CHANGED; CHANGED="$( { git -C "$WT" diff --name-only origin/main; git -C "$WT" ls-files --others --exclude-standard; } | grep -v '\.sig$' || true)"
    if [ -z "$CHANGED" ]; then
      git -C "$WT" checkout -q -- . 2>/dev/null || true
      note "pins already current on main; nothing to land"
      return 0
    fi
  fi
  local B; B="$(mktemp)"; printf '%s\n' "$2" > "$B"
  run_with_dry_flag "$HERE/pr-land.sh" --worktree "$WT" --branch "$(git -C "$WT" rev-parse --abbrev-ref HEAD 2>/dev/null || echo rc)" --title "$1" --body-file "$B"
  # if/fi, not `[ ] &&`: as the last statement a false test would be this
  # function's exit status, and set -e ended every --dry-run here, silently.
  if [ "$DRY" = 0 ]; then
    # The pin PR just merged advanced the trunk. deploy-component.sh (next step)
    # runs Ansible from THIS checkout ($REPO); ansible-preflight refuses a stale
    # tree, so fast-forward $REPO to the merge before the deploy — or it is
    # refused "local main is N behind origin/main" (the pitfall that stalled the
    # first microscope rc, re-staling it on every re-run). The top-of-script
    # guard guarantees $REPO is on main/master; ff-only, with the ADR 0013
    # override the destructive-git hook requires for the master tree.
    RBR="$(git -C "$REPO" symbolic-ref --quiet --short HEAD 2>/dev/null || echo main)"
    git -C "$REPO" fetch -q origin "$RBR"
    GOAT_ALLOW_MASTER_GIT_MUTATE=1 git -C "$REPO" merge --ff-only "origin/$RBR" >/dev/null \
      || die "could not fast-forward $REPO to origin/$RBR after landing the PR — $RBR has diverged; resolve it, then re-run the same command (re-running is safe — the pin PR already landed and rc.sh resumes)"
    git -C "$WT" checkout -q -B "chore/rc-$IMAGE-$(date -u +%Y%m%d-%H%M%S)" origin/main
  fi
}
runner_var() {
  python3 - "$RUNNERS" "$1" "$2" <<'PY'
import sys, yaml
with open(sys.argv[1]) as stream:
    runners = yaml.safe_load(stream)
print((runners['goatnets'].get(sys.argv[2]) or {}).get(sys.argv[3], ''))
PY
}
# EVID_ROOT is the invoking tree: a re-assembled scratch tree is replaced on the
# next landing, and the record names this path.
evidence_path() { printf '%s' "$EVID_ROOT/ops/validation/evidence/$IMAGE-${TAG#*/}-$1.jsonl"; }
browser_toolchain_check() { # $1 rows (comma list) -> notes; returns 1 when a browser row cannot run here
  local e2e="$REPO/ops/goat-id-console/web/e2e"
  case ",$1," in *,intake-form-browser-submit,*|*,review-gate-accepts-open-field-invite,*) ;; *) return 0 ;; esac
  local missing=""
  command -v node >/dev/null 2>&1 || missing="$missing node"
  command -v npm >/dev/null 2>&1 || missing="$missing npm"
  [ -d "$e2e/node_modules" ] || missing="$missing $e2e/node_modules"
  if [ -n "$missing" ]; then
    note "browser row(s) in [$1] need the console's Playwright toolchain here; missing:$missing"
    note "  fix: (cd $e2e && npm ci && npx playwright install chromium)   — then re-run the same command"
    return 1
  fi
  note "browser row(s) in [$1]: node, npm and $e2e/node_modules present"
  return 0
}
rows_on() { # $1 goatnet key -> writes evidence file, returns runner exit
  local g="$1"; local name; name="$(runner_var "$g" name)"; [ -n "$name" ] || die "no runner settings for $g in $RUNNERS"
  # The evidence path before anything can return: an early return wrote to an
  # unset path and died at `: No such file` when no rows were mapped.
  EVID="$(evidence_path "$name")"; mkdir -p "$(dirname "$EVID")"
  # The affected-rows computation must SUCCEED before "no rows" can mean "no
  # rows": a base it cannot resolve used to print nothing, read as an empty set,
  # and the candidate went through both rungs without a single live row. A dry
  # run before tagging has no tag to diff against, so it records no evidence
  # path there (goatbuilder's release --dry-run prints the same; parity gate).
  local flows_json
  if ! flows_json="$(python3 "$REPO/ops/validation/affected-flows.py" --component "$COMP" --head "$TAG" --json 2>/dev/null)"; then
    [ "$DRY" = 1 ] && { EVID=""; return 0; }
    python3 "$REPO/ops/validation/affected-flows.py" --component "$COMP" --head "$TAG" --json >/dev/null || true
    die "could not compute the live rows $TAG affects (affected-flows.py failed above) — fix that, then re-run the same command"
  fi
  local rows; rows="$(printf '%s' "$flows_json" | python3 -c 'import json,sys; print(",".join(json.load(sys.stdin)["live_rows"]))')"
  if [ -z "$rows" ]; then note "no live rows are mapped for what $TAG changed (ops/validation/flow-map.yaml) — evidence is the cheap suites only"; echo "" > "$EVID"; return 0; fi
  # A row that cannot run here is UNTESTED, not FAILED: say what is missing and
  # stop before the runner, rather than let an exit 2 read as a dead candidate.
  if ! browser_toolchain_check "$rows"; then
    [ "$DRY" = 1 ] || die "the live rows on $name cannot run from this machine (see above) — the candidate is UNTESTED here, not dead; install the toolchain and re-run the same command"
  fi
  local req; req="$(python3 - "$REPO/ops/validation/acceptance-tests.yaml" "$rows" <<'PY'
import yaml, sys
rows = set(sys.argv[2].split(",")); d = yaml.safe_load(open(sys.argv[1]))
print(",".join(sorted({r for t in d["tests"] if t["id"] in rows for r in t.get("requires", [])})))
PY
)"
  note "rows on $name: $rows (requires $req)"
  # The contexts the rows run as come from the values file (goat_sponsor_context,
  # probe_context): non-person identities, never an operator key (ADR 1207).
  local via res jump=""; via="$(runner_var "$g" goat_via)"; res="$(runner_var "$g" intake_resolve)"
  # A rung this laptop is not meshed to runs its rows on its own management
  # host through the door: goat_via `door:<env>` resolves to that host's mesh
  # address, and every ssh the rows make (goat rows and host rows) jumps through
  # the door to reach it (docs/design/env-doors.md; ops/validation/probes.py
  # _ssh_jump). The door address itself is never the ssh destination: it is a
  # tunnel port whose host key churns.
  case "$via" in door:*)
    local door="${via#door:}"
    if [ "$DRY" = 1 ]; then note "DRY: $HERE/door-open.sh $door  (the rows run goat on $name's management host through the door)"
    else "$HERE/door-open.sh" "$door" >/dev/null 2>&1 || die "door to $door did not open for the rows on $name"; fi
    jump="$("$HERE/env-door.sh" "$door" --ssh-target 2>/dev/null || true)"
    via="$("$HERE/env-door.sh" "$door" --host-target 2>/dev/null || true)"
    { [ -n "$jump" ] && [ -n "$via" ]; } || [ "$DRY" = 1 ] || die "env-door.sh $door printed no door (--ssh-target) or host (--host-target)" ;;
  esac
  local rc=0
  run env GOAT_ROWS_SSH_JUMP="$jump" INTAKE_RESOLVE="$res" python3 "$REPO/ops/validation/run-acceptance.py" --env "$name" --values "$REPO/$(runner_var "$g" values)" \
      --deployed "$req" --only "$rows" --var "goat_via=$via" --var "env=$name" --json "$EVID" --require-pass || rc=$?
  # exit 2 from the runner is an environment problem (a tool missing, a context
  # not configured) — the rows did not run. That is UNTESTED, never FAIL.
  if [ "$rc" = 2 ]; then die "the live rows on $name did not run (exit 2: a tool or context is missing on this machine — read the output above) — the candidate is UNTESTED here, not dead; fix the environment and re-run the same command"; fi
  return $rc
}

# --- 3 dev pin ---------------------------------------------------------------------
if [ "$FROM" = tag ] || [ "$FROM" = dev ]; then
  step "dev rung: $DEV runs $TAG"
  mkwt
  set_deviation "$DEV" "RC $TAG ($(date -u +%Y-%m-%d), ADR 1173), nominated by $ACK: the dev-rung candidate. Validated by the live acceptance rows the change affects; their PASS is the dev->preprod validation evidence."
  reconcile_superseded "$DEV"   # after the pin: a record dev ran until now goes stale exactly here
  regen_sign_check
  rc_changes
  land "chore(pins): $DEV runs release candidate $TAG (ADR 1173)" "Release candidate $TAG on the dev rung ($DEV) as a declared deviation. Nominated by $ACK. Image built by CI from the tag; digest $DIG identical under both orgs. Validation follows on the goatnet.

What $TAG carries ($CHANGES_LINE):
$CHANGES"
  # --- 4 dev deploy
  step "deploy $TAG to $DEV"
  run_with_dry_flag "$DEPLOY_HERE/deploy-component.sh" "$COMP" --goatnet "$DEV"
fi

# --- 5 rows on dev -------------------------------------------------------------------
EVID=""
if [ "$FROM" != preprod ] && [ "$FROM" != preprod-rows ] && [ "$FROM" != prod ]; then
  step "live rows on $DEV"
  if ! rows_on "$DEV"; then die "a live row FAILED on $DEV — the candidate is dead: fix on main, cut rc.$((RCN+1)) (delete $TAG)"; fi
  DEV_EVID="$EVID"
fi

# --- 6 preprod -----------------------------------------------------------------------
# The open record for $TAG when $PRE already pins it on main, else empty: the
# --from preprod-rows lookup. Exactly one open record for the digest, or none.
preprod_open_record() {
  local pinned d12
  pinned="$(python3 - "$PINS" "$COMP" "$PRE" <<'PY'
import sys, yaml
comp = yaml.safe_load(open(sys.argv[1]))["components"][sys.argv[2]]
print(((comp.get("deviations") or {}).get(sys.argv[3]) or {}).get("value", ""))
PY
)"
  # A dry run has no digest (it does not read the registry): the image and tag
  # decide there, and the digest is the one preprod pins.
  if [ "$DRY" = 1 ]; then [ -n "$pinned" ] && [ "${pinned%@*}" = "${REF%@*}" ] || return 0
  else [ -n "$REF" ] && [ "$pinned" = "$REF" ] || return 0; fi
  d12="${pinned##*:}"; d12="${d12:0:12}"
  python3 - "$REPO" "$COMP" "$d12" <<'PY'
import pathlib, sys, yaml
base, comp, d12 = sys.argv[1:4]
hits = [p for p in sorted(pathlib.Path(base, "ops/release/records", comp).glob(f"*-{d12}.yaml"))
        if (yaml.safe_load(p.read_text()) or {}).get("state") == "candidate"]
print(hits[0].relative_to(base) if len(hits) == 1 else "")
PY
}
# A run that reaches this step with $PRE already on $TAG and its record open (a
# re-run from dev after the candidate reached preprod) reuses that record as
# --from preprod-rows does, rather than opening a second record for one ref,
# which graduate.py check then refuses (#2086).
REC_FOUND=""
if [ "$FROM" != prod ] && [ "$FROM" != preprod-rows ]; then
  REC_FOUND="$(preprod_open_record)"
  if [ -n "$REC_FOUND" ]; then
    note "$PRE already runs $TAG on main and its record $REC_FOUND is open; reusing it, as --from preprod-rows does"
    FROM=preprod-rows
  fi
fi
if [ "$FROM" != prod ]; then
if [ "$FROM" != preprod-rows ]; then
step "preprod rung: open the candidate and put $TAG on $PRE"
# A resume from preprod reads the dev evidence from disk — the dev step wrote it
# there — rather than from a variable only the dev step sets.
if [ -z "${DEV_EVID:-}" ]; then DEV_EVID="$(evidence_path "$(runner_var "$DEV" name)")"; [ -s "$DEV_EVID" ] || DEV_EVID=""; fi
mkwt
rc_changes
# Not inside the capture: a `run` in a dry run prints its DRY line, and a
# captured substitution swallowed it, so a dry run never showed this step
# (the goatbuilder parity gate noticed).
if [ "$DRY" = 1 ]; then
  note "DRY: python3 $WT/ops/release/graduate.py open $COMP --ref $REF --reason RC $TAG (ADR 1173), nominated by $ACK; $CHANGES_LINE; validated on $DEV by the affected live acceptance rows"
  REC="ops/release/records/$COMP/<date>-<ref12>.yaml"
else
  REC="$(python3 "$WT/ops/release/graduate.py" open "$COMP" --ref "$REF" --reason "RC $TAG (ADR 1173), nominated by $ACK; $CHANGES_LINE; validated on $DEV by the affected live acceptance rows" | grep -m1 'record written' | sed -E 's/.*record written: //' || true)"
  [ -n "$REC" ] || die "graduate.py open wrote no record"
fi
run python3 - "$WT/$REC" "$DEV" "$TAG" "${DEV_EVID:-}" <<'PY'
import sys, yaml, os, datetime
p, dev, tag, evid = sys.argv[1:5]
s = open(p).read()
ev = f"{dev}, {datetime.date.today()}: release candidate {tag} deployed as the dev-rung deviation; the affected live acceptance rows PASS ({os.path.relpath(evid, os.getcwd()) if evid else 'no live rows mapped; cheap suites only'})."
s = s.replace("  validation: ''\n", "  validation: >-\n    " + ev + "\n", 1)
open(p, "w").write(s)
PY
# Once $PRE runs $TAG, every other open ladder candidate for $COMP is stale
# (graduate.py check C6) and the landing below refuses. Close each in this same
# PR, naming the new record (C8 wants a record file beside it, not a tag).
RECS_BASE="$WT"; [ -d "$RECS_BASE/ops/release" ] || RECS_BASE="$REPO"
while IFS= read -r OLD; do
  [ -n "$OLD" ] || continue
  run python3 "$WT/ops/release/graduate.py" abandon "$OLD" \
    --reason "Superseded by release candidate $TAG on $PRE (ADR 1173), nominated by $ACK" \
    --superseded-by "$(basename "$REC")"
done < <(python3 - "$RECS_BASE" "$COMP" "$(basename "$REC")" <<'PY'
import pathlib, sys, yaml
base, comp, new = sys.argv[1:4]
for p in sorted(pathlib.Path(base, "ops/release/records", comp).glob("*.yaml")):
    rec = yaml.safe_load(p.read_text()) or {}
    if p.name != new and rec.get("state") == "candidate" and rec.get("class") != "hotfix":
        print(p.relative_to(base))
PY
)
set_deviation "$PRE" "RC $TAG ($(date -u +%Y-%m-%d), ADR 1173), validated on $DEV by the affected live acceptance rows — record $(basename "$REC")"
regen_sign_check
land "chore(pins): $PRE runs release candidate $TAG; record opened with dev evidence (ADR 1173)" "Preprod candidate record $(basename "$REC") for $TAG with the $DEV validation evidence; $PRE deviation set. Nominated by $ACK.

What $TAG carries ($CHANGES_LINE):
$CHANGES"
else
# Preprod already runs $TAG on main (a later step failed, say a live row). Find
# its record instead of reopening it: reopening rewrites the dev evidence from a
# variable only the dev rows set, and re-pinning lands a signature-only PR.
step "preprod rung: $PRE already runs $TAG; resuming at its deploy check and live rows"
if [ -n "$REC_FOUND" ]; then
  REC="$REC_FOUND"
elif [ "$DRY" = 1 ]; then
  REC="ops/release/records/$COMP/<date>-<ref12>.yaml"
else
  pinned_ref="$(python3 - "$PINS" "$COMP" "$PRE" <<'PY'
import sys, yaml
comp = yaml.safe_load(open(sys.argv[1]))["components"][sys.argv[2]]
print(((comp.get("deviations") or {}).get(sys.argv[3]) or {}).get("value", ""))
PY
)"
  [ "$pinned_ref" = "$REF" ] || die "$PRE does not run $TAG on main (it pins ${pinned_ref:-the fleet value}) — resume with --from preprod, which pins it"
  D12="${REF##*:}"; D12="${D12:0:12}"
  REC="$(python3 - "$REPO" "$COMP" "$D12" <<'PY'
import pathlib, sys, yaml
base, comp, d12 = sys.argv[1:4]
hits = [p for p in sorted(pathlib.Path(base, "ops/release/records", comp).glob(f"*-{d12}.yaml"))
        if (yaml.safe_load(p.read_text()) or {}).get("state") == "candidate"]
print(hits[0].relative_to(base) if len(hits) == 1 else "")
PY
)"
  [ -n "$REC" ] || die "no single open record for $TAG under ops/release/records/$COMP/ — resume with --from preprod"
fi
note "record $REC"
fi
step "deploy $TAG to $PRE"
run_with_dry_flag "$DEPLOY_HERE/deploy-component.sh" "$COMP" --goatnet "$PRE"
step "live rows on $PRE"
if ! rows_on "$PRE"; then die "a live row FAILED on $PRE after the dev PASS — leave the candidate open, investigate; graduate.py refuses without convergence evidence"; fi
mkwt
run python3 - "$WT/$REC" "$PRE" "$TAG" "${EVID:-}" <<'PY'
import sys, datetime, os
p, pre, tag, evid = sys.argv[1:5]
s = open(p).read()
ev = f"{pre}, {datetime.date.today()}: deploy-component.sh converged every host's ledger on {tag}; the affected live rows PASS on preprod too ({os.path.relpath(evid, os.getcwd()) if evid else 'no live rows mapped'})."
s = s.replace("  preprod_convergence: ''\n", "  preprod_convergence: >-\n    " + ev + "\n", 1)
open(p, "w").write(s)
PY
run python3 "$WT/ops/release/graduate.py" check
land "chore(release): $TAG converged on $PRE with the live rows PASS (ADR 1173)" "Preprod convergence and validation evidence for $TAG recorded on $(basename "$REC")."

# --- 7 soak -----------------------------------------------------------------------------
step "soak"
MIND="$(python3 -c "import yaml; l=yaml.safe_load(open('$LADDER')); h=[x for x in l['hops'] if x['from']=='preprod'][0]; print(next((g.get('min_days',1) for g in h['gates'] if g['id']=='soak'),1))")"
note "$TAG is the preprod candidate. After $MIND day(s), with an operator ack:"
note "  $0 $COMP --tag $TAG --from prod --ack '<name>'    # graduate, fold into fleet:, deploy $PROD, verify every rung"
fi

# --- 8 prod ----------------------------------------------------------------------------
fold_fleet() {  # fleet: = $REF; delete the value deviations named in $@; ROLLBACK comment
  run python3 - "$WT/ops/pins/fleet-pins.yaml" "$COMP" "$REF" "$FLEET" "$TAG" "$(basename "$REC")" "$@" <<'PY'
import sys, re, datetime
p, comp, ref, old, tag, rec, *goatnets = sys.argv[1:]
s = open(p).read()
m = re.search(rf'(?m)^  {re.escape(comp)}:\n', s); start = m.start()
nxt = re.search(r'(?m)^  [A-Za-z0-9_-]+:\n', s[m.end():]); end = m.end() + (nxt.start() if nxt else len(s) - m.end())
block = s[start:end]
old_tag = old.split("@")[0].rsplit(":", 1)[-1] if old else "<none>"
cmt = (f"  # {datetime.date.today()} graduate {tag} (ADR 1120/1173, record {rec}): the preprod candidate\n"
       f"  # becomes the fleet value; the per-rung deviations that carried it are gone.\n"
       f"  # ROLLBACK: {old.split('/')[-1] if old else '<none>'}\n")
block, n = re.subn(r'(?m)^    fleet: .*\n', cmt + f'    fleet: "{ref}"\n', block, count=1)
assert n == 1, f"{comp}: no fleet: line"
for g in goatnets:
    block = re.sub(rf'(?ms)^      {re.escape(g)}:\n        value:.*?(?=^      [A-Za-z0-9_-]+:|\Z)', '', block)
block = re.sub(r'(?m)^    deviations:\n(?=(?:    [a-z_]+:|  [A-Za-z0-9_-]+:|\Z))', '', block)  # empty map left behind
open(p, "w").write(s[:start] + block + s[end:])
PY
}
if [ "$FROM" = prod ]; then
  step "prod rung: graduate $TAG and fold it into fleet:"
  mkwt
  if [ "$DRY" = 1 ]; then
    note "DRY: python3 ops/release/graduate.py graduate $COMP --ack '$ACK'  (refuses until the soak gate is met)"
    REC="ops/release/records/$COMP/<the open candidate record>.yaml"
  else
    OUT="$(python3 "$WT/ops/release/graduate.py" graduate "$COMP" --ack "$ACK" 2>&1)" || die "$OUT"
    REC="$(printf '%s\n' "$OUT" | sed -n 's/^record graduated: //p')"; [ -n "$REC" ] || die "$OUT"
    note "graduated: $REC"
    grep -q "candidate_ref: $REF\$" "$WT/$REC" || die "$REC names a different candidate_ref than $TAG resolves to ($REF)"
  fi
  # Fold only the pins that carried THIS candidate, plus on prod a pin an open
  # hotfix record carries. Any other value deviation belongs to a newer
  # candidate already on its rung (e.g. rc.N+1 on dev) and stays: deleting it
  # would declare the rung back at this tag while it runs the newer one.
  FOLD_PINS="$WT/ops/pins/fleet-pins.yaml"; [ -f "$FOLD_PINS" ] || FOLD_PINS="$PINS"   # a dry run creates no worktree
  FOLD_RECS="$WT/ops/release/records/$COMP"; [ -d "$WT/ops/release" ] || FOLD_RECS="$REPO/ops/release/records/$COMP"
  FOLD="$(python3 - "$FOLD_PINS" "$COMP" "$REF" "$PROD" "$FOLD_RECS" "$DEV" "$PRE" "$PROD" <<'PY'
import glob, os, sys, yaml
pins, comp, ref, prod, recs, *rungs = sys.argv[1:]
devs = (yaml.safe_load(open(pins))["components"][comp].get("deviations") or {})
dig = lambda v: str(v).rsplit("@", 1)[-1]
hot = {dig(r.get("candidate_ref")) for r in (yaml.safe_load(open(f)) or {} for f in glob.glob(os.path.join(recs, "*.yaml")))
       if r.get("class") == "hotfix" and r.get("state") == "candidate" and r.get("candidate_ref")}
fold = []
for g in rungs:
    v = (devs.get(g) or {}).get("value") if isinstance(devs.get(g), dict) else None
    same = dig(v) == dig(ref) or str(v).split("@")[0] == ref.split("@")[0] if v is not None else True
    if v is None or same or (g == prod and dig(v) in hot):
        fold.append(g)
    else:
        print(f"KEEP keeps {g}'s deviation {v}: it carries a different candidate")
print("FOLD " + " ".join(fold))
PY
)"
  while IFS= read -r l; do case "$l" in "KEEP "*) note "${l#KEEP }" ;; esac; done <<< "$FOLD"
  FOLD="$(printf '%s\n' "$FOLD" | sed -n 's/^FOLD //p')"
  # shellcheck disable=SC2086 # FOLD is a deliberate word list of goatnet keys
  fold_fleet $FOLD
  # The prod hotfix record for the fleet value this graduation replaces is settled by it: abandon as superseded.
  for H in $(python3 - "$WT/ops/release/records/$COMP" "$FLEET" <<'PY'
import sys, glob, yaml, os
d, old = sys.argv[1:3]
for f in sorted(glob.glob(os.path.join(d, "*.yaml"))):
    r = yaml.safe_load(open(f)) or {}
    if r.get("class") == "hotfix" and r.get("state") == "candidate" and r.get("candidate_ref") == old:
        print(os.path.relpath(f, os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(d))))))  # repo root: d is <repo>/ops/release/records/<comp>
PY
); do
    note "hotfix record $H carried the fleet value $TAG replaces; abandoning as superseded"
    run python3 "$WT/ops/release/graduate.py" abandon "$H" --reason "superseded: its ref was the fleet value until $TAG graduated ($(basename "$REC")); the backfill it owed is this graduation" --superseded-by "$(basename "$REC")"
  done
  regen_sign_check
  land "chore(release): $TAG graduates; fleet runs it and $PROD folds (ADR 1120/1173)" "Candidate record $(basename "$REC") graduated with the operator ack of $ACK after the soak on $PRE. fleet: is now $REF; the $DEV, $PRE and $PROD deviations that carried the candidate (or the hotfix it replaces) are deleted, so every rung reads one value. Pins and intents regenerated and re-signed; graduate.py check and generate-pins --check pass."
  step "deploy $TAG to $PROD"
  run_with_dry_flag "$DEPLOY_HERE/deploy-component.sh" "$COMP" --goatnet "$PROD"
  step "every rung reads $TAG"
  for g in "$DEV" "$PRE"; do run_with_dry_flag "$DEPLOY_HERE/deploy-component.sh" "$COMP" --goatnet "$g" --verify-only; done
  if [ "$DRY" = 1 ]; then note "$COMP: DRY: a real run puts fleet $REF on $DEV, $PRE and $PROD and reads every ledger"
  else note "$COMP: fleet $REF on $DEV, $PRE and $PROD (ledger-verified)"; fi
fi
if [ "$ASSEMBLED" = 1 ]; then
  # The scratch tree is a plain directory. The overlay worktree is clean once its
  # PR merged; its branch stays for the reflog, as a single-repository run's does.
  if [ "$DRY" = 0 ]; then
    rm -rf "$WT"
    [ -d "$OWT" ] && git -C "$OVERLAY" worktree remove "$OWT" 2>/dev/null || true
  fi
else
[ -d "$WT" ] && [ "$DRY" = 0 ] && git -C "$REPO" worktree remove "$WT" 2>/dev/null || true
fi
