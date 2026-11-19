#!/usr/bin/env bash
# run.sh — the goat CLI suite's local-stack tier (docs/design/goat-cli-e2e-testing.md
# §T1s; docs/design/deployment-e2e.md §Suites). The real `goat` binary operates a
# real portal, console and (when Docker is present) Zenoh router started by
# stack.sh, and every case asserts on the object the service stored, not only on
# goat's exit code. Each step is one row in the shared run record (rec.py).
#
#   ops/e2e/suites/goat/run.sh <goat-binary> <state-dir>
#
# The cases come from what an operator does with goat and what each command
# promises; the design doc's "classes of escape" table is the check that no
# class is left blind.
set -uo pipefail

GOAT="${1:?goat binary}"
ST="${2:?state directory}"
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../../.." && pwd)"
export E2E_RECORD_DIR="${E2E_RECORD_DIR:-$ST/record}"
export GOAT_PROFILE_DIR="$ST/goathome" GOAT_CONTEXT=e2e GOAT_NO_FUN=1 NO_COLOR=1
export E2E_GOAT_TOKEN="${E2E_GOAT_TOKEN:-e2e-operator-token}"
PORTAL=http://127.0.0.1:18080
CONSOLE=http://127.0.0.1:18081
OBJ="python3 $HERE/obj.py"
XF=()
[ -f "$ST/router" ] || XF=(--xfail "no container runtime on this machine: router-backed cases need Docker (stack.sh)")
FAIL=0
rec() { python3 "$REPO/ops/e2e/rec.py" "$@" || FAIL=1; }

echo "[goat-suite] $("$GOAT" --version 2>&1) — router: $([ -f "$ST/router" ] && echo real || echo none)"

# ---- build identity --------------------------------------------------------------
CRATE_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/crates/goat-cli/Cargo.toml" | head -1)"
rec goat-version-matches-crate --expect-in "goat $CRATE_VERSION" -- "$GOAT" --version

# ---- profile and context state ------------------------------------------------------
rec doctor-json-is-ndjson -- bash -c "\"$GOAT\" doctor --json --all | $OBJ --ndjson 'len(d) >= 4'"
rec doctor-portal-step-passes -- bash -c "\"$GOAT\" doctor --json --all | $OBJ --ndjson '[x for x in d if x[\"name\"]==\"portal-api-orgs\"][0][\"verdict\"]==\"ok\"'"
rec doctor-router-session-passes "${XF[@]}" -- bash -c "\"$GOAT\" doctor --json --all | $OBJ --ndjson '[x for x in d if x[\"name\"]==\"zenoh-mtls-session\"][0][\"verdict\"]==\"ok\"'"
rec missing-context-names-itself --expect-exit 3 --expect-in "E_PROFILE_MISSING" --expect-in "nope" -- "$GOAT" --context nope topic ls
chmod 000 "$GOAT_PROFILE_DIR/contexts/e2e/portal.token"
rec unreadable-token-is-denied-not-missing --expect-not-in "E_PROFILE_MISSING" -- bash -c "\"$GOAT\" topic ls e2e 2>&1; true"
chmod 600 "$GOAT_PROFILE_DIR/contexts/e2e/portal.token"
rec wrong-token-is-token-invalid --expect-exit 4 --expect-in "E_TOKEN_INVALID" --env "GOAT_PORTAL_TOKEN=not-the-token" -- "$GOAT" topic ls e2e

