#!/usr/bin/env bash
# ansible-galaxy-install.sh ARG...: `ansible-galaxy collection install ARG...`, bounded.
#
# Ansible Galaxy is a declared input of a CI job, never the tree under test
# (docs/design/ci-results-follow-inputs.md, rule 1). A download that does not
# answer is tried again, at most ATTEMPTS times, each attempt bounded. When it
# still fails, the step says in one public annotation which input failed:
# "environment" with exit 69 when the output names a network cause, otherwise
# the command's own exit code, so a wrong requirements file is never reported
# as the network.
set -uo pipefail
ATTEMPTS="${GALAXY_ATTEMPTS:-3}"
PER_ATTEMPT="${GALAXY_ATTEMPT_SECONDS:-90}"
NET='Temporary failure in name resolution|Name or service not known|Connection refused|Connection reset|timed out|Timeout|Unable to connect|Network is unreachable|No route to host|HTTP Error 5[0-9][0-9]|Unknown error when attempting to call Galaxy|Galaxy server.*did not respond'
LOG="$(mktemp "${TMPDIR:-/tmp}/ansible-galaxy-install.XXXXXX")"
rc=0
i=1
while [ "$i" -le "$ATTEMPTS" ]; do
  echo "ansible-galaxy-install: attempt $i of $ATTEMPTS (at most ${PER_ATTEMPT}s): collection install $*"
  if timeout -k 10 "$PER_ATTEMPT" ansible-galaxy collection install --timeout 60 "$@" >"$LOG" 2>&1; then
    cat "$LOG"
    rm -f "$LOG"
    exit 0
  else
    rc=$?
  fi
  tail -20 "$LOG"
  if [ "$rc" -ne 124 ] && [ "$rc" -ne 137 ] && ! grep -Eq "$NET" "$LOG"; then
    echo "::error title=ansible collections::ansible-galaxy collection install failed (exit $rc), not a network cause; see the lines above"
    rm -f "$LOG"
    exit "$rc"
  fi
  i=$((i + 1))
  [ "$i" -le "$ATTEMPTS" ] && sleep 5
done
echo "::error title=ansible collections::environment: Ansible Galaxy did not answer after $ATTEMPTS attempts (last exit $rc); the collections are a declared input of this job"
rm -f "$LOG"
exit 69
