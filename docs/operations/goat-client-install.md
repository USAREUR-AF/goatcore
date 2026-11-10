# goat-client install runbook

Operator-side automation that installs goat-client (Block 76 cross-platform daemon + GUI for wg-cp0 onboarding) on a laptop in one command. Replaces the manual quickstart walkthrough end-to-end on Linux and macOS.

## When to use which

| Path | When |
|---|---|
| **This playbook** ([`ops/ansible/goat-client-install.yml`](../../ops/ansible/goat-client-install.yml)) | You're on Linux or macOS and want the install to be repeatable, idempotent, and auto-start-configured. |
| **Manual quickstart** | You're on Windows (Ansible-on-Windows is heavier than the PowerShell snippet); or you want to read every step. |

The playbook does:

1. Download the v0.3.7 release archive + SHA256 + cosign-bundle from `goat-maintainer/goat-client`
2. Verify SHA256 (always) and Sigstore keyless signature (if `cosign` is on `$PATH`)
3. Install `goat-clientd` to `/usr/local/bin` (sudo)
4. Drop trust-roots PEM at the platform-conventional config dir
5. Install the `goat-client` GUI binary from the same verified archive to `/usr/local/bin`
6. Generate a per-user auto-start service (systemd-user on Linux, launchd LaunchAgent on macOS) so the daemon survives login
7. Print the next manual steps (bundle issuance + GUI launch + connect) at the end

It does NOT do:

- Bundle issuance — that's a separate operator-side ceremony with age identity custody (see `operator-bundle-issuance.md`)
- Bundle import — interactive; the operator drags `bundle.cbor` onto the GUI window
- wg-cp0 plumbing — that's `operator-laptop-onboarding.yml`'s job (sister playbook)

## Pre-flight

