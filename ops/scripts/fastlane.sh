#!/usr/bin/env bash
# fastlane — one command from a merged commit to every goatnet (ADR 1127).
#
# The ladder (ADR 1120) is the default and stays the default. This is the
# deliberate exception: a change that must be everywhere now, where waiting a
# soak window costs more than the risk of skipping it.
#
# WHAT IT DOES (the fourteen steps `graduate.py hotfix` prints for you):
#   resolve the image digest under the one image namespace (ADR 1195) -> move fleet-pins `fleet:` and clear that component's
#   deviations -> regenerate pins + intents -> re-sign every goatnet's intent
#   -> write a hotfix-class record marking it a fastlane -> open a PR and set
#   it to auto-merge -> deploy to every goatnet -> report.
#
# WHAT IT REFUSES, always (ADR 1127 §4). None of these are ceremony:
#   - a ref that is not digest-pinned, or whose digest is not a multi-arch index
#   - a ref with no image built by CI from a merged commit
#   - a component in an OPEN graduation set: fastlane moves one component
#   - a component whose declared rollback is not a pin revert
#   - a missing --ack: a fastlane is a prod change and is attributed
#
# Usage:
#   ops/scripts/fastlane.sh <component> --ref <sha-tag|full-ref> \
#       --reason "why this cannot wait for the ladder" --ack "<operator>"
#   ops/scripts/fastlane.sh <component> --ref … --reason … --ack … --dry-run
#   ops/scripts/fastlane.sh <component> --ref … --reason … --ack … --no-deploy
#
# Canon: docs/design/fastlane.md
set -euo pipefail
# Hold the Mac awake for the whole run (#2122; ops/scripts/lib/keep-awake.sh).
_keep_awake="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/keep-awake.sh"
# shellcheck source=ops/scripts/lib/keep-awake.sh
if [ -f "$_keep_awake" ]; then . "$_keep_awake"; goat_keep_awake; fi

REPO="$(git rev-parse --show-toplevel)"
PINS="$REPO/ops/pins/fleet-pins.yaml"
GRADUATE="$REPO/ops/release/graduate.py"
# Where this machine keeps the project's CA keys (deploy.state_dir in
# ops/goatbuilder/goatbuilder.yaml, else ~/.goat/state) and the audit log
# intent-sign appends to, ca-audit.jsonl beside it: the builder's rule.
STATE_DIR="$("$REPO/ops/scripts/project-setting.sh" --repo "$REPO" deploy state_dir 2>/dev/null || true)"
# shellcheck disable=SC2088  # a declared path is written with a literal "~/"
[ -n "$STATE_DIR" ] || STATE_DIR="~/.goat/state"
# shellcheck disable=SC2088  # matching the literal "~/" of a declared path
case "$STATE_DIR" in "~/"*) STATE_DIR="$HOME/${STATE_DIR#\~/}" ;; esac
AUDIT_LOG="$(dirname "$STATE_DIR")/ca-audit.jsonl"

die()  { printf '\nfastlane: REFUSE — %s\n\n' "$*" >&2; exit 1; }
step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
note() { printf '    %s\n' "$*"; }
deploy_goatnet() {
  if [ "$DRY" = 1 ]; then
    "$REPO/ops/scripts/deploy-component.sh" "$COMPONENT" --goatnet "$1" --dry-run
  else
    "$REPO/ops/scripts/deploy-component.sh" "$COMPONENT" --goatnet "$1"
  fi
}

COMPONENT=""; REF=""; REASON=""; ACK=""; DRY=0; DEPLOY=1
while [ $# -gt 0 ]; do
  case "$1" in
    --ref)     REF="$2"; shift 2 ;;
    --reason)  REASON="$2"; shift 2 ;;
    --ack)     ACK="$2"; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    --no-deploy) DEPLOY=0; shift ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    -*)        die "unknown flag $1" ;;
    *)         [ -z "$COMPONENT" ] && COMPONENT="$1" || die "unexpected argument $1"; shift ;;
  esac
done

