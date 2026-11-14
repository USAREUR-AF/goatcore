#!/usr/bin/env bash
# prereqs.sh — every input the hermetic tier needs from the machine it runs on, checked before it runs.
#
# docs/design/deployment-e2e.md §Hermetic. The hermetic tier is the expensive
# step; a missing tool, library or permission must fail HERE, in seconds, naming
# what is missing and how to install it — never minutes into the run. The list
# below is the static scan of what the tier invokes: hermetic.sh, overlay.py,
# the stubs, the drivers it runs (rc.sh, deploy-component.sh, rollback.sh,
# fastlane.sh, pr-land.sh, ansible-preflight.sh), the plays and roles it applies,
# the Dockerfiles (the canary and the host, Ubuntu and Debian), and the Go tools the overlay builds (ca-init, intent-sign).
#
# usage: ops/e2e/prereqs.sh [--no-hosts]     exit 0 ready; 1 something is missing (all named)
set -uo pipefail
HOSTS=1; [ "${1:-}" = "--no-hosts" ] && HOSTS=0
missing=0
need() { # $1 what, $2 test command, $3 fix
  if bash -c "$2" >/dev/null 2>&1; then printf 'ok    %s\n' "$1"; else printf 'MISS  %s — %s\n' "$1" "$3"; missing=$((missing + 1)); fi
}
# controller binaries (hermetic.sh, overlay.py, the drivers)
for t in git python3 go curl jq age-keygen ssh ssh-keygen nc ansible ansible-playbook ansible-inventory ansible-galaxy; do
  need "binary $t" "command -v $t" "install $t (CI: deploy-e2e.yml 'Install the controller's tools')"
done
# rollback.sh parses its connection map with the jq-wrapping yq (python yq), whose `[]?` and `@tsv` it relies on
need "yq (the jq wrapper rollback.sh expects)" "yq --help 2>&1 | grep -q -i jq" "pip install yq"
# python modules (generate-pins.py, graduate.py, intent rendering, the stubs, report.py)
need "python: yaml + jsonschema" "python3 -c 'import yaml, jsonschema'" "pip install pyyaml jsonschema"
# ansible collections the plays and the ledger role use
need "ansible.posix + community.docker collections" "ansible-galaxy collection list 2>/dev/null | grep -q community.docker" "ansible-galaxy collection install -r ops/ansible/requirements.yml"
# the Go tools the overlay builds link piv-go (the YubiKey path) even in dev mode
case "$(uname -s)" in
  Linux) need "libpcsclite (ca-init links piv-go)" "pkg-config --exists libpcsclite" "apt-get install libpcsclite-dev pkg-config" ;;
esac
if [ "$HOSTS" = 1 ]; then
  need "Linux (container addresses reachable from the controller)" "[ \"\$(uname -s)\" = Linux ]" "run with --no-hosts on macOS"
  need "binary docker" "command -v docker" "install Docker"
  need "Docker daemon reachable" "docker info" "start the Docker daemon"
  need "binary openssl (the registry's TLS certificate)" "command -v openssl" "apt-get install openssl"
  need "passwordless sudo (/etc/hosts, /etc/docker/certs.d)" "sudo -n true" "run where sudo needs no password (the CI runner)"
  need "port 443 free for the hermetic registry" "! (exec 3<>/dev/tcp/127.0.0.1/443) 2>/dev/null" "free port 443"
fi
if [ "$missing" -gt 0 ]; then
  echo "prereqs: $missing missing — the hermetic tier would fail on it; install the above first"
  exit 1
fi
echo "prereqs: ready"
