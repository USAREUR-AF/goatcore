#!/usr/bin/env bash
# check-workstation.sh — the goatlite tutorial's laptop setup runs as written on
# a fresh Ubuntu 24.04 (docs/tutorials/00-goatlite.md section 1).
#
#   ops/tutorials/check-workstation.sh            run it (root, in a fresh ubuntu:24.04)
#   ops/tutorials/check-workstation.sh --lint     only the page checks; any machine
#
# The ```bash blocks under "On Ubuntu 24.04" and "Set up Python, Ansible and the
# browser" run in one shell, top to bottom, from the repository root, exactly as
# written. Around them, each prerequisite is proven by a positive run and a
# negative control:
#
#   apt     before the page's `apt update`, an install on the fresh image fails
#   python  the system Python is externally managed; the page's venv is the
#           python3 on PATH and imports yaml, jsonschema, jmespath
#   ansible the pinned ansible-core and the requirements.yml collections
#   render  goatlite-render.py mints a goatlite's CA with the declared toolchain;
#           with yq hidden it fails, naming yq
#   browser the page's launch check prints `chromium launched`; with libatk
#           removed the same launch fails
#
# CI: .github/workflows/goatlite-workstation.yml. sudo is a pass-through when
# the container has none (root needs no elevation).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PAGE="$ROOT/docs/tutorials/00-goatlite.md"
fail() { echo "FAIL  $*" >&2; exit 1; }
pass() { echo "ok    $*"; }

# The bash blocks under one heading, up to the next heading of the same or a higher level.
blocks() {
  awk -v want="$1" '
    /^#+ / { lvl = match($0, /[^#]/) - 1; title = substr($0, lvl + 2)
             if (!infence) { if (on && lvl <= level) on = 0; if (title == want) { on = 1; level = lvl } } }
    /^```/ { if (infence) { infence = 0; next } infence = 1; keep = on && ($0 == "```bash"); next }
    infence && keep { print }
  ' "$PAGE"
}

# ---- page checks (also --lint) ------------------------------------------------
ubuntu="$(blocks "On Ubuntu 24.04")"
checkout="$(blocks "Set up Python, Ansible and the browser")"
[ -n "$ubuntu" ] || fail "no bash block under \"On Ubuntu 24.04\" in ${PAGE#"$ROOT"/}"
[ -n "$checkout" ] || fail "no bash block under \"Set up Python, Ansible and the browser\" in ${PAGE#"$ROOT"/}"
! grep -q -- '--break-system-packages' "$PAGE" || fail "the page overrides the system Python (--break-system-packages); the venv is the route"
first_apt="$(printf '%s\n' "$ubuntu" | grep -m1 -E '^sudo apt(-get)? ' || true)"
case "$first_apt" in *" update"*) ;; *) fail "the first apt command is not apt update: $first_apt" ;; esac
printf '%s\n' "$checkout" | grep -q 'python3 -m venv' || fail "no venv is created"
printf '%s\n' "$checkout" | grep -q "ansible-core==" || fail "ansible-core is not pinned"
printf '%s\n' "$checkout" | grep -q 'ansible-galaxy collection install -r ops/ansible/requirements.yml' || fail "the collections are not installed"
printf '%s\n' "$checkout" | grep -q 'playwright install --with-deps chromium' || fail "Playwright is installed without its system libraries"
printf '%s\n' "$checkout" | grep -q 'chromium launched' || fail "no browser launch check"
pass "the page declares apt update, a venv, pinned ansible-core + collections, Playwright --with-deps and a launch check"
[ "${1:-}" = "--lint" ] && exit 0

[ "$(id -u)" = 0 ] || fail "run as root in a fresh ubuntu:24.04 container"
export DEBIAN_FRONTEND=noninteractive
command -v sudo >/dev/null || { printf '#!/bin/sh\nexec "$@"\n' > /usr/local/bin/sudo; chmod 0755 /usr/local/bin/sudo; }

# ---- apt: the fresh image cannot install before an update ---------------------
if apt-get install -y -qq git >/dev/null 2>&1; then
  fail "apt installed a package before apt update: this image is not fresh, so the control proves nothing"
fi
pass "negative control: apt install before apt update fails on the fresh image"

# ---- the page, as written, then the checks in the same shell ------------------
work="$(mktemp -d)"
cat > "$work/run.sh" <<SCRIPT
set -euo pipefail
$ubuntu
$checkout

