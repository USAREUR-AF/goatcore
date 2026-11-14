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
  local tmp rc=0 failed=0
  if [ "$SYS_HOSTS_ADDED" = 1 ]; then
    tmp="$(mktemp)"
    grep -vxF '127.0.0.1 ghcr.io' "$E2E_HOSTS_FILE" > "$tmp" || rc=$?
    # grep exits 1 when no other line is left; 2 is a read error, whose empty output
    # must never replace the file. cp, not mv: the file keeps its inode, owner and mode.
    if [ "$rc" -le 1 ] && sudo cp "$tmp" "$E2E_HOSTS_FILE" && ! grep -qxF '127.0.0.1 ghcr.io' "$E2E_HOSTS_FILE"; then
      SYS_HOSTS_ADDED=0
    else
      printf "hermetic: NOT RESTORED: %s still points ghcr.io at this machine; run: sudo sed -i '/^127\\.0\\.0\\.1 ghcr\\.io\$/d' %s\n" \
        "$E2E_HOSTS_FILE" "$E2E_HOSTS_FILE" >&2
      failed=1
    fi
    rm -f "$tmp"
  fi
  if [ "$SYS_CERTS_MADE" = 1 ]; then
    if sudo rm -r "$E2E_CERTS_DIR/ghcr.io" && [ ! -e "$E2E_CERTS_DIR/ghcr.io" ]; then
      SYS_CERTS_MADE=0
    else
      printf 'hermetic: NOT RESTORED: %s, which this run made, is still there; run: sudo rm -r %s\n' \
        "$E2E_CERTS_DIR/ghcr.io" "$E2E_CERTS_DIR/ghcr.io" >&2
      failed=1
    fi
  elif [ -n "$SYS_CA_SAVED" ]; then
    if sudo cp "$SYS_CA_SAVED" "$E2E_CERTS_DIR/ghcr.io/ca.crt" && sudo cmp -s "$SYS_CA_SAVED" "$E2E_CERTS_DIR/ghcr.io/ca.crt"; then
      rm -f "$SYS_CA_SAVED"; SYS_CA_SAVED=""
    else
      printf 'hermetic: NOT RESTORED: %s is not the CA this run replaced (kept at %s); run: sudo cp %s %s\n' \
        "$E2E_CERTS_DIR/ghcr.io/ca.crt" "$SYS_CA_SAVED" "$SYS_CA_SAVED" "$E2E_CERTS_DIR/ghcr.io/ca.crt" >&2
      failed=1
    fi
  fi
  # A run whose machine was not put back does not end as a success.
  [ "$failed" = 0 ] || exit 1
}

# sys_sudo_ready: sudo needs no password at all (NOPASSWD). A cached timestamp is not
# enough: it can expire before the undo runs, and the undo has no terminal to ask on.
# `sudo -k -n` ignores the cached credentials for this one command, without removing them.
sys_sudo_ready() {
  sudo -k -n true 2>/dev/null && return 0
  echo "hermetic: with hosts this run needs sudo with no password (NOPASSWD): it puts the hosts file and the certificate directory back on exit, and a cached sudo timestamp can expire before then" >&2
  return 2
}

# sys_arm_undo: undo on any exit; a signal exits, so the EXIT trap runs.
sys_arm_undo() {
  trap sys_undo EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
}

# sys_trust_registry CERT: Docker trusts CERT for ghcr.io.
sys_trust_registry() {
  local saved
  if [ -d "$E2E_CERTS_DIR/ghcr.io" ]; then
    if [ -f "$E2E_CERTS_DIR/ghcr.io/ca.crt" ] && [ -z "$SYS_CA_SAVED" ]; then
      # Saved through sudo (the file may be root's alone), and named for the undo only
      # once the save succeeded: a failed save must never be restored over the CA.
      saved="$(mktemp)"
      if ! sudo cp "$E2E_CERTS_DIR/ghcr.io/ca.crt" "$saved"; then
        rm -f "$saved"
        echo "hermetic: cannot save $E2E_CERTS_DIR/ghcr.io/ca.crt, so it is not replaced; stopping" >&2
        return 1
      fi
      SYS_CA_SAVED="$saved"
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