# ---- the CLI and the portal agree on the contract -----------------------------------
rec org-create -- "$GOAT" admin org create e2e --display-name "E2E Org" --synthetic --auto-approve
rec org-stored-by-portal -- $OBJ "$PORTAL/api/orgs" 'any(o.get("prefix")=="e2e" for o in d.get("orgs",[])+d.get("vendors",[]))'
rec org-ls-shows-it --expect-in "e2e" -- "$GOAT" org ls
rec topic-create --expect-in "created topic" -- "$GOAT" topic create e2e/smoke/v1 --format json --description "e2e smoke topic"
rec topic-stored-as-asked -- $OBJ "$PORTAL/api/orgs/e2e/topics" 'any(t["key_pattern"]=="e2e/smoke/v1" and t["format"]=="json" and t["description"]=="e2e smoke topic" for t in d["topics"])'
rec topic-ls-json-contract -- bash -c "\"$GOAT\" topic ls e2e --json | $OBJ --ndjson 'd and all({\"key_pattern\",\"format\",\"id\"} <= set(x) for x in d)'"
# The whole goatnet: one GET /api/catalog, not one request per org (#2088); the
# rows keep the per-org listing's --json contract, the id included.
rec topic-ls-all-from-the-catalog -- bash -c "\"$GOAT\" topic ls --json | $OBJ --ndjson 'any(x[\"key_pattern\"]==\"e2e/smoke/v1\" and x[\"vendor_prefix\"]==\"e2e\" and x[\"id\"] > 0 for x in d)'"
rec topic-duplicate-is-a-conflict --expect-exit 1 --expect-not-in "E_PORTAL_UNREACHABLE" -- "$GOAT" topic create e2e/smoke/v1 --format json
# The portal answers a topic under an unknown org with a 404 naming the org, and
# goat carries the server's line through as E_PORTAL_NOT_FOUND (exit 7) with the
# request path: an answer, never "unreachable" (#2087).
rec topic-under-unknown-org-names-it --expect-exit 7 --expect-in "no org with prefix" --expect-in "E_PORTAL_NOT_FOUND" -- "$GOAT" topic create nope/smoke/v1 --format json
rec portal-answer-is-not-called-unreachable --expect-exit 7 --expect-in "HTTP 404" --expect-in "/topics" \
  --expect-not-in "E_PORTAL_UNREACHABLE" -- "$GOAT" topic create nope/smoke/v1 --format json

# ---- onboarding: invite, list, revoke, against the console's own record -----------
rec invite-mint --expect-in "minted" -- "$GOAT" admin invite --env zagros --tier browse
INV="$(python3 -c 'import json,sys,urllib.request,os; r=urllib.request.Request(sys.argv[1], headers={"Authorization":"Bearer "+os.environ["E2E_GOAT_TOKEN"]}); d=json.load(urllib.request.urlopen(r)); print(d[0]["id"] if d else "")' "$CONSOLE/api/onboard/invites")"
rec invite-stored-with-its-tier -- $OBJ "$CONSOLE/api/onboard/invites" "any(i['id']=='$INV' and json.loads(i['prebound_json']).get('tier')=='browse' for i in d)"
rec invite-default-lifetime-is-not-an-hour -- $OBJ "$CONSOLE/api/onboard/invites" "[i['expires_at']-i['created_at'] for i in d if i['id']=='$INV'][0] >= 86400"
rec invites-ls-json-lists-it --expect-in "$INV" -- "$GOAT" admin invites ls --json
rec invite-revoke -- "$GOAT" admin invites revoke "$INV"
rec invite-revoked-in-console -- $OBJ "$CONSOLE/api/onboard/invites?status=revoked" "any(i['id']=='$INV' for i in d)"
rec queue-ls-json-is-an-array -- bash -c "\"$GOAT\" admin queue ls --env zagros --json | $OBJ --stdin 'isinstance(d, list)'"

# ---- operator keys: requested only for a person on the onboarding roster ---------
rec opkey-request-for-a-non-roster-handle-is-refused --expect-exit 4 -- "$GOAT" admin operator-key request e2e-person --email e2e-person@example.invalid --env zagros --reason "e2e suite" --json
rec opkey-refused-request-records-nothing --expect-not-in "e2e-person" -- "$GOAT" admin operator-key list

# ---- fabric: a real router, mTLS, a publish received ------------------------------
rec pubsub-round-trip "${XF[@]}" -- bash -c "
  \"$GOAT\" sub 'e2e/smoke/v1' --count 1 > \"$ST/sub.out\" 2>&1 & s=\$!; sleep 3
  \"$GOAT\" pub e2e/smoke/v1 --payload '{\"e2e\":\"round-trip\"}' --confirm
  wait \$s; grep -q 'round-trip' \"$ST/sub.out\""

