# shellcheck shell=bash
# system.sh — the changes ops/e2e/hermetic.sh makes to the machine it runs on, the
# guard that refuses them on a shared machine, and their undo. Sourced by hermetic.sh;
# tests/test_hermetic_system_guard.py drives it with stand-ins.
#
# With hosts, the tier makes the runner resolve ghcr.io to a local registry:
#   - it appends "127.0.0.1 ghcr.io" to /etc/hosts (E2E_HOSTS_FILE);
#   - it trusts a throwaway CA at /etc/docker/certs.d/ghcr.io/ca.crt (E2E_CERTS_DIR);
#   - it runs a registry container bound to :443.
# On a shared machine, that points every session's ghcr.io at an empty registry. So
# sys_guard refuses unless the run is in CI or the caller says the machine is
# throwaway, and sys_arm_undo puts both files back on exit, on failure or on a
# signal. A line or a certificate that was there before is left as it was.
E2E_HOSTS_FILE="${E2E_HOSTS_FILE:-/etc/hosts}"
E2E_CERTS_DIR="${E2E_CERTS_DIR:-/etc/docker/certs.d}"
SYS_HOSTS_ADDED=0; SYS_CERTS_MADE=0; SYS_CA_SAVED=""

sys_changes() {
  echo "  - append '127.0.0.1 ghcr.io' to /etc/hosts"
  echo "  - write a throwaway CA to /etc/docker/certs.d/ghcr.io/ca.crt"
  echo "  - run a registry container bound to :443"
}

# sys_guard HOSTS THROWAWAY: 0 when the run may go on; 2, with the reason, otherwise.
sys_guard() {
  [ "$1" = 1 ] || return 0
  [ "${GITHUB_ACTIONS:-}" = true ] && return 0
  [ "$2" = 1 ] && return 0
  {
    echo "hermetic: refusing to start: with hosts, this run changes the machine it runs on (it uses sudo):"
    sys_changes
    echo "  It undoes the first two on exit, but while it runs every other user of this machine"
    echo "  resolves ghcr.io to its registry. Run it in CI (GITHUB_ACTIONS=true), or pass"
    echo "  --throwaway-machine on a machine nobody else uses, or pass --no-hosts for the"
    echo "  dry-run and refusal cases, which change nothing."
  } >&2
  return 2
}

sys_undo() {
  local tmp
  if [ "$SYS_HOSTS_ADDED" = 1 ]; then
    tmp="$(mktemp)"
    grep -vxF '127.0.0.1 ghcr.io' "$E2E_HOSTS_FILE" > "$tmp" || true
    # cp, not mv: the file keeps its inode, owner and mode.
    sudo cp "$tmp" "$E2E_HOSTS_FILE" && SYS_HOSTS_ADDED=0
    rm -f "$tmp"
  fi
  if [ "$SYS_CERTS_MADE" = 1 ]; then
    sudo rm -r "$E2E_CERTS_DIR/ghcr.io" && SYS_CERTS_MADE=0
  elif [ -n "$SYS_CA_SAVED" ]; then
    sudo cp "$SYS_CA_SAVED" "$E2E_CERTS_DIR/ghcr.io/ca.crt" && rm -f "$SYS_CA_SAVED" && SYS_CA_SAVED=""
  fi
}

# sys_arm_undo: undo on any exit; a signal exits, so the EXIT trap runs.
sys_arm_undo() {
  trap sys_undo EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
}

# sys_trust_registry CERT: Docker trusts CERT for ghcr.io.
sys_trust_registry() {
  if [ -d "$E2E_CERTS_DIR/ghcr.io" ]; then
    if [ -f "$E2E_CERTS_DIR/ghcr.io/ca.crt" ] && [ -z "$SYS_CA_SAVED" ]; then
      SYS_CA_SAVED="$(mktemp)"
      cp "$E2E_CERTS_DIR/ghcr.io/ca.crt" "$SYS_CA_SAVED"
    fi
  else
    SYS_CERTS_MADE=1
  fi
  sudo mkdir -p "$E2E_CERTS_DIR/ghcr.io" && sudo cp "$1" "$E2E_CERTS_DIR/ghcr.io/ca.crt"
}

# sys_point_ghcr: ghcr.io resolves to this machine.
sys_point_ghcr() {
  grep -q ' ghcr.io$' "$E2E_HOSTS_FILE" && return 0
  echo "127.0.0.1 ghcr.io" | sudo tee -a "$E2E_HOSTS_FILE" >/dev/null && SYS_HOSTS_ADDED=1
}
