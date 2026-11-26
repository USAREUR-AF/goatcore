# shellcheck shell=bash disable=SC2034 # read by ops/e2e/vm/run.sh
# network-tap (design section 3.8): an inline tap with two wired interfaces.
# goatrope installs a tap and has no release, so phase 13 is its declared skip and
# the machine is then prepared here by hand, as section 3.8 says a tap must be, to
# hold `doctor --target --live` to a real machine both ways.
VM_TAPS=2
# The type requires an encrypted root, which the checkout cannot show: UNKNOWN.
DOCTOR_CHECKOUT_EXIT=1
DOCTOR_CHECKOUT_SAYS=("UNKNOWN — network-tap requires an encrypted root")
HOST_VARS="platform_family: small-computer
tap:
  mode: inline
  ports: [tap0, tap1]
  management: mgmt0"

tap_prepare() {
  vm_ssh 'sudo sh -eux' <<'VM'
DEBIAN_FRONTEND=noninteractive apt-get update -q; DEBIAN_FRONTEND=noninteractive apt-get install -yq "linux-modules-extra-$(uname -r)" tcpdump ethtool
modprobe i6300esb
mkdir -p /etc/systemd/system.conf.d
printf '[Manager]\nRuntimeWatchdogSec=30s\n' > /etc/systemd/system.conf.d/watchdog.conf
systemctl daemon-reexec
for i in tap0 tap1; do
  f=/etc/systemd/network/05-$i.network
  printf '[Match]\nName=%s\n[Link]\nRequiredForOnline=no\n[Network]\nLinkLocalAddressing=no\nIPv6AcceptRA=no\nLLDP=no\nEmitLLDP=no\nBridge=br0\n' $i > $f
  ethtool -K $i gro off lro off || true
done
printf '[NetDev]\nName=br0\nKind=bridge\n[Bridge]\nSTP=no\nForwardDelaySec=0\nGroupForwardMask=16384\n' > /etc/systemd/network/05-br0.netdev
printf '[Match]\nName=br0\n[Link]\nRequiredForOnline=no\n[Network]\nLinkLocalAddressing=no\nIPv6AcceptRA=no\n' > /etc/systemd/network/05-br0.network
networkctl reload; sleep 3
for i in tap0 tap1 br0; do sysctl -qw net.ipv6.conf.$i.disable_ipv6=1; ip addr flush dev $i; done
echo 0 > /sys/class/net/br0/bridge/forward_delay || true
cat > /etc/systemd/system/tap-record.service <<'U'
[Service]
ExecStart=/usr/bin/tcpdump -i tap0 -w /dev/null
U
systemctl daemon-reload; systemctl start tap-record; sleep 2
VM
}

type_cases() {
  step phase-13 0 "Phase 13 · SKIP" "this machine is not a working tap" -- "${JOIN[@]}" --from 13 --to 13
  step doctor-live-unprepared 1 "tap $NODE inline" "no bridge holds tap0, tap1, so the link is down" "nothing records tap0 or tap1" "the root is not encrypted" -- "${DOCTOR[@]}" --live
  step prepare 0 -- tap_prepare
  # A cloud image's root is not encrypted: that row fails on a machine that is otherwise a tap.
  step doctor-live 1 "one bridge of exactly tap0, tap1, STP off, forward delay zero, LLDP forwarded" \
    "no address on tap0, tap1 or br0, IPv6 disabled" "the default route is on mgmt0" "a recorder is on the port, not on a bridge" \
    "boot waits for none of tap0, tap1" "a hardware watchdog restarts a hung machine" "the root is not encrypted" "!FAIL  tap $NODE inline" "!FAIL  tap $NODE addresses" "!FAIL  tap $NODE offloads" "!FAIL  tap $NODE silent" -- "${DOCTOR[@]}" --live
}
