# shellcheck shell=bash
# apt-fresh-control.sh — the apt negative control of check-workstation.sh:
# on a fresh image, an install before `apt update` fails, so the page's own
# `apt update` is what makes its installs work.
#
#   apt_fresh_control [LISTS_DIR] [CACHE_DIR]
#
# LISTS_DIR defaults to /var/lib/apt/lists and CACHE_DIR to /var/cache/apt.
# The caller defines fail and pass (check-workstation.sh does).
#
# When the job's apt-failover step failed over (APT_FAILOVER=1), it has already
# run `apt-get update` through its cache, so the image has package lists. The
# control empties the lists and apt's package caches first, which makes the
# image fresh again: the install still has to fail, and the page's `apt update`
# still has to fetch the lists. Without failover, lists that are present make
# the control fail, as before.
apt_fresh_control() {
  local lists="${1:-/var/lib/apt/lists}" cache="${2:-/var/cache/apt}"
  if [ "${APT_FAILOVER:-}" = 1 ]; then
    echo "notice: APT_FAILOVER=1: apt-failover ran apt-get update through its cache before this check; emptying $lists and the package caches in $cache so the image is fresh again for the control"
    find "$lists" -mindepth 1 -maxdepth 1 ! -name lock ! -name partial -exec rm -rf {} +
    rm -f "$cache/pkgcache.bin" "$cache/srcpkgcache.bin"
  fi
  if apt-get install -y -qq git >/dev/null 2>&1; then
    fail "apt installed a package before apt update: this image is not fresh, so the control proves nothing"
  fi
  pass "negative control: apt install before apt update fails on the fresh image"
}