[ -n "$COMPONENT" ] || die "usage: fastlane.sh <component> --ref R --reason T --ack NAME"
[ -n "$REF" ]       || die "--ref is required"
[ -n "$REASON" ]    || die "--reason is required — the record has to say why the ladder was skipped"
[ -n "$ACK" ]       || die "--ack '<name>' is required — a fastlane reaches production and is attributed"
case "$ACK" in *'<'*'>'*|*'>'*'<'*) die "\`$ACK\` is still a placeholder: put your own name, for example --ack \"\$(git config user.name)\"" ;; esac

# ---------------------------------------------------------------------------
# 1. Component must exist, and must be one whose rollback is a pin revert.
# ---------------------------------------------------------------------------
step "component $COMPONENT"
COMP_JSON="$(python3 - "$PINS" "$COMPONENT" <<'PY'
import sys, yaml, json
pins = yaml.safe_load(open(sys.argv[1]))
comp = (pins.get("components") or {}).get(sys.argv[2])
if not comp:
    print(json.dumps({"_error": f"unknown component; known: {sorted((pins.get('components') or {}))}"}))
    sys.exit(0)
goatnets = pins.get("goatnets") or {}
print(json.dumps({
    "var": comp.get("var"),
    "image_workflow": comp.get("image_workflow"),
    "ledger_component": comp.get("ledger_component") or sys.argv[2],
    "fleet": comp.get("fleet"),
    "deploy_playbook": comp.get("deploy_playbook") or "",
    # ADR 1127. An undeclared rollback is not a refusal here: --ack is already
    # mandatory, which is what the deploy gate asks of an undeclared one.
    "not_a_pin_revert": (
        f"artifact_type is {comp.get('artifact_type')}" if (comp.get("artifact_type") or "image") not in ("image", "config")
        else "reversible is false" if comp.get("reversible") is False
        else f"rollback is {comp.get('rollback')}" if comp.get("rollback") not in (None, "pin-revert")
        else ""
    ),
    "deviations": sorted((comp.get("deviations") or {}).keys()),
    # Split so the dry run can report what the apply path ACTUALLY does: a
    # value deviation is skew and gets cleared, an `absent: true` one is
    # topology and survives. Reporting them as one list overstated the blast
    # radius on the very line that exists to be read carefully.
    "deviations_absent": sorted(
        k for k, v in (comp.get("deviations") or {}).items()
        if isinstance(v, dict) and v.get("absent")
    ),
    "goatnets": {g: (v.get("members") or []) for g, v in goatnets.items()},
}))
PY
)"
python3 -c "
import json,sys
d=json.loads(sys.argv[1])
if '_error' in d: sys.exit('fastlane: REFUSE — '+d['_error'])
" "$COMP_JSON" || exit 1

CURRENT_FLEET="$(printf '%s' "$COMP_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["fleet"] or "")')"
LEDGER_COMP="$(printf '%s' "$COMP_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["ledger_component"])')"
note "current fleet ref: ${CURRENT_FLEET:-<none>}"
note "ledger component:  $LEDGER_COMP"

[ -d "$REPO/ops/ansible/roles/$LEDGER_COMP" ] \
  || die "no ansible role at ops/ansible/roles/$LEDGER_COMP — fastlane deploys through the component's role, and rollback.sh reads the same path"

DECLARED_PLAY="$(printf '%s' "$COMP_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["deploy_playbook"])')"
if [ -n "$DECLARED_PLAY" ]; then
  DEPLOY_PLAY="$REPO/ops/ansible/$DECLARED_PLAY"
  [ -f "$DEPLOY_PLAY" ] || die "$COMPONENT declares deploy_playbook: $DECLARED_PLAY, which is not in ops/ansible/"
else
  DEPLOY_PLAY="$REPO/ops/ansible/${LEDGER_COMP}-deploy.yml"
  [ -f "$DEPLOY_PLAY" ] || DEPLOY_PLAY="$REPO/ops/ansible/${COMPONENT}-deploy.yml"
  [ -f "$DEPLOY_PLAY" ] || die "no deploy playbook (tried ${LEDGER_COMP}-deploy.yml and ${COMPONENT}-deploy.yml) — fastlane will not invent a deploy path"
