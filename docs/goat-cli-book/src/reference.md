# goat — command reference

_Generated from the `goat` command tree — always in sync with what the binary does._

## `goat`

goat — operator-facing CLI composing panoscope-core into one tool: profile init/reuse, ZTOL-aware reachability doctor, org portal browse, topic CRUD, pub/sub, stethoscope delegation, an interactive command composer, and substrate-cascade narration.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--uuid` | Show raw UUID slot anchors instead of human aliases (ADR 1039). Aliases are the default display; this forces the canonical form. `--json` always carries the UUID regardless of this flag |  |
| `--timeout <TIMEOUT>` | Max seconds to wait establishing a fabric session before failing fast (default 5). Bounds the connect/handshake phase for network commands (health/sub/pub/presence) so an unreachable router errors cleanly instead of blocking. Does NOT cap steady-state streaming — `sub --watch` runs as long as you want. Also reads GOAT_TIMEOUT |  |
| `--profile-dir <PROFILE_DIR>` | Override the profile directory (default: $GOAT_PROFILE_DIR or ~/.goat). |  |
| `--context <CONTEXT>` | Override the active context (default: $GOAT_CONTEXT env, then current_context in config.toml, then "default"). |  |
| `--router <ENDPOINT>` | Connect to this router endpoint (`tls/<ip>:<port>`) instead of the context's `router_endpoint`, for this command only. The context's certificate, key and CA roots are still used. For a local port forward: `env-door.sh <goatnet> --forward <router-ip>:7447`. Also reads GOAT_ROUTER_ENDPOINT |  |
| `--narrate` | Show the substrate cascade a mutating command sets off. |  |
| `--narrate-json` | Emit the substrate cascade as NDJSON (one object per consequence) for incident logs + scripting. Implies --narrate. |  |
| `--verbose, -v` | Enable verbose debug output (or set RUST_LOG=goat=debug). |  |
| `--color <COLOR>` | Color mode for terminal output (auto\|always\|never). | `auto` |
| `--lang <LANG>` | Language for human-facing prose (e.g. en, uk). Falls back to GOAT_LANG, then the OS locale, then English. |  |

**Subcommands**

- [`goat schema`](#goat-schema) — Manage a topic's schema body: upload, ls, show, diff
- [`goat access`](#goat-access) — Data-Access COP: who can access which data, and why
- [`goat audit`](#goat-audit) — Read the operator-action audit trail (`ops/audit/operator/**`) — the who-did-what record of backend admin actions, decrypted with your operator age identity
- [`goat blob`](#goat-blob) — Send or receive a large file over the non-persisted `volatile/blob` class: chunk, BLOCK-publish, manifest, sha256-verify
- [`goat replay`](#goat-replay) — Replay past events from the audit-tail (read, not live subscribe)
- [`goat health`](#goat-health) — Show fabric health (per-class load vs capacity + green/amber/red) and, with --me/--peer, a producer's own footprint
- [`goat top`](#goat-top) — Live single-pane fabric dashboard (the top(1) of the fabric): every environment-health domain at once + a sortable producer ranking, auto-refreshing. Interactive screen is TTY-only; --once is the scriptable plain-text snapshot (ADR 1044)
- [`goat presence`](#goat-presence) — Who is alive on the fabric right now — the native liveliness presence view (one-shot set or --watch join/leave edges)
- [`goat who`](#goat-who) — Identity common operating picture (the top(1) of identities): a searchable roster browser — who is in the goatnet, what each may do, and what they're doing. Interactive screen is TTY-only; --once is the scriptable plain-text snapshot (ADR 1047)
- [`goat profile`](#goat-profile) — Manage the local goat profile (mTLS cert + portal token + endpoint set).
- [`goat context`](#goat-context) — Manage contexts (`ls` / `use` / `show` / `rm`).
- [`goat init`](#goat-init) — First-run wizard: pick sandbox or real, prove it works, point at the tutor.
- [`goat tutor`](#goat-tutor) — A self-paced, hands-on tour of the fabric (vimtutor-style; resumable).
- [`goat docs`](#goat-docs) — Browse the goat docs, or emit the full command reference (--markdown).
- [`goat hello`](#goat-hello) — Round-trip a hello through the fabric end-to-end (sandbox or real env).
- [`goat doctor`](#goat-doctor) — Run a stepped reachability prober against the fabric.
- [`goat org`](#goat-org) — Browse orgs registered in the portal.
- [`goat topic`](#goat-topic) — List, show, create, or delete topics.
- [`goat pub`](#goat-pub) — Publish a payload to a topic key.
- [`goat sub`](#goat-sub) — Subscribe to a topic key expression.
- [`goat inspect`](#goat-inspect) — Delegate a diagnostic verdict request to stethoscope.
- [`goat ui`](#goat-ui) — Interactive merged picker: browse the declared ⊕ live roster and hop to the Diagnose / Tail lens on the selected target.
- [`goat compose`](#goat-compose) — Interactive command builder TUI.
- [`goat narrate`](#goat-narrate) — Reflect over recent substrate activity from the audit-tail as a who-touched-what cascade (--since / --actor / --json).
- [`goat completion`](#goat-completion) — Print shell completion script to stdout.
- [`goat explain`](#goat-explain) — Look up an error code's meaning + remediation in the embedded `goat-cli-error-codes.md` doc (cargo --explain pattern).
- [`goat mock`](#goat-mock) — Publish synthetic samples to a topic. Default: trivial hello-world JSON at 1Hz. Pass `--format <.fds>` + `--message <Name>` to encode operator-supplied protobuf instead.
- [`goat admin`](#goat-admin) — Operator-only administration: org lifecycle, onboard/revoke/rotate, mesh-peer cut, device + operator-tier provisioning, LAN triage. Authorized by your operator credentials (server-side via portal/NetBird token scope); inert for non-admins — an unauthorized call changes nothing and fails with `E_NOT_ADMIN`.
- [`goat status`](#goat-status) — Health synthesis: ZTOL reachability + portal /api/health + local cert expiry, three glyph lines or one --json object.
- [`goat services`](#goat-services) — Probe each hosted service in `ops/serve/services.yaml` and render one glyph line per service (or one --json object).
- [`goat client`](#goat-client) — Wrap the local goat-clientd daemon via its JSON-RPC IPC socket (`~/.goat-client/goat-clientd.sock`): status / up / down / bundle import.
- [`goat plugin`](#goat-plugin) — PATH-based extension discovery (kubectl-style). Any `goat-<name>` executable on PATH is invokable as `goat <name>`; `goat plugin list` enumerates installed plugins, `goat plugin which <name>` resolves the path.

## `goat schema`

Manage a topic's schema body: upload, ls, show, diff

**Subcommands**

- [`goat schema upload`](#goat-schema-upload) — Upload a schema body for a topic. `.proto` source is compiled by the portal on ingress; `.fds` (a serialized FileDescriptorSet, e.g. from `build-fds.sh` for a multi-file org source) is stored as-is. Format is inferred from the extension unless `--format` is given
- [`goat schema ls`](#goat-schema-ls) — List the schema versions registered for a topic
- [`goat schema show`](#goat-schema-show) — Render a topic's schema body as a readable message/field tree
- [`goat schema diff`](#goat-schema-diff) — Diff two schema versions' message + field sets

## `goat schema upload`

Upload a schema body for a topic. `.proto` source is compiled by the portal on ingress; `.fds` (a serialized FileDescriptorSet, e.g. from `build-fds.sh` for a multi-file org source) is stored as-is. Format is inferred from the extension unless `--format` is given

**Arguments**

| Argument | Description |
|---|---|
| `<KEY>` | Topic key (e.g. `acme/observations/v1`) |
| `<FILE>` | Path to the schema file (`.proto` source or `.fds`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--compat <COMPAT>` | Compatibility declaration: backward \| forward \| none | `none` |
| `--format <FORMAT>` | Override the inferred wire format: protobuf-source \| protobuf-fds |  |
| `--dry-run` | Don't POST — print what would be uploaded and exit 0 |  |

## `goat schema ls`

List the schema versions registered for a topic

**Arguments**

| Argument | Description |
|---|---|
| `<KEY>` | Topic key (e.g. `acme/observations/v1`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit one JSON line per schema row |  |

## `goat schema show`

Render a topic's schema body as a readable message/field tree

**Arguments**

| Argument | Description |
|---|---|
| `<KEY>` | Topic key (e.g. `acme/observations/v1`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--proto` | Print the reconstructed `.proto` source instead of the tree |  |
| `--json` | Emit the raw render JSON |  |

## `goat schema diff`

Diff two schema versions' message + field sets

**Arguments**

| Argument | Description |
|---|---|
| `<KEY>` | Topic key (e.g. `acme/observations/v1`) |
| `<FROM>` | Baseline version (e.g. `v1`) |
| `<TO>` | Compared version (e.g. `v2`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit the diff as JSON |  |

## `goat access`

Data-Access COP: who can access which data, and why

**Subcommands**

- [`goat access who`](#goat-access-who) — Forward query: what may this identity write and read, with the reason at each layer. An org may ask about its own prefix; an operator may ask about any identity
- [`goat access topic`](#goat-access-topic) — Reverse query: who may write and read this topic, and which partners may receive it if released. Operator-only
- [`goat access why`](#goat-access-why) — Trace one allow/deny decision for `<id>` against `<key>` across mesh → identity → Zenoh ACL → release
- [`goat access strays`](#goat-access-strays) — Collect every stray (live, no declared match) and bucket it by WHY it didn't reconcile — the safe first step of a stray sweep. Read-only: builds the cut-candidate + verify-first lists; it never cuts. The cut stays the audited `goat admin onboard --revoke`, one at a time
- [`goat access probation`](#goat-access-probation) — Anomaly probation (ADR 1068): hold something that doesn't reconcile instead of leaving or deleting it — verify, then repair or purge. The hold/decide are recorded + audited; the actual repair/purge runs through the existing `goat admin` paths. Operator-only

## `goat access who`

Forward query: what may this identity write and read, with the reason at each layer. An org may ask about its own prefix; an operator may ask about any identity

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Identity selector: an org prefix, person email, peer name, or slot id |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit the raw JSON |  |

## `goat access topic`

Reverse query: who may write and read this topic, and which partners may receive it if released. Operator-only

**Arguments**

| Argument | Description |
|---|---|
| `<KEY>` | Topic key (e.g. `acme/observations/v1`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit the raw JSON |  |

## `goat access why`

Trace one allow/deny decision for `<id>` against `<key>` across mesh → identity → Zenoh ACL → release

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Identity selector (an org may pass its own prefix) |
| `<KEY>` | Topic key |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit the raw JSON |  |

## `goat access strays`

Collect every stray (live, no declared match) and bucket it by WHY it didn't reconcile — the safe first step of a stray sweep. Read-only: builds the cut-candidate + verify-first lists; it never cuts. The cut stays the audited `goat admin onboard --revoke`, one at a time

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit the raw JSON (the frozen stray set, bucketed) |  |

## `goat access probation`

Anomaly probation (ADR 1068): hold something that doesn't reconcile instead of leaving or deleting it — verify, then repair or purge. The hold/decide are recorded + audited; the actual repair/purge runs through the existing `goat admin` paths. Operator-only

**Subcommands**

- [`goat access probation list`](#goat-access-probation-list) — List entities on probation (open by default; `--all` for history), with the cure-by countdown. Expired entries are flagged, never auto-cut
- [`goat access probation hold`](#goat-access-probation-hold) — Hold an entity on probation. `<ref>` is its stable id (a stray's identity id / slot anchor / recon key)
- [`goat access probation decide`](#goat-access-probation-decide) — Record a terminal disposition for a held entry. `repair` regularizes it (via `goat admin onboard`); `purge` removes + cleanses it (via `goat admin onboard --revoke --purge`). Prints the follow-up command

## `goat access probation list`

List entities on probation (open by default; `--all` for history), with the cure-by countdown. Expired entries are flagged, never auto-cut

**Options**

| Flag | Description | Default |
|---|---|---|
| `--all` | Include the decided (repaired/purged) history, not just open holds |  |
| `--json` | Emit the raw JSON |  |

## `goat access probation hold`

Hold an entity on probation. `<ref>` is its stable id (a stray's identity id / slot anchor / recon key)

**Arguments**

| Argument | Description |
|---|---|
| `<REF_>` | The entity to hold |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--reason <REASON>` | Why it's held (e.g. `stray:unregistered-key`) | `` |
| `--label <LABEL>` | Human label for display (email / alias / name) | `` |
| `--kind <KIND>` | Anomaly kind (v1: identity) | `identity` |
| `--cure-days <CURE_DAYS>` | Cure-by window in days (default = the portal policy default, 14d) |  |
| `--json` | Emit the raw JSON |  |

## `goat access probation decide`

Record a terminal disposition for a held entry. `repair` regularizes it (via `goat admin onboard`); `purge` removes + cleanses it (via `goat admin onboard --revoke --purge`). Prints the follow-up command

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | The probation entry id (`prob-…`) |
| `<VERDICT>` | `repair` (keep + regularize) or `purge` (remove + cleanse) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--note <NOTE>` | A note recorded with the decision | `` |

## `goat audit`

Read the operator-action audit trail (`ops/audit/operator/**`) — the who-did-what record of backend admin actions, decrypted with your operator age identity

**Subcommands**

- [`goat audit ops`](#goat-audit-ops) — Read the operator-action audit trail (`ops/audit/operator/**`): query the rocksdb-backed store, decrypt each event with your operator age identity, and print a readable table. Samples not encrypted to your key render an `<undecryptable>` row rather than aborting the whole read

## `goat audit ops`

Read the operator-action audit trail (`ops/audit/operator/**`): query the rocksdb-backed store, decrypt each event with your operator age identity, and print a readable table. Samples not encrypted to your key render an `<undecryptable>` row rather than aborting the whole read

**Options**

| Flag | Description | Default |
|---|---|---|
| `--actor <ACTOR>` | Restrict to one actor (the `<actor>` sub-key). Default: all actors |  |
| `--watch` | Keep the subscription open and print events as they arrive (live tail) instead of a one-shot query of stored events |  |
| `--json` | Raw output: one JSON object per decoded event on stdout |  |

## `goat blob`

Send or receive a large file over the non-persisted `volatile/blob` class: chunk, BLOCK-publish, manifest, sha256-verify

**Subcommands**

- [`goat blob send`](#goat-blob-send) — Send a file as a chunked volatile blob
- [`goat blob get`](#goat-blob-get) — Receive a chunked volatile blob into a file

## `goat blob send`

Send a file as a chunked volatile blob.

Splits the file into <= 1.5 MiB chunks, BLOCK-publishes each under `<to>/chunk/<NNNNNNNN>`, then publishes a `<to>/manifest` with the chunk count, byte size, and sha256. The receiver must already be running `goat blob get` — volatile keys are not retained, so a send with no live receiver is silently lost.

**Arguments**

| Argument | Description |
|---|---|
| `<FILE>` | File to send |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--to <TO>` | Base volatile key, e.g. `volatile/<peer>/blob/<artifact-id>` |  |
| `--dry-run` | Print what would be published without opening a session |  |

## `goat blob get`

Receive a chunked volatile blob into a file.

Subscribes to `<key>/**`, collects the chunks + manifest as they stream in, reassembles in order, and verifies the sha256 before writing. Start this BEFORE the sender publishes: volatile keys have no catch-up.

**Arguments**

| Argument | Description |
|---|---|
| `<KEY>` | Base volatile key the sender published to |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--output, -o <OUTPUT>` | Output file path |  |
| `--timeout-secs <TIMEOUT_SECS>` | Give up if the transfer doesn't complete within this many seconds | `120` |

## `goat replay`

Replay past events from the audit-tail (read, not live subscribe).

Reads the Object-Lock audit-tail via portal-minted presigned URLs — no MinIO credentials on the laptop.

**Arguments**

| Argument | Description |
|---|---|
| `<KEY_PATTERN>` | Key expression to replay from the audit-tail (e.g. `example-vendor/**`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--since <SINCE>` | How far back: a duration (`1h`, `90m`, `2h30m`) or an RFC3339 timestamp. Default: last 1h |  |
| `--count <COUNT>` | Show only the newest N events in the window |  |
| `--json` | Emit one JSON line per event |  |

## `goat health`

Show fabric health (per-class load vs capacity + green/amber/red) and, with --me/--peer, a producer's own footprint

**Options**

| Flag | Description | Default |
|---|---|---|
| `--domain <DOMAIN>` | Which environment-health domain to read | `fabric` |
| `--watch` | Live-refresh: keep printing each health beat as it arrives |  |
| `--peer <PEER>` | Also show this producer's own footprint (its rate, class shares, rank, near-limit). Subscribes ops/fabric/footprint/<peer>. Fabric only (rejected with a non-fabric --domain) |  |
| `--me` | Shorthand for `--peer <your-own-id>`, resolved from the profile |  |

## `goat top`

Live single-pane fabric dashboard (the top(1) of the fabric): every environment-health domain at once + a sortable producer ranking, auto-refreshing. Interactive screen is TTY-only; --once is the scriptable plain-text snapshot (ADR 1044)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--interval <INTERVAL>` | Redraw / channel-drain cadence, in seconds | `2` |
| `--site <SITE>` | Filter readings to one site scope (when multiple sites publish on the same fabric). Matches the `<scope>` of `ops/<domain>/health/<scope>` |  |
| `--sort <SORT>` | Initial producer-table sort; `s` cycles it live | `msgs` |
| `--filter <FILTER>` | Start with the producer table filtered to actors whose alias or anchor contains this substring (case-insensitive). In the interactive screen `/` edits the filter live (htop idiom); for `--once` it's the only way to narrow the scripted snapshot |  |
| `--once` | Batch snapshot (the `top -bn1` analog): collect beats for one bounded window, render ONE frame as plain text to stdout, then exit. Allowed on a non-TTY — the scriptable / CI counterpart to the interactive screen |  |

## `goat presence`

Who is alive on the fabric right now — the native liveliness presence view (one-shot set or --watch join/leave edges)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--key <KEY>` | Key expression to scope the presence view (alias OK for the leading segment). Default: every presence token on the fabric |  |
| `--watch` | Watch live: print the current set, then join (+) / leave (−) edges as they happen, until Ctrl-C |  |
| `--uuid` | Force raw UUID anchors (no alias rendering) |  |
| `--json` | Machine output (one-shot only): JSON array of live tokens. Raw keys, no aliases in machine paths (ADR 1039 invariant); the alias rides alongside as a display field |  |
| `--subscribers-of <SUBSCRIBERS_OF>` | The subscriber-delivery line: count the live catch-up subscribers on a topic (alias OK) via their AdvancedSubscriber detection tokens at `<topic>/@adv/sub/…`. Plain subscribers don't declare one, so this is a floor, not a census |  |

## `goat who`

Identity common operating picture (the top(1) of identities): a searchable roster browser — who is in the goatnet, what each may do, and what they're doing. Interactive screen is TTY-only; --once is the scriptable plain-text snapshot (ADR 1047)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--sort <SORT>` | Initial roster sort; `s` cycles it live. Default LIVE (activity-first; `/` search stays instant, so still searchable-first) | `live` |
| `--filter <FILTER>` | Start with the roster filtered. Space-separated terms are AND-ed; a bare term is a case-insensitive substring of the name/anchor/display-name, and the facet prefixes `live:on\|off` / `use:on\|off` narrow by live/producing state. In the interactive screen `/` edits this live (htop idiom) |  |
| `--site <SITE>` | Filter the presence roll-up to one site scope (matches the `<scope>` of `ops/presence/health/<scope>`) |  |
| `--once` | Batch snapshot (the `top -bn1` analog): collect live state for one bounded window, render ONE plain-text roster frame (name / live / usage / band / registry-membership) to stdout, then exit. Allowed on a non-TTY. Permissions are NOT included in `--once` (an N-call-per-identity fetch) — the scriptable single-identity capability path stays `goat access who` |  |

## `goat profile`

Manage the local goat profile (mTLS cert + portal token + endpoint set).

**Subcommands**

- [`goat profile init`](#goat-profile-init) — Establish a profile from a bundle.cbor — verifies signature, extracts mTLS cert/key/CA + portal token + endpoints, writes to the profile directory (mode 0700 / 0600), then runs `goat doctor`
- [`goat profile show`](#goat-profile-show) — Show the active profile's endpoints, role, org prefix, and cert expiry. Reads files only; does not contact the network
- [`goat profile rotate`](#goat-profile-rotate) — Rotate the profile with a new bundle. The previous profile is preserved at <dir>.bak/ for one rotation cycle
- [`goat profile path`](#goat-profile-path) — Print the resolved profile directory path and exit. Always works, even before `init` has been run

## `goat profile init`

Establish a profile from a bundle.cbor — verifies signature, extracts mTLS cert/key/CA + portal token + endpoints, writes to the profile directory (mode 0700 / 0600), then runs `goat doctor`

**Arguments**

| Argument | Description |
|---|---|
| `<BUNDLE>` | Path to the operator bundle (CBOR) |

## `goat profile show`

Show the active profile's endpoints, role, org prefix, and cert expiry. Reads files only; does not contact the network

## `goat profile rotate`

Rotate the profile with a new bundle. The previous profile is preserved at <dir>.bak/ for one rotation cycle

**Arguments**

| Argument | Description |
|---|---|
| `<BUNDLE>` |  |

## `goat profile path`

Print the resolved profile directory path and exit. Always works, even before `init` has been run

## `goat context`

Manage contexts (`ls` / `use` / `show` / `rm`).

**Subcommands**

- [`goat context ls`](#goat-context-ls) — List contexts known to this profile
- [`goat context use`](#goat-context-use) — Switch the active context (writes `current_context` to config.toml)
- [`goat context show`](#goat-context-show) — Show one context's resolved endpoints, role, org prefix, cert expiry
- [`goat context rm`](#goat-context-rm) — Remove a context. Refuses to remove the active context unless another context is selected via `--switch-to <name>`

## `goat context ls`

List contexts known to this profile

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit one JSON line per context instead of pretty output |  |

## `goat context use`

Switch the active context (writes `current_context` to config.toml)

**Arguments**

| Argument | Description |
|---|---|
| `<NAME>` |  |

## `goat context show`

Show one context's resolved endpoints, role, org prefix, cert expiry

**Arguments**

| Argument | Description |
|---|---|
| `<NAME>` | Defaults to the active context |

## `goat context rm`

Remove a context. Refuses to remove the active context unless another context is selected via `--switch-to <name>`

**Arguments**

| Argument | Description |
|---|---|
| `<NAME>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--switch-to <SWITCH_TO>` | After removal, switch active context to this one |  |
| `--yes, -y` | Skip confirmation prompt |  |

## `goat init`

First-run wizard: pick sandbox or real, prove it works, point at the tutor.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--sandbox` | Go straight to the sandbox (skip the prompt) |  |
| `--yes` | Assume defaults; never prompt (sandbox unless a profile already exists). Useful for scripted first-runs |  |

## `goat tutor`

A self-paced, hands-on tour of the fabric (vimtutor-style; resumable).

**Options**

| Flag | Description | Default |
|---|---|---|
| `--list` | List the lessons and exit |  |
| `--reset` | Forget saved progress and start from the first lesson |  |

## `goat docs`

Browse the goat docs, or emit the full command reference (--markdown).

**Options**

| Flag | Description | Default |
|---|---|---|
| `--markdown` | Emit the full command reference as Markdown (for the hosted docs site or offline reading). Generated from the live command tree, so it always matches `goat --help` |  |

## `goat hello`

Round-trip a hello through the fabric end-to-end (sandbox or real env).

**Options**

| Flag | Description | Default |
|---|---|---|
| `--sandbox` | Run against the in-process sandbox — no profile or mesh needed. Default when no profile is initialized |  |
| `--json` | Emit one JSON proof object instead of the friendly narration (machine-clean: `mode` is `sandbox` or `real`) |  |

## `goat doctor`

Diagnose local readiness → router path → TLS material → mTLS session → portal. Each failure reports the observation and a next action.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit one JSON line per step instead of pretty output |  |
| `--all` | Run network probes despite earlier failures when their inputs exist |  |
| `--topic <TOPIC>` | Write-precheck a specific topic key BEFORE publishing: writes a unique sentinel to a scratch sub-key under <KEY> and reads it back. A round-trip means you can write here; a silent miss is exactly what an ACL ingress-denial looks like to a publisher. Uses only your own publish credential — no COP/admin access — so a denied org can self-diagnose. Replaces the local-readiness and fabric sweep; runs the probe and exits |  |

## `goat org`

Browse orgs registered in the portal.

**Subcommands**

- [`goat org ls`](#goat-org-ls) — List all orgs declared in the portal
- [`goat org show`](#goat-org-show) — Show one org's full record (prefix, display name, topic count)

## `goat org ls`

List all orgs declared in the portal

**Options**

| Flag | Description | Default |
|---|---|---|
| `--search <SEARCH>` | Fuzzy-match against org prefix + display name |  |
| `--json` | Emit one JSON line per org instead of a pretty table |  |

## `goat org show`

Show one org's full record (prefix, display name, topic count)

**Arguments**

| Argument | Description |
|---|---|
| `<PREFIX>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit JSON instead of pretty |  |

## `goat topic`

List, show, create, or delete topics.

**Subcommands**

- [`goat topic ls`](#goat-topic-ls) — List topics. Optional prefix arg drilldown (e.g. `example-vendor` or `example-vendor/observations`); without it, lists across all orgs
- [`goat topic show`](#goat-topic-show) — Show one topic's full record (key, format, schema, publish count)
- [`goat topic create`](#goat-topic-create) — Create a new topic in the portal. Requires the profile's Bearer token to have org-prefix authority server-side
- [`goat topic delete`](#goat-topic-delete) — Delete a topic. Default is soft delete (portal-side semantics); `--hard` passes `?hard=true` and the portal may refuse based on audit-tail retention policy
- [`goat topic export`](#goat-topic-export) — Export an org's topics as a bulk file (the interactive state-as-code on-ramp). Prints to stdout; redirect to a file
- [`goat topic import`](#goat-topic-import) — Import topics from a bulk file (the shape `export` produces). Reads from a file or stdin. `--dry-run` previews the diff (create/update/unchanged/retire) without applying

## `goat topic ls`

List topics. Optional prefix arg drilldown (e.g. `example-vendor` or `example-vendor/observations`); without it, lists across all orgs

**Arguments**

| Argument | Description |
|---|---|
| `<PREFIX>` | Optional org or org/system prefix for hierarchical drilldown |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--search <SEARCH>` | Fuzzy-match against topic key + description (overrides prefix drilldown) |  |
| `--json` | Emit one JSON line per topic |  |

## `goat topic show`

Show one topic's full record (key, format, schema, publish count)

**Arguments**

| Argument | Description |
|---|---|
| `<KEY>` | Topic key (e.g. `example-vendor/observations/v1`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit JSON instead of pretty |  |

## `goat topic create`

Create a new topic in the portal. Requires the profile's Bearer token to have org-prefix authority server-side

**Arguments**

| Argument | Description |
|---|---|
| `<KEY>` | Topic key (e.g. `example-vendor/observations/v1`). The leading slash segment is the org prefix the portal POST targets |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--format <FORMAT>` | Encoding format declaration (e.g. `protobuf:MyMessage` or `json`) |  |
| `--description <DESCRIPTION>` | One-line description |  |
| `--schema-file <SCHEMA_FILE>` | Path to a schema file (e.g. .proto). Optional — a topic can be registered with just a format declaration. Schema-body upload is not yet available |  |
| `--dry-run` | Don't actually POST — print what would happen and exit 0 |  |

## `goat topic delete`

Delete a topic. Default is soft delete (portal-side semantics); `--hard` passes `?hard=true` and the portal may refuse based on audit-tail retention policy

**Arguments**

| Argument | Description |
|---|---|
| `<KEY>` | Topic key (e.g. `example-vendor/observations/v1`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--hard` | Hard delete (default is soft / deprecate) |  |
| `--yes, -y` | Skip confirmation prompt |  |
| `--dry-run` | Don't actually DELETE — print what would happen and exit 0 |  |

## `goat topic export`

Export an org's topics as a bulk file (the interactive state-as-code on-ramp). Prints to stdout; redirect to a file

**Arguments**

| Argument | Description |
|---|---|
| `<VENDOR>` | Vendor prefix whose topics to export |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--format <FORMAT>` | Output format: yaml (default) or json | `yaml` |

## `goat topic import`

Import topics from a bulk file (the shape `export` produces). Reads from a file or stdin. `--dry-run` previews the diff (create/update/unchanged/retire) without applying

**Arguments**

| Argument | Description |
|---|---|
| `<VENDOR>` | Org prefix to import into. Must match the file's `org` (legacy `vendor:` files accepted) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--file <FILE>` | Path to the bulk file; omit (or `-`) to read from stdin |  |
| `--format <FORMAT>` | Body format: yaml (default) or json. Sets the Content-Type | `yaml` |
| `--mode <MODE>` | additive (create/update only, default) or replace (also retire topics absent from the file) | `additive` |
| `--dry-run` | Preview the diff without applying it |  |

## `goat pub`

Publish a payload to a topic key.

**Arguments**

| Argument | Description |
|---|---|
| `<KEY>` | Topic key to publish to (e.g. `example-vendor/observations/v1`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--payload <PAYLOAD>` | Payload text, given inline |  |
| `--payload-file <PAYLOAD_FILE>` | Payload from a file (use --payload for inline text, or pipe stdin) |  |
| `--encoding <ENCODING>` | Stamp the sample's self-describing wire encoding. Shorthands: `json`, `cbor`, `text`, `protobuf`, `protobuf:<MessageName>` (the message name rides along so a consumer knows which descriptor to decode with). Any other value is passed through as a media type verbatim. Unset leaves the Zenoh default (`zenoh/bytes`) |  |
| `--confirm` | After publishing, read the value back from router storage to verify it landed |  |
| `--volatile-stream` | Publish with DROP congestion control for the loss-tolerant `volatile/<peer>/stream/**` firehose, so a high-rate producer never back-pressures the session |  |

## `goat sub`

Subscribe to a topic key expression.

**Arguments**

| Argument | Description |
|---|---|
| `<KEY_PATTERN>` | Zenoh key expression to subscribe to (e.g. `example-vendor/**`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--follow` | Keep the subscription open; without it, exits after `--count` events |  |
| `--decode` | Decode payload bytes against the topic's portal-registered schema |  |
| `--tui` | Interactive Tail lens for live tail (default is line-oriented): event table, rate sparkline, on-demand decoded/hex viewer. Requires a terminal (refuses on a non-TTY) |  |
| `--count <COUNT>` | Exit after this many events when `--follow` is not set (default 1) | `1` |
| `--since <SINCE>` | Replay past events from the audit-tail instead of subscribing live. A duration (`1h`, `90m`) or an RFC3339 timestamp |  |

## `goat inspect`

Delegate a diagnostic verdict request to stethoscope.

**Arguments**

| Argument | Description |
|---|---|
| `<TARGET>` | Target spec: `vendor=<prefix>`, `topic=<key>`, or `system=<name>`. Forwarded verbatim to stethoscope's `/api/diagnose` endpoint |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit one JSON line per outcome instead of pretty glyph render |  |
| `--tui` | Open the interactive Diagnose lens instead of the line-oriented render. Requires a terminal (refuses on a non-TTY); mutually exclusive with --json |  |
| `--endpoint <ENDPOINT>` | Override the stethoscope endpoint (default: derived from the profile's portal_endpoint by substituting `portal.*` → `stethoscope.*`, also honors `$GOAT_STETHOSCOPE_ENDPOINT`) |  |

## `goat ui`

Interactive merged picker: browse the declared ⊕ live roster and hop to the Diagnose / Tail lens on the selected target.

**Arguments**

| Argument | Description |
|---|---|
| `<TARGET>` | Optional target to pre-select on open (a topic key, e.g. `example-vendor/observations/v1`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--snapshot-endpoint <SNAPSHOT_ENDPOINT>` | Override the panoscope snapshot endpoint. Default: derived from the profile's portal_endpoint (`portal.*` → `panoscope.*`), also honors `$GOAT_PANOSCOPE_ENDPOINT` |  |

## `goat compose`

Interactive command builder TUI.

**Arguments**

| Argument | Description |
|---|---|
| `<SUBCMD>` | Subcommand path to start the builder at (e.g. `topic create`). Without it, opens at the root ("which subcommand?") |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--no-context` | Disable the live substrate-context pane (write-prefixes etc.) |  |

## `goat narrate`

Reflect over recent substrate activity from the audit-tail as a who-touched-what cascade (--since / --actor / --json).

**Options**

| Flag | Description | Default |
|---|---|---|
| `--since <SINCE>` | Reflect over events since this point: a duration (`60s`, `5m`, `1h`) or an RFC3339 timestamp. Default: last 60s |  |
| `--actor <ACTOR>` | Filter to a specific actor (an org prefix or its alias) |  |
| `--json` | Emit one NDJSON line per cascade event |  |

## `goat completion`

Print shell completion script to stdout.

**Arguments**

| Argument | Description |
|---|---|
| `<SHELL>` |  |

## `goat explain`

Look up an error code's meaning + remediation in the embedded `goat-cli-error-codes.md` doc (cargo --explain pattern).

**Arguments**

| Argument | Description |
|---|---|
| `<CODE>` | The error code to look up (e.g. `E_PROFILE_MISSING`). Case-insensitive; the leading `E_` is optional |

## `goat mock`

Publish synthetic samples to a topic. Default: trivial hello-world JSON at 1Hz. Pass `--format <.fds>` + `--message <Name>` to encode operator-supplied protobuf instead.

**Arguments**

| Argument | Description |
|---|---|
| `<KEY>` | Topic key to publish to (e.g. `acme/sandbox/v1`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--format <FORMAT>` | Path to a .fds (FileDescriptorSet) for protobuf encoding. Without it, sends the default hello-world JSON |  |
| `--message <MESSAGE>` | Message name within the FDS (fully-qualified or simple). Required when --format is set |  |
| `--payload-template <PAYLOAD_TEMPLATE>` | Operator-supplied JSON seed encoded against the message descriptor. Without it, the default protobuf path emits an all-fields-default instance. Ignored when --format is unset |  |
| `--rate <RATE>` | Publish rate in samples per second (default 1) | `1` |
| `--count <COUNT>` | Send this many samples then exit. Default = forever (Ctrl-C to stop) |  |

## `goat admin`

Operator-only administration: org lifecycle, onboard/revoke/rotate, mesh-peer cut, device + operator-tier provisioning, LAN triage. Authorized by your operator credentials (server-side via portal/NetBird token scope); inert for non-admins — an unauthorized call changes nothing and fails with `E_NOT_ADMIN`.

**Subcommands**

- [`goat admin org`](#goat-admin-org) — Org lifecycle (admin-gated)
- [`goat admin invite`](#goat-admin-invite) — Sponsor a candidate: mint a one-time onboarding invite (ADR 1062)
- [`goat admin queue`](#goat-admin-queue) — Review + act on the onboarding queue from the CLI (the admin's stage-3 gate — mirrors /admin/onboard-queue.html): list proposals, approve (→ the runner mints, or run the emitted command), or reject
- [`goat admin onboard`](#goat-admin-onboard) — Onboard or revoke an operator / org by handle. Wraps `ops/scripts/ansible-preflight.sh operator-onboard.yml -e operator=<handle>` (or `-e action=revoke`). Streams ansible output line-by-line; SIGINT propagates to the child via the process group (tokio::process default)
- [`goat admin rotate`](#goat-admin-rotate) — Re-issue (rotate) a peer's cert in place — same anchor, fresh leaf. The "rotate" sibling of `--revoke`: use it for expiry renewal, post-compromise rotation, or retrofitting a new URI-SAN scheme onto an already-issued cert, all WITHOUT changing the peer's anchor / alias / write namespace / ACL. Pure-portal POST /api/orgs/{anchor}/rotate, operator-gated server-side. The selector is an anchor (32-hex slot_id) or its slot alias; the alias is resolved to the anchor before the call. Prints the fresh one-time bundle URL — fetch it, swap the cert+key on the peer, and restart it. See docs/design/credential-rotation.md
- [`goat admin lan`](#goat-admin-lan) — LAN troubleshooting toolkit for relocated goat mgmt hosts that don't appear on the WAN: `scan` finds them on the local LAN by MAC even when DHCP moved their IP; `doctor` runs a layered L0–L4 triage. Depends on NO goatnet/mesh service — works when the mesh does not. See docs/design/goat-cli.md §"goat admin lan"
- [`goat admin client`](#goat-admin-client) — goat-client device lifecycle (admin-gated). Today: bundle minting via `goat admin client onboard <handle>` which reads the per-device manifest at `ops/devices/<handle>.yml` and translates it into a `bundle-create` invocation. Per-site fleet config (CA-id, mode, age-identity / piv-slot) comes from CLI flags + env vars, NOT the per-device manifest
- [`goat admin peer`](#goat-admin-peer) — Mesh-peer lifecycle — the authoritative mesh-first cut. `ls` lists mesh peers; `cut` ejects one (deletes its NetBird peer, so its WireGuard tunnel drops). On a brokerless slot-pool fabric the Zenoh write-ACL is pre-baked for the whole pool, so deleting an org record does NOT stop a flooder — removing the *peer* does (a cert is inert without a mesh path). This is the one-command emergency cut: `goat health` finds the hog, `goat admin peer cut` ejects it. Needs the NetBird admin token in the environment ($<CONTEXT>_NB_API_TOKEN, e.g. GALAXY_NB_API_TOKEN, or $GOAT_NB_API_TOKEN)
- [`goat admin operator`](#goat-admin-operator) — Backend-OPERATOR tier provisioning (admin-gated). Distinct from `onboard` (the participant tier): `operator onboard <handle>` wraps `ops/ansible/operator-tier-onboard.yml` to mint the operator's `operator-trial-peer` Zenoh leaf (writes `ops/**`), install their host SSH key, grant the Ory admin group on their existing identity, render the operator bundle, and PRINT the two blast-radius additions (portal key, age recipient — never auto-edited). The manifest must carry an `operator:` block. See docs/operations/admin-operator-guide.md

## `goat admin org`

Org lifecycle (admin-gated).

Named `org` since the ADR 1052 vendor→org rename; `vendor` stays a hidden alias so existing scripts keep working.

**Subcommands**

- [`goat admin org create`](#goat-admin-org-create) — Create a new org in the portal. Admin-gated server-side
- [`goat admin org delete`](#goat-admin-org-delete) — Revoke an org. Portal refuses with 409 if the org still has non-retired topics — retire them first via `goat topic delete --hard <key>`

## `goat admin org create`

Create a new org in the portal. Admin-gated server-side

**Arguments**

| Argument | Description |
|---|---|
| `<PREFIX>` | Org prefix (kebab-case, [a-z0-9-]{1,32}). Used as the X.509 URI-SAN authority + the topic-key leading segment |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--display-name <DISPLAY_NAME>` | Human-readable display name (shown in the portal org catalog UI). Defaults to the prefix |  |
| `--synthetic` | Mark as synthetic (trashgoat-style fake org). Defaults to false (real org) |  |
| `--auto-approve` | Auto-approve at create time. Admin tokens can do this; non- admin tokens get 403 if they try |  |
| `--dry-run` | Don't actually POST; print the body + exit 0 |  |

## `goat admin org delete`

Revoke an org. Portal refuses with 409 if the org still has non-retired topics — retire them first via `goat topic delete --hard <key>`

**Arguments**

| Argument | Description |
|---|---|
| `<PREFIX>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--yes, -y` | Skip confirmation prompt |  |
| `--dry-run` | Don't actually DELETE; print the intended request + exit 0 |  |

## `goat admin invite`

Sponsor a candidate: mint a one-time onboarding invite (ADR 1062).

Every field is OPTIONAL (minimal-burden): mint a bare link and let the enrollee fill their own details. Read a spec from a YAML file with `--file`, tweak it in your `$EDITOR` before firing with `--edit`, or pass fields as flags. Prints the one-time nomination link to hand the candidate. Escalated admin tiers (`iam-admin`/`platform-admin`) may be offered here — sponsor-only — and are flagged for scrutiny.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--file <FILE>` | Read the invite spec from a YAML file. Recognized keys (all optional): `env`, `tier`, `affiliation`, `site`, `machine_quota`, `ttl_hours`, `note`. Flags below override matching file keys |  |
| `--edit` | Open the spec in `$EDITOR` before firing (like `git commit`). Starts from `--file` if given, else a commented template. On empty/unchanged save it still fires the (possibly bare) invite |  |
| `--env <ENV>` | Target goatnet (e.g. `zagros`) |  |
| `--tier <TIER>` | Access tier: `developer` \| `browse` \| `iam-admin` \| `platform-admin`. The two admin tiers are ESCALATED (scrutinized). Omit to let the enrollee choose (non-escalated only) |  |
| `--affiliation <AFFILIATION>` | Affiliation kind: `partner` \| `organic` \| `vendor` |  |
| `--site <SITE>` | Site (defaults to the environment) |  |
| `--machine-quota <MACHINE_QUOTA>` | Machine-enrollment quota the person may use later |  |
| `--ttl-hours <TTL_HOURS>` | One-time link lifetime in hours (default 72, max 720) |  |
| `--note <NOTE>` | Free-text note for your own reference |  |
| `--json` | Emit the raw JSON mint response instead of the friendly summary |  |

## `goat admin queue`

Review + act on the onboarding queue from the CLI (the admin's stage-3 gate — mirrors /admin/onboard-queue.html): list proposals, approve (→ the runner mints, or run the emitted command), or reject

**Subcommands**

- [`goat admin queue ls`](#goat-admin-queue-ls) — List proposals. Default `sponsor_approved` (the ones ready for the admin)
- [`goat admin queue approve`](#goat-admin-queue-approve) — Approve a sponsor-approved proposal → the runner mints it (or run the emitted command). Refused unless the sponsor has already vouched
- [`goat admin queue reject`](#goat-admin-queue-reject) — Reject a proposal (at either the sponsor or admin stage)

## `goat admin queue ls`

List proposals. Default `sponsor_approved` (the ones ready for the admin)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--status <STATUS>` | Status filter: `proposed` \| `sponsor_approved` \| `approved` \| `onboarded` \| `rejected` \| `all` | `sponsor_approved` |
| `--env <ENV>` | Filter to one goatnet (e.g. `zagros`) |  |
| `--json` | Emit raw JSON instead of the table |  |

## `goat admin queue approve`

Approve a sponsor-approved proposal → the runner mints it (or run the emitted command). Refused unless the sponsor has already vouched

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Proposal id (`prop-…`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit the raw JSON (manifest + command) instead of the summary |  |

## `goat admin queue reject`

Reject a proposal (at either the sponsor or admin stage)

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Proposal id (`prop-…`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--note <NOTE>` | Reason, recorded on the proposal |  |

## `goat admin onboard`

Onboard or revoke an operator / org by handle. Wraps `ops/scripts/ansible-preflight.sh operator-onboard.yml -e operator=<handle>` (or `-e action=revoke`). Streams ansible output line-by-line; SIGINT propagates to the child via the process group (tokio::process default)

**Arguments**

| Argument | Description |
|---|---|
| `<HANDLE>` | Handle (kebab-case slug). Manifest must exist at `ops/operators/<handle>.yml` (flat / legacy) or, when `--env <env>` is passed, `ops/operators/<env>/<handle>.yml` in the discovered repo root. Omit when using `--org` |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--org <ORG>` | ORG-scope revoke (cascade). Instead of one handle, revoke EVERY active member of the affiliation org `<id>` (the `affiliation.id`, e.g. `org-1`) — persons + systems, the whole unit. Resolves the members from that goatnet's declared registry (`docs/operators/<goatnet>/roster.json`, the affiliation-grouped roster; the goatnet comes from `--env`, or from the `goatnet` key in the context's `profile.toml` for the rotate cascade — ADR 1172) and runs the per-handle revoke for each, after printing the blast radius and asking to confirm (skip the prompt with `--yes`). Requires `--revoke`; mutually exclusive with a positional handle. Identity grammar B.4 / ADR 1031 org-cascade (`docs/design/credential-revocation-and-offboarding.md`) |  |
| `--yes` | Skip the interactive confirmation prompt for `--org` cascades. For scripted/non-interactive use; the blast radius is still printed |  |
| `--env <ENV>` | Environment / goatnet the manifest belongs to (e.g. `zagros`, `nova`). Selects the per-env manifest subfolder `ops/operators/<env>/<handle>.yml` and is passed to the playbook as `-e operator_env=<env>` so the per-site maps (NetBird mgmt URL, Ory Kratos admin host, Zenoh portal) resolve to that goatnet. Omit for the flat legacy layout (the playbook treats no env as the `nova` default) |  |
| `--revoke` | Revoke instead of enroll. Passes `-e action=revoke` to the playbook |  |
| `--repair` | Re-mint a partial bundle in place: run the (hardened) revoke leg then the enroll leg as one operation, so every surface is freshly minted and rendered into one complete bundle. The all-or-nothing render means a half-provisioned bundle can't be completed by a plain re-run — this is the supported repair. The revoke leg frees the surfaces but leaves the manifest `status: active` (via `-e onboard_repair=true`) so the enroll leg re-mints onto it. See P1.5 in docs/design/onboard-rotate-revoke-streamlining.md. Mutually exclusive with --revoke |  |
| `--native` | Force the native fast path (the DEFAULT for a plain enroll — provisions all three identity surfaces directly, no Ansible, seconds not minutes; mirrors `goat admin rotate`). Mints the Zenoh wire-layer (`POST /api/orgs` → slot/cert/key), the NetBird mesh setup key, AND the Ory Kratos login identity (over an SSH shell-out to the site Thor's Kratos admin API) — P2.1/P2.2/P2.4. Native config covers nova/zagros; other envs defer that leg to Ansible with a clear note. The Ory leg runs last — a failure there leaves the Zenoh/NetBird bundle usable, marked `partial`, never silently dropped. The Ory operator-IdP surface stays on Ansible. As of goat-cli 0.4.13 the native path is the DEFAULT; this flag now only FORCES it (erroring if the manifest isn't native-serviceable) — pass `--ansible` for the legacy playbook. Enroll-only (not valid with --revoke / --repair / --org). See docs/design/onboard-rotate-revoke-streamlining.md |  |
| `--ansible` | Force the legacy Ansible playbook (`operator-onboard.yml`) instead of the native fast path. The native path is the default for a plain enroll; use `--ansible` when you need a surface native doesn't do (Ory operator-IdP, an env with no native config, observer/prod Zenoh flavours) or to fall back deliberately. Revoke/repair always use the playbook regardless |  |
| `--repo-root <REPO_ROOT>` | Override repo root (defaults to searching upward from cwd for a `.git` directory) |  |
| `--dry-run` | Don't actually run ansible-playbook; print the command + exit 0. Useful for previewing in CI / scripts |  |

## `goat admin rotate`

Re-issue (rotate) a peer's cert in place — same anchor, fresh leaf. The "rotate" sibling of `--revoke`: use it for expiry renewal, post-compromise rotation, or retrofitting a new URI-SAN scheme onto an already-issued cert, all WITHOUT changing the peer's anchor / alias / write namespace / ACL. Pure-portal POST /api/orgs/{anchor}/rotate, operator-gated server-side. The selector is an anchor (32-hex slot_id) or its slot alias; the alias is resolved to the anchor before the call. Prints the fresh one-time bundle URL — fetch it, swap the cert+key on the peer, and restart it. See docs/design/credential-rotation.md

**Arguments**

| Argument | Description |
|---|---|
| `<SELECTOR>` | Anchor (slot_id) or its slot alias to re-issue. Omit when using `--org` |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--org <ORG>` | ORG-scope rotation (cascade). Instead of one selector, re-issue the leaf cert of EVERY active member of the affiliation org `<id>` (the `affiliation.id`) in place — the rotate sibling of the `onboard --org --revoke` cascade, at the same granularity. Resolves the members from that goatnet's declared registry (`docs/operators/<goatnet>/roster.json`, ADR 1172) and resolves each to its slot anchor via the alias map; members with no resolvable anchor are reported and skipped (fail-visible, never silent). Prints the blast radius and asks to confirm (skip with `--yes`). Mutually exclusive with a positional selector. Identity grammar / ADR 1044 (`docs/design/credential-rotation.md`) |  |
| `--yes` | Skip the interactive confirmation prompt for `--org` cascades. The blast radius is still printed |  |
| `--repo-root <REPO_ROOT>` | Repo root holding `docs/operators/<goatnet>/roster.json` for `--org` resolution. Defaults to the nearest `.git` ancestor of the cwd |  |
| `--dry-run` | Don't actually POST; resolve the selector(s) + print the intended request(s) + exit 0 |  |

## `goat admin lan`

LAN troubleshooting toolkit for relocated goat mgmt hosts that don't appear on the WAN: `scan` finds them on the local LAN by MAC even when DHCP moved their IP; `doctor` runs a layered L0–L4 triage. Depends on NO goatnet/mesh service — works when the mesh does not. See docs/design/goat-cli.md §"goat admin lan"

**Subcommands**

- [`goat admin lan scan`](#goat-admin-lan-scan) — Find goat mgmt hosts on the local LAN even when DHCP moved their IP: sweep each local /24 (TCP 22+443, 200 ms) to warm the ARP cache, then match ARP-table MACs against the host registry (embedded defaults + ~/.goat/lan-hosts.yaml override)
- [`goat admin lan doctor`](#goat-admin-lan-doctor) — Layered triage of a relocated host: L0 local subnet plausibility, L1 ping + TCP reachability, L2 sshd banner health (wedged-sshd detection), L3 mesh-free service-plane probes (443 + :9090 /metrics), L4 on-host checks over one BatchMode ssh (clock / chrony-presync / default route / WAN / netbird / docker)

## `goat admin lan scan`

Find goat mgmt hosts on the local LAN even when DHCP moved their IP: sweep each local /24 (TCP 22+443, 200 ms) to warm the ARP cache, then match ARP-table MACs against the host registry (embedded defaults + ~/.goat/lan-hosts.yaml override)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit one JSON object instead of the pretty table |  |

## `goat admin lan doctor`

Layered triage of a relocated host: L0 local subnet plausibility, L1 ping + TCP reachability, L2 sshd banner health (wedged-sshd detection), L3 mesh-free service-plane probes (443 + :9090 /metrics), L4 on-host checks over one BatchMode ssh (clock / chrony-presync / default route / WAN / netbird / docker)

**Arguments**

| Argument | Description |
|---|---|
| `<TARGET>` | Registry host name (e.g. `mgmt-nova-01-thor`) or a literal IPv4 address. Omit to triage every registry host `scan` finds |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--ssh-user <SSH_USER>` | SSH user for the L4 on-host probe (default: the registry entry's ssh_user, else `goat`) |  |
| `--no-ssh` | Skip the L4 on-host ssh probe even when the banner is healthy |  |
| `--json` | Emit one JSON object instead of pretty layer lines |  |

## `goat admin client`

goat-client device lifecycle (admin-gated). Today: bundle minting via `goat admin client onboard <handle>` which reads the per-device manifest at `ops/devices/<handle>.yml` and translates it into a `bundle-create` invocation. Per-site fleet config (CA-id, mode, age-identity / piv-slot) comes from CLI flags + env vars, NOT the per-device manifest

**Subcommands**

- [`goat admin client onboard`](#goat-admin-client-onboard) — Mint a wg-cp0 bundle for a device. Reads the manifest at `ops/devices/<handle>.yml`, runs the operator-side validate.py pre-flight, then runs `bundle-create` with flags derived from the manifest + per-site fleet config. Output bundle lands at `ops/devices/out/<handle>-device.cbor` (mode 0600). Operator hands that file to the device-side operator who runs `goat client bundle import <path>` on the device

## `goat admin client onboard`

Mint a wg-cp0 bundle for a device. Reads the manifest at `ops/devices/<handle>.yml`, runs the operator-side validate.py pre-flight, then runs `bundle-create` with flags derived from the manifest + per-site fleet config. Output bundle lands at `ops/devices/out/<handle>-device.cbor` (mode 0600). Operator hands that file to the device-side operator who runs `goat client bundle import <path>` on the device

**Arguments**

| Argument | Description |
|---|---|
| `<HANDLE>` | Handle (kebab-case slug). Manifest must exist at `ops/devices/<handle>.yml` |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--ca-id <CA_ID>` | CA identifier passed to bundle-create -ca-id (e.g. `dev-goat-ca-2026-04-20`). Required when $GOAT_DEV_CA_ID is not set |  |
| `--mode <MODE>` | Signing mode: dev (age-encrypted file) \| prod (YubiKey PIV slot 9c). Defaults to dev | `dev` |
| `--age-identity <AGE_IDENTITY>` | Dev mode. Path to age identity file. Defaults to $GOAT_DEV_CA_AGE_IDENTITY when --mode=dev |  |
| `--ca-key <CA_KEY>` | Dev mode. Path to age-encrypted CA private key. Defaults to $GOAT_DEV_CA_KEY when --mode=dev |  |
| `--tls-trust-anchor <TLS_TRUST_ANCHOR>` | Optional. Path to a PEM TLS trust anchor, embedded in the bundle as tls_trust_anchor |  |
| `--snitch-collector <SNITCH_COLLECTOR>` | Optional. Snitch collector URL (Block 61). When the manifest's `bundle.snitch` is true AND this is set, the wrapper registers the minted snitch_probe_key |  |
| `--repo-root <REPO_ROOT>` | Override repo root (defaults to searching upward from cwd for a `.git` directory) |  |
| `--dry-run` | Print the bundle-create invocation + exit 0 without running it. Useful for previewing without offline-CA material loaded |  |

## `goat admin peer`

Mesh-peer lifecycle — the authoritative mesh-first cut. `ls` lists mesh peers; `cut` ejects one (deletes its NetBird peer, so its WireGuard tunnel drops). On a brokerless slot-pool fabric the Zenoh write-ACL is pre-baked for the whole pool, so deleting an org record does NOT stop a flooder — removing the *peer* does (a cert is inert without a mesh path). This is the one-command emergency cut: `goat health` finds the hog, `goat admin peer cut` ejects it. Needs the NetBird admin token in the environment ($<CONTEXT>_NB_API_TOKEN, e.g. GALAXY_NB_API_TOKEN, or $GOAT_NB_API_TOKEN)

**Subcommands**

- [`goat admin peer ls`](#goat-admin-peer-ls) — List mesh peers (id, name, mesh IP, connected, last-seen, groups). The IP shown is the mesh address (100.64.x.x on wt0) — NOT a LAN address. Direct-vs-relayed is peer-local runtime state (`netbird status` / the mesh-health domain), not in the mgmt-API peer list
- [`goat admin peer cut`](#goat-admin-peer-cut) — Eject a peer from the mesh — the authoritative cut. Selector is an exact peer id, an exact mesh IP, or a case-insensitive substring of the peer name/hostname (as shown by `ls`). Refuses if the selector matches more than one peer (re-run with a more specific selector)

## `goat admin peer ls`

List mesh peers (id, name, mesh IP, connected, last-seen, groups). The IP shown is the mesh address (100.64.x.x on wt0) — NOT a LAN address. Direct-vs-relayed is peer-local runtime state (`netbird status` / the mesh-health domain), not in the mgmt-API peer list

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit a JSON array instead of the table |  |
| `--mgmt-url <MGMT_URL>` | Override the NetBird mgmt URL (else env / derived from profile) |  |

## `goat admin peer cut`

Eject a peer from the mesh — the authoritative cut. Selector is an exact peer id, an exact mesh IP, or a case-insensitive substring of the peer name/hostname (as shown by `ls`). Refuses if the selector matches more than one peer (re-run with a more specific selector)

**Arguments**

| Argument | Description |
|---|---|
| `<SELECTOR>` | Peer id \| mesh IP \| name/hostname substring |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--yes, -y` | Skip the confirmation prompt |  |
| `--mgmt-url <MGMT_URL>` | Override the NetBird mgmt URL (else env / derived from profile) |  |
| `--dry-run` | Resolve + print the target peer, but don't delete. Exit 0 |  |

## `goat admin operator`

Backend-OPERATOR tier provisioning (admin-gated). Distinct from `onboard` (the participant tier): `operator onboard <handle>` wraps `ops/ansible/operator-tier-onboard.yml` to mint the operator's `operator-trial-peer` Zenoh leaf (writes `ops/**`), install their host SSH key, grant the Ory admin group on their existing identity, render the operator bundle, and PRINT the two blast-radius additions (portal key, age recipient — never auto-edited). The manifest must carry an `operator:` block. See docs/operations/admin-operator-guide.md

**Subcommands**

- [`goat admin operator onboard`](#goat-admin-operator-onboard) — Provision the operator tier for `<handle>` (a participant manifest that also carries an `operator:` block). Wraps `operator-tier-onboard.yml`

## `goat admin operator onboard`

Provision the operator tier for `<handle>` (a participant manifest that also carries an `operator:` block). Wraps `operator-tier-onboard.yml`

**Arguments**

| Argument | Description |
|---|---|
| `<HANDLE>` | Handle (kebab-case slug). Manifest at `ops/operators/<env>/<handle>.yml` (or flat `ops/operators/<handle>.yml` with no `--env`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | Goatnet the manifest belongs to (e.g. `zagros`). Passed to the playbook as `-e operator_env=<env>`; omit for the flat nova default |  |
| `--repo-root <REPO_ROOT>` | Override repo root (defaults to the nearest `.git` ancestor of cwd) |  |
| `--dry-run` | Don't run ansible — print the command + exit 0 |  |

## `goat status`

Health synthesis: ZTOL reachability + portal /api/health + local cert expiry, three glyph lines or one --json object.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit one JSON object instead of pretty glyph lines |  |

## `goat services`

Probe each hosted service in `ops/serve/services.yaml` and render one glyph line per service (or one --json object).

**Options**

| Flag | Description | Default |
|---|---|---|
| `--repo-root <REPO_ROOT>` | Override repo root (default: walk up from cwd for `.git`) |  |
| `--json` | Emit one JSON object instead of pretty glyph lines |  |
| `--filter <FILTER>` | Probe only services whose name matches this substring |  |

## `goat client`

Wrap the local goat-clientd daemon via its JSON-RPC IPC socket (`~/.goat-client/goat-clientd.sock`): status / up / down / bundle import.

**Subcommands**

- [`goat client status`](#goat-client-status) — Print the daemon's tunnel status (state, bundle metadata, last handshake, configured endpoints, byte counters)
- [`goat client up`](#goat-client-up) — Ask the daemon to bring the tunnel up. The daemon needs root (TUN creation) — if the daemon was started unprivileged, this returns the daemon's own permission-denied message verbatim
- [`goat client down`](#goat-client-down) — Ask the daemon to bring the tunnel down
- [`goat client bundle`](#goat-client-bundle) — Bundle lifecycle (import a CBOR bundle into the daemon)
- [`goat client mode`](#goat-client-mode) — Get or set the daemon's active mode (wg-cp0-only \| netbird-only \| combined)
- [`goat client trust-roots`](#goat-client-trust-roots) — Inspect the offline-CA trust roots the daemon verifies bundles against. Reads the trust-roots.pem from disk (no daemon RPC)
- [`goat client mesh`](#goat-client-mesh) — Inspect and control the inner mesh (the netbird embed, active in netbird-only / combined mode)
- [`goat client diagnostics`](#goat-client-diagnostics) — Daemon / outer-tunnel diagnostics: uptime, last connect attempt, last error, and the recent log tail. Wraps getDiagnostics. (The netbird-style `debug bundle` tarball needs a daemon debugBundle RPC that v0.1.2 doesn't expose — tracked as daemon-side work.)

## `goat client status`

Print the daemon's tunnel status (state, bundle metadata, last handshake, configured endpoints, byte counters)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit one JSON object instead of pretty lines |  |
| `--detail` | Render the full daemon snapshot: active mode, error detail, and the inner-mesh (netbird embed) sub-block when present. Without it, status stays terse (state + bundle + handshake/bytes) |  |
| `--socket <SOCKET>` | Override the daemon socket path (default: `$HOME/.goat-client/goat-clientd.sock`, also honors `$GOAT_CLIENT_SOCKET`) |  |

## `goat client up`

Ask the daemon to bring the tunnel up. The daemon needs root (TUN creation) — if the daemon was started unprivileged, this returns the daemon's own permission-denied message verbatim

**Options**

| Flag | Description | Default |
|---|---|---|
| `--socket <SOCKET>` | Override the daemon socket path |  |

## `goat client down`

Ask the daemon to bring the tunnel down

**Options**

| Flag | Description | Default |
|---|---|---|
| `--socket <SOCKET>` | Override the daemon socket path |  |

## `goat client bundle`

Bundle lifecycle (import a CBOR bundle into the daemon)

**Subcommands**

- [`goat client bundle import`](#goat-client-bundle-import) — Send a CBOR bundle file to the daemon via importBundle. The daemon parses + verifies the offline-CA signature itself; this CLI is just the transport

## `goat client bundle import`

Send a CBOR bundle file to the daemon via importBundle. The daemon parses + verifies the offline-CA signature itself; this CLI is just the transport

**Arguments**

| Argument | Description |
|---|---|
| `<PATH>` | Path to the signed CBOR bundle file |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--socket <SOCKET>` | Override the daemon socket path |  |

## `goat client mode`

Get or set the daemon's active mode (wg-cp0-only | netbird-only | combined)

**Subcommands**

- [`goat client mode get`](#goat-client-mode-get) — Print the daemon's current active mode
- [`goat client mode set`](#goat-client-mode-set) — Set the daemon's active mode. Accepts wg-cp0-only \| netbird-only \| combined; the daemon is authoritative and may reject a mode the loaded bundle can't satisfy

## `goat client mode get`

Print the daemon's current active mode

**Options**

| Flag | Description | Default |
|---|---|---|
| `--socket <SOCKET>` | Override the daemon socket path |  |

## `goat client mode set`

Set the daemon's active mode. Accepts wg-cp0-only | netbird-only | combined; the daemon is authoritative and may reject a mode the loaded bundle can't satisfy

**Arguments**

| Argument | Description |
|---|---|
| `<MODE>` | Target mode: wg-cp0-only \| netbird-only \| combined |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--socket <SOCKET>` | Override the daemon socket path |  |

## `goat client trust-roots`

Inspect the offline-CA trust roots the daemon verifies bundles against. Reads the trust-roots.pem from disk (no daemon RPC)

**Subcommands**

- [`goat client trust-roots show`](#goat-client-trust-roots-show) — Render each offline-CA key in the daemon's trust-roots.pem with its PEM block type and SHA-256 fingerprint, so an operator can verify trust-anchor pinning

## `goat client trust-roots show`

Render each offline-CA key in the daemon's trust-roots.pem with its PEM block type and SHA-256 fingerprint, so an operator can verify trust-anchor pinning

**Options**

| Flag | Description | Default |
|---|---|---|
| `--path <PATH>` | Path to the trust-roots PEM. Defaults to the daemon's DefaultTrustRootsPath (`<config-dir>/goat-client/trust-roots.pem`, falling back to `~/.goat-client/trust-roots.pem`). Pass this if the daemon was started with a non-default `--trust-roots` |  |

## `goat client mesh`

Inspect and control the inner mesh (the netbird embed, active in netbird-only / combined mode)

**Subcommands**

- [`goat client mesh status`](#goat-client-mesh-status) — Print the inner mesh's state, peer count, and byte counters
- [`goat client mesh diag`](#goat-client-mesh-diag) — Print inner-mesh diagnostics: per-peer handshake/transfer stats plus the daemon's recent inner-mesh log tail
- [`goat client mesh enable`](#goat-client-mesh-enable) — Bring the inner mesh up (no-op if the active mode excludes it or it's already up; the daemon is authoritative)
- [`goat client mesh disable`](#goat-client-mesh-disable) — Bring the inner mesh down

## `goat client mesh status`

Print the inner mesh's state, peer count, and byte counters

**Options**

| Flag | Description | Default |
|---|---|---|
| `--socket <SOCKET>` | Override the daemon socket path |  |

## `goat client mesh diag`

Print inner-mesh diagnostics: per-peer handshake/transfer stats plus the daemon's recent inner-mesh log tail

**Options**

| Flag | Description | Default |
|---|---|---|
| `--socket <SOCKET>` | Override the daemon socket path |  |

## `goat client mesh enable`

Bring the inner mesh up (no-op if the active mode excludes it or it's already up; the daemon is authoritative)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--socket <SOCKET>` | Override the daemon socket path |  |

## `goat client mesh disable`

Bring the inner mesh down

**Options**

| Flag | Description | Default |
|---|---|---|
| `--socket <SOCKET>` | Override the daemon socket path |  |

## `goat client diagnostics`

Daemon / outer-tunnel diagnostics: uptime, last connect attempt, last error, and the recent log tail. Wraps getDiagnostics. (The netbird-style `debug bundle` tarball needs a daemon debugBundle RPC that v0.1.2 doesn't expose — tracked as daemon-side work.)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit the raw JSON reply instead of pretty lines |  |
| `--socket <SOCKET>` | Override the daemon socket path |  |

## `goat plugin`

PATH-based extension discovery (kubectl-style). Any `goat-<name>` executable on PATH is invokable as `goat <name>`; `goat plugin list` enumerates installed plugins, `goat plugin which <name>` resolves the path.

**Subcommands**

- [`goat plugin list`](#goat-plugin-list) — List every `goat-*` executable on PATH (kubectl-style plugins). First occurrence per name wins — shadowed entries are noted
- [`goat plugin which`](#goat-plugin-which) — Print the path of the executable `goat <name>` would dispatch to. Exits non-zero if no `goat-<name>` is on PATH

## `goat plugin list`

List every `goat-*` executable on PATH (kubectl-style plugins). First occurrence per name wins — shadowed entries are noted

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit one JSON object per plugin instead of pretty rows |  |

## `goat plugin which`

Print the path of the executable `goat <name>` would dispatch to. Exits non-zero if no `goat-<name>` is on PATH

**Arguments**

| Argument | Description |
|---|---|
| `<NAME>` | Plugin subcommand name (the `<X>` in `goat <X>`) |

