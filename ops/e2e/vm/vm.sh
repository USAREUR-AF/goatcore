# shellcheck shell=bash
# vm.sh: one throwaway Ubuntu VM from a published cloud image, for the target-type
# harness (ops/e2e/vm/run.sh, gate J1). Sourced. Needs no root and changes nothing
# on the machine outside its work directory and its image cache.
#
# VM_TAP_MODEL: a card that reports a link speed, as a wired port does (virtio has none).
VM_TAP_MODEL="${VM_TAP_MODEL:-e1000}"
# Set before vm_up: VM_WORK, VM_ARCH (amd64|arm64), VM_UBUNTU (24.04), VM_PORT,
# VM_MEM, VM_CPUS, VM_TAPS (how many extra wired interfaces), VM_IMAGES (the cache).
# VM_LINK=<local udp port>:<remote udp port>, VM_LINK_ADDR=<address/prefix> and
# VM_LINK_MAC=<n>: one more interface, link0, a wire to another VM of this machine
# whose VM_LINK names the same two ports the other way round.
# Sets: VM_ACCEL (kvm|tcg), VM_PID.

VM_BASE="${VM_BASE:-https://cloud-images.ubuntu.com/releases}"

vm_die() { echo "vm: $*" >&2; return 1; }

# vm_image: the published image, its SHA-256 held to the release's SHA256SUMS on
# every use, a cached one included; SHA256SUMS held to Ubuntu's signature where
# gpgv and the cloud-image keyring are here. VM_SIGNED says whether it was.
vm_image() {
  local f="ubuntu-$VM_UBUNTU-server-cloudimg-$VM_ARCH.img" d="$VM_IMAGES/$VM_UBUNTU" base="$VM_BASE/$VM_UBUNTU/release" want got
  mkdir -p "$d"
  curl -fsSL --retry 3 -o "$VM_WORK/SHA256SUMS" "$base/SHA256SUMS" || vm_die "cannot read $base/SHA256SUMS" || return 1
  export VM_SIGNED=no
  local ring=/usr/share/keyrings/ubuntu-cloudimage-keyring.gpg
  if command -v gpgv >/dev/null && [ -f "$ring" ]; then
    curl -fsSL --retry 3 -o "$VM_WORK/SHA256SUMS.gpg" "$base/SHA256SUMS.gpg" || vm_die "cannot read $base/SHA256SUMS.gpg" || return 1
    gpgv --keyring "$ring" "$VM_WORK/SHA256SUMS.gpg" "$VM_WORK/SHA256SUMS" >/dev/null 2>&1 || vm_die "$base/SHA256SUMS is not signed by Ubuntu's cloud-image key" || return 1
    export VM_SIGNED=yes
  fi
  want="$(sed -n "s/^\([0-9a-f]\{64\}\) [ *]$f\$/\1/p" "$VM_WORK/SHA256SUMS")"
  [ -n "$want" ] || vm_die "$base/SHA256SUMS lists no $f" || return 1
  got="$( [ -f "$d/$f" ] && sha256sum "$d/$f" | cut -d' ' -f1 || true)"
  if [ "$got" != "$want" ]; then
    curl -fsSL --retry 3 -o "$d/$f.$$.partial" "$base/$f" || vm_die "cannot fetch $base/$f" || return 1
    got="$(sha256sum "$d/$f.$$.partial" | cut -d' ' -f1)"
    [ "$got" = "$want" ] || vm_die "$f is sha256 $got, and SHA256SUMS lists $want" || return 1
    mv "$d/$f.$$.partial" "$d/$f"
  fi
  VM_IMAGE="$d/$f"; export VM_IMAGE_SHA256="$want" VM_DISK_SHA256="$want"
  # A prepared image (prepare-image.sh) stands in for the base it was made from,
  # at the digest its record holds, where the run asks for one.
  local p="$d/prepared-$VM_ARCH-${want:0:16}"
  if [ "${VM_PREPARED:-no}" = yes ]; then
    [ -f "$p.qcow2" ] && [ -f "$p.json" ] || vm_die "no prepared image for $f: ops/e2e/vm/prepare-image.sh --arch $VM_ARCH" || return 1
    VM_DISK_SHA256="$(python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d["sha256"] if d["base_sha256"] == sys.argv[2] else "")' "$p.json" "$want")"
    [ -n "$VM_DISK_SHA256" ] && [ "$(sha256sum "$p.qcow2" | cut -d' ' -f1)" = "$VM_DISK_SHA256" ] || vm_die "$p.qcow2 is not the image its record names" || return 1
    VM_IMAGE="$p.qcow2"
  fi
}

