#!/usr/bin/env bash
# The ACL plays' uri ca_path: the goatnet's own root when it is in the repo (a LAN
# goatnet's mgmt API), the system trust store with -e netbird_mgmt_ca=public (an
# internet goatnet that also holds its own CA), and the system store when there is
# no root. Renders each play's own expression on localhost; needs ansible.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; cd "$HERE/../.."     # ops/ansible
command -v ansible-playbook >/dev/null 2>&1 || { echo "[netbird-mgmt-ca] SKIP (no ansible-playbook)"; exit 0; }
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/ansible" "$TMP/enrollment/ca/withroot"; : > "$TMP/enrollment/ca/withroot/root-cert.pem"
fails=0
for play in netbird-acl-bootstrap.yml netbird-acl-cutover.yml; do
  python3 - "$play" > "$TMP/exprs" <<'PY'
import sys, yaml
for p in yaml.safe_load(open(sys.argv[1])):
    e = ((p.get("module_defaults") or {}).get("ansible.builtin.uri") or {}).get("ca_path")
    if e: print(e)
PY
  [ -s "$TMP/exprs" ] || { echo "FAIL  $play: no uri ca_path found"; fails=$((fails+1)); continue; }
  while IFS= read -r expr; do
    for case in "withroot::ROOT" "withroot:public:SYSTEM" "withroot:env:SYSTEM" "noroot::SYSTEM"; do
      site=${case%%:*}; rest=${case#*:}; flag=${rest%%:*}; want=${rest#*:}
      envv=""; [ "$flag" = env ] && { envv=public; flag=""; }
      printf -- '- hosts: localhost\n  gather_facts: false\n  tasks:\n    - ansible.builtin.debug:\n        msg: "%s"\n' "$(printf '%s' "$expr" | sed 's/"/\\"/g')" > "$TMP/ansible/p.yml"
      out=$(NETBIRD_MGMT_CA="$envv" ANSIBLE_NOCOLOR=1 ansible-playbook -i localhost, "$TMP/ansible/p.yml" -e target_site="$site" ${flag:+-e netbird_mgmt_ca=$flag} 2>&1 | grep '"msg"' || true)
      case "$want" in
        ROOT)   echo "$out" | grep -q "enrollment/ca/withroot/root-cert.pem" ;;
        SYSTEM) echo "$out" | grep -q -e "__omit_place_holder__" -e "Hello world!" ;;   # omitted: debug falls back to its default msg
      esac && echo "PASS  $play: site=$site flag=${flag:-none} env=${envv:-none} -> $want" \
           || { echo "FAIL  $play: site=$site flag=${flag:-none} want $want, got: $out"; fails=$((fails+1)); }
    done
  done < "$TMP/exprs"
done
[ "$fails" = 0 ] && echo "[netbird-mgmt-ca] PASS" || { echo "[netbird-mgmt-ca] $fails FAILED"; exit 1; }
