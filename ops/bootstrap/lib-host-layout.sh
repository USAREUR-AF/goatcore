# shellcheck shell=bash
# SPDX-License-Identifier: Apache-2.0
# lib-host-layout.sh — the directories a pre-baked image installs goat into,
# for the image builders (build-image-karbon.sh, build-image-orin.sh), which
# run on a builder host outside ansible.
#
#   GOAT_HOST_OPT_DIR    the repository snapshot   group_vars/all.yml goat_host_opt_dir  (/opt/goat)
#   GOAT_HOST_ETC_DIR    host configuration         group_vars/all.yml goat_host_etc_dir  (/etc/goat)
#   GOAT_HOST_STATE_DIR  the completion marker      /var/lib/<name>                       (/var/lib/goat)
#   GOAT_HOST_LOG_DIR    first-boot's event log     /var/log/<name>                       (/var/log/goat)
#
# Each comes from the environment when set, else from the tree's
# ops/ansible/group_vars/all.yml, the values every play uses; <name> is the
# last component of the opt dir, so /opt/<name> keeps /var/lib/<name> and
# /var/log/<name> beside it. A tree that declares nothing gets the public
# layout in brackets.
#
#   . "$REPO_ROOT/ops/bootstrap/lib-host-layout.sh"
#   host_layout_load "$REPO_ROOT"
#   host_layout_render "$STAGE/ops/bootstrap/first-boot/first-boot.service" ...
#
# host_layout_render rewrites the public layout's paths in STAGED copies
# (first-boot.service, first-boot.sh) to the loaded ones, so the unit, the
# script it starts and the directories the builder creates name one layout.
# The files in the repository keep the public layout.

GOAT_PUBLIC_OPT_DIR=/opt/goat
GOAT_PUBLIC_ETC_DIR=/etc/goat
GOAT_PUBLIC_STATE_DIR=/var/lib/goat
GOAT_PUBLIC_LOG_DIR=/var/log/goat

# _host_layout_var <all.yml> <name> — one plain scalar from group_vars/all.yml.
_host_layout_var() {
  [ -f "$1" ] || return 0
  sed -nE "s/^$2:[[:space:]]*[\"']?([^\"'#[:space:]]+)[\"']?.*\$/\\1/p" "$1" | head -1
}

host_layout_load() {
  local all="$1/ops/ansible/group_vars/all.yml" name
  : "${GOAT_HOST_OPT_DIR:=$(_host_layout_var "$all" goat_host_opt_dir)}"
  : "${GOAT_HOST_OPT_DIR:=$GOAT_PUBLIC_OPT_DIR}"
  : "${GOAT_HOST_ETC_DIR:=$(_host_layout_var "$all" goat_host_etc_dir)}"
  : "${GOAT_HOST_ETC_DIR:=$GOAT_PUBLIC_ETC_DIR}"
  name="$(basename "$GOAT_HOST_OPT_DIR")"
  : "${GOAT_HOST_STATE_DIR:=/var/lib/$name}"
  : "${GOAT_HOST_LOG_DIR:=/var/log/$name}"
  export GOAT_HOST_OPT_DIR GOAT_HOST_ETC_DIR GOAT_HOST_STATE_DIR GOAT_HOST_LOG_DIR
}

host_layout_render() {
  python3 - "$@" <<'PY'
import os, re, sys
pairs = [(os.environ[f"GOAT_PUBLIC_{k}_DIR"], os.environ[f"GOAT_HOST_{k}_DIR"])
         for k in ("OPT", "ETC", "STATE", "LOG")]
for path in sys.argv[1:]:
    with open(path, encoding="utf-8") as fh:
        text = fh.read()
    for public, ours in pairs:
        text = re.sub(r"(?<![\w./])" + re.escape(public) + r"(?![\w.-])", lambda _m, o=ours: o, text)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text)
PY
}
export GOAT_PUBLIC_OPT_DIR GOAT_PUBLIC_ETC_DIR GOAT_PUBLIC_STATE_DIR GOAT_PUBLIC_LOG_DIR
