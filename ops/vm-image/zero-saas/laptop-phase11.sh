#!/usr/bin/env bash
# laptop-phase11.sh — LAB ONLY. Runs inside the lab's laptop VM, in place of the person
# at a goatlite's phase 11 (ops/vm-image/zero-saas/README.md). Not part of goatbuilder;
# no product path calls it. It follows tutorial 00 section 5, "The browser step":
#   2. read the one-time password on the box
#   4. sign in at the dashboard (phase11-answer.mjs login)
#   5. re-run phase 10 once, which promotes the new account
#   6. create the service user svc-<name>-netbird-infra and a token on it
#      (phase11-answer.mjs token)
#   7. store the token where the declared secrets posture says
# Step 1 (the hosts line) and step 3 (the browser's certificate warnings) are lab.sh's
# and the browser context's. Every address it reaches is the box's.
#
#   bash laptop-phase11.sh <name> <zone> <admin email> <phase11-answer.mjs>
set -euo pipefail
NAME="$1"; ZONE="$2"; ADMIN="$3"; ANSWER="$4"
UP="$(printf '%s' "$NAME" | tr '[:lower:]' '[:upper:]')"
cd "$HOME/goatcore"
umask 077
# The one-time password and the token live only in this run's own directory, mode
# 0700, removed on exit; neither is printed or passed as an argument.
D="$(mktemp -d)"; OTP="$D/otp"; TOKEN="$D/token"
trap 'rm -rf "$D"' EXIT
# The file is a note for the person: the password is its "Password:" line.
if ! ssh -o BatchMode=yes ubuntu@10.88.0.2 "sudo cat /opt/mgmt-host/.bootstrap-passwords/$ADMIN.txt" > "$D/note"; then
  echo "laptop-phase11: no one-time password note for $ADMIN on the box" >&2; exit 1
fi
sed -n 's/^Password:[[:space:]]*//p' "$D/note" > "$OTP"; rm -f "$D/note"
[ -s "$OTP" ] || { echo "laptop-phase11: the note for $ADMIN on the box has no Password: line" >&2; exit 1; }
export E2E_DIR="$HOME/goatcore/ops/goat-id-console/web/e2e"
export DASHBOARD_URL="https://dashboard.$NAME.$ZONE" MGMT_URL="https://mgmt.$NAME.$ZONE"
export ADMIN_EMAIL="$ADMIN" ADMIN_PASSWORD_FILE="$OTP" SERVICE_USER="svc-$NAME-netbird-infra" OUT_FILE="$TOKEN"
node "$ANSWER" login
# PHASE10_AVENUE=scripts re-runs phase 10 with ops/scripts/goat-standup.sh (ADR 1201).
# shellcheck disable=SC2086  # GB_FLAGS is a list of flags
if [ "${PHASE10_AVENUE:-cli}" = scripts ]; then
  bash ops/scripts/goat-standup.sh "$NAME" ${GB_FLAGS:-} --from 10 --to 10
else
  goatbuilder ${GB_FLAGS:-} standup "$NAME" --site "$NAME" --from 10 --to 10 --ack zero-saas-lab --yes
fi
node "$ANSWER" token
# posture.py keeps a value already there: anything but "written" is a failure.
out="$(jq -n --arg k "${UP}_NB_API_TOKEN" --rawfile t "$TOKEN" '{($k): ($t | rtrimstr("\n"))}' \
  | python3 ops/secrets/posture.py set --stdin-json)"
printf '%s\n' "$out"
case "$out" in *written*) ;; *) echo "laptop-phase11: ${UP}_NB_API_TOKEN was not written (a value was already there?)" >&2; exit 1 ;; esac
echo "laptop-phase11: ${UP}_NB_API_TOKEN stored under the declared posture"