# vm_seed: a cloud-init seed. The VM's SSH host keys and the only login are the run's own.
vm_seed() {
  local s="$VM_WORK/seed"; mkdir -p "$s"
  [ -f "$VM_WORK/key" ] || ssh-keygen -q -t ed25519 -N '' -C goat-vm-harness -f "$VM_WORK/key"
  # The VM's address is its own machine's loopback: every identity here is the run's.
  cat > "$s/user-data" <<EOF
#cloud-config
hostname: $VM_NAME
users:
  - name: goatvm
    sudo: "ALL=(ALL) NOPASSWD:ALL"
    shell: /bin/bash
    ssh_authorized_keys: ["$(cat "$VM_WORK/key.pub")"]
ssh_pwauth: false
EOF
  printf 'instance-id: %s-%s\nlocal-hostname: %s\n' "$VM_NAME" "$(date +%s)" "$VM_NAME" > "$s/meta-data"
  # The extra interfaces come up with no address and are not waited for at boot.
  # They are named by their place on the bus, the management one first.
  {
    printf 'version: 2\nethernets:\n  mgmt0:\n    match: {macaddress: "%s"}\n    set-name: mgmt0\n    dhcp4: true\n' "$(vm_mac 0)"
    local i
    for i in $(seq 1 "$VM_TAPS"); do
      printf '  tap%s:\n    match: {macaddress: "%s"}\n    set-name: tap%s\n    optional: true\n' "$((i - 1))" "$(vm_mac "$i")" "$((i - 1))"
    done
    [ -z "${VM_LINK:-}" ] || printf '  link0:\n    match: {macaddress: "%s"}\n    set-name: link0\n    addresses: [%s]\n' "$(vm_mac "$VM_LINK_MAC")" "$VM_LINK_ADDR"
  } > "$s/network-config"
  if command -v cloud-localds >/dev/null; then
    cloud-localds -N "$s/network-config" "$VM_WORK/seed.iso" "$s/user-data" "$s/meta-data"
  else
    xorriso -as mkisofs -quiet -V cidata -J -r -o "$VM_WORK/seed.iso" "$s/user-data" "$s/meta-data" "$s/network-config" 2>/dev/null
  fi
}

# vm_mac N: QEMU's own locally administered prefix, and the interface's number.
vm_mac() { printf '%s:%02x' "$(printf '%s:' 52 54 00 00 00 | sed 's/:$//')" "$1"; }