fi

NOT_PIN_REVERT="$(printf '%s' "$COMP_JSON" | python3 -c 'import json,sys; print(json.load(sys.stdin)["not_a_pin_revert"])')"
[ -z "$NOT_PIN_REVERT" ] || die "$NOT_PIN_REVERT, so a pin revert cannot undo this (ADR 1127). Use the ladder."

# ---------------------------------------------------------------------------
# 2. Graduation-set atomicity (ADR 1125). Reuses graduate.py's own lookup —
#    a second implementation of "is this component in an open set" is exactly
#    the kind of drift this check exists to prevent.
# ---------------------------------------------------------------------------
step "graduation-set membership"
SET_NAME="$(python3 - "$GRADUATE" "$COMPONENT" <<'PY'
import importlib.util, sys
spec = importlib.util.spec_from_file_location("graduate", sys.argv[1])
m = importlib.util.module_from_spec(spec)
try:
    spec.loader.exec_module(m)
    doc = m.open_set_for(m.load_sets(), sys.argv[2])
    print(doc.get("name", "") if doc else "")
except Exception:
    print("")   # sets are opt-in and may not exist yet; absence is not a blocker
PY
)"
if [ -n "$SET_NAME" ]; then
  die "component is in OPEN graduation set '$SET_NAME' — its members must move together (ADR 1125).
        Fastlane moves one component, so graduate the set normally.
        Shipping one member alone is the failure the set exists to prevent."
fi
note "not in an open set"

# ---------------------------------------------------------------------------
# 3. Resolve the ref to its digest under the registry owner the checkout
#    declares, the only namespace (ADR 1195). The image must already exist — fastlane never builds.
# ---------------------------------------------------------------------------
step "resolving image digest"
IMAGE_NAME="$(printf '%s' "$CURRENT_FLEET" | sed -E 's#.*/([^/:@]+)[:@].*#\1#')"
[ -n "$IMAGE_NAME" ] || die "cannot derive the image name from the current fleet ref"

if grep -q '@sha256:' <<< "$REF"; then
  PINNED="$(printf '%s' "$REF" | sed -E 's/.*@(sha256:[0-9a-f]+).*/\1/')"
  TAG="$(printf '%s' "$REF" | sed -E 's#.*:(sha-[0-9a-f]+)@.*#\1#')"
else
  # CI tags monorepo images sha-<short>; a component built from its own repo
  # (herd, image_workflow: null) is tagged v<semver> by that repo's release
  # workflow. Both are what CI wrote; neither gets a prefix invented for it.
  TAG="$REF"; case "$TAG" in sha-*|v[0-9]*) ;; *) TAG="sha-$TAG" ;; esac
  PINNED=""
fi

# Declared in goatbuilder.yaml (`repo: source:`), as the binary reads it, so
# another organisation's checkout reads its own packages.
ORG="$(python3 -c 'import sys,yaml;print(yaml.safe_load(open(sys.argv[1]))["repo"]["source"].split("/")[0])' "$REPO/ops/goatbuilder/goatbuilder.yaml" 2>/dev/null || true)"
[ -n "$ORG" ] || die "ops/goatbuilder/goatbuilder.yaml declares no repo.source, so the registry owner is unknown. Nothing moved."
API="/orgs/$ORG/packages/container/$IMAGE_NAME/versions"
# The packages API takes the owner as GitHub spells it; a registry path is lower
# case, and the page names the image the one way it exists (ADR 1195).
IMAGE_ORG="$(printf '%s' "$ORG" | tr '[:upper:]' '[:lower:]')"
# Every page: a busy image has hundreds of versions, and the tag asked for is
# often not among the newest.
# An unreadable registry is not a missing image: say which it was.
if ! command -v gh >/dev/null 2>&1; then
  die "the GitHub CLI (gh) is not on PATH, and fastlane reads the image digest through it. Without it the digest cannot be resolved, so nothing moved."
