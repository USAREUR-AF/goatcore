# Secrets session — one approval for up to 24 hours

**Status: WIP, not for production use.** The gate that lifts it: one live 24-hour run on each store (1Password, OpenBao, Vaultwarden), with `with-secrets` resolving references unattended for the whole period and `end` closing the session.

| Field | Value |
|---|---|
| Status | **Built** in this repository: `ops/scripts/secrets-session`, the session read in `ops/scripts/with-secrets`, `tests/test_secrets_session.py` (stub stores, a stub keychain and a stub kernel keyring). **Unmeasured** against a live store |
| Canon | [ADR 1186](../adr/1186-credentials-at-rest-are-references-only.md) (no value on disk; the OS keychain is where aws-vault already keeps keys), [ADR 1204](../adr/1204-declared-secrets-posture-and-zero-saas-quickstart.md) (one reference grammar), [ADR 1212](../adr/1212-team-secrets-never-decryptable-on-the-server.md) (the OpenBao and Vaultwarden shapes) |
| Related | [secret-store-requirements.md](secret-store-requirements.md) (the injector), [secret-store-vaultwarden.md](secret-store-vaultwarden.md), [pluggable-dependencies.md](pluggable-dependencies.md) |

## What it is

A person approves the secret store once, and for up to 24 hours every command run through `ops/scripts/with-secrets` resolves its references without asking again. Without a session, 1Password asks for desktop approval on each `op run`, so a deploy or an agent's run stops whenever the person is away from the laptop.

The session is one token per store, held in the OS key store with its expiry: the macOS login keychain, or on Linux the kernel user keyring. `with-secrets` reads it, hands it to that store's injector only, and removes it from the environment before the command itself runs. No session, an expired one, or no key store: `with-secrets` behaves as it does without this feature.

The operator's words, which this design implements:

> "we need a way to approve all secret holder service actions for like some number of hours, I'm thinking 24. I say all because it needs to be equivalently implemented for the other methods with openbao and vaultwarden, but right now with everything in 1password we need to approve it for one 24 hour period so that these processes are not blocked whenever I step away."

## Using it

```bash
ops/scripts/secrets-session start                     # the declared store, 24 hours
ops/scripts/secrets-session start --store openbao --hours 8
ops/scripts/secrets-session status                    # live / expired / absent, time left
ops/scripts/secrets-session end                       # every store
```

`--hours` is 1 to 24. `start` with no `--store` uses the declared store (`ops/secrets/posture.py store`); `--store` may be repeated. `GOAT_SECRETS_SESSION=0` makes one `with-secrets` command ignore any session.

## Per store

| Store | What `start` keeps | How it is obtained | What `with-secrets` exports, to whom | What `end` does |
|---|---|---|---|---|
| 1Password | a read-only service account token | an admin creates it with a 24 h expiry; the person pipes it to `start` (stdin, or a hidden prompt); `start` checks it with `op whoami` | `OP_SERVICE_ACCOUNT_TOKEN` to `op run`, with `OP_ACCOUNT` unset | removes the session entry; the token itself ends at its own expiry, or when an admin deletes the service account |
| OpenBao | a child token of the person's current login | `bao token create -ttl=<N>h -explicit-max-ttl=<N>h`, using the login `bao` already holds | `BAO_TOKEN` to `ops/secrets/openbao/run` | `bao token revoke -self`, then removes the entry |
| Vaultwarden | the `bw unlock` session key | `BW_SESSION` from the shell, else `bw unlock --raw`, which the person types the master password to; checked with `bw unlock --check` | `BW_SESSION` to `ops/secrets/vaultwarden/run` | `bw lock`, then removes the entry |

### 1Password: creating the service account

An admin of the 1Password account runs this once per session. `--raw` prints only the token, and the pipe delivers it to the key store without showing it:

```bash
op service-account create "goat-session-<handle>-<yyyymmdd>" --account <account> \
  --expires-in 24h --vault "<vault>:read_items" --raw \
  | ops/scripts/secrets-session start --store 1password
```

Repeat `--vault "<vault>:read_items"` for each vault the team's references name. Read is the only grant: the session resolves references and writes nothing. A service account cannot be granted a Personal or Private vault, so every `op://` reference a session resolves must point at a shared vault. A reference into a vault the service account was not granted fails the run, as any unresolvable reference does. Service accounts have hourly and daily read limits; `op service-account ratelimit` shows what remains.

### OpenBao: the child token

The child token carries the login's policies and cannot outlive the login: when the login has less than N hours left, the session ends with it, and `start` says so. Revoking the person's login (offboarding) revokes the child. Creating a child token needs `update` on `auth/token/create` in the person's login policy; without it `start` refuses and names that path.

### Vaultwarden: the session key

A `bw unlock` key has no expiry of its own. The session's expiry is enforced by this driver: the first read after it ends the session with `bw lock` and removes the entry, and `end` does the same at once.

## Where the token lives

**Plain:** in the macOS login keychain or the Linux kernel keyring, never in a file and never on a command line.

**macOS:** each store has one generic-password item, service `goat-secrets-session-<store>`, account `$USER`, the expiry as Unix seconds in its comment (`expires=<epoch>`). The token is written through `security -i` on stdin, so it never appears in argv, and read with `security find-generic-password -w` into the caller's memory. `secrets-session` never prints a token except through the internal `token <store>` verb, whose stdout `with-secrets` captures into a variable. The item is readable by any process running as that user while the login keychain is unlocked; that is what lets an unattended run resolve without a prompt, and it is why the token is read-only, scoped to named vaults, and at most 24 hours long.

**Linux:** each store has one `user` key in the kernel user keyring (`@u`), description `goat-secrets-session-<store>`, payload `expires=<epoch>` then the token on the next line. It is written with `keyctl padd` on stdin and read with `keyctl pipe` on stdout, so it is never in argv. `start` sets a kernel timeout at the expiry, so the kernel drops the key on time even if `end` never runs, and permissions `0x3f1b0000`: the possessor has all rights, the same user id may view, read, search and link it, and no other user may touch it. `end` revokes and unlinks the key. The user keyring lasts while that user has a running process; a reboot, or the last process of a user without lingering exiting, ends the session. This needs the `keyutils` package and no desktop.

`GOAT_SECRETS_KEYSTORE=keychain|keyring` names the key store where both exist. Where neither `security` nor `keyctl` is on PATH, `start` refuses. There is no file fallback (ADR 1186), and `with-secrets` resolves the usual way.

## What the command sees

The session's token reaches the injector, not the command. `with-secrets` builds the chain `op run -- <bao run> -- <bw run> -- env -u <session vars> <command>`, so `OP_SERVICE_ACCOUNT_TOKEN`, `BAO_TOKEN` and `BW_SESSION` set by a session are gone by the time the command starts. A command that calls a store's CLI itself gets the ordinary per-call approval.

## Tests

`tests/test_secrets_session.py` puts stub `security`, `keyctl`, `op`, `bao` and `bw` binaries first on PATH, with a stub keychain and a stub kernel keyring in a temp directory. It covers start per store, the 1 to 24 hour bound, a refused token kept nowhere, the refusal without a keychain, status, end (revoke, lock, delete), an expired session ended on read, `with-secrets` using a live session and falling back without one, the kernel keyring backend (token and kernel timeout kept, a restart replacing the key, a kernel-expired key not used, `end` removing it), and that no token appears in any stub's argv or reaches the command.
