#!/usr/bin/env bash
# parity-verify-refusals.sh — both avenues refuse the same incomplete ledger
# verifications (#1545). parity-verify.sh compares dry runs only; this runs the
# REAL verify path of `ops/scripts/deploy-component.sh --verify-only` and
# `goatbuilder verify` against a stubbed fixture (every host and credential
# command replaced; nothing is contacted) and requires, for each case, that
# both pass or both refuse — and that each case lands the way it should.
#
# usage: parity-verify-refusals.sh <path-to-goatbuilder-binary>
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
BIN="$(cd "$(dirname "${1:?usage: parity-verify-refusals.sh <goatbuilder binary>}")" && pwd)/$(basename "$1")"

fixture() {   # <dir> — the same fixture as tests/deploy-component-verify.bats
  local fix="$1/repo" stubs="$1/stubs" bin="$1/bin"
  mkdir -p "$fix/ops/scripts" "$fix/ops/pins" "$fix/ops/ansible/group_vars/test_hosts" "$stubs" "$bin"
  cp "$REPO/ops/scripts/deploy-component.sh" "$REPO/ops/scripts/served-payloads.py" "$fix/ops/scripts/"
  cat > "$fix/ops/pins/fleet-pins.yaml" <<'YAML'
goatnets:
  test_site:
    members: [test_hosts]
components:
  widget:
    var: widget_image
    fleet: example/widget@sha256:test
    deploy_playbook: widget-deploy.yml
YAML
  echo 'widget_image: "example/widget@sha256:test"' > "$fix/ops/ansible/group_vars/test_hosts/pins.yml"
  echo 'hosts: "{{ target }}"' > "$fix/ops/ansible/widget-deploy.yml"
  printf '#!/usr/bin/env bash\nexit 1\n' > "$fix/ops/scripts/env-door.sh"
  printf '#!/usr/bin/env bash\nprintf "Profile: x\\nManagement: Disconnected\\n"\n' > "$bin/netbird"
  cat > "$fix/ops/scripts/with-aws" <<'SH'
#!/usr/bin/env bash
shift 2
case "$1" in
  ansible-inventory) cat "$STUBS/inventory"; exit "$(cat "$STUBS/inventory.rc" 2>/dev/null || echo 0)" ;;
  ansible) cat "$STUBS/rows"; exit "$(cat "$STUBS/rows.rc" 2>/dev/null || echo 0)" ;;
  *) exit 91 ;;
esac
SH
  chmod +x "$fix/ops/scripts/"* "$bin/netbird"
  echo '{"test_hosts":{"hosts":["host-a","host-b"]},"_meta":{"hostvars":{"host-a":{"router_role":"A"},"host-b":{"router_role":"B"}}}}' > "$stubs/inventory"
  { row host-a; row host-b; } > "$stubs/rows"
}
row() { printf '%s | CHANGED | rc=0 >> {"ref":"example/widget@sha256:%s","action":"%s"}\n' "$1" "${2:-test}" "${3:-deploy}"; }

# case  expected(0 = converged, 1 = refused)
# one_host*: a component on the one host its group variable names (one_host_var).
CASES="good:0 empty:1 missing:1 pending:1 ledger_rc:1 inv_rc:1 nothing:1 one_host:0 one_host_missing:1 one_host_templated:1 one_host_outside:1"
one_host() {   # <stubs dir> <YAML value of widget_host>
  local fix="${1%/stubs}/repo"
  awk '/^    var: widget_image$/ {print; print "    one_host_var: widget_host"; next} {print}' "$fix/ops/pins/fleet-pins.yaml" > "$fix/ops/pins/fleet-pins.yaml.new"
  mv "$fix/ops/pins/fleet-pins.yaml.new" "$fix/ops/pins/fleet-pins.yaml"
  printf 'widget_host: %s\n' "$2" > "$fix/ops/ansible/group_vars/test_hosts/widget.yml"
}
fail=0
for c in $CASES; do
  name="${c%%:*}"; want="${c##*:}"
  T="$(mktemp -d)"; fixture "$T"; S="$T/stubs"
  case "$name" in
    empty) : > "$S/rows" ;;
    missing) row host-a > "$S/rows" ;;
    pending) { row host-a test pending; row host-b; } > "$S/rows" ;;
    ledger_rc) echo 4 > "$S/rows.rc" ;;
    inv_rc) echo 1 > "$S/inventory.rc" ;;
    nothing) echo '{"test_hosts":{"hosts":[]}}' > "$S/inventory"; : > "$S/rows" ;;
    one_host) one_host "$S" host-b; row host-b > "$S/rows" ;;
    one_host_missing) one_host "$S" host-b; row host-a > "$S/rows" ;;
    one_host_templated) one_host "$S" '"{{ groups.x[0] }}"' ;;
    one_host_outside) one_host "$S" host-z ;;
  esac
  ( cd "$T/repo" && STUBS="$S" GOAT_DEPLOY_LOG_DIR="$T" PATH="$T/bin:$PATH" \
      ./ops/scripts/deploy-component.sh widget --goatnet test_site --verify-only ) > "$T/script.out" 2>&1; a=$?
  ( cd "$T/repo" && STUBS="$S" PATH="$T/bin:$PATH" "$BIN" -C "$T/repo" verify widget --env test_site ) > "$T/binary.out" 2>&1; b=$?
  sa=$([ "$a" = 0 ] && echo 0 || echo 1); sb=$([ "$b" = 0 ] && echo 0 || echo 1)
  if [ "$sa" != "$want" ] || [ "$sb" != "$want" ]; then
    fail=$((fail + 1))
    echo "PARITY FAIL: $name — want $([ "$want" = 0 ] && echo converged || echo refused); script exit $a, goatbuilder exit $b"
    sed 's/^/    script: /' "$T/script.out" | tail -4; sed 's/^/    binary: /' "$T/binary.out" | tail -4
  fi
  rm -rf "$T"
done
n=$(echo $CASES | wc -w | tr -d ' ')
if [ "$fail" = 0 ]; then echo "parity ok: $n verify cases pass or refuse identically (deploy-component.sh --verify-only vs goatbuilder verify, real path, stubbed hosts)"; else echo "parity FAIL: $fail of $n cases"; exit 1; fi
