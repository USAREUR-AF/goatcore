#!/usr/bin/env bash
# The admit_audit PG role is written on the primary only: on a standby the writes
# fail (read-only), which broke every re-run of site-bootstrap on router_role B.
# Runs tasks/pg-role.yml on localhost with a fake docker; needs ansible-playbook.
set -euo pipefail
cd "$(dirname "$0")"
command -v ansible-playbook >/dev/null 2>&1 || { echo "[admit-audit/pg-role] SKIP (no ansible-playbook)"; exit 0; }

TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
mkdir "$TMP/bin"
cat > "$TMP/bin/docker" <<'FAKE'
#!/usr/bin/env bash
# Answers pg_is_in_recovery() with $FAKE_RECOVERY; records every other statement.
last="${*: -1}"
case "$last" in
  *pg_is_in_recovery*) echo "$FAKE_RECOVERY" ;;
  *) printf '%s\n' "$last" >> "$FAKE_LOG" ;;
esac
FAKE
chmod +x "$TMP/bin/docker"

fails=0
run() {  # run <t|f>
  : > "$TMP/log"
  PATH="$TMP/bin:$PATH" FAKE_RECOVERY="$1" FAKE_LOG="$TMP/log" ANSIBLE_NOCOLOR=1 \
    ansible-playbook -i 'localhost,' pg-role.yml > "$TMP/out" 2>&1 || { cat "$TMP/out"; return 1; }
}

run t
if [ ! -s "$TMP/log" ]; then echo "PASS  standby: no write"; else echo "FAIL  standby wrote:"; cat "$TMP/log"; fails=$((fails+1)); fi

run f
if grep -q 'CREATE ROLE admit_audit' "$TMP/log" && grep -q 'ALTER ROLE admit_audit WITH PASSWORD' "$TMP/log" \
   && grep -q 'GRANT CONNECT ON DATABASE netbird TO admit_audit' "$TMP/log" \
   && grep -q 'GRANT SELECT ON public.peers TO admit_audit' "$TMP/log"; then
  echo "PASS  primary: role created, password set, grants given"
else echo "FAIL  primary writes:"; cat "$TMP/log"; fails=$((fails+1)); fi

if [ "$fails" != 0 ]; then echo "[admit-audit/pg-role] $fails FAILED"; exit 1; fi
echo "[admit-audit/pg-role] PASS"
