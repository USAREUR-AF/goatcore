# Team credential store (Vaultwarden) — the zero-knowledge option

**Status: WIP — not for production use.** The `vaultwarden` secrets posture is declared in this repository (`ops/scripts/with-secrets`, `ops/secrets/posture.py`); its injector `ops/secrets/vaultwarden/run`, the server role and the gate scripts are not. Until the injector is on a checkout, `with-secrets` refuses a `bw://` reference and `posture.py doctor` says so. The tag lifts on the gate recorded in ADR 1212.

| Field | Value |
|---|---|
| Canon | [ADR 1212](../adr/1212-team-secrets-never-decryptable-on-the-server.md) (R0 and the two shapes), [ADR 1186](../adr/1186-credentials-at-rest-are-references-only.md) (references only on disk), [ADR 1204](../adr/1204-declared-secrets-posture-and-zero-saas-quickstart.md) (one reference grammar) |
| In this repository | `ops/scripts/with-secrets` (dispatches `bw://` references to the injector), `ops/secrets/posture.py` (`doctor` rows `secrets:vaultwarden*`) |
| Related | [secret-store-requirements.md](secret-store-requirements.md) (the bar), secret-store-openbao.md (the other shape of ADR 1212) |

## What it is

A Bitwarden-compatible server a team runs itself, used through the Bitwarden clients: the web vault, the browser extension, the phone app and the `bw` command line. Every value is encrypted on the person's device with keys derived from their master password before it is sent; the server stores, serves and syncs ciphertext and never holds a key that opens any of it. That is R0 of the requirements, and it is why such a server can stand on the public internet without a VPN.

A laptop's files hold references such as `bw://<collection>/<item>/password`; `ops/scripts/with-secrets` fetches each value through the `bw` CLI for one command and nothing keeps it afterwards. It runs beside other stores: each person chooses with one setting.

```bash
export GOAT_SECRET_STORE=vaultwarden     # unset = the declared default store
```

## The server

The server role knows nothing about any cloud: it puts Vaultwarden behind a TLS-terminating reverse proxy (Caddy) on any host, both images pinned by digest. A deployment that follows this shape has:

| Piece | Shape |
|---|---|
| Door for people | TLS on 443. Nothing else open |
| Door for admins | the host provider's session manager or an equivalent; no SSH port |
| Disk | an encrypted data volume mounted at `/var/lib/vaultwarden` for everything the store writes, separate from the operating system's volume |
| Certificate | ACME DNS-01; the certificate the host proves is the one clients see (the name is not proxied) |
| Backups | nightly online copy of the SQLite database plus attachments, sends, the JWT signing key and `config.json`, encrypted with age on the host to the store's administrators and a break-glass identity, copied to versioned object storage |
| Name resolution | the server must reach the identity provider by the same name the browser does; `vaultwarden_container_dns` can point the container at public resolvers where a private zone shadows the provider's public name |

Moving the store is: run the play against the new host, restore the latest backup into `/var/lib/vaultwarden/data`, point the DNS record at it.

## Signing in

Two things identify a person, and both are needed: the team's OIDC identity provider says who they are, and their master password opens their vault. The identity provider never sees the master password and the server never sees a key.

| Step | What happens |
|---|---|
| Single sign-on | The web vault and the apps send the person to the identity provider (scopes `openid profile email`). Only members of the group the client is scoped to may sign in. `SSO_ONLY` is on, so e-mail + master-password sign-in is refused |
| Master password | Set by the person on their first SSO sign-in, at least 16 characters (`SSO_MASTER_PASSWORD_POLICY`). It derives the key that opens their vault on the device. The server stores a hash for authentication and nothing that reconstructs the key |
| Second factor | A FIDO2 key enrolled by the person; the organisation policy "Require two-step login" makes a second factor mandatory for every member, switched on by the first owner once their own key is enrolled |
| Command line | `bw config server https://<store name>` once; `bw login --apikey` once with the personal API key (the API-key grant is the one grant `SSO_ONLY` leaves open, and it opens nothing by itself); `export BW_SESSION="$(bw unlock --raw)"` per shell |

