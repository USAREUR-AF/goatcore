#!/usr/bin/env bash
# shellcheck disable=SC2016 # each stand-in expands its own arguments when it runs
# machine.sh DIR: a stand-in small computer for ops/scripts/small-computer-endstate.py,
# as the Raspberry Pi 5 builder (ops/rpi-base-image) leaves it, with the YubiKey
# enrolled. Needs no root.
#
#   DIR/root/   the files the collector reads (SMALL_COMPUTER_ROOT)
#   DIR/bin/    the commands it runs (SMALL_COMPUTER_PATH=DIR/bin:/usr/sbin:/usr/bin:/sbin:/bin)
#   DIR/luks.img  a real LUKS2 header, made by the real cryptsetup with the
#                 builder's parameters; lsblk reports it as the root's partition
#
# Tests change one thing after it to break one rule.
set -euo pipefail
dir="$1"
mkdir -p "$dir/root/etc/ssh/sshd_config.d" "$dir/root/proc/device-tree" "$dir/root/boot/firmware" "$dir/root/var/lib/cloud/instance" "$dir/bin"
luks="$dir/luks.img"
truncate -s 40M "$luks"
printf 'fixture-passphrase' > "$dir/pass"
printf 'fixture-yubikey-response' > "$dir/yk"
# encrypt-rootfs.sh's parameters; the iteration count is cut so the test is quick.
cryptsetup luksFormat -q --type luks2 -c aes-xts-plain64 -s 512 -h sha256 --pbkdf pbkdf2 --pbkdf-force-iterations 1000 --key-file "$dir/pass" "$luks"
cryptsetup luksAddKey -q --pbkdf pbkdf2 --pbkdf-force-iterations 1000 --key-file "$dir/pass" "$luks" "$dir/yk"

printf 'PRETTY_NAME="Ubuntu 24.04.4 LTS"\nNAME="Ubuntu"\nVERSION_ID="24.04"\nID=ubuntu\n' > "$dir/root/etc/os-release"
printf 'rootfs  /dev/nvme0n1p2  -  luks,initramfs,keyscript=yubikey_unlock,noauto\n' > "$dir/root/etc/crypttab"
printf 'Raspberry Pi 5 Model B Rev 1.0\0' > "$dir/root/proc/device-tree/model"
printf 'console=serial0,115200 console=tty1 root=/dev/mapper/rootfs rootfstype=ext4 rootwait ip=192.0.2.10::192.0.2.1:255.255.255.0 cryptdevice=/dev/nvme0n1p2:rootfs fips=1\n' > "$dir/root/proc/cmdline"
: > "$dir/root/boot/firmware/initrd.img"
cp "$dir/root/proc/cmdline" "$dir/root/boot/firmware/cmdline.txt"
printf 'Include /etc/ssh/sshd_config.d/*.conf\nKbdInteractiveAuthentication no\nUsePAM yes\n' > "$dir/root/etc/ssh/sshd_config"
printf 'PasswordAuthentication no\n' > "$dir/root/etc/ssh/sshd_config.d/50-cloud-init.conf"
: > "$dir/root/var/lib/cloud/instance/boot-finished"

tool() { printf '#!/bin/sh\n%s\n' "$2" > "$dir/bin/$1"; chmod +x "$dir/bin/$1"; }
tool findmnt 'echo /dev/mapper/rootfs'
tool lsblk "cat <<'J'
{\"blockdevices\": [{\"name\": \"/dev/mapper/rootfs\", \"type\": \"crypt\", \"fstype\": \"ext4\", \"children\": [
  {\"name\": \"$luks\", \"type\": \"part\", \"fstype\": \"crypto_LUKS\", \"children\": [
    {\"name\": \"/dev/nvme0n1\", \"type\": \"disk\", \"fstype\": null}]}]}]}
J"
# cryptsetup reads the real header; only the active mapping is a stand-in.
tool cryptsetup 'if [ "$1" = status ]; then printf "/dev/mapper/%s is active and is in use.\n  type:    LUKS2\n  cipher:  aes-xts-plain64\n  keysize: 512 bits\n" "$2"; exit 0; fi
exec /usr/sbin/cryptsetup "$@"'
tool lsinitramfs 'printf "usr/sbin/cryptsetup\nusr/bin/ykchalresp\nlib/cryptsetup/scripts/yubikey_unlock\nusr/lib/aarch64-linux-gnu/libykpers-1.so.1\n"'
# As 05-enable-fips.sh leaves it: the FIPS provider active beside the default
# one, no default properties set, so MD5 still computes.
tool openssl 'if [ "$1" = dgst ]; then echo "MD5(/dev/null)= d41d8cd98f00b204e9800998ecf8427e"; exit 0; fi
printf "Providers:\n  default\n    name: OpenSSL Default Provider\n    version: 3.0.13\n    status: active\n  fips\n    name: OpenSSL FIPS Provider\n    version: 3.0.13\n    status: active\n"'
tool sshd 'cat <<S
port 22
permitrootlogin prohibit-password
pubkeyauthentication yes
passwordauthentication no
kbdinteractiveauthentication no
hostbasedauthentication no
gssapiauthentication no
ciphers aes256-gcm@openssh.com,aes128-gcm@openssh.com,aes256-ctr,aes192-ctr,aes128-ctr
macs hmac-sha2-512,hmac-sha2-256,hmac-sha2-512-etm@openssh.com,hmac-sha2-256-etm@openssh.com
kexalgorithms ecdh-sha2-nistp521,ecdh-sha2-nistp384,ecdh-sha2-nistp256,diffie-hellman-group16-sha512,diffie-hellman-group14-sha256
hostkeyalgorithms rsa-sha2-512,rsa-sha2-256,ecdsa-sha2-nistp521,ecdsa-sha2-nistp384,ecdsa-sha2-nistp256,ssh-ed25519
pubkeyacceptedalgorithms rsa-sha2-512,rsa-sha2-256,ecdsa-sha2-nistp521,ecdsa-sha2-nistp384,ecdsa-sha2-nistp256,ssh-ed25519
S'
tool cloud-init 'printf "status: done\nextended_status: done\n"'
tool rpi-eeprom-config 'printf "[all]\nBOOT_UART=1\nBOOT_ORDER=0xf416\n"'
