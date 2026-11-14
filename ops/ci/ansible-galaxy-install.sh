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
#
# An attempt cut off by its time bound can leave a collection half written:
# its MANIFEST.json and FILES.json land first, so the next attempt reports it
# "already installed" and skips it, and a playbook that uses a module from the
# missing part fails (couldn't resolve module/action). So every attempt after
# the first passes --force-with-deps, and a finished install is checked with
# `ansible-galaxy collection verify --offline` against each collection's own
# manifest; a mismatch is tried again, forced.
set -uo pipefail
ATTEMPTS="${GALAXY_ATTEMPTS:-3}"
PER_ATTEMPT="${GALAXY_ATTEMPT_SECONDS:-90}"
NET='Temporary failure in name resolution|Name or service not known|Connection refused|Connection reset|timed out|Timeout|Unable to connect|Network is unreachable|No route to host|HTTP Error 5[0-9][0-9]|Unknown error when attempting to call Galaxy|Galaxy server.*did not respond'
LOG="$(mktemp "${TMPDIR:-/tmp}/ansible-galaxy-install.XXXXXX")"
rc=0
UNVERIFIED=0
i=1
while [ "$i" -le "$ATTEMPTS" ]; do
  FORCE=""
  [ "$i" -gt 1 ] && FORCE="--force-with-deps"
  echo "ansible-galaxy-install: attempt $i of $ATTEMPTS (at most ${PER_ATTEMPT}s): collection install ${FORCE:+$FORCE }$*"
  # FORCE is one word or empty; unquoted so that empty adds no argument.
  # shellcheck disable=SC2086
  if timeout -k 10 "$PER_ATTEMPT" ansible-galaxy collection install --timeout 60 $FORCE "$@" >"$LOG" 2>&1; then
    cat "$LOG"
    rc=0
    ansible-galaxy collection verify --offline "$@" >"$LOG" 2>&1 || rc=$?
    if [ "$rc" -eq 0 ]; then
      echo "ansible-galaxy-install: every collection matches its manifest"
      rm -f "$LOG"
      exit 0
    fi
    tail -20 "$LOG"
    echo "ansible-galaxy-install: an installed collection does not match its manifest (an interrupted install?); installing again, forced"
    UNVERIFIED=1
    i=$((i + 1))
    continue
  else
    rc=$?
  fi
  UNVERIFIED=0
  tail -20 "$LOG"
  if [ "$rc" -ne 124 ] && [ "$rc" -ne 137 ] && ! grep -Eq "$NET" "$LOG"; then
    echo "::error title=ansible collections::ansible-galaxy collection install failed (exit $rc), not a network cause; see the lines above"
    rm -f "$LOG"
    exit "$rc"
  fi
  i=$((i + 1))
  [ "$i" -le "$ATTEMPTS" ] && sleep 5
done
if [ "$UNVERIFIED" -eq 1 ]; then
  echo "::error title=ansible collections::the last install finished but a collection does not match its manifest (ansible-galaxy collection verify --offline, exit $rc); see the lines above"
  rm -f "$LOG"
  exit 1
fi
echo "::error title=ansible collections::environment: Ansible Galaxy did not answer after $ATTEMPTS attempts (last exit $rc); the collections are a declared input of this job"
rm -f "$LOG"
exit 69