| Required | What | Why |
|---|---|---|
| ✅ | Ansible ≥ 2.16 (`pip install ansible`) | Driver |
| ✅ | sudo password | Phase 4 + 6 install binaries to `/usr/local/bin` |
| — | ~~Go toolchain~~ | No longer needed. Phase 6 used to build the GUI from source, because the v0.1.0 archive shipped the daemon alone; every release since v0.1.1 carries both binaries, and rebuilding threw away the cosign signature Phase 3 had just verified. |
| 🟡 | `cosign` ([sigstore.dev](https://docs.sigstore.dev/cosign/installation/)) | Phase 3 verifies the Sigstore signature. SHA256 verification still runs without it; the playbook prints a warning and continues. |

## One-line invocation

```bash
cd /path/to/goat

OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES \
  ansible-playbook -i localhost, --connection local -K \
    ops/ansible/goat-client-install.yml
```

`-K` prompts for the sudo password once and caches it for the run. macOS sudo doesn't have NOPASSWD by default; even on Linux, `/usr/local/bin/` writes need root.

`OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES` quiets a macOS Python-fork warning that's otherwise noise. Linux operators can skip it.

## Common overrides

```bash
# Daemon-only install (you're on a server, or don't want the GUI):
... -e goat_client_install_gui=false

# Skip cosign verification (cosign not installed; SHA256 still runs):
... -e goat_client_skip_cosign=true

# Skip per-user auto-start service (you'll run `goat-clientd` in a terminal):
... -e goat_client_install_service=false

# Pin a different version (default is v0.3.7):
... -e goat_client_version=v0.3.7

# Install to a non-default prefix (e.g. ~/.local for user-only install):
... -e goat_client_install_prefix="$HOME/.local"

# Verify-only (no changes):
... --check
```

## Post-run verification

The playbook ends with a "Summary" debug block reporting installed paths, IPC socket location, and service status. You can re-verify any time:

```bash
# Daemon binary present + version
goat-clientd --version

# Service running?
systemctl --user status goat-clientd            # Linux
launchctl print gui/$UID/io.goat-maintainer.goat-clientd # macOS

# IPC socket listening?
ls -la "$XDG_RUNTIME_DIR/goat-clientd.sock"     # Linux
ls -la ~/.goat-client/goat-clientd.sock         # macOS

# GUI starts?
goat-client --daemon-addr "$XDG_RUNTIME_DIR/goat-clientd.sock"
```

## After install — finishing the onboarding

The playbook stops short of bringing wg-cp0 up because that requires an enrollment bundle (which requires the offline-CA ceremony). The full path:

1. **Get a bundle.** Run `operator-bundle-issuance.md` (operator with age identity issues a bundle for the target device).
2. **Import + connect.** Launch `goat-client`, drag the `bundle.cbor` onto the window (or `File → Import Bundle...`), click **Connect**.
3. **Confirm tunnel.** `ip addr show wg-cp0` (Linux) / `ifconfig | grep -A2 utun` (macOS) — the `198.18.0.<X>` address from your bundle should be bound.

Sister playbook `operator-laptop-onboarding.yml` handles steps 2-3 via the legacy `wg-cp0-bundle-apply` CLI path (operator-class, no GUI). Either is correct; goat-client's GUI path is the friendlier one for non-CLI-fluent users.

## Idempotency

Re-run safely. Each phase gates on the observed state:

- Phase 2 always re-downloads (cheap; ensures cache is fresh against an upstream re-tag)
- Phase 3 verifies the freshly-downloaded archive
- Phase 4 reinstalls the binary (atomic copy)
- Phase 5 rewrites the trust-roots PEM (cheap)
- Phase 6 reinstalls the GUI binary from the extracted archive (atomic copy)
- Phase 7 reloads the service definition + restarts

A second consecutive run on a fully-installed laptop completes in ~10-20 seconds and changes nothing meaningful.

## Uninstall

```bash
# Linux
systemctl --user disable --now goat-clientd
rm ~/.config/systemd/user/goat-clientd.service
systemctl --user daemon-reload
sudo rm /usr/local/bin/goat-clientd /usr/local/bin/goat-client
rm -rf ~/.config/goat-client ~/.cache/goat-client

# macOS
launchctl bootout gui/$UID/io.goat-maintainer.goat-clientd 2>/dev/null
rm ~/Library/LaunchAgents/io.goat-maintainer.goat-clientd.plist
sudo rm /usr/local/bin/goat-clientd /usr/local/bin/goat-client
rm -rf "$HOME/Library/Application Support/goat-client" ~/.cache/goat-client
```

## Troubleshooting

| Symptom | Diagnosis | Fix |
|---|---|---|
| `cosign: command not found` warning | `cosign` not on `$PATH` | Install via `brew install cosign` or skip with `-e goat_client_skip_cosign=true` |
| `goat_client_build_gui is retired` | You passed the old override | Use `-e goat_client_install_gui=false`. The playbook fails fast rather than silently installing a GUI you asked it not to. |
| `xattr: No such xattr` on macOS | Already stripped or never had quarantine | Ignore (the task is best-effort, `failed_when: false`) |
| systemd-user `Failed to connect to bus` | Headless / no DBus session | Either run `loginctl enable-linger $USER` (persistent user manager) or skip the service: `-e goat_client_install_service=false` |
| GUI launches but says "daemon unreachable" | Daemon not running, or a socket-path mismatch | The playbook's Summary prints the socket path in use; check `goat-clientd` is up before blaming the GUI |
| `tar: ... goat-clientd: Cannot open: File exists` on re-run | Cache directory dirty | `rm -rf ~/.cache/goat-client` and re-run |

## Maturity caveat

v0.3.7 is operator-class dogfood on the desktop, validated headless: a disposable cloud environment ran the full six-check matrix 6/6 against a real NetBird management plane on 2026-07-31 (`docs/operations/cloud-env-lifecycle.md`, `goat-client-assay` row).

Know what you are installing before you hand it to anyone else. The binaries are cosign-signed but **not** Apple Developer ID-notarized or Authenticode-signed, so macOS needs the quarantine attribute cleared by hand and Windows shows a SmartScreen prompt. There is **no auto-update**: moving to a later release means re-running this playbook. The readiness gap table — signing, updates, mobile, Windows — is in [`docs/design/goat-client.md`](../design/goat-client.md).