# ---- the acceptance assay's goat CLI rows, against this stack ----------------------
# The rows `goatbuilder assay` runs through the operator's CLI, by the assay's own
# runner, with this stack's context. A health beat that nothing publishes here must
# grade FAIL, never green; a context that does not exist is UNKNOWN, not a failure
# of the goatnet (ops/validation/acceptance-tests.yaml, the goat CLI legs).
printf 'site_id: "e2e"\ngoat_context: "e2e"\ngoat_via: "localhost"\n' > "$ST/assay-values.yaml"
ASSAY="PATH=\"$(cd "$(dirname "$GOAT")" && pwd):\$PATH\" python3 \"$REPO/ops/validation/run-acceptance.py\" --env e2e --values \"$ST/assay-values.yaml\""
rec assay-goat-status-chain-passes "${XF[@]}" --expect-in "PASS  goat-status-chain" -- bash -c "$ASSAY --only goat-status-chain"
rec assay-goat-hello-round-trip-passes "${XF[@]}" --expect-in "PASS  goat-hello-roundtrip" -- bash -c "$ASSAY --only goat-hello-roundtrip"
rec assay-unpublished-health-is-not-green "${XF[@]}" --expect-exit 1 --expect-in "FAIL  goat-health-fabric" -- bash -c "$ASSAY --only goat-health-fabric"
rec assay-missing-context-is-unknown --expect-exit 2 --expect-in "E_PROFILE_MISSING" -- bash -c "$ASSAY --only goat-status-chain --var goat_context=nope"
# The bus-roundtrip rows through the real router's REST plugin (no context speaks
# for this goatnet, so the probe falls back from goat to REST): a published nonce
# read back, a beat under this site's key that names this site, and a beat under
# this site's key that names another — row 43's lesson, which must FAIL.
RR="$ASSAY --var goat_context=nope --var orin_a=localhost --var zenoh_rest_port=18000"
beat() { (for _ in $(seq 40); do curl -s -o /dev/null -X PUT -H 'Content-Type: application/json' \
  --data-binary "{\"site\":\"$1\",\"scope\":\"$1\"}" http://127.0.0.1:18000/ops/mesh/health/e2e; sleep 1; done) >/dev/null 2>&1 & echo $!; }
rec assay-router-round-trip-passes "${XF[@]}" --expect-in "PASS  fabric-put-subscribe-roundtrip" -- bash -c "$RR --only fabric-put-subscribe-roundtrip"
B="$(beat e2e)"
rec assay-health-beat-under-own-key-passes "${XF[@]}" --expect-in "PASS  health-keys-carry-own-site-id" -- bash -c "$RR --only health-keys-carry-own-site-id"
kill "$B" 2>/dev/null; B="$(beat zagros)"
rec assay-health-beat-naming-another-site-fails "${XF[@]}" --expect-exit 1 --expect-in "FAIL  health-keys-carry-own-site-id" -- bash -c "$RR --only health-keys-carry-own-site-id"
kill "$B" 2>/dev/null

# ---- the sandbox simulator (T1's keystone) on the same binary ---------------------
rec hello-sandbox-round-trip -- bash -c "\"$GOAT\" hello --sandbox --json | $OBJ --stdin 'd[\"mode\"]==\"sandbox\" and d[\"round_trip\"]'"

# ---- a service that is down is said to be down, never an empty answer -------------
kill "$(sed -n 3p "$ST/pids")" 2>/dev/null; sleep 1
rec console-down-is-not-an-empty-queue --expect-not-in "[]" -- bash -c "\"$GOAT\" admin queue ls --env zagros --json; test \$? -ne 0"

python3 "$REPO/ops/e2e/report.py" "$E2E_RECORD_DIR" 2>/dev/null | tail -40 || true
exit "$FAIL"
