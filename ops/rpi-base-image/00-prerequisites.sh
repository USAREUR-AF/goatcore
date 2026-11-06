#!/usr/bin/env bash
# The colour variables are fixed escape codes, so they are safe in printf formats.
# shellcheck disable=SC2059
set -euo pipefail

# =============================================================
# 00-prerequisites.sh
# Run on your workstation (macOS or Linux) before starting the golden
# image build. Checks for the required tools, downloads the Ubuntu image
# and puts your SSH key into user-data.
#
# macOS: installs missing tools with Homebrew.
# Linux: finds the tools that are missing, names the apt / dnf / pacman
#        command that installs them, and runs it only if you answer y.
#
# Usage: ./00-prerequisites.sh
# =============================================================

# --- Colors & helpers ---
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
WHITE='\033[1;37m'
BOLD='\033[1m'
NC='\033[0m'

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

banner "Prerequisites Setup"

OS="$(uname -s)"
case "$OS" in
    Darwin|Linux) ;;
    *) error "Unsupported OS: $OS (macOS or Linux only)"; exit 1 ;;
esac

# sed_inplace <sed args...> <file>: `sed -i.bak` is the in-place form GNU and
# BSD sed both accept; the backup is removed afterwards.
sed_inplace() {
    local file="${*: -1}"
    sed -i.bak "$@"
    rm -f "${file}.bak"
}

# linux_pkg_manager: apt, dnf or pacman, whichever is on PATH first.
linux_pkg_manager() {
    if command -v apt-get &>/dev/null; then echo apt
    elif command -v dnf &>/dev/null; then echo dnf
    elif command -v pacman &>/dev/null; then echo pacman
    fi
}

# linux_pkg_name <manager> <tool>: the package that provides the tool.
linux_pkg_name() {
    case "$1:$2" in
        apt:xz) echo xz-utils ;;
        *) echo "$2" ;;
    esac
}

# linux_install_cmd <manager> <packages...>
linux_install_cmd() {
    local mgr="$1"; shift
    case "$mgr" in
        apt)    echo "sudo apt-get install -y $*" ;;
        dnf)    echo "sudo dnf install -y $*" ;;
        pacman) echo "sudo pacman -S --needed $*" ;;
    esac
}

if [[ "$OS" == "Darwin" ]]; then
    # --- Check Homebrew ---
    step "Step 1: Checking Homebrew"
    if ! command -v brew &>/dev/null; then
        warn "Homebrew not found. Installing..."
        /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
        ok "Homebrew installed"
    else
        ok "Homebrew found"
    fi

    # --- Install tools ---
    step "Step 2: Installing required tools"

    TOOLS=(pv nmap ansible xz)
    for tool in "${TOOLS[@]}"; do
        if command -v "$tool" &>/dev/null; then
            ok "$tool already installed"
        else
            info "Installing $tool..."
            brew install "$tool"
            ok "$tool installed"
        fi
    done
    TOOLS_SUMMARY="pv, nmap, ansible, xz"
else
    # --- Package manager ---
    step "Step 1: Checking the package manager"
    PKG_MGR="$(linux_pkg_manager)"
    if [ -n "$PKG_MGR" ]; then
        ok "Package manager: $PKG_MGR"
    else
        warn "No apt-get, dnf or pacman found; install missing tools by hand"
    fi

    # --- Check tools ---
    step "Step 2: Checking required tools"
    MISSING=""
    MISSING_PKGS=""
    for tool in xz pv nmap curl; do
        if command -v "$tool" &>/dev/null; then
            ok "$tool found"
        else
            warn "$tool not found"
            MISSING="$MISSING $tool"
            MISSING_PKGS="$MISSING_PKGS $(linux_pkg_name "$PKG_MGR" "$tool")"
        fi
    done
    if [ -n "$MISSING" ]; then
        if [ -z "$PKG_MGR" ]; then
            error "Missing:$MISSING. Install them with your distribution's package manager, then re-run."
            exit 1
        fi
        # shellcheck disable=SC2086 # the package list is meant to split
        INSTALL_CMD="$(linux_install_cmd "$PKG_MGR" $MISSING_PKGS)"
        info "Missing:$MISSING. They are installed with:"
        printf "    ${CYAN}%s${NC}\n" "$INSTALL_CMD"
        INSTALL=""
        read -rp "Run that command now? (y/n): " INSTALL || INSTALL=""
        if [ "$INSTALL" = "y" ]; then
            $INSTALL_CMD
            ok "Installed:$MISSING"
        else
            error "Install them, then re-run ./00-prerequisites.sh"
            exit 1
        fi
    fi
    if command -v ansible-galaxy &>/dev/null; then
        ok "ansible found"
    else
        warn "ansible not found. It is used for application provisioning after the base image, not by these scripts."
        if [ -n "$PKG_MGR" ]; then
            info "Install with: $(linux_install_cmd "$PKG_MGR" ansible)"
        fi
    fi
    TOOLS_SUMMARY="xz, pv, nmap, curl"
fi

# --- Ansible collections ---
step "Step 3: Ansible collections"
if ! command -v ansible-galaxy &>/dev/null; then
    warn "ansible-galaxy not found; skipping community.general"
elif ansible-galaxy collection list 2>/dev/null | grep -q "community.general"; then
    ok "community.general already installed"
else
    info "Installing community.general..."
    ansible-galaxy collection install community.general
    ok "community.general installed"
fi

# --- Download Ubuntu image ---
step "Step 4: Ubuntu Server image"

