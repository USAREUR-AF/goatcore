# shellcheck shell=bash disable=SC2034 # read by ops/e2e/vm/run.sh
# octo-pod (design section 3.2): an edge gateway, built by push. Phase 13 installs
# octo-pod's published release, verified against this pin on the operator's
# machine, with ops/ansible/octo-pod-deploy.yml: the package, then octo-pod's own
# bootstrap, init, apply and verify.
OCTO_POD_PIN="${OCTO_POD_PIN:-USAREUR-AF/octo-pod@octo-pod-v0.6.0@sha256:9a6db6919fad34486a6643066a30e26b487e7e4f9307ac6b62d61d7dcc86a538}"
OCTO_POD_DEB="${OCTO_POD_DEB:-octo-pod_0.6.0_all.deb}"
HOST_VARS="platform_family: host-class"
[ "$VM_ARCH" != arm64 ] || HOST_VARS="platform_family: small-computer"
# The pod's containers and their images.
VM_MEM="${VM_MEM_POD:-4096}"
COMPONENTS="  octo-pod:
    var: octo_pod_release
    artifact_type: release
    owns_state: false
    rollback: pin-revert
    fleet: \"$OCTO_POD_PIN\"
    release:
      checksums: SHA256SUMS
      files: {amd64: $OCTO_POD_DEB, arm64: $OCTO_POD_DEB}
      signer: https://github.com/USAREUR-AF/octo-pod/.github/workflows/release.yml"
GROUP_PINS="octo_pod_release: \"$OCTO_POD_PIN\""
# A type that requires an encrypted root reads UNKNOWN for it from the checkout.
DOCTOR_CHECKOUT_EXIT=1
DOCTOR_CHECKOUT_SAYS=("UNKNOWN — octo-pod requires an encrypted root")

# Phase 13, and the play's own output, which goatbuilder keeps in the run's logs.
join_13() {
  local rc=0 f
  "${JOIN[@]}" --from 13 --to 13 || rc=$?
  for f in "$(find "$HOME/.goat/state/runs" -maxdepth 1 -mindepth 1 | sort | tail -1)"/logs/*-13.log; do cat "$f"; done
  return "$rc"
}

type_cases() {
  step phase-13 0 "release verified: $OCTO_POD_DEB" "octo-pod-deploy.yml" "octo-pod | verify (octo-pod's own pass or fail)" "failed=0" "!fatal:" -- join_13
  step installed 0 "$OCTO_POD_PIN" "\"artifact_type\": \"release\"" "0.6.0" -- vm_ssh 'cat "$(dirname "$(readlink -e /opt/goat/releases/octo-pod/current)")/.ref"; echo; cat /var/lib/goat/octo-pod/deploy-history.jsonl; dpkg-query --showformat "\${Version}\n" --show octo-pod; sudo octo-pod status'
  # A second run converges what is there and changes nothing of the pod's identity.
  step phase-13-again 0 "failed=0" "!fatal:" -- join_13
  step verify 0 -- vm_ssh 'sudo /opt/octo-pod/bin/octo-pod verify'
}
