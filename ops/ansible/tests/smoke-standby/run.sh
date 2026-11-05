#!/usr/bin/env bash
# site-bootstrap's smoke test checks management-a only where postgres-a is not a
# streaming standby: on a standby it crash-loops by design until promotion
# (roles/keepalived-mgmt/README.md). Runs the smoke play on localhost with a fake
# docker; needs ansible-playbook.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; cd "$HERE/../.."   # ops/ansible
command -v ansible-playbook >/dev/null 2>&1 || { echo "[smoke-standby] SKIP (no ansible-playbook)"; exit 0; }
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT; mkdir "$TMP/bin"
cat > "$TMP/bin/docker" <<'FAKE'
#!/usr/bin/env bash
case "$*" in
  *pg_is_in_recovery*) echo "$FAKE_RECOVERY" ;;
  *name=management-a*) echo "Restarting (1) 5 seconds ago" ;;
  *name=postgres-a*)   echo "Up 9 minutes (healthy)" ;;
esac
FAKE
chmod +x "$TMP/bin/docker"
printf '[t_hosts]\nlocalhost ansible_connection=local ansible_python_interpreter=%s\n' "$(command -v python3)" > "$TMP/inv"
fails=0
run() { PATH="$TMP/bin:$PATH" FAKE_RECOVERY="$1" ANSIBLE_NOCOLOR=1 ansible-playbook -i "$TMP/inv" "$PWD/site-bootstrap.yml" \
          --tags after-ory -e site_id=t -e has_mgmt_api=true -e mgmt_stack_profile=full -e ansible_become=false \
          --start-at-task 'Ask whether postgres-a is a streaming standby (when has_mgmt_api)' > "$TMP/out" 2>&1; }
run t || true   # later smoke tasks fail against the fake; only this task is asserted
grep -A1 'TASK \[management-a is up' "$TMP/out" | grep -q skipping \
  && echo "PASS  standby: management-a not checked" || { echo "FAIL  standby"; grep -A2 'TASK \[management-a' "$TMP/out"; fails=$((fails+1)); }
run f || true
grep -A1 'TASK \[management-a is up' "$TMP/out" | grep -q 'fatal\|failed' \
  && echo "PASS  primary: a restarting management-a fails the smoke test" || { echo "FAIL  primary"; grep -A2 'TASK \[management-a' "$TMP/out"; fails=$((fails+1)); }
[ "$fails" = 0 ] && echo "[smoke-standby] PASS" || { echo "[smoke-standby] $fails FAILED"; exit 1; }
