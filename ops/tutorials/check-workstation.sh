#!/usr/bin/env bash
# check-workstation.sh — the goatlite tutorial's laptop setup runs as written on
# a fresh Ubuntu 24.04 (docs/tutorials/00-goatlite.md section 1).
#
#   ops/tutorials/check-workstation.sh            run it (root, in a fresh ubuntu:24.04)
#   ops/tutorials/check-workstation.sh --macos    run the macOS path (a Mac with Homebrew)
#   ops/tutorials/check-workstation.sh --lint     only the page checks; any machine
#
# The ```bash blocks under "On Ubuntu 24.04" and "Set up Python, Ansible and the
# browser" run in one shell, top to bottom, from the repository root, exactly as
# written. Around them, each prerequisite is proven by a positive run and a
# negative control:
#
#   apt     before the page's `apt update`, an install on the fresh image fails
#   rust    the page's rustup block runs unattended and leaves cargo on PATH
#   python  the system Python is externally managed; the page's venv is the
#           python3 on PATH and imports yaml, jsonschema, jmespath
#   ansible the pinned ansible-core and the requirements.yml collections
#   render  goatlite-render.py mints a goatlite's CA with the declared toolchain;
#           with yq hidden it fails, naming yq
#   browser the page's launch check prints `chromium launched`; with libatk
#           removed the same launch fails
#
# --macos runs the ```bash blocks under "On macOS" and "Set up Python, Ansible
# and the browser" the same way, on a Mac that already has Homebrew (the page
# installs Homebrew in a ```text block). Its controls:
#
#   python  python3 is the venv's and is Python 3.14; a venv made with
#           Homebrew's python@3.15 cannot start ansible-playbook, which is why
#           the page installs python@3.14
#   rust    cargo is on PATH from rustup, not Homebrew's keg-only formula
#   render  as above
#   browser the page's launch check prints `chromium launched`
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
macos="$(blocks "On macOS")"
checkout="$(blocks "Set up Python, Ansible and the browser")"
[ -n "$ubuntu" ] || fail "no bash block under \"On Ubuntu 24.04\" in ${PAGE#"$ROOT"/}"
[ -n "$macos" ] || fail "no bash block under \"On macOS\" in ${PAGE#"$ROOT"/}"
printf '%s\n' "$macos" | grep -q 'brew install .*python@3\.14' || fail "macOS does not install python@3.14 (ansible-core 2.20 stops on Homebrew's python, 3.15)"
printf '%s\n' "$macos" | grep -q 'libexec/bin:\$PATH' || fail "macOS does not put python@3.14's python3 first on PATH before the venv is made"
printf '%s\n' "$macos" | grep -q 'sh.rustup.rs | sh -s -- -y' || fail "macOS does not install Rust with rustup -y"
! printf '%s\n' "$macos" | grep -qE 'brew install .*(rustup|python( |$))' || fail "macOS installs Homebrew's rustup or python formula (no cargo on PATH / Python 3.15)"
[ -n "$checkout" ] || fail "no bash block under \"Set up Python, Ansible and the browser\" in ${PAGE#"$ROOT"/}"
! grep -q -- '--break-system-packages' "$PAGE" || fail "the page overrides the system Python (--break-system-packages); the venv is the route"
first_apt="$(printf '%s\n' "$ubuntu" | grep -m1 -E '^sudo apt(-get)? ' || true)"
case "$first_apt" in *" update"*) ;; *) fail "the first apt command is not apt update: $first_apt" ;; esac
printf '%s\n' "$ubuntu" | grep -q 'sh.rustup.rs | sh -s -- -y' || fail "Rust is not installed non-interactively (rustup -y)"
printf '%s\n' "$checkout" | grep -q 'python3 -m venv' || fail "no venv is created"
printf '%s\n' "$checkout" | grep -q "ansible-core==" || fail "ansible-core is not pinned"
printf '%s\n' "$checkout" | grep -q 'ansible-galaxy collection install -r ops/ansible/requirements.yml' || fail "the collections are not installed"
printf '%s\n' "$checkout" | grep -q 'playwright install --with-deps chromium' || fail "Playwright is installed without its system libraries"
printf '%s\n' "$checkout" | grep -q 'chromium launched' || fail "no browser launch check"
pass "the page declares apt update, rustup -y, a venv, pinned ansible-core + collections, Playwright --with-deps and a launch check"
pass "macOS: python@3.14 first on PATH, rustup -y, neither Homebrew rustup nor Homebrew python"
[ "${1:-}" = "--lint" ] && exit 0

