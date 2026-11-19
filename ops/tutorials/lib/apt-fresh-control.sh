# shellcheck shell=bash
# apt-fresh-control.sh — the apt negative control of check-workstation.sh:
# on a fresh image, an install before `apt update` fails, so the page's own
# `apt update` is what makes its installs work.
#
#   apt_fresh_control
#
# The caller defines fail and pass (check-workstation.sh does).
apt_fresh_control() {
  if apt-get install -y -qq git >/dev/null 2>&1; then
    fail "apt installed a package before apt update: this image is not fresh, so the control proves nothing"
  fi
  pass "negative control: apt install before apt update fails on the fresh image"
}
