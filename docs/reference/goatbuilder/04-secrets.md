# 4. Secrets

A deploy's Ansible play reads its secrets from the environment, as
`lookup('env', NAME)`. Each team declares once where those values come from.
goatbuilder and every script read that declaration through one module, so both
avenues always agree on which store is in force and what is missing.

| Field | Value |
|---|---|
| Declaration | `secrets: {store: <name>}` in `ops/goatbuilder/goatbuilder.yaml` |
| Override for one command | `GOAT_SECRET_STORE=<name>` |
| The one resolver | `ops/secrets/posture.py`, called by `ops/scripts/with-secrets`, `ops/scripts/ansible-preflight.sh` and goatbuilder |
| Decision | [ADR 1204](../../adr/1204-declared-secrets-posture-and-zero-saas-quickstart.md); goat's own team keeps [ADR 1186](../../adr/1186-credentials-at-rest-are-references-only.md) |
| Design | [pluggable-dependencies.md](../../design/pluggable-dependencies.md) |

## The stores



| Store | What sits on the laptop | How a command gets the values | What the team accepts |
|---|---|---|---|
| `1password` | references, `NAME=op://vault/item/field`, in `.envrc.local.d/*.refs` | `op run` per command | nothing on disk; goat's own posture |
| `openbao` | references, `NAME=bao://mount/path/field` | the injector `ops/secrets/openbao/run` | running a self-hosted store |
| `vaultwarden` | references, `NAME=bw://collection/item/field` | the injector `ops/secrets/vaultwarden/run`, over the Bitwarden CLI | running a self-hosted store; WIP, not for production use |
| `custom` | references for another store | the command in `$GOAT_SECRET_RUN` | depends on the store |
| `sops` | values encrypted in a committed `secrets.sops.env` | `sops -d` into the command's environment | revoking a person means rotating every value they could decrypt |
| `git-crypt` | values in a committed `secrets/goat.env` under a git-crypt filter | read from the unlocked file | the same, and the unlocked file is plaintext on that disk |
| `env-file` | plaintext values in `.envrc.local.d/secrets.env`, mode 0600, gitignored | read from the file per command | a copied disk, a backup or a stray `cat` exposes every value |
| `env` | values already exported in the shell | nothing is resolved | the same as `env-file`, and the values sit in the shell |

Status: every store is **built**. `openbao` and `vaultwarden` are WIP, not for production use (ADR 1212). The figure above predates `vaultwarden`.

## How a command gets its values

```bash
ops/scripts/with-secrets <command>        # the scripts avenue
goatbuilder secrets exec -- <command>     # the CLI avenue; the same module
```

A reference store resolves each reference scheme with its own injector, chained,
so one `.refs` file may hold both `op://` and `bao://` lines. A value store
always injects its values. Any other store name dispatches to an executable
`ops/secrets/<name>/run`.

## The `secrets` verb

| Command | What it answers |
|---|---|
| `goatbuilder secrets store` | which store is in force. `doctor`'s first `secrets:` row also says whether it came from the config or `GOAT_SECRET_STORE` |
| `goatbuilder secrets available [NAME…]` | every name with a value or a reference, without decrypting anything; given names, only those of them. A verb's own presence check passes the names it reads, so its run record lists no others |
| `goatbuilder secrets missing NAME…` | which of those names cannot be supplied |
| `goatbuilder secrets hint NAME…` | how to supply them under this store |
| `goatbuilder secrets doctor` | whether the store can serve: installed, signed in, file mode, gitignored |
| `goatbuilder secrets init --env <goatnet> [--file PATH] [--force]` | write the zero-SaaS quickstart's env file |
| `goatbuilder secrets exec -- CMD…` | run a command with the store's values |

Under a value store, a variable whose value is still an `op://` or `bao://`
pointer counts as not available, because nothing would resolve it.

## The zero-SaaS quickstart

A laptop with no 1Password, AWS or other hosted account can stand an
environment up. One command writes a strong random value for every variable the
goatnet's plays template.

```bash
goatbuilder secrets init --env <goatnet>
export GOAT_SECRET_STORE=env-file      # or declare secrets: {store: env-file}
goatbuilder doctor --env <goatnet>
```

`init` collects every `lookup('env', NAME)` in the goatnet's member groups. Each
name gets 32 bytes of URL-safe randomness; a name containing `COOKIE` gets 32
hex characters. A re-run keeps every existing value and only adds new names, so
it never rotates a secret a running service holds. Names that only an outside
service can issue are written empty and listed by name. `init` refuses to write
to a path git would commit (X-SECRETS-003).

## Guards that follow the posture

| Posture | Guard |
|---|---|
| `1password` | the `.envrc` loader refuses a `.refs` file holding anything but references; the pre-commit hook refuses staging a live value |
| `env-file` | `doctor` row `secrets:env-file-mode` fails a file readable beyond its owner; `secrets:env-file-ignored` fails a file git would commit |
| every posture | `doctor --env` and `deploy` name every variable a component's play templates and which are missing, before any host is touched (GB-007) |

## Other operator-path dependencies and their fallbacks

| Dependency | Default | Fallback |
|---|---|---|
| AWS credentials | `aws-vault` with the OS keychain | `ops/scripts/with-aws` with `GOAT_AWS_MODE=profile` (a plain AWS profile) or `GOAT_AWS_MODE=none`. A deploy over the mesh needs no AWS |
| Image digests | `crane` | `ops/scripts/image-digest` tries crane, then skopeo, then docker (X-DIGEST-001 when none works) |
| Release signature | cosign | `sha256sum -c SHA256SUMS` alone proves the download matches the published sums, not who published them |
| Image registry | `ghcr.io/<org>` | set `repo.image_namespace` and `ghcr_org` to your own or a local registry |
| Invitation mail | SES | `id_admin_console_smtp_host: ""` turns mail off |
| Public TLS | Cloudflare DNS-01 overlay | the overlay is opt-in; without it the goatnet's internal CA serves mesh names |

Five dependencies have no fallback, each for a stated reason: the SSM door for
a laptop off a cloud-hosted goatnet's mesh, cosign on an air-gapped image
import, GitHub pull requests and CI waits on the release path, candidate
nomination by a repository administrator, and the NetBird API token a goatnet
mints for itself during standup.

## Common refusals

| Code | Meaning |
|---|---|
| GB-007 | a secret the play needs is not in your environment |
| GB-031 | an `op://` reference somewhere in your shell is incomplete; `op run` refuses the whole run. `env \| grep op://` finds it |
| GB-044 | a release refuses before cutting the tag because a secret is not set |
| GB-061 | a secret a standup's plays read is one the posture cannot supply |
| X-OP-001 | the 1Password CLI is not signed in |
| X-SECRETS-001 | the declared store is unknown |
| X-SECRETS-002 | the env-file store has no file; run `secrets init` |
| X-SECRETS-004 | `bao://` references need the OpenBao injector |
| X-SECRETS-005 | a git-crypt file is still encrypted; unlock it |
