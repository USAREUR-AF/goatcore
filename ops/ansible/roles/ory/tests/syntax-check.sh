#!/usr/bin/env bash
# ory role: parse the Kratos upgrade window playbook, and exercise the role's
# per-site refusal (tasks/main.yml, "kratos-upgrade-window | Refuse ...") on
# four recorded inputs. The assertion is lifted from tasks/main.yml itself, so
# this tests the shipped task, not a copy. No Docker, no Postgres.
#   pending migrations, no window      -> refused
#   pending migrations, inside window  -> allowed
#   schema already current             -> allowed
#   status command failed              -> refused, even inside the window
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ANS="$(cd "$HERE/../../.." && pwd)"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

echo "[syntax-check] ory-kratos-upgrade-window.yml"
( cd "$ANS" && ANSIBLE_ROLES_PATH="$ANS/roles" ansible-playbook --syntax-check -i localhost, \
    ory-kratos-upgrade-window.yml -e window_groups=g )

echo "[gate] the per-site refusal"
python3 - "$ANS/roles/ory/tasks/main.yml" "$WORK/gate.yml" <<'PY'
import sys, yaml
tasks = yaml.safe_load(open(sys.argv[1]))
gate = [t for t in tasks if t.get("name", "").startswith("kratos-upgrade-window | Refuse")]
assert len(gate) == 1, "refusal task not found in tasks/main.yml"
gate = dict(gate[0]); gate.pop("when", None)
gate.update(ignore_errors=True, register="r")
cases = [
    ("pending-no-window", '[{"state":"Applied","name":"a"},{"state":"Pending","name":"b"}]', 0, "", True),
    ("pending-in-window", '[{"state":"Pending","name":"b"}]', 0, "run1", False),
    ("current", '[{"state":"Applied","name":"a"}]', 0, "", False),
    ("status-failed", "not json", 1, "run1", True),
]
plays = []
for name, out, rc, run, refused in cases:
    plays.append({"name": name, "hosts": "localhost", "gather_facts": False,
                  "vars": {"ory_kratos_status": {"stdout": out, "rc": rc, "stderr": "x"},
                           "ory_kratos_applied": {"stdout": "5"},
                           "ory_kratos_upgrade_window_run": run, "ory_kratos_image": "img"},
                  "tasks": [gate, {"ansible.builtin.assert": {
                      "that": ["r is failed" if refused else "r is not failed"],
                      "fail_msg": f"{name}: expected {'refused' if refused else 'allowed'}"}}]})
yaml.safe_dump(plays, open(sys.argv[2], "w"))
PY
out="$(ansible-playbook -i localhost, -c local "$WORK/gate.yml" 2>&1)" || { echo "$out"; echo "[gate] FAIL"; exit 1; }
echo "$out" | grep -E '^localhost' ; echo "[syntax-check] PASS"
