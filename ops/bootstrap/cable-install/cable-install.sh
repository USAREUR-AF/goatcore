#!/usr/bin/env bash
# cable-install.sh <hostname> — wipe a board and reinstall the Ubuntu 24.04 baseline over one cable.
#
# Run on the helper machine, from this directory, after the one-time setup in README.md.
# The board is cabled to the helper's spare port (10.77.0.0/24, helper/05-cable-install-link.network)
# and must be running a baseline this script installed: the installer kernel is staged in its
# /boot and booted once; the installer pulls the ISO and the unattended seed from
# cable-install-http.service on 10.77.0.1:8099. No KVM, no USB stick, no monitor.
# EVERYTHING ON THE BOARD IS ERASED.
#
#   CABLE_MODE=plain (default)  plain LVM root
#   CABLE_MODE=luks             LUKS2 root (installer layout) bound to the TPM (Clevis, PCR 7),
#                               dropbear in the initramfs as the remote fallback
#   CABLE_USER=goat (default)   the account the seed creates: key-only SSH, NOPASSWD sudo
#   CABLE_IP, CABLE_IP_AFTER    the board's address before and after (default 10.77.0.5, the
#                               static lease)
#
# While an install runs the installer stays reachable: ssh root@<board> with the keys in
# ./authorized_keys; its state is in /var/log/installer/subiquity-server-info.log. An install
# takes 6 to 14 minutes. Do not power-cycle a board that looks stuck: look first.
# The new host key lands in ./known_hosts and its fingerprint in ./hostkey.
set -euo pipefail
MODE="${CABLE_MODE:-plain}"
H="${1:?usage: cable-install.sh <hostname>}"
U="${CABLE_USER:-goat}"
IP="${CABLE_IP:-10.77.0.5}"
IP_AFTER="${CABLE_IP_AFTER:-10.77.0.5}"
cd "$(dirname "$0")"

case "$MODE" in plain|luks) ;; *) echo "CABLE_MODE must be plain or luks" >&2; exit 1 ;; esac
[ -s authorized_keys ] || { echo "authorized_keys is missing: one SSH public key per line" >&2; exit 1; }
[ -s console.hash ] || { echo "console.hash is missing: a SHA-512 crypt hash for the console password" >&2; exit 1; }
[ -s kb/vmlinuz ] && [ -s kb/initrd ] || { echo "kb/vmlinuz and kb/initrd are missing: extract them from the ISO (README.md)" >&2; exit 1; }
[ -e www/ubuntu.iso ] || { echo "www/ubuntu.iso is missing" >&2; exit 1; }
systemctl --user is-active -q cable-install-http.service || { echo "cable-install-http.service is not running" >&2; exit 1; }

SSH_OPTS="-o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5"
# shellcheck disable=SC2086
before() { ssh $SSH_OPTS "$U@$IP" "$@"; }
# shellcheck disable=SC2086
after() { ssh $SSH_OPTS "$U@$IP_AFTER" "$@"; }

export HOST="$H" USER_NAME="$U"
HASH="$(cat console.hash)"; export HASH
LUKS=""
if [ "$MODE" = luks ]; then
  [ -s luks.pass ] || { echo "luks.pass is missing: the disk passphrase, mode 600" >&2; exit 1; }
  LUKS="$(tr -d '\n' < luks.pass)"
fi
export LUKS

mkdir -p www/nocloud
install -m 644 authorized_keys www/authorized_keys
: > www/nocloud/vendor-data
printf 'instance-id: %s-%s\nlocal-hostname: %s\n' "$H" "$(date -u +%Y%m%dT%H%M%SZ)" "$H" > www/nocloud/meta-data
# The seed and late.sh carry the passphrase only while the install runs.
scrub() {
  : > www/late.sh
  sed -i 's|password: ".*"|password: "removed-after-install"|' www/nocloud/user-data 2>/dev/null || true
}
trap scrub EXIT
python3 render.py "user-data-$MODE.tmpl" > www/nocloud/user-data
if [ "$MODE" = luks ]; then python3 render.py late.sh.tmpl > www/late.sh; fi

OLD="$(before cat /etc/machine-id)"
# shellcheck disable=SC2086
scp -q $SSH_OPTS kb/vmlinuz kb/initrd grub-reinstall-entry.cfg "$U@$IP:/tmp/"
before 'set -e
sudo install -d /boot/cable-installer
sudo install -m 644 /tmp/vmlinuz /tmp/initrd /boot/cable-installer/
BOOTUUID=$(findmnt -no UUID /boot)
{ printf "%s\n" "#!/bin/sh" "exec tail -n +3 \$0"; sed "s|__BOOTUUID__|$BOOTUUID|" /tmp/grub-reinstall-entry.cfg; } | sudo tee /etc/grub.d/45_cable_reinstall >/dev/null
sudo chmod 755 /etc/grub.d/45_cable_reinstall
echo GRUB_DEFAULT=saved | sudo tee /etc/default/grub.d/90-cable-install.cfg >/dev/null
sudo update-grub 2>/dev/null
sudo grep -q "menuentry .cable-reinstall" /boot/grub/grub.cfg
sudo grub-set-default 0
sudo grub-reboot cable-reinstall
sudo systemd-run --on-active=2 systemctl reboot >/dev/null 2>&1'
echo "rebooting $IP into the installer at $(date -u +%T)Z"

i=0
while [ "$i" -lt 150 ]; do
  i=$((i + 1))
  sleep 10
  NEW="$(after cat /etc/machine-id 2>/dev/null || true)"
  if [ -n "$NEW" ] && [ "$NEW" != "$OLD" ]; then
    echo "baseline is back at $(date -u +%T)Z: $(after 'hostname; lsb_release -ds' | tr '\n' ' ')"
    if [ "$MODE" = luks ]; then
      after 'sudo clevis luks list -d /dev/$(lsblk -rno NAME,FSTYPE | awk "\$2==\"crypto_LUKS\"{print \$1; exit}")' | sed 's/^/root is LUKS2, clevis slot: /'
    fi
    # Every install makes a new host key. Read over the direct cable, for callers to pin.
    ssh-keyscan -t ed25519 "$IP_AFTER" 2>/dev/null > known_hosts
    ssh-keygen -lf known_hosts | tee hostkey
    exit 0
  fi
done
echo "the board did not come back in 25 minutes. If it is still in the installer: ssh root@$IP_AFTER and read /var/log/installer/" >&2
exit 1
