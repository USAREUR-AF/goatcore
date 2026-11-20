#!/usr/bin/env bash
set -euo pipefail

# =============================================================
# 05-enable-fips.sh
# Run ON THE PI after rootfs encryption is complete and unlocked.
#
# Free FIPS-MODE path (no Ubuntu Pro, no NIST CMVP cert):
#   1. Add fips=1 (documented no-op on the stock linux-raspi kernel)
#   2. OpenSSL 3 FIPS provider built from apt-source + activated
#   3. SSH restricted to FIPS-approved Ciphers/MACs/KEX/HostKeyAlgs
#   4. dropbear-initramfs kept by default; optionally purged
#   5. initramfs rebuilt
#
# This produces a system that USES FIPS-approved algorithms but is
# NOT NIST-validated. For self-imposed hardening only.
#
# Usage: sudo bash 05-enable-fips.sh
# =============================================================

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'
BLUE='\033[0;34m'; CYAN='\033[0;36m'; WHITE='\033[1;37m'
BOLD='\033[1m'; NC='\033[0m'

info()  { printf "${CYAN}>${NC} %s\n" "$*"; }
step()  { printf "\n${BOLD}${BLUE}=== %s ===${NC}\n" "$*"; }
ok()    { printf "${GREEN}[ok]${NC} %s\n" "$*"; }
warn()  { printf "${YELLOW}[warn]${NC} %s\n" "$*" >&2; }
error() { printf "${RED}[error]${NC} %s\n" "$*" >&2; }

banner() {
    echo ""
    printf "${BLUE}${BOLD}  ┌──────────────────────────────────┐${NC}\n"
    printf "${BLUE}${BOLD}  │  ${WHITE}ODT ${NC}${BLUE}${BOLD}│${NC} ${GREEN}%-24s${BLUE}${BOLD}  │${NC}\n" "$1"
    printf "${BLUE}${BOLD}  └──────────────────────────────────┘${NC}\n\n"
}

cleanup() {
    if [ $? -ne 0 ]; then
        error "Script failed. Check the output above."
    fi
}
trap cleanup EXIT

if [ "$(id -u)" -ne 0 ]; then
    error "Run as root (sudo bash 05-enable-fips.sh)"
    exit 1
fi

banner "Enable FIPS Mode"

# Sanity: must run after encryption is set up
if ! cryptsetup status rootfs >/dev/null 2>&1; then
    error "rootfs mapper not active. Complete 03-prepare-encryption.sh and encrypt-rootfs.sh first."
    exit 1
fi

ARCH_TRIPLET="$(dpkg-architecture -qDEB_HOST_MULTIARCH 2>/dev/null || echo aarch64-linux-gnu)"
OSSL_MOD_DIR="/usr/lib/${ARCH_TRIPLET}/ossl-modules"
FIPS_CNF="/usr/lib/ssl/fipsmodule.cnf"
OSSL_CNF="/usr/lib/ssl/openssl.cnf"

# ---------------------------------------------------------------
# Step 1: kernel fips=1
# ---------------------------------------------------------------
step "Step 1/6: Setting kernel fips=1"

CMDLINE="/boot/firmware/cmdline.txt"
[ -f "$CMDLINE" ] || CMDLINE="/boot/cmdline.txt"

if [ ! -f "$CMDLINE" ]; then
    error "cmdline.txt not found"
    exit 1
fi

if grep -q 'fips=1' "$CMDLINE"; then
    ok "fips=1 already present in $CMDLINE"
else
    sed -i 's|$| fips=1|' "$CMDLINE"
    ok "Added fips=1 to $CMDLINE"
fi

# The stock Ubuntu linux-raspi kernel is built without CONFIG_CRYPTO_FIPS.
# It ignores fips=1, does not run kernel FIPS self-tests, and does not expose
# /proc/sys/crypto/fips_enabled. The userspace OpenSSL provider is independent.

# ---------------------------------------------------------------
# Step 2: enable deb-src and install build deps
# ---------------------------------------------------------------
step "Step 2/6: Enabling deb-src + installing build deps"

# Ubuntu 24.04 uses deb822 sources at /etc/apt/sources.list.d/ubuntu.sources
UBUNTU_SOURCES="/etc/apt/sources.list.d/ubuntu.sources"
if [ -f "$UBUNTU_SOURCES" ]; then
    if grep -qE '^Types: deb$' "$UBUNTU_SOURCES"; then
        sed -i 's/^Types: deb$/Types: deb deb-src/' "$UBUNTU_SOURCES"
        ok "Enabled deb-src in $UBUNTU_SOURCES"
    else
        ok "deb-src already enabled"
    fi
else
    # Fallback for older /etc/apt/sources.list layout
    sed -i 's/^# *deb-src/deb-src/' /etc/apt/sources.list || true
fi

