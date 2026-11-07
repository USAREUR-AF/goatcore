#!/usr/bin/env bash
# The enrollment-apex loopback pin (tasks/main.yml) goes on a mgmt host that serves
# management, and comes OFF a streaming standby (postgres-a in recovery), whose
# management is down by design. Lifts the role's own pin/unpin tasks, points them at
# a temp hosts file, and runs them with a fake docker; needs ansible-playbook.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
command -v ansible-playbook >/dev/null 2>&1 || { echo "[apex-pin-standby] SKIP (no ansible-playbook)"; exit 0; }
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
mkdir "$TMP/bin"
# shellcheck disable=SC2016 # The fixture expands this variable when it runs.
printf '#!/bin/sh\necho "$FAKE_RECOVERY"\n' > "$TMP/bin/docker"; chmod +x "$TMP/bin/docker"
python3 - "$HERE/../tasks/main.yml" "$TMP" <<'PY'
import sys, yaml
tasks = yaml.safe_load(open(sys.argv[1]))
want = ("Ask whether this host's postgres is a streaming standby",
        "Pin the enrollment apex to local Traefik",
        "Remove the enrollment-apex split-horizon on a standby")
picked = [t for t in tasks if any(t.get("name", "").startswith(w) for w in want)]
assert len(picked) == 3, [t.get("name") for t in picked]
for t in picked:
    li = t.get("ansible.builtin.lineinfile")
    if li:
        li["path"] = sys.argv[2] + "/hosts"
        li.pop("owner", None); li.pop("group", None)
play = [{"hosts": "localhost", "connection": "local", "gather_facts": False,
         "vars": {"mesh_coredns_host_zone_route_enabled": True, "site_dns_zone": "netbird.example.net"},
         "tasks": picked}]
yaml.safe_dump(play, open(sys.argv[2] + "/play.yml", "w"), sort_keys=False)
PY
fails=0
run() {  # run <recovery answer> <start with pin? yes|no>
  printf '127.0.0.1 localhost\n' > "$TMP/hosts"
  [ "$2" = yes ] && echo '127.0.0.1 netbird.example.net' >> "$TMP/hosts"
  PATH="$TMP/bin:$PATH" FAKE_RECOVERY="$1" ANSIBLE_NOCOLOR=1 ansible-playbook -i localhost, "$TMP/play.yml" > "$TMP/out" 2>&1 || { cat "$TMP/out"; return 1; }
}
pinned() { grep -qx '127.0.0.1 netbird.example.net' "$TMP/hosts"; }
if run f no  && pinned; then echo "PASS  primary: apex pinned local"; else echo "FAIL  primary: not pinned"; fails=$((fails+1)); fi
if run t yes && ! pinned; then echo "PASS  standby: an existing pin is removed"; else echo "FAIL  standby: pin kept"; fails=$((fails+1)); fi
if run t no  && ! pinned; then echo "PASS  standby: never pinned"; else echo "FAIL  standby: pinned"; fails=$((fails+1)); fi
if run "" no && ! pinned; then echo "PASS  unknown role: no new pin"; else echo "FAIL  unknown role: changed pin"; fails=$((fails+1)); fi
if run "" yes && pinned; then echo "PASS  unknown role: existing pin preserved"; else echo "FAIL  unknown role: removed pin"; fails=$((fails+1)); fi
if [ "$fails" = 0 ]; then echo "[apex-pin-standby] PASS"; else echo "[apex-pin-standby] $fails FAILED"; exit 1; fi