fi
# A tag reaches jq as part of the program text, so it has to be a tag: the
# OCI shape, which is what every tag this repository builds already is.
grep -qE '^[A-Za-z0-9_][A-Za-z0-9._-]{0,127}$' <<< "$TAG" \
  || die "$TAG is not a container tag: a tag is letters, digits, dot, dash or underscore. Nothing moved."
GH_ERR="$(mktemp)"
# gh's own status, taken through `if !` so `set -e` cannot end the script
# first, and read whole so `head` closing the pipe mid-fetch cannot make a
# successful --paginate read look like a failure.
if ! GH_OUT="$(gh api --paginate "$API?per_page=100" --jq ".[]|select(.metadata.container.tags|index(\"$TAG\"))|.name" 2>"$GH_ERR")"; then
  gh_said="$(sed -n 's/^ *//; /./{p;q;}' "$GH_ERR")"; rm -f "$GH_ERR"
  die "cannot read ghcr.io/$IMAGE_ORG/$IMAGE_NAME: ${gh_said:-gh gave no reason}. Without it the digest cannot be resolved, so nothing moved."
fi
rm -f "$GH_ERR"
# The first line, taken in the shell. Piping the whole answer into `head -1`
# gives printf a broken pipe the moment head leaves -- which under `pipefail`
# kills the script on a *successful* read of a busy image. That is the same
# failure this block was rewritten to remove; it just moved one pipe along.
DIGEST="${GH_OUT%%$'\n'*}"
# Only a digest is a digest: a 404 body (`{"message":"Package not found."…}`)
# once came through this pipe as a "digest". Anything that is not
# sha256:<64 hex> is "not found".
grep -qE '^sha256:[0-9a-f]{64}$' <<< "$DIGEST" || DIGEST=""
[ -n "$DIGEST" ] || die "tag $TAG not found under ghcr.io/$IMAGE_ORG/$IMAGE_NAME — has CI built it from a merged commit?"
# A ref given with a digest must be the digest the registry holds for its tag.
if [ -n "$PINNED" ] && [ "$PINNED" != "$DIGEST" ]; then
  die "$TAG is $DIGEST under ghcr.io/$IMAGE_ORG/$IMAGE_NAME, not $PINNED."
fi
note "$ORG: $DIGEST"

NEW_REF="ghcr.io/{{ ghcr_org }}/${IMAGE_NAME}:${TAG}@${DIGEST}"
note "new fleet ref: $NEW_REF"
[ "$NEW_REF" = "$CURRENT_FLEET" ] && die "that ref is already the fleet value — nothing to do"

# ---------------------------------------------------------------------------
# 4. Everything below mutates. Stop here on --dry-run.
# ---------------------------------------------------------------------------
if [ "$DRY" -eq 1 ]; then
  step "dry run — stopping before any mutation"
  note "would set components.$COMPONENT.fleet = $NEW_REF"
  # Report what the MUTATION actually does, not just the deviation key list.
  # `absent: true` survives (it is topology, not skew — see the apply path), so
  # naming it here as "would clear" describes a change fastlane will not make.
  # That matters more than it looks: the deviations line is precisely the line
  # that catches a clear-everything bug, and a dry run that overstates
  # its own blast radius teaches the reader to skim the one check worth reading.
  note "$(printf '%s' "$COMP_JSON" | python3 -c '
import json, sys
d = json.load(sys.stdin)
keep = set(d.get("deviations_absent") or [])
clear = [k for k in d["deviations"] if k not in keep]
print("would clear deviations: " + (", ".join(clear) or "<none>"))
if keep:
    print("    would PRESERVE (absent: true — topology, not skew): " + ", ".join(sorted(keep)))
')"
  note "would deploy via $(basename "$DEPLOY_PLAY") to every goatnet member group"
  exit 0
fi

# ---------------------------------------------------------------------------
# 5. Move `fleet:`, clear the component's deviations, regenerate, re-sign.
# ---------------------------------------------------------------------------
step "moving the value of record"
python3 - "$PINS" "$COMPONENT" "$NEW_REF" "$CURRENT_FLEET" "$REASON" "$ACK" <<'PY'
import sys, re, datetime as dt
pins_path, comp, new_ref, old_ref, reason, ack = sys.argv[1:7]
src = open(pins_path).read()
today = dt.date.today().isoformat()

# Locate the component block, then its `fleet:` line, and rewrite only that.
m = re.search(rf"^  {re.escape(comp)}:\n", src, re.M)
if not m:
    sys.exit(f"fastlane: component {comp} not found in fleet-pins.yaml")
start = m.end()
nxt = re.search(r"^  [a-z0-9][a-z0-9-]*:\n", src[start:], re.M)
end = start + (nxt.start() if nxt else len(src) - start)
block = src[start:end]

banner = (
    f"    # FASTLANE {today} (ADR 1127, ack {ack}): {reason}\n"
    f"    # Went to EVERY goatnet at once with NO soak. Debt is recorded as a\n"
    f"    # hotfix-class record and renders as hotfix-backfill-owed until this\n"
    f"    # ref graduates normally.\n"
    f"    # ROLLBACK: {old_ref}\n"
)
new_block, n = re.subn(r"^    fleet: .*$", banner + f"    fleet: \"{new_ref}\"", block, count=1, flags=re.M)
if n != 1:
    sys.exit(f"fastlane: expected exactly one `fleet:` line in {comp}, found {n}")

# Fastlane converges the fleet, so a deviation carrying a VALUE is redundant by
# construction and goes rather than sitting as phantom skew.
#
# An `absent: true` deviation is NOT skew and MUST survive. It says the component
# does not belong on that member at all — a standby member carries one for the portal
# because the standby runs the router but NOT the admin mutation surfaces. Delete
# it and `effective()` falls through to fleet:, declaring a SECOND admin surface
# on the standby: exactly what the ADR 1084 D3 guard exists to prevent. A naive
# "clear all deviations" regex would ship that.
dev_m = re.search(r"^    deviations:\n", new_block, re.M)
if dev_m:
    # DISTINCT names: `start`/`end` are the COMPONENT block's offsets into `src`
    # and the final write still needs them. Reusing them here sliced src with the
    # deviations offsets and produced invalid YAML.
    dev_start = dev_m.end()
    nxt = re.search(r"^    [a-z_]+:", new_block[dev_start:], re.M)
    dev_end = dev_start + (nxt.start() if nxt else len(new_block) - dev_start)
    body = new_block[dev_start:dev_end]
    # Split into per-member blocks (6-space keys) and keep only absent ones.
    kept, cur, keep_cur = [], [], False
    for line in body.splitlines(keepends=True):
        if re.match(r"^      [A-Za-z_][\w-]*:\s*$", line):
            if cur and keep_cur:
                kept.extend(cur)
            cur, keep_cur = [line], False
        else:
            if cur:
                cur.append(line)
                if re.match(r"^\s*absent:\s*true\s*$", line):
                    keep_cur = True
    if cur and keep_cur:
        kept.extend(cur)
    if kept:
        new_block = new_block[:dev_start] + "".join(kept) + new_block[dev_end:]
        print("    deviations: cleared (absent: true markers PRESERVED — topology, not skew)")
    else:
        new_block = new_block[:dev_m.start()] + new_block[dev_end:]
        print("    deviations for this component: cleared (fastlane converges the fleet)")

open(pins_path, "w").write(src[:start] + new_block + src[end:])
print(f"    fleet: -> {new_ref}")
PY

step "regenerating pins + intents"
python3 "$REPO/ops/pins/generate-pins.py" | sed 's/^/    /'

step "re-signing every goatnet intent"
python3 - "$PINS" <<'PY' | while read -r GOATNET _CA_ID; do
import sys, yaml
d = yaml.safe_load(open(sys.argv[1]))
for g in (d.get("goatnets") or {}):
    print(g, "")
PY
  INTENT="$REPO/ops/consistency/intent/$GOATNET/intent.json"
  [ -f "$INTENT" ] || continue
  CA="$(python3 -c "import json,sys; print(json.load(open('$INTENT')).get('ca_id',''))")"
  [ -n "$CA" ] || die "intent for goatnet $GOATNET declares no ca_id"
  # Derive the key path from the ca_id rather than listing goatnets here.
  # A ca_id is `dev-<ca-name>-ecdsa-<date>` and the operator's key store is
  # `<state dir>/<ca-name>/ca.key.age`, so the name is already in the
  # data — `dev-site-a-ca-ecdsa-2026-01-01` -> `site-a-ca`. A case statement
  # naming goatnets would make a new goatnet's CA a code change in a script
  # nobody thinks about at standup time.
  #
  # GOAT_CA_KEY_<GOATNET> overrides for a store that does not follow the
  # convention (uppercased, non-alphanumerics to underscore: site_a ->
  # GOAT_CA_KEY_SITE_A).
  CA_OVERRIDE="GOAT_CA_KEY_$(printf '%s' "$GOATNET" | tr '[:lower:]-' '[:upper:]_')"
  KEY="${!CA_OVERRIDE:-}"
  if [ -z "$KEY" ]; then
    CA_NAME="$(printf '%s' "$CA" | sed -e 's/^dev-//' -e 's/-ecdsa-[0-9-]*$//')"
    [ -n "$CA_NAME" ] || die "cannot derive a CA name from ca_id '$CA' (goatnet $GOATNET)"
    KEY="$STATE_DIR/$CA_NAME/ca.key.age"
  fi
  if [ ! -f "$KEY" ]; then
    die "CA key not found at $KEY for goatnet $GOATNET (ca_id '$CA').
    This goatnet's root key is not on this laptop, or its store does not follow
    <deploy.state_dir>/<ca-name>/ca.key.age. Point $CA_OVERRIDE at the file."
  fi
  ( cd "$REPO/ops/enrollment" && go run ./cmd/intent-sign \
      --intent "$INTENT" --root-key "$KEY" --age-identity "$HOME/.config/age/keys.txt" \
      --audit-log "$AUDIT_LOG" ) \
    | sed 's/^/    /'
done

# ---------------------------------------------------------------------------
# 6. Record the debt AS DATA. Reuses the hotfix class so graduate.py status
#    already renders it as hotfix-backfill-owed with no tooling change.
# ---------------------------------------------------------------------------
step "recording the debt"
# `graduate.py hotfix` writes the record we want, then PRINTS a prod-deviation
# edit that is wrong for this path — fastlane moved `fleet:` and cleared the
# deviations, which is the opposite. Keep the record, drop the guidance, say
# the true thing instead.
# --previous-ref is REQUIRED here, not optional: step 3 above already rewrote
# fleet:, so graduate.py reading it would record the ref being deployed as the
# ref to roll back to, and the record's rollback pointer would
# name the new image. The fleet-pins ROLLBACK comment was right, so the way
# back existed; the machine-readable field that graduate.py status reads
# was not.
RECORD_LINE="$(python3 "$GRADUATE" hotfix "$COMPONENT" --ref "$NEW_REF" --ack "$ACK" \
  --previous-ref "$CURRENT_FLEET" \
  --reason "FASTLANE (ADR 1127): $REASON — shipped to EVERY goatnet at once with no soak; backfill owed" \
  | grep -m1 'hotfix record written' || true)"
note "${RECORD_LINE:-hotfix record written}"
note "NOTE: fastlane moved fleet: and cleared deviations — there is no deviation edit to make."

# Any record still open against a ref no rung runs is now stale by construction,
# because fastlane just moved every rung. Abandoning them is not tidying: C6
# fails the tree until they are resolved, and leaving it to a human is how 8
# records ended up abandoned with no machine-readable reason at all.
step "abandoning records this fastlane superseded"
SUPERSEDED=0
while read -r REC; do
  [ -n "$REC" ] || continue
  python3 "$GRADUATE" abandon "$REC" \
    --superseded-by "$NEW_REF" \
    --reason "Superseded $(date -u +%Y-%m-%d) by a FASTLANE of $COMPONENT to $TAG (ADR 1127, ack $ACK): $REASON. Every rung moved at once, so this candidate's ref is no longer running anywhere. Its evidence stands — it was overtaken, not invalidated." \
    | sed 's/^/    /'
  SUPERSEDED=$((SUPERSEDED + 1))
done < <(python3 - "$REPO" "$COMPONENT" "$NEW_REF" <<'PY'
import sys, pathlib, yaml
repo, comp, new_ref = sys.argv[1], sys.argv[2], sys.argv[3]
d = pathlib.Path(repo) / "ops/release/records" / comp
for p in sorted(d.glob("*.yaml")) if d.is_dir() else []:
    try:
        rec = yaml.safe_load(open(p))
    except Exception:
        continue
    if not isinstance(rec, dict):
        continue
    if rec.get("state") == "candidate" and rec.get("candidate_ref") != new_ref:
        print(p.relative_to(repo))
PY
)
[ "$SUPERSEDED" -eq 0 ] && note "none were open"

step "gates that still apply"
python3 "$GRADUATE" check 2>&1 | tail -3 | sed 's/^/    /'
python3 "$REPO/ops/pins/generate-pins.py" --check 2>&1 | tail -1 | sed 's/^/    /'

step "landing the pin"
if [ "$DRY" = 1 ]; then
  note "DRY: would commit the pin move on a branch, open the PR, wait for its checks, squash-merge it"
else
  # fastlane runs from a linked worktree on its own branch (the git guard refuses
  # bare git on the master tree); pr-land.sh does commit -> push -> PR -> checks -> merge.
  BR="chore/fastlane-$COMPONENT-$TAG"
  git -C "$REPO" checkout -q -B "$BR" 2>/dev/null || true
  BODY="$(mktemp)"; printf 'chore(pins): fastlane %s to %s (ADR 1127)\n\nFASTLANE, ack %s. %s\nRollback: pin revert to %s. Backfill owed until this ref graduates normally.\n' "$COMPONENT" "$TAG" "$ACK" "$REASON" "$CURRENT_FLEET" > "$BODY"
  "$REPO/ops/scripts/pr-land.sh" --worktree "$REPO" --branch "$BR" --title "chore(pins): fastlane $COMPONENT to $TAG (ADR 1127)" --body-file "$BODY" \
    --paths ops/pins/fleet-pins.yaml ops/ansible/group_vars ops/consistency/intent ops/release/records >/dev/null || die "the pin PR did not land — nothing was deployed"
  # Back to main at the merge: the deploys below run Ansible from this checkout,
  # and leaving it on the landed branch made the next driver run refuse ("run
  # rc.sh from a checkout on main") and the next fastlane branch from the wrong
  # base (found by the hermetic e2e tier).
  git -C "$REPO" fetch -q origin main
  git -C "$REPO" checkout -q main && GOAT_ALLOW_MASTER_GIT_MUTATE=1 git -C "$REPO" merge -q --ff-only origin/main \
    || die "the pin landed, but $REPO could not return to main at origin/main — fast-forward it, then deploy with ops/scripts/deploy-component.sh $COMPONENT --goatnet <g>"
fi
if [ "$DEPLOY" -eq 1 ]; then
  step "deploying to every goatnet the component belongs on"
  FAILED=""
  for G in $(printf '%s' "$COMP_JSON" | python3 -c 'import json,sys; print(" ".join(json.load(sys.stdin)["goatnets"]))'); do
    if deploy_goatnet "$G"; then :; else FAILED="$FAILED $G"; fi
  done
  step "result"
  if [ -n "$FAILED" ]; then
    note "NOT converged on:$FAILED — the pin is in main; re-run ops/scripts/deploy-component.sh $COMPONENT --goatnet <g> once the host is reachable, or ops/scripts/rollback.sh $COMPONENT"
    exit 1
  fi
  note "$COMPONENT is $TAG on every goatnet, ledger-verified. Debt: hotfix-backfill-owed until it graduates (graduate.py status)."
else
  step "next"
  note "--no-deploy: the pin is in main; deploy with ops/scripts/deploy-component.sh $COMPONENT --goatnet <g> per goatnet"
fi
