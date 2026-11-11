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
# It changes /etc/apt (retries, and the mirror on a fallback) and never restores it: only on a
# CI runner or inside a container, never on an operator's own machine.
[ "${GITHUB_ACTIONS:-}" = true ] || [ -f /.dockerenv ] \
  || { echo "apt-install-fallback: missing ${want[*]}; not changing this machine's apt outside CI"; exit 69; }
printf '%s\n' 'Acquire::Retries "5";' 'Acquire::http::Timeout "20";' 'Acquire::https::Timeout "20";' 'Acquire::ForceIPv4 "true";' \
  | $SUDO tee /etc/apt/apt.conf.d/99ci-retries >/dev/null
LOG="$(mktemp "${TMPDIR:-/tmp}/apt-install-fallback.XXXXXX")"
# The failover cache (.github/actions/apt-failover) sets an apt proxy; when one is set it is
# the path, and its host counts as a source.
PROXY="$(apt-config dump 2>/dev/null | sed -n 's/^Acquire::http::Proxy "\(.*\)";$/\1/p' | head -1)"
PROXY_HOST="$(printf '%s' "$PROXY" | sed -E 's#^[a-z]+://##; s#/.*$##')"
# Network causes only (not "Failed to fetch", which a stale third-party source prints for a
# 404 or a hash mismatch), and only on a source that can carry the package.
NET='Temporary failure resolving|Could not resolve|Could not connect to|Cannot initiate the connection|Connection timed out|Unable to connect|Network is unreachable|No route to host'
net_ubuntu() { grep -E "$NET" "$1" | grep -qE "ubuntu\.com${PROXY_HOST:+|$(printf '%s' "$PROXY_HOST" | sed 's/[.]/\\./g')}"; }
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
    if ! net_ubuntu "$one" \
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
if [ -n "$PROXY" ]; then
  # Through the failover cache the archive's own names are what the cache serves; pointing
  # the sources at the archive would not help while the archive is down.
  if net_ubuntu "$LOG"; then
    echo "::error::environment: neither the configured mirror nor the failover cache ($PROXY) provided ${want[*]}; a declared input of the job, not the tree under test"
    exit 69
  fi
  echo "::error::apt-install-fallback: apt could not install ${want[*]} through $PROXY, for a reason other than the network (log $LOG)"
  exit 1
fi
# The runner's mirror, pointed at the Ubuntu archive itself. Sources that read the mirror list
# (mirror+file:) keep doing so, and apt tries its entries in order: archive.ubuntu.com, then
# security.ubuntu.com. Sources that name a mirror directly are pointed at archive.ubuntu.com.
srcs=(/etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources)
if [ -f /etc/apt/apt-mirrors.txt ] && grep -qs 'mirror+file:/etc/apt/apt-mirrors.txt' "${srcs[@]}"; then
  echo "apt-install-fallback: the configured mirror did not answer; the mirror list now names archive.ubuntu.com, then security.ubuntu.com"
  printf '%s\n' 'http://archive.ubuntu.com/ubuntu/	priority:1' 'http://security.ubuntu.com/ubuntu/	priority:2' | $SUDO tee /etc/apt/apt-mirrors.txt >/dev/null
else
  echo "apt-install-fallback: the configured mirror did not answer; the sources now name archive.ubuntu.com"
  for f in "${srcs[@]}"; do
    [ -f "$f" ] || continue
    $SUDO sed -i -E 's#https?://[a-z0-9.-]*archive\.ubuntu\.com/ubuntu/?#http://archive.ubuntu.com/ubuntu/#g' "$f"
  done
fi
rc=0; attempt || rc=$?
[ "$rc" = 0 ] && exit 0
[ "$rc" = 2 ] && no_such
if net_ubuntu "$LOG"; then
  echo "::error::environment: the Ubuntu mirror did not provide ${want[*]} (also tried the Ubuntu archive directly); a declared input of the job, not the tree under test"
  exit 69
fi
echo "::error::apt-install-fallback: apt could not install ${want[*]}, for a reason other than the network (log $LOG)"
exit 1
