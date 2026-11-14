# Promoting the passive node to SPIRE server

**When to use this.** The active node holding the SPIRE server is GONE — hardware
loss, or a rebuild you have decided not to wait for. Not for a reboot, and not as
a drill against a healthy active node: two SPIRE servers in one trust domain both
mint CAs, and the SVIDs they issue do not chain to each other.

**What this preserves.** The trust domain. Every SVID already issued keeps
chaining, and every relying party's trust bundle stays valid. Without the staged
CA key, promotion mints a *new* CA and you are re-issuing every workload identity
and re-distributing every trust bundle by hand.

---

## Which failure this covers

| Failure | Answer | Status |
|---|---|---|
| The node holding the SPIRE server is lost | Promote a passive node with the staged CA key (this runbook). A person does it; workloads hold a one-hour SVID, so identity lapses if promotion takes longer | play and runbook written |
| A site is cut off from the issuer | A SPIRE server and trust domain per site, recognised by federation ([ADR 1164](../adr/1164-per-site-spire-trust-domains.md)) | designed, not built |
| The server is lost and nobody is there to promote | Not built. SPIRE's own high availability needs every server to see one consistent datastore; the `spire` database is one instance on one node, and a Spock-replicated copy would not qualify. [ADR 1236](../adr/1236-a-sites-spire-server-is-made-redundant-by-two-independent-servers-after-a-gated-trial.md) proposes two independent servers behind spire-ha-agent, gated on a trial | not built |

## What survives the loss, and what does not

| SPIRE state | Where it lives | Survives? |
|---|---|---|
| Registration entries, attested nodes | `spire` database on the lost node's `postgres-a` | **Not on the promoted node** — the `spire` database is deliberately in no Spock replication set ([`island-mode-multi-writer.md`](../design/island-mode-multi-writer.md)), so the passive node holds no copy. Promotion does not need one: the deploy in step 4 creates the database, re-creates the registration entries from the declared configuration and attests the agent with a fresh join token. The lost node's copy is also in its pgBackRest backup (the stanza is merely *named* `<site>-netbird`). |
| **CA private key** | `/opt/spire/data/server/keys.json`, local disk | **No** — `KeyManager "disk"`, not in Postgres, not in the Spock set (`spock_mesh_database` is one database, `netbird`), not in any backup. |
| CA public cert | `/opt/spire/data/server/spire-ca.pem` | Derived from the key |
| Agent SVID | `/opt/spire/data/agent` | Regenerated via a fresh join token |

`ops/ansible/spire-ca-standby.yml` closes the one gap by staging the CA key,
age-encrypted, onto every passive member. **That play is a snapshot, not a
stream** — check the stamp before trusting it.

---

## Before you start

Confirm the staged material exists and is current:

```bash
sudo cat /opt/spire/data/standby/spire-ca-staged.json.age.stamp
```

Read three fields:

- `staged_from` — must name the node that is gone.
- `source_key_mtime` — how old the snapshot is.
- `spire_ca_ttl` — default `2160h` (90 days).

**If the snapshot is older than `spire_ca_ttl`, the CA may have rotated since it
was taken.** Promotion will still work and still preserve the trust domain if the
bundle still carries that key; if it does not, you are in the new-CA case below.
Say which case you are in, in writing, before proceeding — a promotion recorded as
"clean" that silently re-issued identities is the worst outcome here.

You need an age identity that is one of the recipients — the operator key or the
break-glass key. There are deliberately two, so one lost identity is not a lost
trust domain.

---

## Promotion

1. **Confirm the old server is really gone.** Not reachable, not merely quiet.
   If it can be brought back, bring it back instead; that is strictly less work
   than this.

2. **Decrypt the staged CA into place** on the node being promoted:

   ```bash
   sudo install -d -m 0700 -o root -g root /opt/spire/data/server
   age -d -i ~/.config/age/keys.txt \
     /opt/spire/data/standby/spire-ca-staged.json.age \
     | sudo install -m 0600 -o root -g root /dev/stdin /opt/spire/data/server/keys.json
   ```

   The pipe avoids a plaintext temp file. Verify it parses before continuing:

   ```bash
   sudo python3 -c "import json;print(sorted(json.load(open('/opt/spire/data/server/keys.json'))))"
   ```

   Expect `['keys']`.

3. **Make this node the designated server.** Set `spire_enabled: true` in its
   `host_vars`, and set the old node's to `false` in the same change. The role is
   fail-closed on this flag and `spire-ca-standby.yml` asserts exactly one server
   per group, so leaving both true blocks the next run rather than creating a
   second CA.

4. **Run the deploy** against the promoted host only:

   ```bash
   cd ops/ansible
   OBJC_DISABLE_INITIALIZE_FORK_SAFETY=YES ../scripts/ansible-preflight.sh \
     -i inventory/mgmt-hosts.ini -i inventory/doors-groupvars.ini \
     spire-deploy.yml -e target=<group> --limit <promoted-host> \
     -e postgres_superuser_password="$<SITE>_PG_PASS"
   ```

   The role finds an existing `keys.json` and does **not** mint a new CA.

5. **Verify the trust domain is the same one**, which is the whole point:

   ```bash
   sudo docker ps --filter name=spire --format '{{.Names}} {{.Status}}'
   sudo /opt/spire/bin/spire-server bundle show -socketPath /run/spire/sockets/*.sock | head -3
   ```

   The trust domain must match the `trust_domain` line in the stamp. If it does
   not, step 2 did not take and you are running a new CA.

6. **Re-stage onto the remaining passive members**, or the next failure has no
   copy:

   ```bash
   ../scripts/ansible-preflight.sh -i inventory/mgmt-hosts.ini \
     -i inventory/doors-groupvars.ini spire-ca-standby.yml -e target=<group>
   ```

---

## If the CA did rotate and the staged key is stale

The trust domain name survives; the signing key may not. Then:

- Existing SVIDs fail to chain as they are re-issued against the new key.
- Every relying party that pins the bundle needs the new bundle. On ZAGROS that is
  the zenoh router's `root_ca` — and `zenohd` does not hot-reload it, so it is a
  restart, handled by the `spire-trust-sync` arm.
- Registration entries are unaffected: they are in Postgres and survive.

This is recoverable but it is not transparent. **Prevent it instead:** re-run
`spire-ca-standby.yml` on the normal deploy cadence and after any known rotation.

---

## Related

- Decision of record and the issuer-only posture: `../adr/0412-spiffe-spire-workload-id.md`
- Service classes and what a passive node runs: `ops/standby/service-classes.yaml`
- Generic standby promotion for everything else: `ops/standby/promote-zagros-b.sh`
- Continuity doctrine: `../design/ha-failover.md`
