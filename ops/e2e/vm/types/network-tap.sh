# shellcheck shell=bash disable=SC2034 # read by ops/e2e/vm/run.sh
# network-tap (design section 3.8): a tap with two wired interfaces. Phase 13 installs
# goatrope, the tap's own software, from its published release: the archive verified on
# the operator's machine against this pin (its checksums file's keyless signature, by
# goatrope's release workflow), and its play run from that archive (`release.play`).
# The machine is joined inline, then declared listening and deployed again.
#
# The refusals: no mode, and management on a tap port, before any phase (GB-380); a
# root that is not encrypted, by `doctor --target --live` (a cloud image's is not).
# Not run here: a known feed across the link, and the far end losing none when the
# machine stops. Each tap interface is a QEMU hub with nothing else on it, so no frame
# reaches the machine and none can be counted at a far end.
VM_TAPS=2
GOATROPE_PIN="USAREUR-AF/goatrope@goatrope-v0.2.0@sha256:1b8029c6705b95f233454c255fa8d74e4530a6a26bcd4d87e60bb72700aaa194"
GOATROPE_ARCHIVE="goatrope-0.2.0.tar.gz"
COMPONENTS="  goatrope:
    var: goatrope_release
    artifact_type: release
    owns_state: true
    rollback: pin-revert
    fleet: \"$GOATROPE_PIN\"
    release:
      checksums: SHA256SUMS
      files: {amd64: $GOATROPE_ARCHIVE, arm64: $GOATROPE_ARCHIVE}
      signer: https://github.com/USAREUR-AF/goatrope/.github/workflows/release.yml
      play: deploy/ansible/goatrope-deploy.yml"
GROUP_PINS="goatrope_release: \"$GOATROPE_PIN\""
# The type requires an encrypted root, which the checkout cannot show: UNKNOWN.
DOCTOR_CHECKOUT_EXIT=1
DOCTOR_CHECKOUT_SAYS=("UNKNOWN — network-tap requires an encrypted root")
TAP_RECORD="goatrope_record:
  filter: \"\"
  file_mb: 5
  files: 4"
TAP_INLINE="platform_family: small-computer
tap:
  mode: inline
  ports: [tap0, tap1]
  management: mgmt0
$TAP_RECORD
  ports: [tap0]"
TAP_LISTEN="platform_family: small-computer
tap:
  mode: listen
  source: bypass-relay
  ports: [tap0, tap1]
  management: mgmt0
$TAP_RECORD
  ports: [tap0, tap1]"
HOST_VARS="$TAP_INLINE"

# The machine's declaration, changed in the goatnet's repository as a person would.
tap_declare() {
  printf '%s\n' "$2" > "$CLONE/ops/ansible/host_vars/$NODE.yml"
  git -C "$CLONE" -c user.name="vm harness" -c user.email=vm@localhost -c commit.gpgsign=false commit -q -am "vm harness: $1"
  git -C "$CLONE" push -q origin HEAD:main
  echo "declared: $1"
}

# goatrope installs no package: the machine needs tcpdump and ethtool, and the watchdog
# device's module, as goatrope's own deploy guide says.
tap_packages() {
  vm_ssh 'sudo sh -eux' <<'VM'
DEBIAN_FRONTEND=noninteractive apt-get update -q
DEBIAN_FRONTEND=noninteractive apt-get install -yq "linux-modules-extra-$(uname -r)" tcpdump ethtool
modprobe i6300esb
VM
}

