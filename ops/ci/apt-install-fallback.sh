#!/usr/bin/env bash
# apt-install-fallback.sh PACKAGE...: install the packages that are not installed yet, through
# apt, retrying; when the configured mirror does not answer (a GitHub runner's
# azure.archive.ubuntu.com, say), point apt at archive.ubuntu.com and security.ubuntu.com
# directly and try again. Anything still failing exits 69 with "environment:": the
# distribution mirror is a declared input of the job, never the tree under test
# (docs/design/ci-results-follow-inputs.md, rule 1).
set -uo pipefail
SUDO=""; [ "$(id -u)" = 0 ] || SUDO="sudo"
want=()
for p in "$@"; do dpkg -s "$p" >/dev/null 2>&1 || want+=("$p"); done
[ "${#want[@]}" -gt 0 ] || { echo "apt-install-fallback: already installed: $*"; exit 0; }
echo "apt-install-fallback: installing ${want[*]}"
printf '%s\n' 'Acquire::Retries "5";' 'Acquire::http::Timeout "20";' 'Acquire::https::Timeout "20";' 'Acquire::ForceIPv4 "true";' \
  | $SUDO tee /etc/apt/apt.conf.d/99ci-retries >/dev/null
LOG="$(mktemp "${TMPDIR:-/tmp}/apt-install-fallback.XXXXXX")"
NET='Temporary failure resolving|Could not connect to|Failed to fetch|Connection timed out|Unable to connect|Could not resolve'
attempt() {
  local i one
  for i in 1 2 3; do
    one="$LOG.$i"; : > "$one"
    # A failing third-party source on the runner fails `update`; the install is tried anyway.
    $SUDO apt-get update -qq 2>&1 | tee -a "$one" || true
    if $SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends "${want[@]}" 2>&1 | tee -a "$one"; then
      cat "$one" >> "$LOG"; return 0
    fi
    cat "$one" >> "$LOG"
    # A package that does not exist, on an attempt whose sources all answered, will not
    # appear on a retry. With a source unreachable, apt cannot tell it is missing.
    if ! grep -qE "$NET" "$one" \
       && grep -qE 'Unable to locate package|has no installation candidate|is not available, but is referred to' "$one"; then
      return 2
    fi
    echo "apt-install-fallback: attempt $i failed"; sleep $((i * 10))
  done
  return 1
}
no_such() { echo "::error::apt-install-fallback: no such package: ${want[*]} (a defect of the caller, not environment)"; exit 1; }
rc=0; attempt || rc=$?
[ "$rc" = 0 ] && exit 0
[ "$rc" = 2 ] && no_such
# The runner's mirror list and sources, pointed at the Ubuntu archive itself.
echo "apt-install-fallback: the configured mirror did not answer; using archive.ubuntu.com and security.ubuntu.com"
if [ -f /etc/apt/apt-mirrors.txt ]; then
  printf '%s\n' 'http://archive.ubuntu.com/ubuntu/	priority:1' 'http://security.ubuntu.com/ubuntu/	priority:2' | $SUDO tee /etc/apt/apt-mirrors.txt >/dev/null
fi
for f in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
  [ -f "$f" ] || continue
  $SUDO sed -i -E 's#mirror\+file:/etc/apt/apt-mirrors\.txt#http://archive.ubuntu.com/ubuntu/#g; s#https?://[a-z0-9.-]*archive\.ubuntu\.com/ubuntu/?#http://archive.ubuntu.com/ubuntu/#g' "$f"
done
rc=0; attempt || rc=$?
[ "$rc" = 0 ] && exit 0
[ "$rc" = 2 ] && no_such
if grep -qE "$NET" "$LOG"; then
  echo "::error::environment: the Ubuntu mirror did not provide ${want[*]} (also tried archive.ubuntu.com and security.ubuntu.com directly); a declared input of the job, not the tree under test"
  exit 69
fi
echo "::error::apt-install-fallback: apt could not install ${want[*]}, for a reason other than the network (log $LOG)"
exit 1
