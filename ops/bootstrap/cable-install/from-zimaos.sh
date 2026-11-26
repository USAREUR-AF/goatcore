#!/usr/bin/env bash
# from-zimaos.sh <board-ip> — first install: a factory ZimaBoard 2 (ZimaOS 1.5.0) to Ubuntu,
# over the cable, with no USB stick, KVM, monitor or keyboard.
# Run on the helper machine after helper/ is set up and the board has a DHCP address.
# Status: WIP, not for production use — gate: this script run once, as one script, on a factory board.
# (Its commands were run by hand on a factory board and worked; cable-install.sh is the proven part.)
#   HOSTNAME_NEW=<name> [CABLE_USER=goat] ./from-zimaos.sh <board-ip>
set -euo pipefail
IP="${1:?usage: HOSTNAME_NEW=<name> from-zimaos.sh <board-ip>}"; cd "$(dirname "$0")"; umask 077
ISO=${ISO:-ubuntu-24.04.5-live-server-amd64.iso}        # verified against SHA256SUMS, served as www/ubuntu.iso

# 1. First-run account on ZimaOS, then turn on its SSH (off by default; the API has a switch).
[ -s zimaos.pw ] || openssl rand -hex 16 > zimaos.pw     # throwaway: the OS is about to be erased
PW=$(cat zimaos.pw)
KEY=$(curl -s "http://$IP/v1/users/status" | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"]["key"])')
curl -s -X POST "http://$IP/v1/users/register" -H 'Content-Type: application/json' -d "{\"username\":\"goatinstall\",\"password\":\"$PW\",\"key\":\"$KEY\"}" >/dev/null
JWT=$(curl -s -X POST "http://$IP/v1/users/login" -H 'Content-Type: application/json' -d "{\"username\":\"goatinstall\",\"password\":\"$PW\"}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"]["token"]["access_token"])')
curl -s -X PUT -H "Authorization: $JWT" "http://$IP/v2/zimaos/ssh?status=enable" >/dev/null
SSH="sshpass -f zimaos.pw ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR goatinstall@$IP"
SCP="sshpass -f zimaos.pw scp -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"

# 2. Installer kernel + initrd in an lzo squashfs (ZimaOS's GRUB reads squashfs; lzo is what its
#    own kernel partition uses), built HERE, never on the board.
rm -rf kb && mkdir kb
xorriso -osirrox on -indev "$ISO" -extract /casper/vmlinuz kb/vmlinuz -extract /casper/initrd kb/initrd
chmod 644 kb/*; rm -f boot.sqsh; mksquashfs kb boot.sqsh -comp lzo -noappend -all-root -quiet

# 3. Seed for the unattended install (same templates cable-install.sh uses).
HOST="${HOSTNAME_NEW:?set HOSTNAME_NEW to the name the board gets}"; export HOST
USER_NAME="${CABLE_USER:-goat}"; export USER_NAME
HASH="$(cat console.hash)"; export HASH
mkdir -p www/nocloud
install -m 644 authorized_keys www/authorized_keys
python3 render.py user-data-plain.tmpl > www/nocloud/user-data
printf 'instance-id: %s-%s\nlocal-hostname: %s\n' "$HOST" "$(date -u +%Y%m%dT%H%M%SZ)" "$HOST" > www/nocloud/meta-data
: > www/nocloud/vendor-data

# 4. Stage on the unused slot-B system partition and add a ONE-SHOT GRUB entry.
#    A power cycle after a failed boot returns to ZimaOS.
$SSH 'cat /mnt/boot/EFI/BOOT/grub.cfg' > zimaos-grub.cfg
python3 - <<'PY'
src = open("zimaos-grub.cfg").read()
anchor = "save_env A_TRY A_OK B_TRY B_OK ORDER MACHINE_ID\n"
assert src.count(anchor) == 1
src = src.replace(anchor, anchor + '''
if [ "$CABLE_INSTALL" == "1" ]; then
    CABLE_INSTALL=0
    save_env CABLE_INSTALL
    default=4
fi
''')
src += '''
menuentry "Ubuntu autoinstall over the cable (one shot)" {
    linux (${boothd},gpt5)/vmlinuz ip=dhcp url=http://10.77.0.1:8099/ubuntu.iso autoinstall "ds=nocloud-net;s=http://10.77.0.1:8099/nocloud/" modprobe.blacklist=cdc_ncm,cdc_ether,rndis_host ---
    initrd (${boothd},gpt5)/initrd
}
'''
open("install-grub.cfg", "w").write(src)
PY
$SCP boot.sqsh install-grub.cfg "goatinstall@$IP:/tmp/"
SZ=$(stat -c %s boot.sqsh); SUM=$(sha256sum boot.sqsh | cut -d' ' -f1)
cat > stage.sh <<S
set -e
dd if=/tmp/boot.sqsh of=/dev/mmcblk0p5 bs=4M conv=fsync
[ "\$(head -c $SZ /dev/mmcblk0p5 | sha256sum | cut -d' ' -f1)" = "$SUM" ]
cp /mnt/boot/EFI/BOOT/grub.cfg /mnt/boot/EFI/BOOT/grub.cfg.zimaos
cp /tmp/install-grub.cfg /mnt/boot/EFI/BOOT/grub.cfg
grub-editenv /mnt/boot/EFI/BOOT/grubenv set CABLE_INSTALL=1
sync
reboot
S
$SCP stage.sh "goatinstall@$IP:/tmp/"
$SSH "echo $PW | sudo -S -p '' sh /tmp/stage.sh" || true
rm -f zimaos.pw stage.sh
echo "the board is rebooting into the installer; it returns as $HOST on SSH (key-only) in 10 to 15 minutes."
echo "while it installs: ssh root@<its address> reaches the installer; logs in /var/log/installer/"
