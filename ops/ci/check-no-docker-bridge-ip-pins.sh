#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# check-no-docker-bridge-ip-pins.sh — refuse a literal docker-bridge host
# address in a shared deploy artifact.
#
# THE FAILURE THIS EXISTS FOR. A role can pin the goatnet's public management FQDN
# inside a container with `extra_hosts`. A default address such as a bridge IP
# observed on one goatnet may belong to an unrelated container on another, so
# every call goes to a closed port and fails with
#
#     netbird: NetBird GET /api/groups failed: network or parse error
#
# The fix is `host-gateway`. The class is the literal itself: a docker bridge address is
# handed out per host and per container start, so one written into a role
# default, a template, or a compose file shared across goatnets is right on at
# most one of them, for a while. Nothing warned; `check-site-config-bleed.sh`
# derives its literal set from group_vars and does not know this class.
#
# WHAT IT CHECKS. Every tracked file under the trees that are rendered or
# copied to more than one host — the same scope as the bleed guard:
#
#   ops/compose/**
#   ops/ansible/roles/*/defaults/**   ops/ansible/roles/*/templates/**
#   ops/ansible/roles/*/vars/**       ops/ansible/templates/**
#
# A HOST address inside 172.16.0.0/12 (docker's default address pool, and the
# whole of it, because a user-defined network takes the next free /16) is
# refused. Not refused, because they are not a host and not a pin:
#
#   * CIDR notation          172.16.0.0/12          (a range: firewall, ACL, exporter)
#   * a dash range           172.16.0.0-172.31.255.255  (coturn denied-peer-ip)
#   * comment lines          `# ...` and `{# ... #}`, and a trailing `  # ...`
#   * *.md, README*, */tests/*
#
# A line that has to carry one — a network's OWN ipam gateway, which is the
# definition rather than a reference — says why on the line, and the reason is
# required (a bare marker is refused):
#
#     gateway: 172.20.0.1   # docker-bridge-ip-ok: this line defines the network
#
# 192.168.0.0/16, docker's fallback pool, is out of scope on purpose: it is
# every LAN, and the mesh guards already own the addresses that matter there.
#
# Usage:
#   ops/ci/check-no-docker-bridge-ip-pins.sh              # tracked tree, scoped
#   ops/ci/check-no-docker-bridge-ip-pins.sh --staged     # staged files, scoped
#   ops/ci/check-no-docker-bridge-ip-pins.sh path ...     # these files/dirs, unscoped
#   ops/ci/check-no-docker-bridge-ip-pins.sh --self-test  # RED on the fixtures it must catch
#
# Exit: 0 clean · 1 a literal was refused (file:line listed) · 2 usage/self-test error
set -euo pipefail

MARKER='docker-bridge-ip-ok:'

# A docker-pool host address with nothing address-like on either side: not a
# digit or dot (mid-address), not `/` after (CIDR), not `-` on either side (range).
HOST_RE='(^|[^0-9.-])172\.(1[6-9]|2[0-9]|3[01])\.[0-9]{1,3}\.[0-9]{1,3}([^0-9./-]|$)'
# Cheap pre-filter so only candidate lines go through the per-line rules.
CANDIDATE_RE='172\.(1[6-9]|2[0-9]|3[01])\.'