# Phase 13, and the play's own output, which goatbuilder keeps in the run's logs.
join_13() {
  local rc=0 f
  "${JOIN[@]}" --from 13 --to 13 || rc=$?
  for f in "$(find "$HOME/.goat/state/runs" -maxdepth 1 -mindepth 1 | sort | tail -1)"/logs/*-13.log; do cat "$f"; done
  return "$rc"
}

# What Ansible said of the task that runs `goatrope deploy` (the play prints no output
# of it): changed, or ok when the machine already matched.
tap_deploy_was() {
  local f
  f="$(find "$HOME/.goat/state/runs" -maxdepth 1 -mindepth 1 | sort | tail -1)"
  cat "$f"/logs/*-13.log | awk 'p && NF {print "goatrope deploy task: " $1; exit} /^TASK \[goatrope-tap : goatrope-tap \| goatrope deploy\]/ {p=1}'
}

# goatrope's own comparison of the machine with its declaration.
# shellcheck disable=SC2016 # it runs on the VM
tap_check() { vm_ssh 'sudo goatrope check; rc=$?; sudo goatrope status; exit $rc'; }

# A listening tap sends nothing: no frame leaves either port in 30 seconds.
tap_silent() {
  # shellcheck disable=SC2016 # it runs on the VM
  vm_ssh 'set -e
a=$(cat /sys/class/net/tap0/statistics/tx_packets /sys/class/net/tap1/statistics/tx_packets | tr "\n" " ")
sleep 30
b=$(cat /sys/class/net/tap0/statistics/tx_packets /sys/class/net/tap1/statistics/tx_packets | tr "\n" " ")
echo "frames sent by tap0, tap1: before $a, after $b"
[ "$a" = "$b" ] && echo "no frame sent"'
}

type_cases() {
  tap_declare "no tap declared" "platform_family: small-computer
$TAP_RECORD
  ports: [tap0]" > /dev/null
  step refused-no-mode 2 "GB-380" "declare no tap" "!Phase 1 ·" -- "${JOIN[@]}" --dry-run
  tap_declare "management on a tap port" "platform_family: small-computer
tap:
  mode: inline
  ports: [tap0, tap1]
  management: tap0
$TAP_RECORD
  ports: [tap0]" > /dev/null
  step refused-management-on-a-port 2 "GB-380" "tap0" "!Phase 1 ·" -- "${JOIN[@]}" --dry-run
  step declared-inline 0 "declared: inline" -- tap_declare inline "$TAP_INLINE"
  step packages 0 -- tap_packages
  step phase-13-inline 0 "release verified: $GOATROPE_ARCHIVE" "TASK [goatrope | install the tap]" "failed=0" "!fatal:" -- join_13
  step goatrope-installed 0 "goatrope deploy task: changed:" -- tap_deploy_was
  step goatrope-check-inline 0 "matches the declaration" -- tap_check
  # A cloud image's root is not encrypted: that row fails on a machine that is otherwise a tap.
  step doctor-live-inline 1 "one bridge of exactly tap0, tap1, STP off, forward delay zero, LLDP forwarded" \
    "no address on tap0, tap1" "the default route is on mgmt0" "a recorder is on the port, not on a bridge" \
    "boot waits for none of tap0, tap1" "a hardware watchdog restarts a hung machine" "the root is not encrypted" \
    "!FAIL  tap $NODE inline" "!FAIL  tap $NODE addresses" "!FAIL  tap $NODE recording" "!FAIL  tap $NODE silent" "!FAIL  tap $NODE watchdog" -- "${DOCTOR[@]}" --live
  step phase-13-inline-again 0 "failed=0" "!fatal:" -- join_13
  step goatrope-left-alone 0 "goatrope deploy task: ok:" -- tap_deploy_was
  step declared-listen 0 "declared: listen" -- tap_declare listen "$TAP_LISTEN"
  step phase-13-listen 0 "failed=0" "!fatal:" -- join_13
  step goatrope-reinstalled 0 "goatrope deploy task: changed:" -- tap_deploy_was
  step goatrope-check-listen 0 "matches the declaration" -- tap_check
  step doctor-live-listen 1 "tap0, tap1 in no bridge, and every frame they would send is dropped" \
    "no address on tap0, tap1" "a recorder is on the port, not on a bridge" "the root is not encrypted" \
    "!FAIL  tap $NODE listen" "!FAIL  tap $NODE addresses" "!FAIL  tap $NODE recording" "!FAIL  tap $NODE silent" -- "${DOCTOR[@]}" --live
  step listen-sends-nothing 0 "no frame sent" -- tap_silent
  not_run "a known feed recorded across the link, and the far end losing none when the machine stops: each tap interface is a QEMU hub with nothing else on it"
}
