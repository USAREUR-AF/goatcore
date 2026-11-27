#!/usr/bin/env bash
# laptop-provision.sh — runs INSIDE the lab's laptop VM, before the clock, with outside
# network. It does what docs/tutorials/00-goatlite.md section 1 tells a learner on
# Ubuntu 24.04 to do: the packages, yq, Go, Rust, the checkout, the two programs, the
# Python environment with Ansible, the browser for the acceptance checks. Then it fills
# the Go module cache phase 1's certificate tools build from, and writes the network
# the clock runs on. Lab only (ops/vm-image/zero-saas/README.md).
#
#   bash laptop-provision.sh <git-bundle> <commit>
set -euo pipefail
BUNDLE="$1"; COMMIT="$2"
GO_VERSION="${GO_VERSION:-1.26.11}"; YQ_VERSION="${YQ_VERSION:-v4.44.3}"
ARCH="$(dpkg --print-architecture)"
say() { printf '\n=== %s ===\n' "$*"; }

say "packages (tutorial 00 section 1, Ubuntu 24.04)"
sudo cloud-init status --wait >/dev/null || true
sudo apt-get -o Acquire::Retries=3 update -qq
sudo DEBIAN_FRONTEND=noninteractive apt-get -o Acquire::Retries=3 install -y -qq \
  git openssh-client curl ca-certificates build-essential python3 python3-venv nodejs npm age pkg-config libpcsclite-dev jq

say "yq $YQ_VERSION, Go $GO_VERSION, Rust"
sudo curl -fsSL -o /usr/local/bin/yq "https://github.com/mikefarah/yq/releases/download/$YQ_VERSION/yq_linux_$ARCH"
sudo chmod 0755 /usr/local/bin/yq
curl -fsSL "https://go.dev/dl/go$GO_VERSION.linux-$ARCH.tar.gz" | sudo tar -C /usr/local -xz
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null
# shellcheck disable=SC1091
. "$HOME/.cargo/env"
cat >> "$HOME/.profile" <<'EOF'
export PATH="$HOME/.local/bin:/usr/local/go/bin:$HOME/.cargo/bin:$PATH"
[ -f "$HOME/.venvs/goatlite/bin/activate" ] && . "$HOME/.venvs/goatlite/bin/activate"
EOF
export PATH="$HOME/.local/bin:/usr/local/go/bin:$PATH"

say "the checkout at $COMMIT"
git clone -q "$BUNDLE" "$HOME/goatcore"
# The checkout a learner's clone of the published repository is at that commit: main and
# origin/main both name it, and origin is the published repository.
git -C "$HOME/goatcore" checkout -q -B main "$COMMIT"
git -C "$HOME/goatcore" update-ref refs/remotes/origin/main "$COMMIT"
git -C "$HOME/goatcore" branch -q --set-upstream-to=origin/main main
git -C "$HOME/goatcore" remote set-url origin https://github.com/usareur-af/goatcore.git
cd "$HOME/goatcore"

say "goatbuilder and goat, built from the checkout"
(cd crates && cargo build -q --release --locked -p goatbuilder -p goat-cli)
mkdir -p "$HOME/.local/bin"
install -m 0755 crates/target/release/goatbuilder crates/target/release/goat "$HOME/.local/bin/"
goatbuilder --version; goat --version
rm -rf crates/target  # the programs are installed; the build tree is not needed on the clock

say "Python, Ansible and the collections"
python3 -m venv "$HOME/.venvs/goatlite"
# shellcheck disable=SC1091
. "$HOME/.venvs/goatlite/bin/activate"
python3 -m pip install -q pyyaml jsonschema jmespath 'ansible-core==2.20.11'
ansible-galaxy collection install -r ops/ansible/requirements.yml >/dev/null

say "the browser for the acceptance checks"
(cd ops/goat-id-console/web/e2e && npm ci --no-audit --no-fund >/dev/null && npx playwright install --with-deps chromium >/dev/null)

say "phase 1's certificate tools: their Go modules, fetched now"
(cd ops/enrollment && go mod download)
# Phase 5 builds ops/time-hygiene on this machine.
(cd ops/time-hygiene && go mod download)

say "an SSH key, the box's administrator key"
[ -f "$HOME/.ssh/id_ed25519" ] || ssh-keygen -q -t ed25519 -N '' -f "$HOME/.ssh/id_ed25519" -C lab-laptop

say "the clock's network, used from the next boot"
# The next boot is the clock: the outside NIC is qemu user networking with restrict=on,
# so the default route goes to its gateway, which drops everything, and every attempt is
# on the wire the lab records. Names resolve through the box, as tutorial 00 section 5
# step 1 offers ("set the laptop's DNS server to the box's address").
sudo tee /etc/cloud/cloud.cfg.d/99-lab-no-network.cfg >/dev/null <<'EOF'
network: {config: disabled}
EOF
sudo rm -f /etc/netplan/50-cloud-init.yaml
sudo tee /etc/netplan/60-lab.yaml >/dev/null <<'EOF'
network:
  version: 2
  ethernets:
    up:
      match: {macaddress: "52:54:00:00:01:01"}  # opsec-allow: synthetic QEMU NIC address the lab assigns to its own VM
      addresses: [10.0.2.15/24, "fec0::15/64"]
      routes: [{to: default, via: 10.0.2.2}, {to: "::/0", via: "fec0::2"}]
    lan:
      match: {macaddress: "52:54:00:00:01:02"}  # opsec-allow: synthetic QEMU NIC address the lab assigns to its own VM
      addresses: [10.88.0.1/24, "fd88::1/64"]
      nameservers: {addresses: [10.88.0.2]}
EOF
sudo chmod 0600 /etc/netplan/60-lab.yaml
say "done"
