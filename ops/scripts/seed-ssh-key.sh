#!/usr/bin/env bash
# seed-ssh-key.sh — first-contact bootstrap for fresh-install mgmt-hosts.
#
# Idempotent. Two atomic steps, each safe to re-run:
#   1. Push operator pubkey into ~/.ssh/authorized_keys (ssh-copy-id)
#   2. Add NOPASSWD sudoers entry at /etc/sudoers.d/<user> so Ansible
#      become tasks work without `--ask-become-pass`
#
# After this returns 0, every Ansible playbook in this repo can connect
# over key-auth and run privileged tasks without further bootstrap.
#
# Use case: a new mgmt-host (e.g. vega Orin pair) was installed by a third
# party with password auth only. The default user/pass is typically weak; we
# trade it for key-auth + NOPASSWD sudo as the standard Ansible-managed
# baseline.
#
# Usage:
#   ops/scripts/seed-ssh-key.sh <user>@<host> <PASSWORD_ENV_VAR>
#
# Examples:
#   ops/scripts/seed-ssh-key.sh "goat@${VEGA_A_HOST}" VEGA_A_PASS
#   ops/scripts/seed-ssh-key.sh "goat@${VEGA_B_HOST}" VEGA_B_PASS
#
# After running, operators SHOULD (Phase-2 follow-up):
#   1. Rotate the password (`ssh ... 'passwd'` or via Ansible)
#   2. Disable password auth (`PasswordAuthentication no` in sshd_config)
# Once password auth is off, key-auth + NOPASSWD-sudo is the standard
# Ansible-managed posture. Until then, the weak default password is still
# a path of compromise.
#
# Requires: sshpass on operator laptop (`brew install sshpass` on macOS).

set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <user>@<host> <PASSWORD_ENV_VAR>" >&2
  echo "example: $0 \"goat@\${VEGA_A_HOST}\" VEGA_A_PASS" >&2
  exit 64
fi

target="$1"
pass_var="$2"
pass="${!pass_var:-}"

if [[ -z "$pass" ]]; then
  echo "error: env var '$pass_var' is empty or unset." >&2
  echo "  set it in .envrc.local and run 'direnv allow' before re-running." >&2
  exit 65
fi

# A value that still holds a secret store's reference was not resolved: run
# through ops/scripts/with-secrets, which resolves it. Sent as it is, the node
# would take the reference as its password. Refused before anything reaches the
# host, naming the variable and the scheme, never the reference itself (GB-269).
for scheme in op:// bao:// bw:// ${GOAT_SECRET_REF_PREFIX:+"$GOAT_SECRET_REF_PREFIX"}; do
  if [[ "$pass" == "$scheme"* ]]; then
    echo "GB-269: $pass_var still holds a $scheme secret reference: it was not resolved, so nothing was sent to $target." >&2
    echo "  run through ops/scripts/with-secrets with the store signed in (goatbuilder join-node does), or export the value itself." >&2
    exit 1
  fi
done

if ! command -v sshpass >/dev/null 2>&1; then
  echo "error: sshpass not installed (brew install sshpass on macOS)." >&2
  exit 66
fi

# ---------------------------------------------------------------------------
# Step 1: pubkey. Probe first — skip if key auth already works.
# ---------------------------------------------------------------------------

if ssh -o ConnectTimeout=5 -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
       "$target" "true" 2>/dev/null; then
  echo "[seed-ssh-key] key auth already works for $target — skipping pubkey push."
else
  echo "[seed-ssh-key] pushing pubkey to $target via password auth..."
  SSHPASS="$pass" sshpass -e \
    ssh-copy-id -o StrictHostKeyChecking=accept-new "$target"

  if ssh -o ConnectTimeout=5 -o BatchMode=yes "$target" "echo seeded" >/dev/null; then
    echo "[seed-ssh-key] verified — key auth works for $target."
  else
    echo "error: post-seed key-auth probe failed for $target." >&2
    exit 1
  fi
fi

# ---------------------------------------------------------------------------
# Step 2: NOPASSWD sudoers entry for this user. Idempotent — only writes
# if /etc/sudoers.d/<user> doesn't already grant NOPASSWD ALL.
# ---------------------------------------------------------------------------

remote_user="${target%@*}"

# Probe: does NOPASSWD sudo already work?
if ssh -o BatchMode=yes "$target" "sudo -n true" 2>/dev/null; then
  echo "[seed-ssh-key] NOPASSWD sudo already configured for $remote_user — skipping."
else
  echo "[seed-ssh-key] seeding NOPASSWD sudoers entry for $remote_user via password auth..."
  # Use sshpass for the one-time sudo password since key auth doesn't grant
  # us NOPASSWD yet. Write to a temp file then move atomically with
  # `visudo -c -f` validation to avoid leaving a malformed sudoers file.
  # The password reaches sudo on stdin, never in ssh's argument list.
  printf '%s\n' "$pass" | SSHPASS="$pass" sshpass -e ssh -o BatchMode=no "$target" \
    "sudo -S -p '' bash -c '
      tmpf=\$(mktemp)
      echo \"$remote_user ALL=(ALL) NOPASSWD:ALL\" > \"\$tmpf\"
      chmod 0440 \"\$tmpf\"
      visudo -c -f \"\$tmpf\" >/dev/null
      mv \"\$tmpf\" /etc/sudoers.d/$remote_user
      chown root:root /etc/sudoers.d/$remote_user
    '" 2>&1 | grep -v '^\[sudo\]' || true

  if ssh -o BatchMode=yes "$target" "sudo -n true" 2>/dev/null; then
    echo "[seed-ssh-key] verified — NOPASSWD sudo works for $remote_user."
  else
    echo "error: NOPASSWD sudo seed failed for $remote_user." >&2
    exit 1
  fi
fi

echo "[seed-ssh-key] $target ready for Ansible. Phase-2 follow-up: rotate password + disable password auth."