# The checks both paths share, run in the page's shell after its blocks.
common_checks() {
  cat <<'COMMON'
# ansible
ansible-playbook --version | head -1 | grep -q 'core 2.20.11' || fail "ansible-playbook is not ansible-core 2.20.11: $(ansible-playbook --version 2>&1 | tail -1)"
for c in ansible.posix community.general community.docker community.crypto; do
  ansible-galaxy collection list "$c" 2>/dev/null | grep -q "^$c " || fail "collection $c is not installed"
done
pass "ansible-core 2.20.11 with the requirements.yml collections"
cargo --version >/dev/null || fail "cargo is not on PATH after the page's rustup block"
pass "cargo is on PATH: $(cargo --version)"
yq --version 2>&1 | grep -qi mikefarah || fail "yq is not mikefarah's: $(yq --version 2>&1)"
COMMON
}

if [ "${1:-}" = "--macos" ]; then
  [ "$(uname -s)" = Darwin ] || fail "--macos runs on a Mac"
  command -v brew >/dev/null || fail "--macos needs Homebrew (the page installs it in a text block first)"
  work="$(mktemp -d)"
  {
    echo 'set -euo pipefail'
    printf '%s\n' "$macos" "$checkout"
    cat <<'SCRIPT'
set -euo pipefail
fail() { echo "FAIL  $*" >&2; exit 1; }
pass() { echo "ok    $*"; }
pass "the page's macOS and checkout blocks ran to exit 0"
case "$(command -v python3)" in "$HOME/.venvs/goatlite/bin/python3") ;; *) fail "python3 on PATH is $(command -v python3), not the venv" ;; esac
python3 -c 'import sys; sys.exit(0 if sys.version_info[:2] == (3, 14) else 1)' || fail "the venv is not Python 3.14: $(python3 --version)"
python3 -c 'import yaml, jsonschema, jmespath' || fail "the venv lacks a module"
pass "python3 is the venv's, Python 3.14, and imports yaml, jsonschema, jmespath"
SCRIPT
    common_checks
    cat <<'SCRIPT'
# rust came from the page's rustup installer (a runner may also carry
# Homebrew's keg-only rustup on PATH; a clean Mac has neither)
"$HOME/.cargo/bin/cargo" --version >/dev/null 2>&1 || fail "the page's rustup block left no cargo in ~/.cargo/bin"
pass "the page's rustup block installed cargo in ~/.cargo/bin"

# the control: the Python the page steers away from cannot start ansible-core 2.20
brew list python@3.15 >/dev/null 2>&1 || brew install python@3.15 >/dev/null
bad="$(mktemp -d)/venv"; "$(brew --prefix python@3.15)/bin/python3.15" -m venv "$bad"
"$bad/bin/python3" -m pip install -q --disable-pip-version-check 'ansible-core==2.20.11'
if out="$("$bad/bin/ansible-playbook" --version 2>&1)"; then fail "ansible-core 2.20.11 started on Python 3.15: the pin to python@3.14 is not needed"; fi
printf '%s\n' "$out" | grep -q "_is_type" || { printf '%s\n' "$out" | tail -5; fail "ansible failed on 3.15 but not with the error the page names"; }
pass "negative control: on Homebrew's python@3.15 ansible-playbook stops with the error the page names"

[ -d .git ] || git init -q .
mkdir -p "$HOME/.ssh"; [ -f "$HOME/.ssh/id_ed25519.pub" ] || ssh-keygen -q -t ed25519 -N '' -f "$HOME/.ssh/id_ed25519"
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
python3 ops/scripts/goatlite-render.py --site cleanroom || fail "the render failed with the declared toolchain"
[ -s ops/enrollment/ca/cleanroom/root-cert.pem ] || fail "no root certificate rendered"
state_dir="$(python3 ops/scripts/goatnet-facts.py deploy state_dir 2>/dev/null || true)"; state_dir="${state_dir:-$HOME/.goat/state}"
[ "$(stat -f %Lp "$state_dir/cleanroom-ca/ca.key.age")" = 600 ] || fail "the CA key is not mode 0600"
pass "the render minted the goatlite's CA with the declared toolchain"

(cd ops/goat-id-console/web/e2e && node -e "import('@playwright/test').then(({ chromium }) => chromium.launch()).then((b) => { console.log('chromium launched'); return b.close(); })") | grep -q 'chromium launched' || fail "chromium did not launch"
pass "chromium launches"
SCRIPT
  } > "$work/run.sh"
  cd "$ROOT"
  exec /bin/bash "$work/run.sh"
fi

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

# cargo: the page's rustup block put it on PATH (the build in "Get the checkout" needs it)
cargo --version >/dev/null || fail "cargo is not on PATH after the page's rustup block"
pass "cargo is on PATH: \$(cargo --version)"

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
