# Secret store requirements — what can take 1Password's place

goat keeps no credential value on an operator's disk. Values live in a secret
store; operator files hold **references** to them; a value reaches a command
only while that command runs ([ADR 1186](../adr/1186-credentials-at-rest-are-references-only.md)).
The goat team uses 1Password. Any store that meets the requirements below can
replace it, and the two integration points in goat are already store-neutral.

**Who this is for:** an organisation adopting goat that uses, or must use,
something other than 1Password — for cost, sovereignty, an existing enterprise
contract, or a network that cannot reach a SaaS.

---

## 1. What goat code relies on

goat touches the store in three places. Everything else in this document
is about whether the store is safe to rely on, not whether goat can call it.

| Integration point | File | What it expects of the store |
|---|---|---|
| The loader | `.envrc` | A reference is a string with a fixed prefix (`GOAT_SECRET_REF_PREFIX`, default `op://`). The loader exports `.envrc.local.d/*.refs` lines as references and refuses a file holding anything else. It never calls the store. |
| The injector | `ops/scripts/with-secrets` | A command that takes another command and runs it with every reference in the environment replaced by its value. `GOAT_SECRET_STORE=1password` runs `op run --`; `GOAT_SECRET_STORE=openbao` runs `ops/secrets/openbao/bao-run` (secret-store-openbao.md); `GOAT_SECRET_STORE=custom` runs `$GOAT_SECRET_RUN`. `ops/scripts/ansible-preflight.sh` calls it when `GOAT_REFERENCES_ONLY=1`. A live secrets session (`ops/scripts/secrets-session`, [secrets-session.md](secrets-session.md); WIP, not for production use) is used first: one approval, up to 24 hours, per store. |
| The site generator | `ops/scripts/newsite-secrets.sh` | Persist missing values and return references, without rotating existing values on retry. 1Password uses JSON over CLI stdin and an explicitly selected vault. A custom store supplies `GOAT_SECRET_INIT`; its JSON protocol is documented in [newsite secret generation](../operations/newsite-secret-generation.md). |

A store whose own injector reads references from the environment (1Password's
`op run`) plugs in with two variables. A store whose injector works differently
(it injects a whole project by key name, or reads an encrypted file) needs a
short adapter script as `GOAT_SECRET_RUN`: read each reference from the
environment, fetch its value, `exec` the command. The adapter is held to R2
below — no value in a file, in argv, or in the parent shell.

---

## 2. Requirements

Each requirement says why goat needs it and how to test a candidate. **Must**
requirements are the bar; a store that fails one is not a substitute, however
it scores elsewhere. **Should** requirements separate adequate from good.

### Must

| # | Requirement | Why | Test |
|---|---|---|---|
| R0 | **The server never holds a key that can decrypt a stored value.** Values are encrypted on the person's device; sharing re-wraps keys on the device, never on the server; backups are ciphertext. | The store holds every credential the team has. A server that can decrypt them is the single most valuable target in the estate and must then be hidden behind a VPN; a server that cannot may stand on the public internet. 1Password works this way; the OpenBao store built in October 2026 did not and was stopped. | Plant one value; image the server's disk and memory with root; confirm the value is not recoverable from either. Remove a member; confirm their cached key opens nothing written after the rotation. |
| R1 | **A reference names exactly one value and contains none of it.** One field, not a whole item or project. | References are the only credential-related thing left on disk and in shared templates; they must be safe to commit and to read aloud. | Put a reference file through gitleaks and the store's own leak scanner: clean. Resolve one reference: one value. |
| R2 | **Values reach one process only, at run time.** A CLI verb resolves references into a child process's environment without writing the value to a file, a command line, or the calling shell. | This is the rule itself. A value in argv is visible in `ps` to every local user; a value in a temp file is a copy with no owner. | Run `sleep 60` through the injector; read `/proc/<pid>/cmdline` (Linux) or `ps -ww` (macOS): no value. Watch file writes during the run (`fs_usage` / `strace -e trace=file`): no value-bearing file. |
| R3 | **No readable cache on disk.** Anything the client caches locally is encrypted and unlocked only by the person's authentication. | Otherwise the store re-creates the plaintext-on-disk problem one directory over. | Store a unique test value, resolve it, then search the client's data directory and swap for that value: not found. |
| R4 | **Access by group, in partitions sized to blast radius.** At least as many partitions as goat's five (cloud and third party; host access; per-site services; operator identity roster; build and software). A person is granted a partition by joining a group, never secret by secret. | A per-secret grant cannot be audited or revoked at the speed of a departure, and one partition for everything makes every leak total. | Create two partitions and two groups; confirm a member of one cannot resolve a reference into the other. |
| R5 | **Revoking a person is one action and takes effect at their next resolve.** | Offboarding and a suspected compromised laptop are the same operation, and both are time-critical (credential-revocation-and-offboarding.md). | Remove a member from the group; their next resolve fails without waiting for a cache to expire beyond a documented, bounded time. |
| R6 | **Every read is attributed to a person or a machine identity.** Who resolved which value, when. | Leak response starts with "who could have read it, and who did" (credential-leak-runbook.md — in `goat-credentials`). A store that cannot say is a store whose leaks cannot be scoped. | Resolve a value; find the event naming the person, the value and the time. Record which subscription tier this needs. |
| R7 | **Strong human authentication.** Multi-factor at minimum; a hardware-held factor (FIDO2/passkey, platform authenticator) available. | The store now holds every credential; its login is the new single point of compromise. | Enforce the policy for a group and confirm a password-only login is refused. |
| R8 | **Machine identities scoped to one partition and revocable on their own.** | CI and hosts resolve secrets too. A human's session must never be what a pipeline runs on, and one pipeline's token must not reach another partition. | Create a machine identity on one partition; confirm it cannot read another; revoke it without touching any person. |
| R9 | **Recovery without a single-person lockout.** At least two administrators can restore a member's access; no one person holds the only recovery path. | A store only one person can recover is a store the organisation loses when that person is unavailable. | Walk the recovery of a member who has lost their device, performed by the second administrator. |
| R10 | **Previous values stay retrievable.** Per-value history, or a documented way to keep an old value beside the new one. Under R0 a reader's removal rotates the partition key and destroys the versions that key could open; keep a value that must outlive a rotation as its own field, not as history. | Rotation in goat is overlap-first (add new, repoint, verify, remove old), and some old values stay needed: a pre-rotation backup key still decrypts the backups taken under it. | Change a value; retrieve the previous one. |
| R11 | **Complete export to a documented format.** | Replacing the store later must not require re-minting every credential. | Export a partition; confirm every value and field name is present. |