set -euo pipefail
fail() { echo "FAIL  \$*" >&2; exit 1; }
pass() { echo "ok    \$*"; }
pass "the page's Ubuntu 24.04 and checkout blocks ran to exit 0"

# python: the venv, not the system interpreter
ls /usr/lib/python3*/EXTERNALLY-MANAGED >/dev/null 2>&1 || fail "the system Python is not externally managed: the control proves nothing"
case "\$(command -v python3)" in "\$HOME/.venvs/goatlite/bin/python3") ;; *) fail "python3 on PATH is \$(command -v python3), not the venv" ;; esac
python3 -c 'import yaml, jsonschema, jmespath' || fail "the venv lacks a module"
pass "python3 is the venv's (the system one is externally managed) and imports yaml, jsonschema, jmespath"

# ansible
ansible-playbook --version | head -1 | grep -q 'core 2.20.11' || fail "ansible-playbook is not ansible-core 2.20.11: \$(ansible-playbook --version | head -1)"
for c in ansible.posix community.general community.docker community.crypto; do
  ansible-galaxy collection list "\$c" 2>/dev/null | grep -q "^\$c " || fail "collection \$c is not installed"
done
pass "ansible-core 2.20.11 with the requirements.yml collections"

# yq is mikefarah's
yq --version 2>&1 | grep -qi mikefarah || fail "yq is not mikefarah's: \$(yq --version 2>&1)"

# render: the declared toolchain mints a goatlite's CA; without yq it names yq
# A learner's checkout is a git clone; actions/checkout in a container with no
# git yet has none, so make it one (the render asks git what is ignored).
[ -d .git ] || git init -q .
git config --global --add safe.directory "\$PWD"
mkdir -p "\$HOME/.ssh"; [ -f "\$HOME/.ssh/id_ed25519.pub" ] || ssh-keygen -q -t ed25519 -N '' -f "\$HOME/.ssh/id_ed25519"
cat > goatlite.yaml <<'YAML'
schema: goatbuilder/v1
preset: lite
lite:
  name: cleanroom
  host: 192.0.2.50
  ssh_user: ubuntu
  admin: learner@example.org
  disk_encryption: false
YAML
yq_path="\$(command -v yq)"; mv "\$yq_path" "\$yq_path.hidden"
if out="\$(python3 ops/scripts/goatlite-render.py --site cleanroom 2>&1)"; then fail "the render passed with yq absent"; fi
mv "\$yq_path.hidden" "\$yq_path"
printf '%s\n' "\$out" | grep -q 'yq is required' || { printf '%s\n' "\$out" | tail -20; fail "the render failed without yq but did not name it"; }
pass "negative control: with yq absent the render fails and names yq"
python3 ops/scripts/goatlite-render.py --site cleanroom || fail "the render failed with the declared toolchain"
[ -s ops/enrollment/ca/cleanroom/root-cert.pem ] || fail "no root certificate rendered"
# The state directory the render keeps the CA key in: this checkout's
# deploy.state_dir (ops/goatbuilder/project.yml), else ~/.goat/state.
state_dir="\$(python3 ops/scripts/goatnet-facts.py deploy state_dir 2>/dev/null || true)"; state_dir="\${state_dir:-\$HOME/.goat/state}"
[ "\$(stat -c %a "\$state_dir/cleanroom-ca/ca.key.age")" = 600 ] || fail "the CA key is not mode 0600"
[ "\$(stat -c %a "\$HOME/.config/age/keys.txt")" = 600 ] || fail "the age identity is not mode 0600"
pass "the render minted the goatlite's CA with the declared toolchain (key and identity 0600)"

# browser: the launch check needs the system libraries --with-deps installed
launch="(cd ops/goat-id-console/web/e2e && node -e \"import('@playwright/test').then(({ chromium }) => chromium.launch()).then((b) => { console.log('chromium launched'); return b.close(); })\")"
eval "\$launch" | grep -q 'chromium launched' || fail "chromium did not launch"
pass "chromium launches"
dpkg -r --force-depends libatk1.0-0t64 >/dev/null 2>&1 || fail "could not remove libatk for the control"
if out="\$(eval "\$launch" 2>&1)"; then fail "chromium launched with libatk removed"; fi
printf '%s\n' "\$out" | grep -q 'libatk' || { printf '%s\n' "\$out" | tail -10; fail "the launch failed without libatk but did not name it"; }
pass "negative control: with libatk removed the launch fails and names it"
SCRIPT
cd "$ROOT"
bash "$work/run.sh"
