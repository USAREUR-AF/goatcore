---
name: goat-cli
description: "Drive a goatnet through the `goat` CLI — fabric/mesh/time/net health, whether data is flowing, who is on the goatnet and what they may do, topics and pub/sub, ACL and warrant assertions, data access, operator admin, and federation packages. Use when someone names a `goat` verb, asks whether a goatnet is healthy or flowing, asks who has access or why something is blocked, or wants to change who is on a goatnet. Also use BEFORE reaching for a raw curl, docker exec, ssh or zenoh call against a goatnet: `goat` usually has a verb, and it carries honesty grading and error codes the raw call does not. Covers context selection, which verbs have --json, and the gotchas. Examples use the synthetic goatnet zagros."
allowed-tools: Bash, Read, Grep, Glob
audience: both
---

# The `goat` CLI

One operator binary over the whole fabric. Source: `crates/goat-cli` (a binary crate in
the workspace at `crates/`). Build it with `cd crates && cargo build -p goat-cli --bin goat`.
The user guide is the mdBook in `docs/goat-cli-book/src/`; error codes are
`docs/design/goat-cli-error-codes.md`, read by `goat explain`.

**`goat <command> --help` is authoritative.** Everything below was checked against
`--help` of goat 0.5.16 built from this tree. The installed binary may be older than the
tree: run `goat --version` before assuming a verb or flag exists.

## First: which goatnet, and with which token

There is **no global `--env` flag.** The goatnet is chosen by `--context`:

```bash
goat context ls                    # * marks the active one
goat context use zagros
goat --context zagros-prod status  # one-shot override; also $GOAT_CONTEXT
goat context show                  # endpoints, role, cert expiry of the active context
```

A first run with no profile: `goat init` (pick sandbox or a real goatnet; `--sandbox`
skips the prompt), then `goat hello` to round-trip a message, then `goat tutor`.

**Two tokens; mixing them up is the commonest 403.** A profile made by
`goat profile init <bundle>` holds the bundle's `vp_` participant token. Operator-gated
calls (`goat admin …`) need the person's `op_` operator key, which is never in a bundle.
`goat admin operator-key request` and `collect <id>` are how a person gets that key, and
they run on the `vp_` token. Token precedence: `GOAT_PORTAL_TOKEN` (non-empty) → the
context's `portal.token` file. A convenient pattern is a paired context, `zagros` and
`zagros-op`, with the `op_` key in the second.

An unauthorized `goat admin` call **changes nothing** and fails `E_NOT_ADMIN` (exit 4).
That is the design. `goat explain E_NOT_ADMIN` says what to do.

Check the context before believing any output: a healthy answer from the wrong goatnet
is still the wrong answer.

## Reading a goatnet

| Reach for | When |
|---|---|
| `goat status` | Am I connected? Router port + a real mTLS Zenoh session + portal + local cert expiry. Trust the session line, not the port line: a wedged router still accepts TCP. One-shot; each run opens a router session, so do not poll it on a timer |
| `goat top --once` | One frame of every health domain plus a producer ranking |
| `goat health --domain <d>` | One domain in depth; `--watch` to keep reading |
| `goat flow [KEY_EXPR]` | Is data flowing, and when did it last? Per-topic freshness |
| `goat who --once` | Who is on the goatnet and what each may do |
| `goat presence` | Who is alive right now (liveliness, not the roster) |
| `goat microscope` | One entity in depth: spine, capability, activity timeline |
| `goat services` | Probe each hosted service in `ops/serve/services.yaml` |
| `goat doctor` | Stepped reachability prober when something is wrong |
| `goat heal` | Bounce the mesh client and re-probe when the mesh is down |

### The seven health domains

`fabric` (default) · `mesh` · `presence` · `time` · `net` · `crypto` · `consistency`.
`--me` / `--peer <peer>` add a producer's own footprint and work with `fabric` only.

### `--json` is not uniform

| Has `--json` | Has `--once` (plain text) instead |
|---|---|
| `status` `flow` `presence` `doctor` `heal` `services` `microscope` `inspect` `replay` `narrate` `health`, and most `admin` / `access` / `acl` / `warrant` / `federate` / `topic` subcommands | `top` `who` |

`top` and `who` have no `--json`; `--once` is their scriptable snapshot and the
interactive screen needs a TTY. Parse `--json` where it exists, never the human
rendering.

### Read the honesty grading; do not summarise it away

- **`flow` separates three states:** no observer could answer; an observer looked back
  over a stated horizon and found nothing; data is flowing. It never claims a topic has
  never published. The human table shows flowing rows only; `--all` shows the rest, and
  `--json` always carries the full set.
- **`health` says `unknown` when it cannot see.** `unknown` means unmeasured, not fine.
- **Every row names the observer that produced it.** When two rows disagree, the labels
  are the answer.

## Gotchas

**`goat sub` / `goat pub` resolve the first path segment as a slot alias**
(`resolve_key` in `crates/panoscope-core/src/alias.rs`), so `ops/fabric/health/...`
tries to resolve `ops` as an alias. Read operational surfaces with
`goat health --domain <d>` instead. It fails closed only when the alias registry is
non-empty; with an empty registry (no aliases, or the portal unreachable) the token
passes through and the same command appears to work. `goat sub --literal` skips alias
resolution.

**`goat flow`'s live tier listens on `*/**`, not `**`.** A subscription naming the whole
keyspace is a named grant on the router (`zenoh_fabric_wide_read`). Without it the router
refuses the declaration silently and the live tier reads `0 event(s)`, which looks exactly
like a quiet fabric. `*/**` matches every published key and needs no grant. Filters work:
`goat flow '**/tracks/v1'`.