### Should

| # | Requirement | Why | Test |
|---|---|---|---|
| S1 | **Resolves while the provider is unreachable**, for a bounded, documented time, from the encrypted cache. | goat is built to run at a denied edge with no internet; an operator who cannot resolve a credential cannot operate there. | Cut the network after one successful resolve; resolve again; record how long it keeps working. |
| S2 | **Self-hostable, or a choice of hosting jurisdiction.** | Partner and national organisations may be barred from holding credentials in another country's SaaS. | Vendor documentation plus contract terms. |
| S3 | **A stated cryptographic posture** — ideally a FIPS 140-3 validated module; at minimum published algorithms and key hierarchy. | goat targets FIPS 140-3 for its own crypto (fips-pqc-path.md); the store holding its keys should not be the weakest link. | Vendor certificate number or security whitepaper. |
| S4 | **Masks resolved values in the child's output.** | Terminal output and session transcripts are a common leak path. | Echo a resolved value through the injector: masked. |
| S5 | **Tags or metadata on each value** — owning team, environment, class, origin. | Retiring an environment's secrets needs a reuse check: a value named for a dead environment can still be load-bearing for a live one. | Tag a value; list values by tag. |

---

## 3. Candidates

What each store offers **as its vendor documents it**. goat has exercised only
the 1Password path, and only in parts (below); every other row is a starting
point for the tests in §2, not a result. "Adapter" means the store's own tool
does not read references from the environment, so a short script sits between
goat and the store (§1).