apt-get update -qq
apt-get install -y -qq build-essential perl libssl-dev dpkg-dev

# ---------------------------------------------------------------
# Step 3: build OpenSSL FIPS provider from apt source
# ---------------------------------------------------------------
step "Step 3/6: Building OpenSSL FIPS provider (~10–20 min on Pi 5)"

if [ -f "$OSSL_MOD_DIR/fips.so" ] && [ -f "$FIPS_CNF" ]; then
    ok "fips.so + fipsmodule.cnf already installed — skipping build"
else
    BUILD_DIR="/var/tmp/openssl-fips-build"
    mkdir -p "$BUILD_DIR"
    cd "$BUILD_DIR"

    info "Fetching openssl source via apt..."
    apt-get source openssl
    OSSL_SRC=$(find . -maxdepth 1 -type d -name 'openssl-*' | head -1)
    if [ -z "$OSSL_SRC" ]; then
        error "apt-get source openssl produced no source tree"
        exit 1
    fi
    cd "$OSSL_SRC"

    info "Configuring with enable-fips..."
    ./Configure enable-fips \
        --prefix=/usr \
        --openssldir=/usr/lib/ssl \
        --libdir=lib/${ARCH_TRIPLET}

    info "Building (this is the long part)..."
    # NOTE: -j2, not $(nproc). On Pi 5 with NVMe + LUKS, full parallelism can
    # saturate the NVMe write queue (especially with DRAM-less drives like
    # WD SN770M), trigger NVMe controller resets, and cascade into ext4 I/O
    # errors that brick the rootfs. j2 is gentle enough to be safe.
    make -j2 build_libs
    # NOTE: -j2, not $(nproc). On Pi 5 with NVMe + LUKS, full parallelism can
    # saturate the NVMe write queue (especially with DRAM-less drives like
    # WD SN770M), trigger NVMe controller resets, and cascade into ext4 I/O
    # errors that brick the rootfs. j2 is gentle enough to be safe.
    make -j2 providers/fips.so

    info "Installing FIPS module..."
    install -d "$OSSL_MOD_DIR"
    install -m 0644 providers/fips.so "$OSSL_MOD_DIR/fips.so"

    info "Running fipsinstall self-tests + writing fipsmodule.cnf..."
    /usr/bin/openssl fipsinstall \
        -out "$FIPS_CNF" \
        -module "$OSSL_MOD_DIR/fips.so"

    ok "FIPS provider installed at $OSSL_MOD_DIR/fips.so"
fi

# ---------------------------------------------------------------
# Step 4: activate FIPS provider in openssl.cnf
# ---------------------------------------------------------------
step "Step 4/6: Activating FIPS provider in $OSSL_CNF"

# Backup once
[ -f "${OSSL_CNF}.pre-fips" ] || cp "$OSSL_CNF" "${OSSL_CNF}.pre-fips"

# Strip any previously-appended FIPS block from earlier runs (idempotent).
# Earlier versions of this script appended a duplicate [provider_sect],
# which OpenSSL silently ignored (it reads the FIRST [provider_sect]).
sed -i '/^# ----- FIPS provider activation/,$d' "$OSSL_CNF"

# Activate the FIPS provider by uncommenting Ubuntu's existing scaffolding
# in /usr/lib/ssl/openssl.cnf. The shipped file has commented-out lines
# specifically for this — we just enable them.
sed -i \
    -e "s|^# \\.include fipsmodule\\.cnf|.include $FIPS_CNF|" \
    -e "s|^# fips = fips_sect|fips = fips_sect|" \
    -e "s|^# activate = 1\$|activate = 1|" \
    "$OSSL_CNF"

if openssl list -providers 2>/dev/null | grep -q 'name: OpenSSL FIPS'; then
    ok "openssl.cnf updated — FIPS provider is active"
else
    error "FIPS provider still not active after openssl.cnf edit. Inspect:"
    error "  /usr/lib/ssl/openssl.cnf"
    error "  Look for [provider_sect] — should have 'fips = fips_sect' uncommented"
    exit 1
fi

# Verify provider activates
if openssl list -providers 2>/dev/null | grep -q '^  *name: OpenSSL FIPS'; then
    ok "openssl list -providers reports FIPS provider active"
else
    warn "FIPS provider not yet visible to openssl — will activate on reboot"
fi

# ---------------------------------------------------------------
# Step 5: SSH hardening (FIPS-approved algorithms only)
# ---------------------------------------------------------------
step "Step 5/6: Restricting SSHD to FIPS-approved algorithms"

