# Pluggable dependencies — a declared secrets posture and a zero-SaaS quickstart

| Field | Value |
|---|---|
| Status | **Built** (2026-10-28): the secrets posture with six stores, one resolver for both avenues, `goatbuilder secrets init` for the quickstart's env file, `with-aws` and `image-digest` fallbacks. **Measured** on zagros_dev: an env file generated for every variable the goatnet's plays template, then read by `goatbuilder doctor` and injected by `ops/scripts/with-secrets`. The local-registry and SMTP-sink recipes and the off-when-empty alert channels are **built**. The forge adapter (rows 19, 20) is **designed**, not built |
| Decision | [ADR 1204](../adr/1204-declared-secrets-posture-and-zero-saas-quickstart.md), amending [ADR 1186](../adr/1186-credentials-at-rest-are-references-only.md) for teams other than goat's own |
| Related | [secret-store-requirements.md](secret-store-requirements.md) (what a store must do), ADR 1203 (the self-hosted OpenBao store, run beside 1Password), ADR 1201 (both avenues stay functional) |

## What this is

A team adopting goat chooses how its deploy secrets are held and declares that choice once. goat's own choice stays what ADR 1186 says: references only, resolved by 1Password per command, so no credential value sits on a disk. Another team may choose a simpler store — a gitignored plaintext file, values encrypted in git, or variables already in its shell — and owns that trade. A developer with only a laptop and the hardware or VMs can stand a goat environment up without first opening any SaaS account.

The operator's words, which this design implements:

> "ADR 1186 says no credential value ever sits on disk" we hold this true for ourselves, but we can't enforce this for others and devs will want an ability to rapidly get a goat env up without provisioning and configuring SaaS accounts and things like that

## The posture

**Plain:** one key names the store; both avenues read it through one module, so the scripts and `goatbuilder` cannot disagree about which store is in force, which variables are available, or what to tell the operator when one is missing.

**Detail.** The key is `secrets: {store: <name>}` in `ops/goatbuilder/goatbuilder.yaml`, schema-gated (`ops/goatbuilder/schema/goatbuilder-v1.schema.json`). goat's generated config declares `store: 1password`. `GOAT_SECRET_STORE` overrides it for one command. Every reader goes through `ops/secrets/posture.py`:

| Store | What is on the laptop | How a command gets values | What the team gives up |
|---|---|---|---|
| `1password` | references (`NAME=op://vault/item/field`) in `.envrc.local.d/*.refs` | `op run` per command, against the pinned `op_account` | nothing: goat's posture |
| `openbao` | references (`bao://mount/path/field`) | the store's injector, `ops/secrets/openbao/run` (ADR 1203) | nothing beyond running the store |
| `custom` | references for another store | `$GOAT_SECRET_RUN` (e.g. `bws run --`) | depends on the store |
| `sops` | values encrypted in a committed dotenv file (`secrets.sops.env`) | `sops -d` into the command's environment, never to disk | revoking a person means rotating every value they could decrypt |
| `git-crypt` | values in a committed dotenv file under a git-crypt filter (`secrets/goat.env`), readable once unlocked | read from the unlocked file | the same, and the unlocked file is plaintext on that disk |
| `env-file` | **plaintext values** in a gitignored, mode-0600 file (`.envrc.local.d/secrets.env`) | read from the file per command | everything ADR 1186 protects: a copied laptop disk, a backup, or a stray `cat` exposes every value |
| `env` | values already exported in the shell | nothing is resolved | the same as `env-file`, plus the values sit in the shell's environment |

`posture.py` answers these questions, and nothing else decides them:

| Call | Used by |
|---|---|
| `store` | `with-secrets`, `ansible-preflight.sh`, `goatbuilder` |
| `available` — every name with a value or a reference, without decrypting | `deploy-secrets-needed.py` (the scripts' preflight), `goatbuilder doctor` and `deploy` |
| `hint NAME…` — how to supply those names under this store | `deploy-component.sh`, `goatbuilder doctor` and `deploy` |
| `op-account` — the 1Password account to pin, or refuse (exit 3) if it cannot be resolved and `op` would fan out | `with-secrets` |
| `doctor` — whether the store can serve | `goatbuilder doctor` (rows `secrets:*`) |
| `exec -- CMD` — run a command with a value store's values | `with-secrets` |

**Pinning the 1Password account.** Under `1password`, `op run` resolves against the account named by `OP_ACCOUNT`. With it unset and more than one account signed in on the machine, `op` prompts across **every** account — alarming, and it reaches anyone running a deploy from a laptop that also holds a personal account. So `with-secrets` pins the account before any `op` call, from the first of: `OP_ACCOUNT` in the environment (a per-command override), then `secrets: op_account:` in `goatbuilder.yaml`. If neither resolves it **and** `op account list` shows more than one account, the run is **refused** (exit 3) with the account names and the fix, rather than letting `op` fan out. A single configured account is unambiguous and runs unpinned. This gate is on the `op` path only: a team on `env-file`, `sops`, `git-crypt` or `openbao` never meets it (ADR 1204).

Presence is the posture's, not the shell's. Under a value store the values live in its file until `with-secrets` injects them, so a check that read only the shell would call every variable missing. Under a value store a variable whose value is still an `op://` or `bao://` pointer counts as **not** available, since nothing would resolve it.

**The guards follow the posture.** Under `1password` the existing guards apply unchanged: the `.envrc` loader refuses a `.refs` file holding anything but references, and the pre-commit hook (`ops/git-hooks/pre-commit:25`) refuses staging a live value. Under `env-file` the guard is the store's doctor: `secrets:env-file-mode` fails a file readable beyond its owner, and `secrets:env-file-ignored` fails a file git would commit (`git check-ignore`). `secrets init` refuses to write to a path that is not gitignored.

## One resolver, both avenues

**Plain:** `ops/scripts/with-secrets` stays the one injector; the posture only decides which store it hands the command to.

**Detail.** Reference stores (`1password`, `openbao`, `custom`) keep their existing rule: no reference in the environment, the command runs unchanged. Value stores (`env-file`, `git-crypt`, `sops`) always inject. Any other store name dispatches to an executable `ops/secrets/<name>/run` — the seam the OpenBao track uses for its injector, so the two tracks add stores without editing each other's code. `ansible-preflight.sh` routes a play through `with-secrets` under a value store, so the values reach the hosts exactly once. `goatbuilder` calls the same module, so parity holds by construction.

## 1Password and OpenBao: symmetric where the stores are alike

**Plain:** a developer uses the two reference stores the same way — the same reference grammar, the same run command, the same doctor rows and the same "sign in" fix — and meets a difference only where the stores really differ: how you authenticate, where the store lives, and OpenBao's CA certificate.

The operator's words: "if possible and appropriate I'd like to hold symmetry in design of cred management and cli usage for op and openbao … but if that is not appropriate or it would make it contrived then don't force it."

| Concern | Symmetric or different | What both do, or what differs and why |
|---|---|---|
| Reference grammar | symmetric | three parts after the scheme, `op://<vault>/<item>/<field>` and `bao://<mount>/<path>/<field>` (mount `goat`); both may share one `.envrc.local.d/*.refs` file of `NAME=<ref>` lines; the `.envrc` loader and `ansible-preflight.sh` accept either scheme and refuse anything else under a references posture; `GOAT_SECRET_REF_PREFIX` narrows to one |
| Run verb | symmetric | `ops/scripts/with-secrets <cmd>` on the scripts avenue, `goatbuilder secrets exec -- <cmd>` on the CLI; each scheme present is resolved by its own injector, chained — `op run -- ops/secrets/openbao/run -- <cmd>` — and each touches only its own references. The name `with-secrets` is kept: it is in every runbook |
| Doctor | symmetric | per backend in play: installed, signed in (`op whoami`; `bao token lookup`), references well-formed. Neither resolves every value: each read is attributed per value, and the run resolves them anyway |
| Sign-in fix text | symmetric | `sign in: eval $(op signin)`; `sign in: bao login -method=oidc` |
| Authentication | different | 1Password approves a session through the desktop app; OpenBao issues a token from SSO against the team's identity provider, a YubiKey certificate, or a password with a one-time code (`ops/secrets/openbao/people.py`, ADR 1203), and machines use AppRole. The ways differ, so the commands do |
| Where the store lives | different | 1Password is a hosted service reached over the internet; OpenBao is reached at its certificate-only public edge (`BAO_ADDR=https://store.team.goat.example:8443`, a card or team YubiKey certificate completes the handshake) or, for admins, through a Session Manager port-forward to its private address; it is never inside a goatnet. Only OpenBao's doctor checks `BAO_ADDR` |
| CA certificate | different (OpenBao only) | the OpenBao instance presents its own CA; `BAO_CACERT` names it, and the doctor checks it exists |
| Offline cache | different (OpenBao only) | OpenBao's injector keeps an age-encrypted per-laptop cache (`GOAT_SECRET_CACHE`, `GOAT_SECRET_CACHE_RECIPIENT`). 1Password offers no seam for one; building it would mean reading every value out of `op` and writing it to disk, which is the thing ADR 1186 forbids for goat's team |

## The zero-SaaS quickstart

**Plain:** one command writes every secret a goatnet's plays need, as strong random values in a gitignored 0600 file; the values only an outside service can issue are listed by name, each with its bypass below.

```bash
goatbuilder secrets init --env <goatnet>        # or: python3 ops/secrets/posture.py init --env <goatnet>
export GOAT_SECRET_STORE=env-file               # or declare secrets: {store: env-file} in goatbuilder.yaml
goatbuilder doctor --env <goatnet>              # secrets:env-file ok; every component's variables available
```

**Detail.** `init` collects every `lookup('env', NAME)` in the goatnet's member groups (`ops/pins/fleet-pins.yaml` `goatnets.<g>.members`). Each name gets `token_urlsafe(32)`; a name containing `COOKIE` gets 32 hex characters, the length oauth2-proxy accepts. A re-run keeps every value already in the file and adds only new names, so it never rotates a secret a running service holds. Names an outside service must issue (pattern `EXTERNAL` in `posture.py`) are written empty and listed. Measured on zagros_dev, 2026-10-28: 55 generated, 11 listed as external, second run unchanged, and `with-secrets` injected a 32-character netvis cookie secret into a child process.

## The audit: every operator-path dependency, its fallback, and the quickstart bypass

**Plain:** 21 places on the operator path assume a specific tool or service. 16 now have, or are designed to have, a simpler fallback. 5 are load-bearing for the path they serve, each with the reason.

| # | Dependency | Where | What it assumes | Fallback / quickstart bypass | Verdict | Status |
|---|---|---|---|---|---|---|
| 1 | 1Password injector | `ops/scripts/with-secrets` | `op run` resolves every reference | any posture store; quickstart: `env-file` from `secrets init` | fallback | built |
| 2 | `op://` loader | `.envrc:34` | references live in `.envrc.local.d/*.refs` | value stores use their own file; the loader only acts on `.refs` files | fallback | built |
| 3 | 1Password fix text | `deploy-component.sh:135`, `doctor.rs`, `push.rs` | the fix is `op run --env-file=…` | rendered by `posture.py hint` for the store in force | fallback | built |
| 4 | references-only preflight | `ansible-preflight.sh:274`, `:293` | values arrive through `op run` | value stores routed through `with-secrets` | fallback | built |
| 5 | presence check | `deploy-secrets-needed.py`, `doctor.rs`, `push.rs` | a value is in the shell | `posture.py available` | fallback | built; reads the reached roles' defaults too (goat-get's release token: any one of `GOAT_CLI_RELEASE_TOKEN`, `GH_TOKEN`, `GHCR_READ_TOKEN`; a line ending `# or: NAME` adds an alternative; a line ending `# when: <var>=<value>` is needed only where the goatnet's value, else the role's, is that value), 2026-10-30. Follows `include_role` / `import_role` and meta dependencies into the roles a component pulls in, and resolves a lookup whose argument is a role variable holding the name (`portal_db_password_env`), so the portal and console database passwords on Postgres goatnets are named before the play, 2026-11-07 |
| 6 | live-value pre-commit guard | `ops/git-hooks/pre-commit:25` | values never reach a staged file | stays; under `env-file` the doctor's gitignored and 0600 rows guard the file | fallback | built |
| 7 | the reference catalog | goat-credentials `refs/goat.refs` | every name has a 1Password item | `secrets init` generates the values | fallback | built |
| 8 | new-site value generator | `ops/scripts/newsite-secrets.sh` | new values are written into 1Password | the same SPEC of names under every store: `1password`, `openbao` and `custom` take references as before; `env-file`, `sops` and `git-crypt` take the values through `posture.py set --stdin-json` (kept, never rotated, never printed); `env` is refused. Command: `ops/scripts/newsite-secrets.sh <site>` under the declared posture | fallback | built 2026-11-02 (goatlite phase B, ADR 1213) |
| 9 | aws-vault | `deploy-component.sh`, `push.rs`, `standup.rs`, `joinnode.rs`, `federate.rs`, `goat-standup.sh:156`, `peering-standup.sh`, `env-door.sh` | keys in the OS keychain via aws-vault | `ops/scripts/with-aws`: `GOAT_AWS_MODE=profile` (a plain AWS profile) or `none`; quickstart: `none` for a mesh-only deploy | fallback | built |
| 10 | NetBird profile name | `deploy-component.sh:145` | the profile names the goatnet | already a fast path only; reaching the host on tcp/22 decides | fallback | built (existing) |
| 11 | crane | `rc.sh:172`, `release.rs` | crane resolves image digests | `ops/scripts/image-digest`: crane, skopeo or docker, each tool's error kept | fallback | built |
| 12 | cosign on install | `goatbuilder-first-run.md:21` | the release signature is verified | `sha256sum -c SHA256SUMS` alone, which proves the download matches the published sums but not who published them | fallback | built (documented) |
| 13 | GHCR registry | `goatbuilder.yaml:9`, group_vars `ghcr_org`, every image pin (`ghcr.io/{{ ghcr_org }}/…`) | images under `ghcr.io/usareur-af` | public images pull without a token; `ghcr_org` moves them to another namespace on ghcr.io; a goatlite's `lite.image_registry` moves every goat image to a registry the team holds, same name and digest, filled by [`ops/recipes/local-registry/`](../../ops/recipes/local-registry/README.md) | fallback | built: `local-registry.sh up`/`mirror`/`verify` and the renderer's `image_registry` (a registry off the box goes into `docker_engine_insecure_registries`). A full-preset goatnet declares the mirrored references as deviations in its own pin table; not automated |
| 14 | SES mail | `group_vars/zagros_dev_hosts/vars.yml:375` | invites are mailed through SES | `id_admin_console_smtp_host: ""` turns mail off (the role's default); or the SMTP sink: Mailpit on the goatnet's docker network keeps every invite for the administrator to hand over ([`ops/recipes/smtp-sink/`](../../ops/recipes/smtp-sink/README.md)) | fallback | built: role `smtp-sink` and `smtp-sink.yml`; the console sends without SMTP AUTH when its username is empty (`internal/mailer` `auth()`). On a goatnet once a console image with that mailer is pinned |
| 15 | Cloudflare DNS-01 | `traefik-acme.yml` overlay, `CLOUDFLARE_API_TOKEN_PROD` | public certificates through Cloudflare DNS | the ACME overlay is optional; without it Traefik serves the goatnet's internal CA on mesh names | fallback | built (overlay is opt-in); a hardware run without it is unmeasured |
| 16 | alert channels | `ALERT_SMTP_*`, `ALERT_EMAIL_DIGEST_RECIPIENTS`, `SLACK_ALERT_WEBHOOK_URL`, `PAGERDUTY_INTEGRATION_KEY` (`_EU`) | alerts leave the goatnet | empty values; alerts stay on the goatnet's own ntfy and Grafana | fallback | built: every binding falls back to empty; both Alertmanager templates (`obs-host`, `observability`) render no PagerDuty, Slack or e-mail receiver when empty and `amtool check-config` accepts the result; `deadman-watch` runs with paging off. The deploy presence check and `goatbuilder doctor` skip the names in [`ops/secrets/off-when-empty.txt`](../../ops/secrets/off-when-empty.txt). `tests/test_alert_channels_off_when_empty.py` fails if a role requires one |
| 17 | SSM doors | `env-door.sh:105` | a laptop off a goatnet's mesh reaches it through AWS SSM | none needed when the laptop is on the mesh (the transport tries the mesh first) | load-bearing for off-mesh access to a cloud-hosted management host; the quickstart is on the mesh | n/a |
| 18 | cosign on air-gapped import | `ops/scripts/goat-image-import.sh:15` | an islanded bundle is signed | none: the signature is the only proof an imported image is ours | load-bearing | n/a |
| 19 | GitHub pull requests and CI waits | `ops/scripts/pr-land.sh:65`, `rc.sh:129` | the ladder lands pins by PR and waits on CI | none on the release path: the PR is the record. `goatbuilder deploy` needs no GitHub, so the quickstart deploys without it | load-bearing for `release`; a forge adapter is designed, not built |
| 20 | candidate nomination | `.github/workflows/rc-validate.yml:6` | a repository admin on GitHub | the same rule on another forge | load-bearing for `release` on GitHub; forge adapter as row 19 |
| 21 | NetBird API token | `ZAGROS_DEV_NB_API_TOKEN` | a service user minted on the goatnet | none: minted by the goatnet's own NetBird during standup, not by a SaaS | load-bearing (internal) | n/a |

## Status and next

| Piece | Status |
|---|---|
| posture key, schema, `posture.py`, `with-secrets` dispatch (per-scheme for references), preflight routing, the shared reference grammar in the loader and preflight, both preflights and both fix texts | built; 22 tests in `tests/test_secret_posture.py`; the parity gates re-run green |
| `goatbuilder secrets` (`store`, `available`, `missing`, `hint`, `doctor`, `init`, `exec`) and the `secrets:*` doctor rows | built; measured on zagros_dev |
| `openbao` store | the dispatch is built: `with-secrets` runs `ops/secrets/openbao/run -- <cmd>` for `bao://` references, which ADR 1203 provides. That injector is not in this repository: a `bao://` reference stops the run naming the missing injector, and the doctor's `secrets:openbao` row says so. |
| `with-aws`, `image-digest` | built; probed on this laptop (profile, none, a real digest) |
| row 8 store-generic generator | built 2026-11-02 |
| rows 13, 14, 16 recipes | built (rows above) |
| forge adapter (rows 19, 20) | designed, not built |