# vm_up: boot, and wait for SSH.
vm_up() {
  vm_image && vm_seed || return 1
  qemu-img create -q -f qcow2 -F qcow2 -b "$VM_IMAGE" "$VM_WORK/disk.qcow2" 20G
  VM_ACCEL=tcg
  local -a m net
  if [ "$VM_ARCH" = "$(dpkg --print-architecture 2>/dev/null || echo amd64)" ] && [ -r /dev/kvm ] && [ -w /dev/kvm ]; then VM_ACCEL=kvm; fi
  net=(-netdev "user,id=n0,hostfwd=tcp:127.0.0.1:$VM_PORT-:22" -device "virtio-net-pci,netdev=n0,mac=$(vm_mac 0)")
  # Each extra interface is one end of a QEMU hub with nothing else on it: a link
  # that is up and carries what the test sends.
  local i
  for i in $(seq 1 "$VM_TAPS"); do
    net+=(-netdev "hubport,id=t$i,hubid=$i" -device "$VM_TAP_MODEL,netdev=t$i,mac=$(vm_mac "$i")")
  done
  # The wire to the other VM: its frames go as UDP datagrams between two loopback ports.
  [ -z "${VM_LINK:-}" ] || net+=(-netdev "dgram,id=l0,local.type=inet,local.host=127.0.0.1,local.port=${VM_LINK%%:*},remote.type=inet,remote.host=127.0.0.1,remote.port=${VM_LINK##*:}" -device "virtio-net-pci,netdev=l0,mac=$(vm_mac "$VM_LINK_MAC")")
  case "$VM_ARCH" in
    amd64) m=(qemu-system-x86_64 -machine "q35,accel=$VM_ACCEL" -cpu "$([ "$VM_ACCEL" = kvm ] && echo host || echo max)" -device i6300esb -action watchdog=none) ;;
    arm64) m=(qemu-system-aarch64 -machine "virt,accel=$VM_ACCEL" -cpu "$([ "$VM_ACCEL" = kvm ] && echo host || echo cortex-a72)" -bios "${VM_UEFI:-/usr/share/qemu-efi-aarch64/QEMU_EFI.fd}" -device i6300esb -action watchdog=none) ;;
    *) vm_die "no architecture $VM_ARCH"; return 1 ;;
  esac
  local -a wrap=()
  if ! command -v "${m[0]}" >/dev/null; then
    # A machine with no emulator for this architecture runs it from a container.
    docker image inspect goat-vm-qemu >/dev/null 2>&1 || docker build -q -t goat-vm-qemu "$(dirname "${BASH_SOURCE[0]}")" >/dev/null || vm_die "no ${m[0]}, and the container that has one did not build" || return 1
    VM_CONTAINER="goat-vm-$$"
    # On this machine's own network: through a published port the VM would see SSH come
    # from docker's bridge address, which the VM's own docker takes when it is installed,
    # and no reply would leave the VM again. The port is on the loopback only.
    wrap=(docker run --rm --name "$VM_CONTAINER" --network host -v "$VM_WORK:$VM_WORK" -v "$VM_IMAGES:$VM_IMAGES:ro" goat-vm-qemu)
  fi
  # shellcheck disable=SC2086 # no word of these holds a space
  ${wrap[@]+"${wrap[@]}"} "${m[@]}" -m "$VM_MEM" -smp "$VM_CPUS" -display none -serial "file:$VM_WORK/console.log" \
    -drive "file=$VM_WORK/disk.qcow2,if=virtio,format=qcow2" -drive "file=$VM_WORK/seed.iso,if=virtio,format=raw,readonly=on" \
    "${net[@]}" > "$VM_WORK/qemu.log" 2>&1 &
  VM_PID=$!
  local tries=$((VM_BOOT_WAIT / 5))
  for i in $(seq 1 "$tries"); do
    kill -0 "$VM_PID" 2>/dev/null || { vm_die "the VM exited before SSH came up: $(tail -2 "$VM_WORK/qemu.log" | tr '\n' ' ')"; return 1; }
    if vm_ssh true 2>/dev/null; then vm_ssh 'sudo cloud-init status --wait >/dev/null 2>&1 || true'; return 0; fi
    sleep 5
  done
  vm_die "no SSH after ${VM_BOOT_WAIT} s ($VM_WORK/console.log)"
}

vm_ssh() {
  ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 -o BatchMode=yes -i "$VM_WORK/key" -p "$VM_PORT" goatvm@127.0.0.1 "$@"
}

vm_down() {
  [ -z "${VM_CONTAINER:-}" ] || docker rm -f "$VM_CONTAINER" >/dev/null 2>&1 || true
  if [ -n "${VM_PID:-}" ]; then kill "$VM_PID" 2>/dev/null || true; wait "$VM_PID" 2>/dev/null || true; fi
  VM_PID=""
}