mkdir -p /etc/ssh/sshd_config.d
cat > /etc/ssh/sshd_config.d/10-fips.conf <<'EOF'
# FIPS-approved SSH crypto suite. Ed25519 retained for usability
# (FIPS 186-5 lists EdDSA; OpenSSH ed25519 does not depend on libcrypto).
Ciphers aes256-gcm@openssh.com,aes128-gcm@openssh.com,aes256-ctr,aes192-ctr,aes128-ctr
MACs hmac-sha2-512,hmac-sha2-256,hmac-sha2-512-etm@openssh.com,hmac-sha2-256-etm@openssh.com
KexAlgorithms ecdh-sha2-nistp521,ecdh-sha2-nistp384,ecdh-sha2-nistp256,diffie-hellman-group16-sha512,diffie-hellman-group14-sha256
HostKeyAlgorithms rsa-sha2-512,rsa-sha2-256,ecdsa-sha2-nistp521,ecdsa-sha2-nistp384,ecdsa-sha2-nistp256,ssh-ed25519
PubkeyAcceptedAlgorithms rsa-sha2-512,rsa-sha2-256,ecdsa-sha2-nistp521,ecdsa-sha2-nistp384,ecdsa-sha2-nistp256,ssh-ed25519
EOF

# Drop DSA host key — not FIPS-approved at any size
[ -f /etc/ssh/ssh_host_dsa_key ] && rm -f /etc/ssh/ssh_host_dsa_key /etc/ssh/ssh_host_dsa_key.pub
ok "SSHD hardened (drop-in: /etc/ssh/sshd_config.d/10-fips.conf)"

# Sanity-check config before restart
if sshd -t; then
    systemctl restart ssh || systemctl restart sshd || true
    ok "sshd restarted with FIPS-approved suite"
else
    error "sshd config check failed — not restarting. Fix before reboot."
    exit 1
fi

# ---------------------------------------------------------------
# Step 6: apply dropbear policy + rebuild initramfs
# ---------------------------------------------------------------
step "Step 6/6: Applying dropbear policy + rebuilding initramfs"

# Dropbear policy: dropbear's algos are FIPS-approved (AES-GCM/CTR, HMAC-SHA2,
# ECDH-NIST), but it bundles its own crypto (libtomcrypt) instead of going
# through the validated OpenSSL FIPS provider. Strict FIPS posture purges it;
# pragmatic posture keeps it for remote LUKS unlock over Ethernet.
#
# Default: KEEP dropbear (more useful in the field). Set FIPS_PURGE_DROPBEAR=1
# to remove it for strict-mode builds.
FIPS_PURGE_DROPBEAR="${FIPS_PURGE_DROPBEAR:-0}"

if [ "$FIPS_PURGE_DROPBEAR" = "1" ]; then
    if dpkg -l dropbear-initramfs 2>/dev/null | grep -q '^ii'; then
        apt-get purge -y -qq dropbear-initramfs dropbear-bin || true
        ok "dropbear-initramfs removed (FIPS_PURGE_DROPBEAR=1)"
    else
        ok "dropbear-initramfs already absent"
    fi

    # With dropbear gone, the IP= line in initramfs.conf is dead AND harmful:
    # netplan inherits it on hand-off and writes /run/netplan/eth0.yaml,
    # causing both interfaces to claim the same IP on WiFi-only setups.
    if grep -q '^IP=' /etc/initramfs-tools/initramfs.conf; then
        sed -i '/^IP=/d' /etc/initramfs-tools/initramfs.conf
        ok "Removed dead IP= line from initramfs.conf"
    fi
else
    ok "Keeping dropbear-initramfs (set FIPS_PURGE_DROPBEAR=1 to remove)"
    warn "Note: dropbear's libtomcrypt is not a NIST-validated module."
    warn "      Algorithms used are FIPS-approved; implementation is not certified."
fi

update-initramfs -u
ok "initramfs rebuilt"

# ---------------------------------------------------------------
# Verification summary
# ---------------------------------------------------------------
step "Verification (post-reboot checks listed below)"

printf "\n${GREEN}${BOLD}=========================================${NC}\n"
printf "${GREEN}${BOLD}  FIPS-mode setup complete.${NC}\n"
echo ""
echo "  Reboot now, then verify on the running system:"
echo ""
printf "    ${CYAN}cat /proc/sys/crypto/fips_enabled${NC}      # absent on stock linux-raspi\n"
printf "    ${CYAN}openssl list -providers${NC}                # FIPS provider listed\n"
printf "    ${CYAN}sshd -T | egrep '^(ciphers|macs|kexalgorithms)'${NC}\n"
printf "    ${CYAN}cryptsetup luksDump /dev/nvme0n1p2 | grep -i pbkdf${NC}  # pbkdf2 (not argon2id)\n"
echo ""
echo "  This image is FIPS-MODE (uses approved algorithms)."
echo "  It is NOT NIST CMVP-validated. No certificate # exists."
echo ""
echo "  After reboot + verification, run:"
printf "${GREEN}${BOLD}=========================================${NC}\n"
