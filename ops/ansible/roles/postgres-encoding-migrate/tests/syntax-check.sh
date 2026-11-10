#!/usr/bin/env bash
# Parse-only checks for postgres-encoding-migrate; no containers. Run in CI by
# .github/workflows/ansible-syntax-check.yml. The functional test is
# run-test.sh (Docker).
#
#   1. --syntax-check of the playbook;
#   2. every tasks file imported statically, so its task keywords parse
#      (the playbook includes them dynamically, which --syntax-check skips);
#   3. pg-utf8-stream.py: scan, filter and repair on a fixed input.

set -euo pipefail
cd "$(dirname "$0")"
ANS="$(cd ../../.. && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export ANSIBLE_CONFIG="$ANS/ansible.cfg"

printf '[g]\nh ansible_connection=local\n' > "$TMP/inv.ini"

echo "[syntax-check] playbook"
( cd "$ANS" && ansible-playbook -i "$TMP/inv.ini" postgres-encoding-migrate.yml --syntax-check -e migrate_groups=g )

echo "[syntax-check] every tasks file, imported statically"
{
  echo '- hosts: g'
  echo '  gather_facts: false'
  echo '  tasks:'
  for f in ../tasks/*.yml; do
    echo "    - ansible.builtin.import_role: {name: postgres-encoding-migrate, tasks_from: $(basename "$f")}"
  done
} > "$ANS/.pem-syntax-$$.yml"
trap 'rm -rf "$TMP" "$ANS/.pem-syntax-$$.yml"' EXIT
( cd "$ANS" && ansible-playbook -i "$TMP/inv.ini" ".pem-syntax-$$.yml" --syntax-check )

echo "[syntax-check] pg-utf8-stream.py"
T=../files/pg-utf8-stream.py
python3 -m py_compile "$T"
printf "SET client_encoding = 'SQL_ASCII';\nCREATE DATABASE d WITH TEMPLATE = template0 ENCODING = 'SQL_ASCII' LOCALE = 'C';\nCOPY public.t (id, v) FROM stdin;\n1\tcaf\xc3\xa9\n2\tet\xe9\n\\\\.\n" > "$TMP/in.sql"
set +e
python3 "$T" scan --label t < "$TMP/in.sql" > "$TMP/scan.json"; rc=$?
set -e
[[ $rc == 3 ]] || { echo "scan: want exit 3, got $rc"; exit 1; }
python3 - "$TMP/scan.json" <<'EOF'
import json, sys
r = json.load(open(sys.argv[1]))
assert not r["ok"], r
assert r["invalid"]["public.t"]["rows"] == 1, r
assert r["invalid"]["public.t"]["samples"][0]["first_field"] == "2", r
EOF
if python3 "$T" filter < "$TMP/in.sql" > /dev/null 2>&1; then echo "filter: passed an invalid byte"; exit 1; fi
python3 "$T" filter --repair latin1 < "$TMP/in.sql" > "$TMP/out.sql"
grep -q "^SET client_encoding = 'UTF8';$" "$TMP/out.sql"
grep -q "ENCODING = 'UTF8' LOCALE = 'C';$" "$TMP/out.sql"
grep -q "$(printf '^1\tcaf\xc3\xa9$')" "$TMP/out.sql"
grep -q "$(printf '^2\tet\xc3\xa9$')" "$TMP/out.sql"
python3 "$T" scan < "$TMP/out.sql" > /dev/null

echo "[syntax-check] PASS"