**`goat sub` streams until stopped.** `--follow` / `--watch` are accepted and do nothing.
Bound a read with `--count N` (alias `--take`) and put a wall-clock stop on it with
`--for 30s`, so a check fails instead of hanging. `--keys-only` prints keys only.

**A silent denial looks like a broken link.** The fabric's access layer refuses without
telling the refused party (see `docs/design/zenoh-brokerless-consequences.md`). If a peer
reports silence, check access (`goat access why`) before the network.

**`goat update` is per-goatnet.** It installs from the active context's `get.<site>`
download surface; `--dry-run` shows what it would do.

## Who is on the goatnet

```bash
goat admin invite --env zagros --email person@example.org --tier <tier>   # stage 1 of three-stage onboarding
goat admin queue ls --env zagros                                          # then approve / reject
goat admin onboard <handle> --env zagros --revoke                         # offboarding
goat admin onboard <handle> --env zagros --reissue-device-key --uses 1    # fresh mesh key, audited
```

`goat admin onboard` no longer enrolls anyone; that direction is retired and refuses. It
is the revoke path (`--revoke`, `--purge`, `--org`, `--from <file>`) and the key-reissue
path (`--reissue-device-key`, `--reissue-handset-key`, `--requested-by`). Onboarding is
the three-stage candidate flow in `docs/design/three-stage-candidate-onboarding.md`; the
roster is reconciled with `ops/scripts/sync-onboarded-manifests.py`.

Operator manifests live at `ops/operators/<env>/<handle>.yml` with `--env`, or
`ops/operators/<handle>.yml` without; device manifests at `ops/devices/<handle>.yml`.
This repository carries only the `*.example` shapes and schemas (`ops/operators/README.md`);
a deployment's manifests live in its overlay.

`--env` exists only on: `admin invite`, `admin invites ls`, `admin members move`,
`admin onboard`, `admin operator onboard`, `admin operator-key request|reconcile`,
`admin platform-admin grant|revoke|status`, `admin queue ls`, `admin role reactivate`,
`admin ssh-key install|ls|submit`, and `audit emit`. Everywhere else it is `--context`.

## Policy and access questions

```bash
goat acl test [scope]          # zagros | zagros-prod | zagros-dev | ex-demo; omit for all
goat warrant why-blocked --scope zagros --from DEVELOPER --to INFRA --port 443 [--proto tcp]
goat warrant port-sets         # the port-set vocabulary
goat access who|topic|why|strays
```

`goat acl test` reads the ACL declaration compiled into the binary and runs its
`netbird_acl_tests` assertions through the same evaluator as CI. No network, no context,
no credentials, nothing applied. It exits non-zero on a violation or on anything
undecidable, and also when no assertion is declared: a build of this tree declares none,
so it fails until assertions are added under `netbird_acl_tests:` in
`ops/warrant/rule-intents/netbird-acl.yaml` and regenerated with
`ops/warrant/generate.py`, or `--none-expected` is passed. `--verbose` also prints the
assertions that hold.

`goat warrant why-blocked` answers from the declaration in git, without the network. When
the gap is real it prints the smallest port-set that closes it. Design:
`docs/design/warrant.md`; data access: `docs/design/data-access-cop.md`.

## Federation (`goat federate`, moving to `goatbuilder federate`)

Federating with another sovereign goatnet is a goatbuilder job: `goatbuilder federate <partner>`,
whose stages `plan | deploy | test | status` arrive beside today's fourteen-phase run (WIP until
each default switch lands; procedure: `docs/operations/sovereign-federation-runbook.md` step 1).
The package verbs below move to `goatbuilder federate <verb>` with the same flags and exit codes;
`goat federate <verb>` keeps working and prints one line naming the goatbuilder command.

Format: `docs/design/federation-package-spec.md`.

| Verb | Privilege |
|---|---|
| `init` | reads local state, writes only the `--out` package; an underivable value is an error, never a default |
| `lint` · `compose` · `diff` | pure: no network, no credentials |
| `verify` | dry-run: could we deliver this agreement? |
| `apply` | the only verb that renders configuration |

`diff` is the verb for two digests that disagree: it names the differing field, or why
byte-identical agreements hashed differently.

## Exit codes (`crates/goat-cli/src/exit_codes.rs`)

`0` ok · `1` generic · `2` usage · `3` profile · `4` portal unreachable, token rejected or
refused · `5` off-mesh · `6` cert · `7` portal 404 · `8` portal 429 (retry) · `9` portal
5xx · `124` timeout · `130` SIGINT. Federation: `20` refused · `21` malformed · `22` digest
mismatch · `23` underived · `24` not conformant.

## Discovering the rest

```bash
goat docs --markdown       # the full command reference
goat <command> --help      # authoritative per command
goat explain <code|text>   # an error code or pasted error text -> what, why, fix, where to read
goat compose               # interactive command builder, when you know the goal but not the verb
goat completion <shell>
```

Global flags on every command: `--context`, `--profile-dir`, `--timeout` (`GOAT_TIMEOUT`,
bounds session set-up, not streaming), `--uuid` (raw slot anchors instead of aliases),
`--narrate` / `--narrate-json`, `--lang`, `--color`, `-v`. `goat <name>` also runs any
`goat-<name>` executable on PATH; `goat plugin list` shows them.

Prefer a `goat` verb over a raw `curl`, `docker exec`, `ssh` or zenoh call: the verb
carries the honesty grading, the observer labels and the error codes.
