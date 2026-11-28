# shellcheck shell=bash disable=SC2034 # read by ops/e2e/vm/run.sh
# first-bite (design section 3.2): a decoder on one capture interface. Phase 13 installs
# first-bite's published release, each archive verified against its own GitHub artifact
# attestation by first-bite's release workflow at the pin's tag, with
# ops/ansible/first-bite-deploy.yml: the archive's own install.sh. Then a heartbeat of the
# release's weather-station dictionary, sent on the capture link, is decoded.
VM_TAPS=1
FB_PIN="USAREUR-AF/first-bite@v1.1.0@sha256:b8776f12584ae4bf283ca647b87c86d21b5e0835ff3ae153c4d8e7d89f4bc499"
FB_amd64="first-bite-1.1.0-x86_64-linux-gnu.tar.gz"
FB_arm64="first-bite-1.1.0-aarch64-linux-gnu.tar.gz"
FB_FILE="$(eval echo "\$FB_$VM_ARCH")"
HOST_VARS="platform_family: host-class
first_bite_interface: tap0
first_bite_dictionary: weather-station"
[ "$VM_ARCH" != arm64 ] || HOST_VARS="${HOST_VARS/host-class/small-computer}"
COMPONENTS="  first-bite:
    var: first_bite_release
    artifact_type: release
    owns_state: true
    rollback: pin-revert
    fleet: \"$FB_PIN\"
    release:
      checksums: SHA256SUMS
      bundle: attestation
      files: {amd64: $FB_amd64, arm64: $FB_arm64}
      signer: https://github.com/USAREUR-AF/first-bite/.github/workflows/release.yml"
GROUP_PINS="first_bite_release: \"$FB_PIN\""

join_13() {
  local rc=0 f
  "${JOIN[@]}" --from 13 --to 13 || rc=$?
  for f in "$(find "$HOME/.goat/state/runs" -maxdepth 1 -mindepth 1 | sort | tail -1)"/logs/*-13.log; do cat "$f"; done
  return "$rc"
}

# The weather-station dictionary's heartbeat vector, as a UDP datagram on the capture
# link (documentation addresses; the far end is a static neighbour, so it is sent).
fb_feed() {
  vm_ssh 'sudo sh -eu' <<'VM'
ip link set tap0 up
ip -4 addr replace 198.51.100.1/24 dev tap0
ip neigh replace 198.51.100.2 lladdr 02:00:00:00:00:02 dev tap0  # opsec-allow: a synthetic, locally administered neighbour on a VM link
python3 - <<'PY'
import socket
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
for _ in range(3):
    s.sendto(bytes.fromhex("01000c4b5837000001518060"), ("198.51.100.2", 40100))
PY
sleep 3
tail -n 50 /var/log/first-bite/mind.log
VM
}

type_cases() {
  step phase-13-undeclared 1 "GB-415 first-bite's capture is not declared" -- join_13_with "grep -v ^first_bite_"
  step phase-13 0 "release verified: $FB_FILE" "first-bite-deploy.yml" "first-bite | 5 the service is active and has not restarted" "failed=0" "!fatal:" -- join_13
  # shellcheck disable=SC2016 # expanded on the VM
  step installed 0 "$FB_PIN" "\"artifact_type\": \"release\"" "FIRST_BITE_INTERFACE=tap0" "BITE_DICT_PATH=/etc/first-bite/dictionary" -- vm_ssh 'set -e; cat "$(dirname "$(readlink -e /opt/goat/releases/first-bite/current)")/.ref"; echo; cat /var/lib/goat/first-bite/deploy-history.jsonl; systemctl is-active first-bite; sudo grep -E "^(FIRST_BITE_INTERFACE|BITE_DICT_PATH)=" /etc/first-bite/first-bite.env'
  step decodes 0 '"message_type":"Heartbeat"' -- fb_feed
  step phase-13-again 0 "failed=0" "!fatal:" -- join_13
  # A dictionary the release does not have: refused before the installer; the running
  # first-bite stays, and no file is current, so verify says not converged.
  step phase-13-no-dictionary 1 "GB-415 the release has no no-such-dictionary" -- join_13_with "sed s/^first_bite_dictionary:.*/first_bite_dictionary:\ no-such-dictionary/"
  step no-current-kept-running 0 "active" -- vm_ssh 'set -e; test ! -e /opt/goat/releases/first-bite/current; systemctl is-active first-bite'
  # A settings file that captures on another interface: GB-416, until it is put right.
  step phase-13-other-interface 1 "GB-416 /etc/first-bite/first-bite.env captures on eth9" -- fb_env_then_13 "s/^FIRST_BITE_INTERFACE=.*/FIRST_BITE_INTERFACE=eth9/"
  step phase-13-put-right 0 "failed=0" "!fatal:" -- fb_env_then_13 "s/^FIRST_BITE_INTERFACE=.*/FIRST_BITE_INTERFACE=tap0/"
}

fb_env_then_13() {
  vm_ssh "sudo sed -i '$1' /etc/first-bite/first-bite.env"
  join_13
}

# Phase 13 with the machine's host_vars passed through a filter (one commit), then put
# back (another): the play's refusals, on the real machine.
join_13_with() {
  local hv="ops/ansible/host_vars/$NODE.yml" rc=0
  cp "$hv" "$WORK/hv.keep"
  eval "$1" < "$WORK/hv.keep" > "$hv"
  git -c user.name="vm harness" -c user.email=vm@localhost -c commit.gpgsign=false commit -qam "vm harness: host_vars changed for one run"
  git push -q origin HEAD:main
  join_13 || rc=$?
  cp "$WORK/hv.keep" "$hv"
  git -c user.name="vm harness" -c user.email=vm@localhost -c commit.gpgsign=false commit -qam "vm harness: host_vars put back"
  git push -q origin HEAD:main
  return "$rc"
}
