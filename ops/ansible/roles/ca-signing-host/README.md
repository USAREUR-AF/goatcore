# ca-signing-host

> ## DORMANT — not wired to any playbook
>
> This role provisioned the **Path B** signing host for the dev-mode CA. Path B
> was **retired**: its host, Orin-C (`mgmt-vega-03-orin`), was
> decommissioned with the vega site, its driver playbook
> `ops/ansible/ca-signing-host.yml` was deleted, and the `vega_obs_hosts`
> inventory group was archived. See
> ADR 0406.
>
> **The role is kept deliberately, not by neglect.** Retiring the path is meant to
> be reversible: if an operator ever needs to mint from a machine without a Go
> toolchain, or an audit needs the per-issuance OS identity rather than the git
> author, restoring Path B should be a retarget rather than a rewrite. Nothing can
> run this role by accident — there is no playbook that includes it.
>
> **To bring it back** you need three things, none of which exist today: a host in
> a **live** goatnet (not vega), a driver playbook, and an inventory group. Then
> repopulate `ca_signing_host_operators` — the three SSH public keys collected for
> the old host were released with the path and are not held for this purpose any
> more. Read the ADR amendment first; the audit consequence is the part that
> motivated the retirement.
>
> The `tests/syntax-check.sh` fixture still runs and still passes. It uses this
> role's own `tests/playbook.yml`, not the deleted driver, so it keeps the role
> from rotting silently while dormant.

Provisions a signing host (vega Orin-C in the first instance) as **Path B** of the dev-mode CA interim distribution model — see ADR 0406 §"Interim mode".

This role was for operators who chose to mint via SSH-to-Orin-C (Path B) rather than running `bundle-create` on their own laptop (Path A). Both paths produced identical bundles. **The role was never a prerequisite for Path A** — Path A operators build `bundle-create` from the repo themselves and use their local clone's encrypted blob. Path A is now the only path.

## What this role does

1. Installs Go (if not present) and builds `bundle-create` + `bundle-extract` from the cloned repo into `/usr/local/bin/`.
2. Creates `/opt/goat-ca/` containing the encrypted CA root, `root-cert.pem`, the operator-curated `wg-cp0-relays.json`, and the `ca-id` marker file.
3. Clones the repo to `/opt/goat-ca/repo/` for the allowlist git checkout.
4. Drops the `issue-bundle` wrapper script at `/usr/local/bin/issue-bundle` (Section "Operator workflow" below).
5. Creates the `goat-ca-operators` UNIX group.
6. Provisions one UNIX account per operator (named in `ca_signing_host_operators`) with their SSH pubkey + git author identity.
7. Configures auditd rules to record every `execve` of `bundle-create`, `bundle-extract`, and `issue-bundle`, keyed for log-search.
8. Configures sshd to allow the `goat-ca-operators` group via `Match Group`.

## What this role does NOT do

- Does NOT decrypt the CA root. The encrypted blob is opaque on Orin-C; decryption happens at signing time using the operator's age identity piped via SSH stdin (`/dev/stdin`).
- Does NOT install age. Operators decrypt on their own laptops; Orin-C only sees the decrypted CA private key in RAM via `/dev/stdin` for the duration of one `bundle-create` invocation.
- Does NOT push allowlist changes to relays. Operators commit + push the allowlist update; an operator with appropriate access (or the trunk owner remotely) runs `wg-cp0-bootstrap.yml` afterward to propagate. Future enhancement: a post-receive hook on Orin-C that triggers the propagation automatically.
- Does NOT ship audit records to the obs-host. Local `/var/log/audit/audit.log` is the durable record for the interim period; obs-host shipping is a follow-on (Block 27 wide-events tier integration).

## Operator workflow

```
# On operator's laptop:
ssh orin-c.vega issue-bundle \
  --site vega \
  --device-id mgmt-foo-orin \
  --wg-cp0-address 198.18.0.20/24 \
  --acl-groups VEGA,VEGA-MGMT \
  --out /tmp/foo.bundle.cbor \
  < ~/.config/age/keys.txt

# On orin-c, the wrapper resolves to:
bundle-create \
  --ca-key /opt/goat-ca/ca.key.age \
  --age-identity /dev/stdin \
  --ca-id "$(cat /opt/goat-ca/ca-id)" \
  --tls-trust-anchor /opt/goat-ca/root-cert.pem \
  --wg-cp0-relays /opt/goat-ca/wg-cp0-relays.json \
  --update-allowlist /opt/goat-ca/repo/ops/enrollment/wg-cp0-bundle-allowlist.json \
  "$@"
```

Operator's age identity flows: laptop → SSH stdin → `/dev/stdin` on Orin-C → `age.ParseIdentities` → in-memory only → garbage-collected when bundle-create exits. No file written.

## Variables

`ca_signing_host_operators`: list of dicts with required fields `name`, `email`, `ssh_pubkey`, optional `comment`.

`ca_signing_host_ca_id`: the dev CA ID string (e.g. `dev-goat-ca-2026-04-20`). Required, no default.

`ca_signing_host_repo_url`: git URL of the repository whose allowlist the signing host trusts. Required, no default.

`ca_signing_host_repo_branch`: branch to track (default `main`).

`ca_signing_host_relays_file`: path on the Ansible controller of the operator-curated `wg-cp0-relays.json` (default copies from `ops/enrollment/wg-cp0-relays-prod.json`).

`ca_signing_host_allowlist_relpath`: path within the cloned repo of the allowlist file (default `ops/enrollment/wg-cp0-bundle-allowlist.json`).

## Security notes

- `/opt/goat-ca/ca.key.age` is mode 0644 (group-readable, world-readable). The blob is age-encrypted; readability is harmless and lets every operator check its presence without sudo.
- `/opt/goat-ca/wg-cp0-relays.json` is mode 0644 — relay endpoints + pubkeys, no privkeys. Sensitive in the operational sense (maps peers to relays) but not cryptographically.
- The `goat-ca-operators` group is the only authorized group for SSH; `Match Group` denies everyone else. Keep group membership tight and audited via the operator roster.
- auditd records every `execve` of the three load-bearing binaries. Log search:

  ```
  ausearch -k bundle_create -ts today
  ausearch -k issue_bundle  -ts today
  ```

- The age identity never lands on Orin-C disk. If an operator's SSH session is killed mid-issuance, the identity bytes in pipe + RAM are reaped by the kernel along with the bundle-create process.
