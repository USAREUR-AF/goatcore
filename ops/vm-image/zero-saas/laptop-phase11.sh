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
OTP="$HOME/.lab-otp"; TOKEN="$HOME/.lab-nbtoken"
trap 'rm -f "$OTP" "$TOKEN"' EXIT
# The file is a note for the person: the password is its "Password:" line.
ssh -o BatchMode=yes ubuntu@10.88.0.2 "sudo cat /opt/mgmt-host/.bootstrap-passwords/$ADMIN.txt" \
  | sed -n 's/^Password:[[:space:]]*//p' > "$OTP"
[ -s "$OTP" ] || { echo "laptop-phase11: no one-time password for $ADMIN on the box" >&2; exit 1; }
export E2E_DIR="$HOME/goatcore/ops/goat-id-console/web/e2e"
export DASHBOARD_URL="https://dashboard.$NAME.$ZONE" MGMT_URL="https://mgmt.$NAME.$ZONE"
export ADMIN_EMAIL="$ADMIN" ADMIN_PASSWORD_FILE="$OTP" SERVICE_USER="svc-$NAME-netbird-infra" OUT_FILE="$TOKEN"
node "$ANSWER" login
# shellcheck disable=SC2086  # GB_FLAGS is a list of flags
goatbuilder ${GB_FLAGS:-} standup "$NAME" --site "$NAME" --from 10 --to 10 --ack zero-saas-lab --yes
node "$ANSWER" token
jq -n --arg k "${UP}_NB_API_TOKEN" --rawfile t "$TOKEN" '{($k): ($t | rtrimstr("\n"))}' \
  | python3 ops/secrets/posture.py set --stdin-json
echo "laptop-phase11: ${UP}_NB_API_TOKEN stored under the declared posture"