| Store | Injector for R2 | Plugs into goat | Where it falls short | Offline (S1) | Hosting (S2) |
|---|---|---|---|---|---|
| **1Password** (Business/Teams) | `op run -- <cmd>` resolves `op://vault/item/field` references in the environment | directly — the default | R6 read attribution needs a tier that exposes item-usage events: check the tier you buy. S3: no FIPS validation claimed that we know of | encrypted local cache via the desktop app; not measured for `op run` | SaaS only |
| **Bitwarden Secrets Manager** | `bws run -- <cmd>` injects the secrets of a project by key name | adapter: map each reference to a secret id, or use one project per `.refs` file | references are secret ids, not paths; confirm R1 one-value-per-reference in your layout | check | SaaS or self-hosted |
| **Infisical** | `infisical run -- <cmd>` injects an environment's secrets | adapter, as for Bitwarden | R6 and R9 depend on edition: check | check | SaaS or self-hosted (open source core) |
| **OpenBao as built** (server-side decryption, [ADR 1203](../adr/1203-self-hosted-credential-store-openbao.md), superseded) | `bao-run` resolves `bao://` references | directly | **fails R0**: the server unseals the storage key and answers plaintext; values were decryptable by anyone with the host's disk. Stopped and emptied 2026-10-31 | yes, via the client cache | self-hosted |
| **OpenBao with client-side encryption** (goat-built client-side encryption in `with-secrets` and `people.py`, [ADR 1212](../adr/1212-team-secrets-never-decryptable-on-the-server.md)) | `with-secrets` fetches ciphertext and decrypts on the device | directly, once the layer exists | the encryption layer is goat's to write and review; two keys per person; membership change re-encrypts the partition | cache to be built | self-hosted, public-safe |
| **Vaultwarden + Bitwarden CLI** (self-hosted Bitwarden-compatible server; [secret-store-vaultwarden.md](secret-store-vaultwarden.md), [ADR 1212](../adr/1212-team-secrets-never-decryptable-on-the-server.md)) | `ops/secrets/vaultwarden/run` resolves `bw://<collection>/<item>/<field>` through the `bw` CLI: one sync, two listing calls, N references | directly (`GOAT_SECRET_STORE=vaultwarden`); WIP, not for production use: the injector is not in this repository | R0 holds by the client model: the server holds ciphertext (disk-image gate a). R6 is the organisation event log, client-reported, with no row for a read. R8: no machine identity — a `bw` login is a user account with a master password; machine secrets stay in SOPS+age files. The server serves the web vault's code; the CLI from a signed release is the adapter's client | yes: `GOAT_BW_SYNC=0` answers from the CLI's encrypted local copy | self-hosted, public-safe |
| **HashiCorp Vault** | none in the `vault` CLI; OpenBao's `bao-run` speaks the same API | adapter (`bao-run` with `VAULT_ADDR`) | no longer open source (BSL licence) | no | self-hosted |
| **Doppler** | `doppler run -- <cmd>` | adapter | SaaS-only rules it out wherever S2 is binding | check | SaaS only |
| **AWS Secrets Manager / GCP Secret Manager / Azure Key Vault** | none native; adapter over the cloud CLI | adapter | R4 groups become IAM policy per secret path — workable, but review is harder than a vault-per-partition model; R7 is the cloud account's login | no | the cloud provider's regions |
| **SOPS + age** (encrypted files) | `sops exec-env <file> <cmd>` | adapter | **R5 and R6 fail**: a file already copied cannot be un-shared, so revoking a person means rotating every value they could decrypt; there is no read log. R4 is recipient lists per file | yes — fully offline | wherever the files are |

**How to read this for a decision.** If the organisation must run disconnected
or sovereign (S1, S2 binding), the realistic choices are a self-hosted Vault or
OpenBao on-site, or self-hosted Bitwarden or Infisical, each behind an adapter.
If it cannot run a server at all, SOPS + age meets the at-rest rule but moves
revocation and audit into process: every departure is a rotation. A SaaS store
is the simplest fit where neither constraint binds.

### What goat has exercised with 1Password

| Piece | Status |
|---|---|
| `op://` references resolving to the same bytes as the plaintext they replace | **Measured** 2026-09-10 on one operator laptop, a sample of references |
| `.envrc` loader exporting references and refusing a non-reference line | **Measured** 2026-09-22 with direnv on test files |
| `with-secrets` dispatch: custom runner and unknown-store refusal | **Measured** 2026-09-22 |
| `with-secrets` default path handing the command to `op run` | **Measured** 2026-09-22 up to the hand-off; `op run` then refused the command on a malformed reference (`op://Private/<item>`, no field) in that shell — one bad reference stops every run |
| A full Ansible run with `GOAT_REFERENCES_ONLY=1` through `op run` | **Unmeasured** — needs a signed-in operator; the first retirement (goat-credentials local-secrets-pattern.md, step 4) is that test |
| `op run` resolving ~500 references per command: latency | **Unmeasured** |
| S1 offline resolution | **Unmeasured** |

---

## 4. Checking a store you already run

Run the tests in §2 with one throwaway value, in order R1–R11, and record
pass, fail or not-applicable with the evidence. Then point goat at it:

```bash
export GOAT_SECRET_STORE=custom
export GOAT_SECRET_RUN="/path/to/your-adapter --"     # or the store's own run verb
export GOAT_SECRET_REF_PREFIX="vault://"               # whatever your references start with
```

A laptop is done when no file under `.envrc.local` or `.envrc.local.d/` holds
an assignment that does not start with that prefix, no stale copy (`.bak`,
`.pre-sync`) of one exists, and every such file is mode 0600. The goat team
checks this with the `at-rest` audit in its private `goat-credential-exposure`
repository (`--prefix` takes your prefix); the check is those three conditions
and nothing more, so any organisation can reproduce it.