# Image cache resolution order:
#   1. $IMAGES_DIR env var if set (full override)
#   2. <repo-parent>/artifacts/images   (the standard SAUDI-ARABIA layout)
#   3. <repo-parent>/images             (legacy, pre-artifacts/ consolidation)
#   4. ~/Downloads                      (final fallback for fresh machines)
SCRIPT_PARENT="$(cd "$(dirname "$0")/.." && pwd)"
if   [ -n "${IMAGES_DIR:-}" ];                   then DOWNLOAD_DIR="$IMAGES_DIR"
elif [ -d "${SCRIPT_PARENT}/artifacts/images" ]; then DOWNLOAD_DIR="${SCRIPT_PARENT}/artifacts/images"
elif [ -d "${SCRIPT_PARENT}/images" ];           then DOWNLOAD_DIR="${SCRIPT_PARENT}/images"
else                                                  DOWNLOAD_DIR="$HOME/Downloads"
fi
ok "Image cache directory: $DOWNLOAD_DIR"

IMAGE_PATTERN="ubuntu-*-preinstalled-server-arm64+raspi.img.xz"
# Look in DOWNLOAD_DIR first, then in ~/Downloads as a fallback for legacy users
EXISTING=$(find "$DOWNLOAD_DIR" "$HOME/Downloads" -maxdepth 1 -name "$IMAGE_PATTERN" -type f 2>/dev/null | head -1)

if [ -n "$EXISTING" ]; then
    ok "Found image: $(basename "$EXISTING")"
    IMAGE_PATH="$EXISTING"
else
    info "Downloading Ubuntu Server 24.04 ARM64 preinstalled image..."
    IMAGE_URL="https://cdimage.ubuntu.com/releases/24.04/release/ubuntu-24.04.4-preinstalled-server-arm64+raspi.img.xz"
    IMAGE_PATH="$DOWNLOAD_DIR/ubuntu-24.04.4-preinstalled-server-arm64+raspi.img.xz"

    mkdir -p "$DOWNLOAD_DIR"
    if command -v curl &>/dev/null; then
        curl -L --progress-bar -o "$IMAGE_PATH" "$IMAGE_URL"
    else
        error "curl not found. Download manually from:"
        echo "  https://ubuntu.com/download/raspberry-pi"
        exit 1
    fi

    if [ -f "$IMAGE_PATH" ]; then
        ok "Downloaded: $(basename "$IMAGE_PATH")"
    else
        error "Download failed. Download manually from:"
        echo "  https://ubuntu.com/download/raspberry-pi"
        exit 1
    fi
fi

# --- Check SSH key ---
step "Step 5: SSH key"
if [ -f "$HOME/.ssh/id_ed25519.pub" ]; then
    ok "SSH key found: ~/.ssh/id_ed25519.pub"
    echo ""
    info "Your public key:"
    cat "$HOME/.ssh/id_ed25519.pub"
else
    warn "No ed25519 SSH key found."
    read -rp "Generate one now? (y/n): " GENERATE
    if [ "$GENERATE" = "y" ]; then
        ssh-keygen -t ed25519 -f "$HOME/.ssh/id_ed25519" -N ""
        ok "SSH key generated"
        echo ""
        info "Your public key:"
        cat "$HOME/.ssh/id_ed25519.pub"
    else
        warn "You'll need an SSH key before flashing. Generate one with:"
        echo "  ssh-keygen -t ed25519"
    fi
fi

# --- Remind to edit configs ---
step "Step 6: Configure cloud-init"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
USER_DATA="$SCRIPT_DIR/cloud-init-configs/user-data"

if [ -f "$USER_DATA" ] && grep -q "REPLACE_WITH_YOUR_PUBLIC_KEY" "$USER_DATA"; then
    warn "user-data still has placeholder SSH key."
    echo ""
    read -rp "Auto-configure user-data with your SSH key? (y/n): " AUTOCONFIG
    if [ "$AUTOCONFIG" = "y" ] && [ -f "$HOME/.ssh/id_ed25519.pub" ]; then
        PUB_KEY=$(cat "$HOME/.ssh/id_ed25519.pub")
        sed_inplace "s|REPLACE_WITH_YOUR_PUBLIC_KEY|${PUB_KEY}|g" "$USER_DATA"
        ok "user-data updated with your SSH key"
    else
        info "Edit manually before flashing:"
        printf "  ${CYAN}nano cloud-init-configs/user-data${NC}\n"
    fi
else
    ok "user-data already configured"
fi

# --- Summary ---
printf "\n${GREEN}${BOLD}=========================================${NC}\n"
printf "${GREEN}${BOLD}  Prerequisites complete!${NC}\n"
echo ""
echo "  Tools checked: $TOOLS_SUMMARY"
echo "  Image: $(basename "${IMAGE_PATH:-not found}")"
echo ""
if [[ "$OS" == "Darwin" ]]; then
    DISK_EXAMPLE="/dev/diskN"
    FIND_DISK="diskutil list"
else
    DISK_EXAMPLE="/dev/sdX"
    FIND_DISK="lsblk -d -o NAME,SIZE,RM,TRAN,MODEL"
fi
echo "  Next step:"
printf "    ${CYAN}./01-flash.sh %s %s${NC}\n" "$DISK_EXAMPLE" "${IMAGE_PATH:-~/Downloads/ubuntu-*.img.xz}"
echo ""
echo "  Find your SD card with:"
printf "    ${CYAN}%s${NC}\n" "$FIND_DISK"
printf "${GREEN}${BOLD}=========================================${NC}\n"