Accounts exist only by invitation from an organisation owner (registration off); only the store's administrators may create an organisation (`ORG_CREATION_USERS`). The `/admin` page is gated by a token whose Argon2id hash is the only copy on the host.

## Who can read what

One organisation with a small number of collections, each sized by what a leak would cost. A person is granted whole collections by the organisation's owners, never items one at a time.

Sharing happens on the device: an organisation key is wrapped to each member's public key when they are confirmed, and every collection item is encrypted under that key. Removing a member deletes their wrapped copy; what they already synced stays readable to them, so removal is paired with rotation of what they could have cached.

Machines have no place in this shape: a `bw` login is a user account with a master password. Machine secrets stay where ADR 1212 puts them for this shape — SOPS-encrypted files with age, each host decrypting with its own key.

## The injector

`ops/secrets/vaultwarden/run` is the injector `with-secrets` hands `bw://` references to. It replaces every environment variable whose value is a `bw://` reference with the value at that reference, in the child's environment only, then execs the command. One unresolvable reference stops the run.

| Rule | How it is kept |
|---|---|
| Grammar | `bw://<collection>/<item>/<field>`; `<field>` is `username`, `password`, `totp`, `uri`, `notes`, or a custom field's name. Exactly three parts; an item name is unique within its collection or the run refuses |
| Two calls, not N | One `bw sync`, then `bw list collections` and `bw list items`; every reference resolves from that one decrypted listing in memory. `GOAT_BW_SYNC=0` skips the sync and the CLI answers from its encrypted local copy |
| No master password | The injector needs `BW_SESSION` from the person's own `bw unlock` and refuses without it. It never prompts for, reads or passes a master password |
| No value on disk or argv | Values travel from `bw`'s stdout into memory and into the child's environment. `BW_SESSION` travels in the environment of each `bw` call, never its arguments |
| One grammar | `op://`, `bao://` and `bw://` references may share one `.refs` file; `with-secrets` chains one injector per scheme present (ADR 1204). `GOAT_SECRET_STORE=vaultwarden` reads `.envrc.local.d/vaultwarden/*.refs` |
| Doctor | `posture.py doctor` reports the injector, the CLI, the unlocked session and malformed references |

## What the server can and cannot see

| The server holds | The server cannot |
|---|---|
| Ciphertext of every item, encrypted under the organisation key, which is wrapped to each member's public key | Decrypt any item, collection name or attachment |
| Each person's e-mail, name, master-password hash, encrypted private key and encrypted user key | Reconstruct a master password or user key from what it stores |
| The organisation's members, roles and collection grants, as metadata | — |
| The event log: sign-ins, creates, edits, shares, and the cipher ids a client reported viewing, with address and time (`ORG_EVENTS_ENABLED`) | Know which reference a run used: the CLI reports a view of every organisation item it lists, so one injector run is one "viewed" row per listed item. The row is the client's report; a client built to stay silent leaves none |
| The web vault's static code, served to browsers | — |

A root attacker on the host obtains the encrypted database and attachments, the TLS private key, the JWT signing key (`rsa_key.pem`, which mints sessions but opens no vault), the admin token's Argon2id hash, the SSO client secret and the SMTP credential, and the ability to serve altered web-vault code to the next browser that visits. The last is the standing weakness of every server-served client, and it is why the `bw` CLI, installed from a signed release on the device, is the injector's client.

## Gates

| Gate | What it proves |
|---|---|
| (a) plant one value, image the data volume as root, search the image | the value appears neither raw nor base64-encoded; a control string the server stores in plaintext (an account e-mail) is found |
| (b) the planted value through `with-secrets` | the bytes match what `bw get` shows (same sha256) |
| (c) a removed member syncs after removal | the next sync shows no organisation items and empties the local store; after account deletion the next sync is 401 |
| (d) ten references in one `with-secrets` call | ten distinct values from one sync and two listings |
| (e) what the event log records for a read | one `Cipher_ClientViewed` (type 1107) row per organisation item the CLI returned, with acting user, cipher id, address and time |

## Not in this shape

Machine identities (R8), a server-side read log naming the value (R6 here is client-reported events, gate (e)), and government-card sign-in. ADR 1212 names these as what decides between the two shapes.
