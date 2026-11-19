#!/usr/bin/env bash
# GHSA-x44g: the collector's read access is by group, never root. Runs
# tasks/access.yml on localhost as the invoking user against scratch files
# (journal dir, audit log, auditd.conf, Docker socket); never the host's.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fails=0
fail() { echo "  FAIL $1"; fails=$((fails + 1)); }
play() { ansible-playbook -i localhost, --connection local "$SCRIPT_DIR/access.yml" "$@" 2>&1; }
report() { printf '%s\n' "$1" | sed -n 's/.*"ACCESS=\(.*\)"$/\1/p' | head -1 | tr -d '\\'; }

# A group the invoker belongs to other than their primary one stands in for
# systemd-journal / adm / docker, so regrouping is possible without root.
me="$(id -un)"; my_gid="$(id -g)"
tg="$(id -G | tr ' ' '\n' | grep -vx "$my_gid" | grep -vx 0 | head -1 || true)"
[ -n "$tg" ] || { echo "[goat-logship/access] SKIP (invoker has no supplementary group)"; exit 0; }
tgn="$(getent group "$tg" | cut -d: -f1)"

mkdir -p "$work/journal" "$work/audit"
chgrp "$tg" "$work/journal"; chmod 2755 "$work/journal"
: > "$work/audit/audit.log"; chmod 0600 "$work/audit/audit.log"; chmod 0700 "$work/audit"
printf 'log_file = %s\nlog_group = root\n' "$work/audit/audit.log" > "$work/auditd.conf"
: > "$work/docker.sock"; chgrp "$tg" "$work/docker.sock"
base=(-e logship_user="$me" -e '{"logship_create_user": false, "logship_auditd_reload": false}'
      -e "{\"logship_journal_dirs\": [\"$work/journal\"]}"
      -e logship_auditd_log_path="$work/audit/audit.log" -e logship_auditd_conf_path="$work/auditd.conf"
      -e logship_docker_socket="$work/docker.sock" -e logship_docker_proxy_user=nobody)

# 1. Audit log root-only (0600, dir 0700): managed, it is regrouped, and the
#    collector gets that group, not root.
if out="$(play "${base[@]}" -e logship_auditd_log_group="$tgn")"; then
  r="$(report "$out")"
  [ "$(stat -c '%g %a' "$work/audit/audit.log")" = "$tg 640" ] && [ "$(stat -c '%g %a' "$work/audit")" = "$tg 750" ] \
    && echo "  ok   audit log and its dir regrouped to $tgn (0640 / 0750)" || fail "audit log: $(stat -c '%g %a' "$work/audit/audit.log") dir $(stat -c '%g %a' "$work/audit")"
  grep -qx "log_group = $tgn" "$work/auditd.conf" && ! grep -qx 'log_group = root' "$work/auditd.conf" \
    && echo "  ok   auditd.conf log_group set to $tgn" || fail "auditd.conf: $(grep log_group "$work/auditd.conf")"
  [ "$r" = "{\"uid\": \"$(id -u)\", \"gids\": [\"$tg\"], \"docker_gid\": \"$tg\"}" ] \
    && echo "  ok   collector groups are the paths' groups ($r)" || fail "groups: $r"
else fail "managed audit log: play failed"; printf '%s\n' "$out" | tail -6; fi

# 2. Audit log root-only and not managed: the play stops, naming the path.
chmod 0600 "$work/audit/audit.log"; chgrp "$my_gid" "$work/audit/audit.log"
if out="$(play "${base[@]}" -e '{"logship_auditd_manage_log_group": false}')"; then fail "unreadable audit log: play passed"
elif printf '%s\n' "$out" | grep -q "cannot read $work/audit/audit.log"; then echo "  ok   an owner-only log stops the play"
else fail "unreadable audit log: failed for another reason"; printf '%s\n' "$out" | tail -6; fi

# 3. A path readable only through group root does not count.
if [ "$(id -u)" != 0 ]; then
  if out="$(play "${base[@]}" -e logship_user=root)"; then fail "logship_user root: play passed"
  elif printf '%s\n' "$out" | grep -q "must not run as root"; then echo "  ok   a uid-0 collector user stops the play"
  else fail "root user: failed for another reason"; printf '%s\n' "$out" | tail -6; fi
fi

# 3b. The proxy may not share the collector's uid (it would own the socket dir).
chmod 0640 "$work/audit/audit.log"; chgrp "$tg" "$work/audit/audit.log"
if out="$(play "${base[@]}" -e logship_docker_proxy_user="$me")"; then fail "proxy user = collector user: play passed"
elif printf '%s\n' "$out" | grep -q "must have its own non-root uid"; then echo "  ok   a proxy user equal to the collector's stops the play"
else fail "proxy user: failed for another reason"; printf '%s\n' "$out" | tail -6; fi

# 4. No container-stdout source: the Docker socket is not looked at.
rm -f "$work/docker.sock"; chmod 0640 "$work/audit/audit.log"; chgrp "$tg" "$work/audit/audit.log"
if out="$(play "${base[@]}" -e '{"logship_docker_sources_enabled": false}')"; then
  r="$(report "$out")"; [ "$r" = "{\"uid\": \"$(id -u)\", \"gids\": [\"$tg\"], \"docker_gid\": \"\"}" ] \
    && echo "  ok   no Docker socket needed without container-stdout sources" || fail "no docker: $r"
else fail "no docker sources: play failed"; printf '%s\n' "$out" | tail -6; fi

[ "$fails" -eq 0 ] || { echo "goat-logship: access FAILED ($fails)"; exit 1; }
echo "goat-logship: access OK"
