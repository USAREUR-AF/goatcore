#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# project-setting.sh — print one value this checkout declares about how it is
# deployed, from ops/goatbuilder/goatbuilder.yaml. The shell drivers read their
# project's names here (the AWS profile, the state directory, the source
# repository, the image namespace), in the order the builder reads them
# (crates/goatbuilder/src/settings.rs, docs/design/goatbuilder-portability.md):
# the environment variable a person sets for one command, then this
# declaration, then a value that names nobody's project.
#
# usage: project-setting.sh [--repo DIR] <section> <key>
#   project-setting.sh deploy aws_profile
#   project-setting.sh deploy state_dir
#   project-setting.sh repo source
#   project-setting.sh repo image_namespace
#   project-setting.sh --repo DIR trunk-remote     the git remote of DIR whose
#                                                  URL is repo.source
#   project-setting.sh ca-audit-log                the append-only audit log
#                                                  the signing tools write
#                                                  (cert-sign, intent-sign,
#                                                  name-snapshot-sign):
#                                                  ca-audit.jsonl beside
#                                                  deploy.state_dir (else beside
#                                                  ~/.goat/state), `~` kept. Every
#                                                  driver passes it as
#                                                  --audit-log.
#   project-setting.sh var <name>                  one plain `<name>: value`
#                                                  line of the fleet-wide
#                                                  variables file,
#                                                  ops/ansible/group_vars/all.yml:
#                                                  a name of this deployment
#                                                  that a play and an operator
#                                                  script both read. A templated
#                                                  value prints nothing.
#
# Prints the value and exits 0. Prints nothing and exits 0 when the file, the
# section or the key is absent, so a caller writes its own fallback:
#   profile="${GOAT_AWS_PROFILE:-$(project-setting.sh deploy aws_profile)}"
#   profile="${profile:-default}"
#
# Reads the block form the generator writes (ops/goatbuilder/gen-config.py):
#   <section>:
#     <key>: value
# with awk alone, so it runs where python3 has no PyYAML.
set -euo pipefail

# The script's own directory without `dirname`, so a caller on a bare PATH
# (bash and awk only) still gets an answer.
case "${BASH_SOURCE[0]}" in */*) HERE="${BASH_SOURCE[0]%/*}" ;; *) HERE=. ;; esac
REPO="$(cd "$HERE/../.." && pwd)"
if [ "${1:-}" = "--repo" ]; then REPO="${2:?--repo needs a directory}"; shift 2; fi

# unquote <name> <raw>: a scalar as YAML reads it. A plain value, or a quoted one
# with no escape in it, is unquoted here. A double-quoted value with a backslash, or
# a single-quoted one with a doubled quote, is read through PyYAML, so it means what
# goatbuilder's own YAML reader says it means; without PyYAML it is refused by name
# (exit 3) rather than printed with its escapes still in it.
unquote() {
  local name="$1" raw="$2" inner
  case "$raw" in
    \"*\") inner="${raw:1:${#raw}-2}"; case "$inner" in *\\*) ;; *) printf '%s\n' "$inner"; return 0 ;; esac ;;
    \'*\') inner="${raw:1:${#raw}-2}"; case "$inner" in *\'\'*) ;; *) printf '%s\n' "$inner"; return 0 ;; esac ;;
    *) printf '%s\n' "$raw"; return 0 ;;
  esac
  if python3 -c 'import yaml' >/dev/null 2>&1; then
    python3 -c 'import sys, yaml; print(yaml.safe_load(sys.argv[1]))' "$raw"
  else
    echo "project-setting.sh: $name is a quoted YAML value with escapes; reading it needs python3 with PyYAML (pip install pyyaml), or declare it without escapes" >&2
    exit 3
  fi
}

declared() {  # declared <section> <key>
  local f="$REPO/ops/goatbuilder/goatbuilder.yaml" raw
  [ -f "$f" ] || return 0
  raw="$(awk -v section="$1" -v key="$2" '
    /^[^ \t#]/ { inside = ($0 ~ ("^" section ":[ \t]*(#.*)?$")) ; next }
    inside && $0 ~ ("^  " key ":") {
      sub("^  " key ":[ \t]*", "")
      sub(/[ \t]+#.*$/, "")
      sub(/[ \t]+$/, "")
      print
      exit
    }
  ' "$f")"
  [ -n "$raw" ] || return 0
  unquote "$1.$2" "$raw"
}

ca_audit_log() {
  local state
  state="$(declared deploy state_dir)"
  # shellcheck disable=SC2088  # the tilde is kept: a caller prints or expands it
  [ -n "$state" ] || state="~/.goat/state"
  state="${state%/}"
  case "$state" in */*) printf '%s/ca-audit.jsonl\n' "${state%/*}" ;; *) printf './ca-audit.jsonl\n' ;; esac
}

fleet_var() {  # fleet_var <name>
  local f="$REPO/ops/ansible/group_vars/all.yml"
  [ -f "$f" ] || return 0
  awk -v key="$1" '
    $0 ~ ("^" key ":") {
      sub("^" key ":[ \t]*", "")
      sub(/[ \t]+#.*$/, "")
      sub(/[ \t]+$/, "")
      if ($0 ~ /^".*"$/ || $0 ~ /^\047.*\047$/) $0 = substr($0, 2, length($0) - 2)
      if ($0 !~ /[{][{%]/) print
      exit
    }
  ' "$f"
}

# The remote of this checkout that IS the source of record: the one whose URL
# ends in the declared repo.source (owner/name, with or without .git). A clone
# that keeps the trunk under a name other than `origin` is then judged against
# the trunk, not against a mirror or a fork.
trunk_remote() {
  local source name url
  source="$(declared repo source | tr '[:upper:]' '[:lower:]')"
  [ -n "$source" ] || return 0
  for name in $(git -C "$REPO" remote 2>/dev/null); do
    url="$(git -C "$REPO" remote get-url "$name" 2>/dev/null | tr '[:upper:]' '[:lower:]' || true)"
    url="${url%.git}"
    case "$url" in
      */"$source"|*:"$source") printf '%s\n' "$name"; return 0 ;;
    esac
  done
}

case "$#:${1:-}" in
  1:trunk-remote) trunk_remote ;;
  1:ca-audit-log) ca_audit_log ;;
  2:var) fleet_var "$2" ;;
  2:*) declared "$1" "$2" ;;
  *) echo "usage: project-setting.sh [--repo DIR] <section> <key> | trunk-remote | ca-audit-log | var <name>" >&2; exit 2 ;;
esac