in_scope() {
  case "$1" in
    *.md|*/README*|*/tests/*) return 1 ;;
  esac
  case "$1" in
    ops/compose/*|ops/ansible/templates/*) return 0 ;;
    ops/ansible/roles/*/defaults/*|ops/ansible/roles/*/templates/*|ops/ansible/roles/*/vars/*) return 0 ;;
  esac
  return 1
}

# scan_file <path> — print one refusal per offending line; return 0 when clean.
scan_file() {
  local f="$1" n text stripped bad=0
  [ -f "$f" ] || return 0
  while IFS= read -r hit; do
    n="${hit%%:*}"
    text="${hit#*:}"
    # Whole-line comments carry no config.
    if [[ "$text" =~ ^[[:space:]]*# ]]; then
      continue
    fi
    # Jinja block comments on one line.
    stripped="$(printf '%s' "$text" | sed -E 's/\{#.*#\}//g')"
    # The marker: allowed with a reason, refused without one.
    if [[ "$stripped" == *"$MARKER"* ]]; then
      local reason="${stripped#*"$MARKER"}"
      reason="${reason#"${reason%%[![:space:]]*}"}"
      if [ -n "$reason" ]; then
        continue
      fi
      echo "FAIL: ${f}:${n}: ${text}" >&2
      echo "      \`${MARKER}\` needs a reason after the colon; a bare marker is a bypass, not a record" >&2
      bad=1
      continue
    fi
    # Trailing comments do not configure anything either.
    stripped="$(printf '%s' "$stripped" | sed -E 's/[[:space:]]#.*$//')"
    if printf '%s\n' "$stripped" | grep -qE "$HOST_RE"; then
      echo "FAIL: ${f}:${n}: ${text}" >&2
      bad=1
    fi
  done < <(grep -nE "$CANDIDATE_RE" "$f" 2>/dev/null || true)
  return $bad
}

# scan <file>... — scan every file, print the remedy once, return 1 on any refusal.
scan() {
  local f any=0 count=0
  for f in "$@"; do
    count=$((count + 1))
    scan_file "$f" || any=1
  done
  if [ "$any" -eq 1 ]; then
    cat >&2 <<'MSG'
      A docker bridge address is assigned per host and per container start, so a
      literal one in a shared artifact is right on at most one goatnet, for a while
      (PR #958: zagros's onboard-runner pinned its mgmt FQDN at an nginx).
        /etc/hosts pin for a name traefik serves on the host  ->  host-gateway
        the network's own ipam gateway                        ->  keep it, and say so:
                                                                  # docker-bridge-ip-ok: <reason>
MSG
    return 1
  fi
  echo "OK: no docker-bridge host literals in shared deploy artifacts (${count} files scanned)"
  return 0
}

# expand_paths <path>... — files under each path (a file is itself).
expand_paths() {
  local p
  for p in "$@"; do
    if [ -d "$p" ]; then
      find "$p" -type f | sort
    else
      printf '%s\n' "$p"
    fi
  done
}

self_test() {
  local fx
  fx="$(mktemp -d)"
  trap 'rm -rf "$fx"' RETURN
  mkdir -p "$fx/bad" "$fx/ok"

  printf '%s\n' "addr: \"{{ traefik_ip | default('172.18.0.5', true) }}\"" > "$fx/bad/default.yml"
  printf '%s\n' '      - "{{ public_mgmt_fqdn }}:172.18.0.1"' > "$fx/bad/gateway-pin.j2"
  printf '%s\n' 'endpoint: "tls/172.20.0.1:7447"' > "$fx/bad/endpoint.yml"
  printf '%s\n' "gateway: 172.20.0.1   # ${MARKER}" > "$fx/bad/bare-marker.yml"

  printf '%s\n' 'internal_cidrs: "172.16.0.0/12"' > "$fx/ok/cidr.yml"
  printf '%s\n' 'denied-peer-ip=172.16.0.0-172.31.255.255' > "$fx/ok/range.conf"
  printf '%s\n' '# the exporter binds 172.17.0.1 on the host' 'key: value  # reachable at 172.17.0.1' > "$fx/ok/comments.yml"
  printf '%s\n' '{# was 172.18.0.5 before host-gateway #}' 'x: 1' > "$fx/ok/jinja-comment.j2"
  printf '%s\n' "gateway: 172.20.0.1   # ${MARKER} this line defines the network" > "$fx/ok/ipam.yml"
  printf '%s\n' 'a: 10.0.0.5' 'b: 172.15.0.1' 'c: 172.32.0.1' 'd: 1172.18.0.5' > "$fx/ok/other-ranges.yml"  # oss-allow: the two addresses either side of 172.16.0.0/12, the boundary the guard's regex must respect

  local f
  for f in "$fx"/bad/*; do
    if scan_file "$f" 2>/dev/null; then
      echo "self-test FAIL: guard stayed GREEN on $(basename "$f")" >&2
      return 2
    fi
  done
  for f in "$fx"/ok/*; do
    if ! scan_file "$f" 2>/dev/null; then
      echo "self-test FAIL: guard went RED on $(basename "$f")" >&2
      return 2
    fi
  done
  echo "OK: self-test — guard goes RED on a bridge-IP default, a gateway pin, an endpoint literal and a bare marker; GREEN on CIDR, range, comments, a reasoned marker and other address space"
  return 0
}

mode="tree"; paths=()
case "${1:-}" in
  --self-test) self_test; exit $? ;;
  --staged) mode="staged" ;;
  "") ;;
  --*) echo "usage: $0 [--staged|--self-test|path ...]" >&2; exit 2 ;;
  *) mode="paths"; paths=("$@") ;;
esac

files=()
if [ "$mode" = "paths" ]; then
  while IFS= read -r f; do files+=("$f"); done < <(expand_paths "${paths[@]}")
else
  cd "$(git rev-parse --show-toplevel)"
  if [ "$mode" = "staged" ]; then
    list="$(git diff --cached --name-only --diff-filter=d)"
  else
    list="$(git ls-files -- ops/compose ops/ansible/roles ops/ansible/templates)"
  fi
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    in_scope "$f" && files+=("$f")
  done <<< "$list"
fi

if [ "${#files[@]}" -eq 0 ]; then
  echo "OK: no docker-bridge host literals in shared deploy artifacts (0 files scanned)"
  exit 0
fi
scan "${files[@]}"
