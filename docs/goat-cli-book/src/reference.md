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

- [`goat ca`](#goat-ca) — The CA roots this machine trusts (`ls` / `add` / `rm`)
- [`goat federate`](#goat-federate) — Federate with a partner: our half, their half, the agreement, and the configuration it implies (docs/design/federation-package-spec.md)
- [`goat update`](#goat-update) — Update the running `goat` binary from this goatnet's download surface (get.<site>). Verifies the published checksum, then atomically replaces the installed binary. Per-goatnet: uses the active context's surface. See `--dry-run` / `--force`
- [`goat schema`](#goat-schema) — Manage a topic's schema body: upload, ls, show, diff
- [`goat acl`](#goat-acl) — Mesh ACL assertions — check what the declared ACL must and must not permit, WITHOUT applying anything
- [`goat warrant`](#goat-warrant) — Least-privilege network authorizations (ADR 1129)
- [`goat access`](#goat-access) — Data-Access COP: who can access which data, and why
- [`goat machine`](#goat-machine) — Enroll and manage machines (system principals) against a custodian's enrollment grant (ADR 1096). `machine enroll <handle> --node-class …`
- [`goat audit`](#goat-audit) — Read the operator-action audit trail (`ops/audit/operator/**`) — the who-did-what record of backend admin actions, decrypted with your operator age identity
- [`goat demo`](#goat-demo) — Run a fabric demo end-to-end: open the browser-side traffic generator and tail the resulting Zenoh traffic in this terminal. With no name it runs the demo service the goatnet publishes (`demo_service`)
- [`goat blob`](#goat-blob) — Send or receive a large file over the non-persisted `volatile/blob` class: chunk, BLOCK-publish, manifest, sha256-verify
- [`goat replay`](#goat-replay) — Replay past events from the audit-tail (read, not live subscribe)
- [`goat health`](#goat-health) — Show fabric health (per-class load vs capacity + green/amber/red) and, with --me/--peer, a producer's own footprint
- [`goat flow`](#goat-flow) — Is data flowing, and when did it last? Per-topic freshness across whatever observers are reachable — panoscope, the audit-tail, and a bounded live listen — with every row labelled by the observer that answered it. Distinguishes "no observer could answer" from "an observer looked back over a stated horizon and found nothing", and never claims a topic has never published. A bare invocation is the one-frame scriptable snapshot; --json is the stable machine contract
- [`goat names`](#goat-names) — Name-fallback self-claims: publish this device's signed claim, or collect published claims for the get tier (ADR 1082 Amendment 2)
- [`goat top`](#goat-top) — Live single-pane fabric dashboard (the top(1) of the fabric): every environment-health domain at once + a sortable producer ranking, auto-refreshing. Interactive screen is TTY-only; --once is the scriptable plain-text snapshot (ADR 1044)
- [`goat presence`](#goat-presence) — Who is alive on the fabric right now — the native liveliness presence view (one-shot set or --watch join/leave edges)
- [`goat microscope`](#goat-microscope) — Per-entity behavior lens (ADR 1090): spine + capability + merged activity timeline + per-source honesty, rendered from the mesh-side microscope service. Plain one-frame output; --json is the stable machine contract
- [`goat who`](#goat-who) — Identity common operating picture (the top(1) of identities): a searchable roster browser — who is in the goatnet, what each may do, and what they're doing. Interactive screen is TTY-only; --once is the scriptable plain-text snapshot (ADR 1047)
- [`goat heal`](#goat-heal) — One-action mesh recovery: run the status probes, and when the mesh is down bounce the mesh client (netbird down → up, or goat-clientd disconnect/connect), wait for the router to come back, and re-probe — reports what it did and the verified end state, or explains why it couldn't heal
- [`goat profile`](#goat-profile) — Manage the local goat profile (mTLS cert + portal token + endpoint set).
- [`goat context`](#goat-context) — Manage contexts (`ls` / `use` / `show` / `rm`).
- [`goat init`](#goat-init) — First-run wizard: pick sandbox or real, prove it works, point at the tutor.
- [`goat tutor`](#goat-tutor) — A self-paced, hands-on tour of the fabric (vimtutor-style; resumable).
- [`goat docs`](#goat-docs) — Browse the goat docs, or emit the full command reference (--markdown).
- [`goat hello`](#goat-hello) — Round-trip a hello through the fabric end-to-end (sandbox or real env).
- [`goat doctor`](#goat-doctor) — Diagnose local setup, mesh client, router path, session, and portal.
- [`goat org`](#goat-org) — Browse orgs registered in the portal.
- [`goat topic`](#goat-topic) — List, show, create, or delete topics.
- [`goat pub`](#goat-pub) — Publish a payload to a topic key.
- [`goat sub`](#goat-sub) — Subscribe to a topic key expression.
- [`goat inspect`](#goat-inspect) — Delegate a diagnostic verdict request to stethoscope.
- [`goat ui`](#goat-ui) — Interactive merged picker: browse the declared ⊕ live roster and hop to the Diagnose / Tail lens on the selected target.
- [`goat compose`](#goat-compose) — Interactive command builder TUI.
- [`goat narrate`](#goat-narrate) — Reflect over recent substrate activity from the audit-tail as a who-touched-what cascade (--since / --actor / --json).
- [`goat completion`](#goat-completion) — Print shell completion script to stdout.
- [`goat explain`](#goat-explain) — Interpret an error: a code (`E_OFF_MESH`, `GB-007`, `X-SSH-001`) or any error text, goat's own or a tool's words it carried — what happened, why, what to do, where to read.
- [`goat mock`](#goat-mock) — Publish synthetic samples to a topic. Default: trivial hello-world JSON at 1Hz. Pass `--format <.fds>` + `--message <Name>` to encode operator-supplied protobuf instead.
- [`goat admin`](#goat-admin) — Operator-only administration: org lifecycle, onboard/revoke/rotate, mesh-peer cut, device + operator-tier provisioning, LAN triage. Authorized by your operator credentials (server-side via portal/NetBird token scope); inert for non-admins — an unauthorized call changes nothing and fails with `E_NOT_ADMIN`.
- [`goat status`](#goat-status) — Health synthesis: router port reachability + a real mTLS Zenoh session open + portal /api/health + local cert expiry + name, PQC and device state, one glyph line per check or one --json object.
- [`goat services`](#goat-services) — Probe each hosted service in `ops/serve/services.yaml` and render one glyph line per service (or one --json object).
- [`goat client`](#goat-client) — Wrap the local goat-clientd daemon via its JSON-RPC IPC socket (`~/.goat-client/goat-clientd.sock`): status / up / down / bundle import.
- [`goat plugin`](#goat-plugin) — PATH-based extension discovery (kubectl-style). Any `goat-<name>` executable on PATH is invokable as `goat <name>`; `goat plugin list` enumerates installed plugins, `goat plugin which <name>` resolves the path.

## `goat ca`

The CA roots this machine trusts (`ls` / `add` / `rm`).

goat ships knowing about no goatnet — every root is one you add. That is what makes one binary usable against any environment: adding a goatnet, or rotating a CA, is not a new release.

**Subcommands**

- [`goat ca ls`](#goat-ca-ls) — Show every CA root this machine trusts, with fingerprints
- [`goat ca add`](#goat-ca-add) — Trust a CA root: copies the PEM into the trust directory and prints its fingerprint so you can check it out of band
- [`goat ca rm`](#goat-ca-rm) — Stop trusting a CA root, by the id `goat ca ls` shows

## `goat ca ls`

Show every CA root this machine trusts, with fingerprints

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit JSON |  |

## `goat ca add`

Trust a CA root: copies the PEM into the trust directory and prints its fingerprint so you can check it out of band

**Arguments**

| Argument | Description |
|---|---|
| `<PATH>` | Path to the CA root PEM — the PUBLIC key, never a private key |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--force` | Replace an existing root with the same id |  |

## `goat ca rm`

Stop trusting a CA root, by the id `goat ca ls` shows

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` |  |

## `goat federate`

Federate with a partner: our half, their half, the agreement, and the configuration it implies (docs/design/federation-package-spec.md).

Two parties each publish one signed package, both compose the pair independently, and both compare the resulting digest. Equal digests prove both sides hold the identical agreement — and that comparison may travel by any channel at all, because altering a digest in transit causes a visible mismatch and causing a false match would mean forging a signature.

`lint`, `compose` and `diff` are pure: they read the files you name and nothing else, hold no credential and open no socket, so a partner can trust that handing you their package cannot touch anything. `apply` is the only verb that writes.

When two digests disagree, the verb you want is `diff`.

**Subcommands**

- [`goat federate init`](#goat-federate-init) — Generate our half of a federation, read from the running system
- [`goat federate lint`](#goat-federate-lint) — Check one package: is it well-formed, and does it agree with itself?
- [`goat federate compose`](#goat-federate-compose) — Combine two packages into one agreement and print its digest
- [`goat federate diff`](#goat-federate-diff) — Two digests disagree? Run this. It names the field
- [`goat federate verify`](#goat-federate-verify) — Could we actually deliver this agreement? Dry-run it here
- [`goat federate apply`](#goat-federate-apply) — Render the access rules and configuration this agreement implies

## `goat federate init`

Generate our half of a federation, read from the running system.

Every capability in the package is MEASURED here, not typed: what transports are installed and configured, what image the router is actually running, what version the fabric is. A capability an operator types is a claim, and a wrong claim stays wrong until the link fails in front of a partner.

Where a value cannot be derived this fails and names it. It never prompts and never fills one in — a silent default is how a link ends up with no rate limit.

Exit 23 means something could not be derived; the list is the output.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--with-transport` | Also derive `transport_offer` — that we are willing to connect |  |
| `--with-data` | Also derive `data_offer` — that we are willing to exchange |  |
| `--signing-key <SIGNING_KEY>` | Our Ed25519 signing key: 64 hex characters (the RFC 8032 seed) or 44 base64 characters. Without it this prints the exact bytes to sign and stops — it will not invent a key |  |
| `--out <OUT>` | Write the package's canonical bytes here |  |
| `--json` | One JSON object instead of the glyph lines |  |
| `--declaration <DECLARATION>` | Read the declaration from this file instead of federation.toml in the context directory: a peering's own federation.toml, which goatbuilder generates from the environment's. Identity is still read from the context directory |  |

## `goat federate lint`

Check one package: is it well-formed, and does it agree with itself?

Runs the structural rules on a single file — required members, integers that are really integers, no duplicate member names, complete blocks, a signature of the right shape. It does NOT verify the signature: the verifying key is bound to a party over a channel authenticated independently of this link, so a lone package cannot be authenticated by looking at it. `compose` does that.

Reads one file and nothing else. No network, no credentials.

Exit 21 means the package is refused, and the finding names the rule.

**Arguments**

| Argument | Description |
|---|---|
| `<PACKAGE>` | The package to check |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | One JSON object instead of the glyph lines |  |

## `goat federate compose`

Combine two packages into one agreement and print its digest.

Both parties compose independently and compare the digest. Equal digests prove both sides hold the identical agreement — that is the whole point of the format, and the comparison may travel by any channel at all, including one nobody trusts.

A refusal is an outcome, not a failure: it is the mechanism working. Exit 20, with the rule that fired.

Reads two files and the keys you pass. No network, no credentials, no host access — this runs in an empty container, which is what lets a partner trust it with their package.

**Arguments**

| Argument | Description |
|---|---|
| `<FIRST>` | One party's package |
| `<SECOND>` | The other party's package. Order does not matter: the agreement has no "us" and "them", so both sides compute the same bytes whichever way round they pass the files |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--key <KEYS>` | A verifying key bound to a party, as `<sovereign_id>=<44 base64 characters>`. Repeat for each party |  |
| `--keyring <KEYRING>` | A JSON file of bound keys, `{"<sovereign_id>": "<base64>"}`. Convenient for several partners; it establishes nothing that --key does not |  |
| `--now <NOW>` | The evaluation date, `YYYY-MM-DD`. Defaults to today in UTC — never the local date, which would let two parties either side of local midnight reach different verdicts on the day a package falls due |  |
| `--out <OUT>` | Write the agreement's canonical bytes here — no trailing newline, no byte-order mark, because that is the digest input |  |
| `--print` | Print the agreement's canonical form on stdout |  |
| `--json` | One JSON object instead of the glyph lines |  |

## `goat federate diff`

Two digests disagree? Run this. It names the field.

This is the verb for the one loud failure this design has. Two parties compose the same pair, compare digests, and they differ — so each side prints its agreement and this diffs them, and the offending field names itself. Loud in the first hour instead of silent for three weeks.

It also answers the case that reads as impossible: the digests differ and the two agreements are identical. There is then no field to show, and the instinct is to re-run everything. The answer is one measurement — a trailing newline, a byte-order mark, or an encoding difference — and this prints which of the three it is.

Exit 22 when they differ, 0 when they match.

**Arguments**

| Argument | Description |
|---|---|
| `<OURS>` | The agreement we composed — the exact bytes we hashed |
| `<THEIRS>` | The agreement they composed |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | One JSON object instead of the glyph lines |  |

## `goat federate verify`

Could we actually deliver this agreement? Dry-run it here.

Agreeing is not the same as being able to deliver. This runs the agreement through our own configuration generator WITHOUT applying it and asserts the output would be valid on this system, then prints the conformance attestation to publish: one line over the digest, signed with the same key that signed our package.

Reads local state. Changes nothing.

Exit 24 means we could not deliver it, and the failing check says why.

**Arguments**

| Argument | Description |
|---|---|
| `<AGREEMENT>` | The composed agreement, as the bytes that were hashed |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--as <AS_PARTY>` | Which party we are. Read from the declaration file when omitted |  |
| `--signing-key <SIGNING_KEY>` | Sign the attestation with this Ed25519 key — the same key that signed our package (§10). Without it the line is printed unsigned and says so |  |
| `--json` | One JSON object instead of the glyph lines |  |

## `goat federate apply`

Render the access rules and configuration this agreement implies.

The only verb here that writes. The agreement is the sole input, so the release set we forward and the prefix the far side admits cannot drift apart by being typed twice.

Prints what it would write and stops. Pass --confirm to write.

**Arguments**

| Argument | Description |
|---|---|
| `<AGREEMENT>` | The composed agreement |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--as <AS_PARTY>` | Which party we are. Read from the declaration file when omitted |  |
| `--out <OUT>` | Where to write. Files land under this directory, mirroring the repo paths they belong at |  |
| `--edge-group <EDGE_GROUP>` | The ansible group the edge runs in. The generated group_vars land at `group_vars/<GROUP>/`, and `--limit <GROUP>` is what deploys them |  |
| `--partner-audience <PARTNER_AUDIENCE>` | Release-sets this partner holds an audience slot in, so the edge grants it a READ of `keys/<set>/**` and it can obtain a data key (ADR 1169 decision 3). Repeatable |  |
| `--confirm` | Write. Without this, the rendering is printed and nothing is touched |  |
| `--json` | One JSON object instead of the glyph lines |  |

## `goat update`

Update the running `goat` binary from this goatnet's download surface (get.<site>). Verifies the published checksum, then atomically replaces the installed binary. Per-goatnet: uses the active context's surface. See `--dry-run` / `--force`

**Options**

| Flag | Description | Default |
|---|---|---|
| `--force` | Re-install even if already at the advertised-latest version (repairs a corrupt binary; re-verifies the checksum either way) |  |
| `--dry-run` | Resolve + report what would happen, but don't touch the installed binary |  |

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

## `goat acl`

Mesh ACL assertions — check what the declared ACL must and must not permit, WITHOUT applying anything.

`goat acl test [scope]` reads the ACL declaration compiled into this binary and runs its `netbird_acl_tests` block through the same evaluator the PR gate and netvis `/assertions` use. No network, no context, no credentials — a check that needs a working profile is a check people skip. A scope, or a whole declaration, with no assertions fails: nothing was checked. `--none-expected` accepts that outcome when it is intended.

**Subcommands**

- [`goat acl test`](#goat-acl-test) — Check the deployment's ACL declaration. Violations and undetermined results fail. The declaration is read from --declaration, else GOAT_ACL_DECLARATION_DIR, else the active context (its `acl_declaration_dir`, or its `acl/` directory), else the checkout the working directory is in

## `goat acl test`

Check the deployment's ACL declaration. Violations and undetermined results fail. The declaration is read from --declaration, else GOAT_ACL_DECLARATION_DIR, else the active context (its `acl_declaration_dir`, or its `acl/` directory), else the checkout the working directory is in

**Arguments**

| Argument | Description |
|---|---|
| `<SCOPE>` | Scope to check. Omit to check every declared scope |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--declaration <DECLARATION>` | Directory holding netbird_acl_taxonomy.yml and netbird_acl_matrix.yml |  |
| `--json` | Emit the full report as JSON |  |
| `--verbose` | Include satisfied assertions in the text report |  |
| `--none-expected` | Accept a check in which no assertion exists. Without it, a scope (or a whole declaration) that declares no assertions fails: nothing was checked, and an empty check is not a pass |  |

## `goat warrant`

Least-privilege network authorizations (ADR 1129).

`goat warrant why-blocked` answers "why can't A reach B on port P" from the declaration in git — no network, no credentials. Most such reports are DNS, membership or a dead service rather than an ACL gap; when it IS a gap it prints the minimal port-set that closes it and the diff to put in a PR. `goat warrant apply --membership` converges a governed scope's peer membership to `ops/warrant/membership/<scope>.yaml`.

**Subcommands**

- [`goat warrant why-blocked`](#goat-warrant-why-blocked) — Why can't this source reach that destination on that port?
- [`goat warrant port-sets`](#goat-warrant-port-sets) — The port-set vocabulary: seven names, what each is for, who owns it
- [`goat warrant apply`](#goat-warrant-apply) — Converge a scope's live peer membership to its declaration in git (the directory `ops/warrant/membership/<scope>/`) — report by default
- [`goat warrant pause`](#goat-warrant-pause) — Hold a scope's membership converger in report mode for a while — the audited, expiring way to make an emergency membership edit (docs/design/mesh-change-control.md §2)
- [`goat warrant resume`](#goat-warrant-resume) — End a `goat warrant pause` early: removes the pause file and records one `warrant.membership.resume` operator-action event

## `goat warrant why-blocked`

Why can't this source reach that destination on that port?

**Options**

| Flag | Description | Default |
|---|---|---|
| `--scope <SCOPE>` | Scope — a site (`zagros`, `zagros-prod`) or an exercise scope |  |
| `--from <FROM>` | Source group. Bare suffix (`DEVELOPER`) or fully qualified |  |
| `--to <TO>` | Destination group. Bare suffix (`INFRA`) or fully qualified |  |
| `--port <PORT>` | Destination port |  |
| `--proto <PROTO>` | `tcp` \| `udp` \| `icmp` \| `any` | `tcp` |
| `--json` |  |  |
| `--declaration <DECLARATION>` | Directory holding the goatnet's netbird_acl_taxonomy.yml and netbird_acl_matrix.yml. Omitted: GOAT_ACL_DECLARATION_DIR, the active context, then the checkout (see `goat acl test --help`) |  |

## `goat warrant port-sets`

The port-set vocabulary: seven names, what each is for, who owns it

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` |  |  |

## `goat warrant apply`

Converge a scope's live peer membership to its declaration in git (the directory `ops/warrant/membership/<scope>/`) — report by default.

Reads peers, groups, policies, setup keys and the activity log from the NetBird management API and computes, per peer, the managed groups it should hold (docs/design/mesh-change-control.md §4.2). `--mode enforce` adds before it removes and re-reads to verify; the file's `converge:` is the ceiling. Token: $GOAT_NB_API_TOKEN or $<SCOPE>_NB_API_TOKEN.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--membership` | Converge peer membership (the only kind `apply` does today) |  |
| `--scope <SCOPE>` | Taxonomy scope, e.g. `zagros-dev`. Must equal `_scope.yaml`'s `scope:` |  |
| `--file <FILE>` | The declaration DIRECTORY, e.g. `ops/warrant/membership/zagros-dev` (`_scope.yaml`, fragments, `_baseline.generated.yaml`). `--dir` is an alias |  |
| `--acl-declaration <ACL_DECLARATION>` | Directory holding the goatnet's netbird_acl_taxonomy.yml and netbird_acl_matrix.yml, read to check the scope's prefix. Omitted: GOAT_ACL_DECLARATION_DIR, the active context, then the checkout |  |
| `--mode <MODE>` | `report` changes nothing. `enforce` converges, and is refused when `_scope.yaml` says `converge: report` | `report` |
| `--max-changes <MAX_CHANGES>` | Refuse the whole run, changing nothing, if it would make more peer-group changes than this | `10` |
| `--json` | Print one stable JSON object instead of the table |  |
| `--textfile-dir <TEXTFILE_DIR>` | Write `goat_warrant_membership.prom` here (node_exporter textfile dir) |  |
| `--mgmt-url <MGMT_URL>` | NetBird management URL (else $GOAT_NB_MGMT_URL) |  |
| `--state-dir <STATE_DIR>` | Where `goat warrant pause` writes `pause-<scope>.json`. While a pause is in force the run is report-only, whatever `--mode` says | `/var/lib/goat-warrant` |

## `goat warrant pause`

Hold a scope's membership converger in report mode for a while — the audited, expiring way to make an emergency membership edit (docs/design/mesh-change-control.md §2).

Writes `<state-dir>/pause-<scope>.json`; run it on the scope's management host, as the user the converger runs as (root). At most 24h, a reason is required, and one `warrant.membership.pause` operator-action event is recorded. It ends on its own.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--scope <SCOPE>` | Taxonomy scope whose converger to pause, e.g. `zagros-dev` |  |
| `--for <DURATION>` | How long: `30m`, `1h`, `90s`, `2h30m`. At most 24h |  |
| `--reason <REASON>` | Why — recorded in the pause file and the operator-action event |  |
| `--state-dir <STATE_DIR>` | The converger's state directory | `/var/lib/goat-warrant` |

## `goat warrant resume`

End a `goat warrant pause` early: removes the pause file and records one `warrant.membership.resume` operator-action event

**Options**

| Flag | Description | Default |
|---|---|---|
| `--scope <SCOPE>` | Taxonomy scope whose pause to end, e.g. `zagros-dev` |  |
| `--state-dir <STATE_DIR>` | The converger's state directory | `/var/lib/goat-warrant` |

## `goat access`

Data-Access COP: who can access which data, and why

**Subcommands**

- [`goat access who`](#goat-access-who) — Forward query: what may this identity write and read, with the reason at each layer. An org may ask about its own prefix; an operator may ask about any identity
- [`goat access topic`](#goat-access-topic) — Reverse query: who may write and read this topic, and which partners may receive it if released. Operator-only
- [`goat access govern`](#goat-access-govern) — Who administers an org's data governance, nominate an admin, or remove one (ADR 1102 / U9). An org starts in BOOTSTRAP (every member governs) and NARROWS itself with its first nomination — from then on only nominated admins govern, and the last one cannot be removed
- [`goat access key`](#goat-access-key) — Grant-encryption leaf-key operations. With no subcommand: is this anchor's decrypt key published to the recipient-cert store (U7 publish-my-key)? Defaults to the anchor in your own profile cert; a miss names the remedy. The `rotate` subcommand rotates the member leaf key used for grant-encryption unwrapping (a convenience over the grant-crypto SDK's `rotate`; distinct from `goat admin rotate` = org cert reissue)
- [`goat access why`](#goat-access-why) — Trace one allow/deny decision for `<id>` against `<key>` across mesh → identity → Zenoh ACL → release
- [`goat access strays`](#goat-access-strays) — Collect every stray (live, no declared match) and bucket it by WHY it didn't reconcile — the safe first step of a stray sweep. Read-only: builds the cut-candidate + verify-first lists; it never cuts. The cut stays the audited `goat admin onboard --revoke`, one at a time
- [`goat access probation`](#goat-access-probation) — Anomaly probation (ADR 1068): hold something that doesn't reconcile instead of leaving or deleting it — verify, then repair or purge. The hold/decide are recorded + audited; the actual repair/purge runs through the existing `goat admin` paths. Operator-only
- [`goat access repo`](#goat-access-repo) — Per-onboardee GitHub repo-access audibles (docs/design/onboarding-github-repo-grants.md §2): preview the effective set (on-rails policy ⊕ overrides) for a proposal, or add/remove a repo override on top of the policy at review time
- [`goat access grant`](#goat-access-grant) — Preemptively GRANT a producer's data to a grantee (ADR 1075 §12) — the mutation half of the access language. Operator or the producer's custodian only; the same grant row the request-approve path writes
- [`goat access audience`](#goat-access-audience) — AUDIENCE — who can actually read a producer's stream: the machine anchors its grant-keyed DEK is wrapped to (the derived audience, WS4). Shows the members plus any honesty notes (browser-only readers, grantee-less orgs)
- [`goat access revoke`](#goat-access-revoke) — REVOKE a grant by id (ADR 1031 reversibility) — end-dates it, keeping the row for audit history. Operator or the producer's custodian only
- [`goat access co-publish`](#goat-access-co-publish) — Co-publish onto multi-publisher topics (ADR 1105, PUBSUB-ORG-ADMIN): single-owner roots are inviolable; create a marked multi-publisher topic and add your own org's publishers to it

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

## `goat access govern`

Who administers an org's data governance, nominate an admin, or remove one (ADR 1102 / U9). An org starts in BOOTSTRAP (every member governs) and NARROWS itself with its first nomination — from then on only nominated admins govern, and the last one cannot be removed

**Subcommands**

- [`goat access govern list`](#goat-access-govern-list) — List an org's governance admins and which mode it is in (bootstrap vs nominated/narrowed)
- [`goat access govern nominate`](#goat-access-govern-nominate) — Nominate a governance admin (at-most-own-scope; nominee must be an active member of the org). THE FIRST NOMINATION NARROWS THE ORG
- [`goat access govern remove`](#goat-access-govern-remove) — Remove a nomination by id (end-dated, kept for audit). Removing the org's LAST active admin is refused — nominate a replacement first

## `goat access govern list`

List an org's governance admins and which mode it is in (bootstrap vs nominated/narrowed)

**Arguments**

| Argument | Description |
|---|---|
| `<ORG>` | Org id (kebab prefix) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--all` | Include removed (end-dated) rows — the audit history |  |
| `--json` | Emit the raw JSON |  |

## `goat access govern nominate`

Nominate a governance admin (at-most-own-scope; nominee must be an active member of the org). THE FIRST NOMINATION NARROWS THE ORG

**Arguments**

| Argument | Description |
|---|---|
| `<ORG>` | Org id |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--email <EMAIL>` | Nominee: their login email, or an identity UUID via --identity |  |
| `--identity <IDENTITY>` | Nominee identity UUID (alternative to --email) |  |
| `--justification <JUSTIFICATION>` | The recorded why | `` |
| `--json` | Emit the raw JSON |  |

## `goat access govern remove`

Remove a nomination by id (end-dated, kept for audit). Removing the org's LAST active admin is refused — nominate a replacement first

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Nomination id (gadm-…), from `goat access govern list` |

## `goat access key`

Grant-encryption leaf-key operations. With no subcommand: is this anchor's decrypt key published to the recipient-cert store (U7 publish-my-key)? Defaults to the anchor in your own profile cert; a miss names the remedy. The `rotate` subcommand rotates the member leaf key used for grant-encryption unwrapping (a convenience over the grant-crypto SDK's `rotate`; distinct from `goat admin rotate` = org cert reissue)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--anchor <ANCHOR>` | (status) Anchor to check (default: your own profile cert's anchor) |  |
| `--pem` | (status) Print the full PEM instead of the summary line |  |

**Subcommands**

- [`goat access key rotate`](#goat-access-key-rotate) — Rotate this member's grant-encryption LEAF KEY — the key used to unwrap the DEKs that open sealed streams (`goat sub --open`). A convenience over the grant-crypto SDK's `rotate` (ADR 1106); it computes the rotation locally (the platform holds no keys and never re-wraps) and prints a structured RotationReceipt

## `goat access key rotate`

Rotate this member's grant-encryption LEAF KEY — the key used to unwrap the DEKs that open sealed streams (`goat sub --open`). A convenience over the grant-crypto SDK's `rotate` (ADR 1106); it computes the rotation locally (the platform holds no keys and never re-wraps) and prints a structured RotationReceipt.

Two policies: `--destroy` (DEFAULT) drops the retiring key and re-wraps nothing — full forward secrecy for pre-rotation generations; `--keep-access` re-wraps this member's still-live DEKs to the new key, which REDUCES forward secrecy for those generations.

This is DISTINCT from `goat admin rotate` (org cert reissue). Because a member's grant-encryption leaf key IS its mTLS profile key — whose re-issue is a CA operation (ADR 1044) — this verb NEVER overwrites your profile: it writes the freshly generated key to `--out` for you to install via the CA-coupled leaf re-issue so your cert continues to match. Until then your live key is unchanged.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--release-set <RELEASE_SET>` | Release-set (producer anchor) whose visible generations the rotation is computed against. Default: your own producer anchor |  |
| `--keep-access` | Re-wrap this member's still-live DEKs to the new key before retiring the old one. REDUCES forward secrecy for those generations. Mutually exclusive with `--destroy` |  |
| `--destroy` | Drop the retiring key, re-wrap nothing (the DEFAULT — full forward secrecy). Explicit here only for symmetry / clarity |  |
| `--out <OUT>` | Write the freshly generated new private key (PKCS#8 PEM) here. Never your profile key — installing it is the CA-coupled leaf re-issue |  |
| `--rewrap-out <REWRAP_OUT>` | (with --keep-access) Write the re-wrapped DEK sidecar (JSON) here. Default: `<out>.rewrap.json`. These stay LOCAL — they are not published to the shared `keys/**` bundle tree |  |
| `--key-service-bundle <KEY_SERVICE_BUNDLE>` | The key service's SPIRE trust bundle PEM (fetch/verify the bundles) |  |
| `--trust-domain <TRUST_DOMAIN>` | The goatnet's SPIFFE trust domain (e.g. zagros.zagros.example) |  |
| `--key-service-id <KEY_SERVICE_ID>` | The key service's SPIFFE id (default spiffe://<trust-domain>/grant-keyd) |  |
| `--json` | Emit the raw JSON (the receipt plus the emitted paths) |  |

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

## `goat access repo`

Per-onboardee GitHub repo-access audibles (docs/design/onboarding-github-repo-grants.md §2): preview the effective set (on-rails policy ⊕ overrides) for a proposal, or add/remove a repo override on top of the policy at review time

**Subcommands**

- [`goat access repo policy`](#goat-access-repo-policy) — Show the loaded on-rails repo-access policy (default owner + rules) — the default set a proposal's effective grants start from
- [`goat access repo preview`](#goat-access-repo-preview) — Preview the effective repo-access set (policy ⊕ overrides) for a proposal, each repo tagged with its source (policy / audible)
- [`goat access repo add`](#goat-access-repo-add) — Add a repo override onto a proposal (or raise its permission). GETs the current overrides, applies this one add, POSTs them back
- [`goat access repo remove`](#goat-access-repo-remove) — Remove a repo override from a proposal (drops it from the effective set — wins over the policy AND any add). GETs the current overrides, applies this one remove, POSTs them back

## `goat access repo policy`

Show the loaded on-rails repo-access policy (default owner + rules) — the default set a proposal's effective grants start from

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit the raw JSON |  |

## `goat access repo preview`

Preview the effective repo-access set (policy ⊕ overrides) for a proposal, each repo tagged with its source (policy / audible)

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Proposal id (`prop-…`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit the raw JSON |  |

## `goat access repo add`

Add a repo override onto a proposal (or raise its permission). GETs the current overrides, applies this one add, POSTs them back

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Proposal id (`prop-…`) |
| `<REPO>` | Repo as `owner/name` (or a bare `name` for the policy default owner), optionally `owner/name:perm` (perm ∈ pull\|triage\|push\|maintain\|admin; default pull) |

## `goat access repo remove`

Remove a repo override from a proposal (drops it from the effective set — wins over the policy AND any add). GETs the current overrides, applies this one remove, POSTs them back

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Proposal id (`prop-…`) |
| `<REPO>` | Repo as `owner/name` (or a bare `name`) |

## `goat access grant`

Preemptively GRANT a producer's data to a grantee (ADR 1075 §12) — the mutation half of the access language. Operator or the producer's custodian only; the same grant row the request-approve path writes

**Arguments**

| Argument | Description |
|---|---|
| `<PRODUCER>` | Producer key whose data is being shared (the write anchor / org prefix) |
| `<GRANTEE>` | Grantee: an org prefix (default) or a principal id with `--principal` |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--principal` | Grant to a principal (person/machine id) instead of an org |  |
| `--justification <JUSTIFICATION>` | Why — recorded on the grant row (audited) |  |
| `--expires <EXPIRES>` | Optional expiry as a unix timestamp (seconds); omit for open-ended |  |
| `--json` | Emit the raw JSON |  |

## `goat access audience`

AUDIENCE — who can actually read a producer's stream: the machine anchors its grant-keyed DEK is wrapped to (the derived audience, WS4). Shows the members plus any honesty notes (browser-only readers, grantee-less orgs)

**Arguments**

| Argument | Description |
|---|---|
| `<PRODUCER>` | Producer key / release-set (rs ≡ producer) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit the raw JSON |  |

## `goat access revoke`

REVOKE a grant by id (ADR 1031 reversibility) — end-dates it, keeping the row for audit history. Operator or the producer's custodian only

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Grant id (from `goat access who` / the grant row) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--confirm <CONFIRM>` | Confirm on the fabric that the revoke took effect, by reading the release-set's key bundle back and reporting who can still decrypt. Revoking end-dates a grant row; the read cut only lands once the key service publishes a new generation excluding the member, and a portal call returning success says nothing about that |  |
| `--expect-excluded <EXPECT_EXCLUDED>` | With `--confirm`, assert this member anchor holds no slot in the new generation. Without it the command reports the resulting audience but asserts nothing about who left |  |
| `--confirm-timeout-secs <CONFIRM_TIMEOUT_SECS>` | How long to wait for the new generation (default 90s — three of the key service's 30s poll intervals) | `90` |
| `--key-service-bundle <KEY_SERVICE_BUNDLE>` | Signer-trust inputs for the bundle verification, as `goat pub --sealed` |  |
| `--trust-domain <TRUST_DOMAIN>` |  |  |
| `--key-service-id <KEY_SERVICE_ID>` |  |  |
| `--json` | Emit the raw JSON |  |

## `goat access co-publish`

Co-publish onto multi-publisher topics (ADR 1105, PUBSUB-ORG-ADMIN): single-owner roots are inviolable; create a marked multi-publisher topic and add your own org's publishers to it

**Subcommands**

- [`goat access co-publish create-topic`](#goat-access-co-publish-create-topic) — CREATE a multi-publisher topic for an org (a deliberate act). Single-owner roots are never involved — this is a separate, marked class
- [`goat access co-publish list-topics`](#goat-access-co-publish-list-topics) — LIST/discover multi-publisher topics (the MULTI-PUBLISHER marker, owning org, and declared publishers)
- [`goat access co-publish grant`](#goat-access-co-publish-grant) — ADD a publisher to a multi-publisher topic (at-most-own-org, enforced server-side)
- [`goat access co-publish revoke`](#goat-access-co-publish-revoke) — REVOKE a co-publish grant by id
- [`goat access co-publish export`](#goat-access-co-publish-export) — EXPORT the materialized zenoh_copublish_grants group_var (operator-only)

## `goat access co-publish create-topic`

CREATE a multi-publisher topic for an org (a deliberate act). Single-owner roots are never involved — this is a separate, marked class

**Arguments**

| Argument | Description |
|---|---|
| `<ORG>` | Owning org id (the naming org — you must govern it) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--name <NAME>` | Human display name (what a discoverer sees) |  |
| `--justification <JUSTIFICATION>` | Why (recorded, audited) |  |
| `--no-persist` | Routed-only: do NOT render a history store for it |  |
| `--json` |  |  |

## `goat access co-publish list-topics`

LIST/discover multi-publisher topics (the MULTI-PUBLISHER marker, owning org, and declared publishers)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--org <ORG>` | Narrow to one org |  |
| `--json` |  |  |

## `goat access co-publish grant`

ADD a publisher to a multi-publisher topic (at-most-own-org, enforced server-side)

**Arguments**

| Argument | Description |
|---|---|
| `<WRITER>` | The publisher's anchor (grantee) — the slot uuid that gains write |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--to <MULTIPUB_TOPIC>` | The target REGISTERED multi-publisher topic id (from `list-topics`) |  |
| `--justification <JUSTIFICATION>` | Why (required, audited) |  |
| `--expires <EXPIRES>` | Optional expiry as a unix timestamp (seconds) |  |
| `--json` |  |  |

## `goat access co-publish revoke`

REVOKE a co-publish grant by id

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Grant id |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` |  |  |

## `goat access co-publish export`

EXPORT the materialized zenoh_copublish_grants group_var (operator-only)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit the raw JSON (the export is always JSON) |  |

## `goat machine`

Enroll and manage machines (system principals) against a custodian's enrollment grant (ADR 1096). `machine enroll <handle> --node-class …`

**Subcommands**

- [`goat machine affiliation`](#goat-machine-affiliation) — Preview or repair verified legacy affiliation spellings. Operator-only
- [`goat machine enroll`](#goat-machine-enroll) — Enroll one machine (system principal) against a custodian's grant: mints its OWN slot + cert + api_key and records the person→system custody edge. Gear node-classes (ADR 1116) are custodian self-service; infrastructure classes are operator-only (the server enforces both). With `--bulk <file>`, N machines under ONE custodian in one act
- [`goat machine mesh-key`](#goat-machine-mesh-key) — Re-mint the one-off mesh setup key for an ALREADY-ENROLLED machine — quota-neutral (the enrollment already consumed its unit; a join window is not a second machine). The recovery when the enroll-time key expired before the box joined: groups are re-derived from the machine's BORN-AS node-class, nothing else changes
- [`goat machine revoke`](#goat-machine-revoke) — Retire an enrolled machine: cut its peer and any UNCONSUMED setup key from the mesh, then free its slot and delete its anchor row. Mesh-first (ADR 1031) — if the mesh cut fails nothing is changed, so a retry is a retry. Quota returns by construction. Re-enrolling the handle later mints a NEW anchor; the retired cert is not restorable
- [`goat machine ls`](#goat-machine-ls) — List enrolled machines and their custody — a custodian's fleet, or (as an operator, with no `--custodian`) every enrolled machine. A non-operator always sees only its own machines
- [`goat machine ssh`](#goat-machine-ssh) — NetBird SSH on an enrolled machine's mesh peer (ADR 1216 `device-admin`, tcp 22022). Needs the NetBird admin token; recorded in the operator-action audit
- [`goat machine grant`](#goat-machine-grant) — Set (create or adjust) a custodian's machine enrollment quota. Operator-only. This is the management path for the enrollment grant that onboarding seeds: `goat machine grant <custodian> --quota 5`. Include `--affiliation` the FIRST time a custodian is granted (a machine inherits its custodian's unit); omit it to only change the quota of a grant that already exists

## `goat machine affiliation`

Preview or repair verified legacy affiliation spellings. Operator-only

**Subcommands**

- [`goat machine affiliation repair`](#goat-machine-affiliation-repair) — Preview exact legacy records, then apply the reviewed revision atomically. Only use after verifying that all selected spellings describe one organization

## `goat machine affiliation repair`

Preview exact legacy records, then apply the reviewed revision atomically. Only use after verifying that all selected spellings describe one organization

**Options**

| Flag | Description | Default |
|---|---|---|
| `--from <FROM>` | Exact stored spelling to repair. Repeat for each approved legacy spelling |  |
| `--to <TO>` | Approved spelling to store on the selected grants and slots |  |
| `--reason <REASON>` | Why these records are known to represent one organization (recorded in audit) |  |
| `--apply` | Apply instead of previewing. Requires the revision from a reviewed preview |  |
| `--expected-revision <EXPECTED_REVISION>` | Revision printed by preview. A changed group is refused without updates |  |
| `--json` | Emit the server's structured preview or application receipt |  |

## `goat machine enroll`

Enroll one machine (system principal) against a custodian's grant: mints its OWN slot + cert + api_key and records the person→system custody edge. Gear node-classes (ADR 1116) are custodian self-service; infrastructure classes are operator-only (the server enforces both). With `--bulk <file>`, N machines under ONE custodian in one act

**Arguments**

| Argument | Description |
|---|---|
| `<HANDLE>` | The machine's handle — its slot alias (lowercase `[a-z0-9-]`). Omitted in `--bulk` mode (each spec carries its own) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--node-class <NODE_CLASS>` | Node-class — the function the box performs (ADR 1116), or a standalone appliance boundary; enroll once per function. Custodian self-service: SENSOR-FEED / SENSOR-TELEMETRY / SENSOR-ARBITER, EFFECTOR-CUE / EFFECTOR-TRACK / EFFECTOR-TELEMETRY / EFFECTOR-ARBITER, POD / MOON / KID / EXTERNAL-KID / EXTERNAL-INTERMEDIARY, FUSION, CONTROL; or PLATFORM, a composition that needs `--workloads`. NETWORK (bearer gear — routers, switches, radios) and EDGE-SERVER (a server that supports edge devices): operator, or a custodian holding the network-admin entitlement for its own site (ADR 1113, ADR 1216). Operator-only infrastructure: MGMT-HOST / ZENOH / SNITCH / SERVICES / BREAKGLASS / NETWORK-JUMP. Refused at the mint: SENSOR and EFFECTOR (reserved), SENSOR-TASKING and EFFECTOR-COMMAND (intake-only, never a primary class), PROCESSOR (legacy, no mesh group — use FUSION). Case-insensitive; normalized to uppercase |  |
| `--autonomy <AUTONOMY>` | Autonomy: manual / remote / supervised / autonomous. Case-insensitive | `manual` |
| `--custodian <CUSTODIAN>` | Custodian handle. Omit for self-service (your own identity is used); required when an operator enrolls on someone's behalf, or for any infrastructure node-class. In `--bulk` mode it names the ONE custodian the whole act draws down |  |
| `--display-name <DISPLAY_NAME>` | Optional display name (defaults to the handle) |  |
| `--workloads <WORKLOADS>` | PLATFORM sub-workload composition: a comma-separated list of the composable gear functions the box performs (e.g. `effector-cue,effector-arbiter`); its mesh groups are their union. POD and MOON are standalone classes, not PLATFORM workloads. Valid only when `--node-class PLATFORM`. Case-insensitive |  |
| `--bulk <BULK>` | Bulk mode: read a YAML or JSON LIST of machine specs (`machine_handle`, `node_class`, `autonomy`, optional `display_name` / `workloads`) from a file — or stdin with `-` — and enroll them all in one act (`POST /api/machines/enroll-bulk`): N independent mints, one custodian, one single-use mesh key each |  |
| `--exercise <EXERCISE>` | Scope every minted MESH KEY to an exercise's synthetic-site groups (`EX-<PREFIX>-<class>`) instead of the goatnet site scope. The name must be in the portal's exercise map; identity is untouched — the exercise is a key scope, not a cert attribute |  |
| `--json` | Emit the raw JSON bundle instead of the human summary |  |

## `goat machine mesh-key`

Re-mint the one-off mesh setup key for an ALREADY-ENROLLED machine — quota-neutral (the enrollment already consumed its unit; a join window is not a second machine). The recovery when the enroll-time key expired before the box joined: groups are re-derived from the machine's BORN-AS node-class, nothing else changes

**Arguments**

| Argument | Description |
|---|---|
| `<MACHINE>` | The machine: its 32-hex slot anchor, or its handle (resolved via the machine list; an ambiguous handle is refused — pass the anchor) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--exercise <EXERCISE>` | Scope the key to an exercise's synthetic-site groups instead of the goatnet site scope (portal exercise map; same contract as enroll) |  |
| `--json` | Emit the raw JSON |  |

## `goat machine revoke`

Retire an enrolled machine: cut its peer and any UNCONSUMED setup key from the mesh, then free its slot and delete its anchor row. Mesh-first (ADR 1031) — if the mesh cut fails nothing is changed, so a retry is a retry. Quota returns by construction. Re-enrolling the handle later mints a NEW anchor; the retired cert is not restorable

**Arguments**

| Argument | Description |
|---|---|
| `<MACHINE>` | The machine: its 32-hex slot anchor, or its handle (resolved via the machine list; an ambiguous handle is refused — pass the anchor) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--yes, -y` | Skip the confirmation prompt |  |
| `--json` | Emit the raw JSON |  |

## `goat machine ls`

List enrolled machines and their custody — a custodian's fleet, or (as an operator, with no `--custodian`) every enrolled machine. A non-operator always sees only its own machines

**Options**

| Flag | Description | Default |
|---|---|---|
| `--custodian <CUSTODIAN>` | Custodian handle to scope to. Omit as operator for the whole fleet |  |
| `--json` | Emit the raw JSON |  |

## `goat machine ssh`

NetBird SSH on an enrolled machine's mesh peer (ADR 1216 `device-admin`, tcp 22022). Needs the NetBird admin token; recorded in the operator-action audit

**Subcommands**

- [`goat machine ssh enable`](#goat-machine-ssh-enable) — Turn on NetBird SSH (tcp 22022) on an enrolled machine's mesh peer. Reads the peer, then writes only `ssh_enabled`: every other setting is sent back as read. Needs the NetBird admin token (`<CONTEXT>_NB_API_TOKEN` or `GOAT_NB_API_TOKEN`). Recorded in the operator-action audit. The device's NetBird client must also allow an SSH server (`netbird up --allow-server-ssh` on the device)

## `goat machine ssh enable`

Turn on NetBird SSH (tcp 22022) on an enrolled machine's mesh peer. Reads the peer, then writes only `ssh_enabled`: every other setting is sent back as read. Needs the NetBird admin token (`<CONTEXT>_NB_API_TOKEN` or `GOAT_NB_API_TOKEN`). Recorded in the operator-action audit. The device's NetBird client must also allow an SSH server (`netbird up --allow-server-ssh` on the device)

**Arguments**

| Argument | Description |
|---|---|
| `<MACHINE>` | The machine: its handle or its 32-hex slot anchor (`goat machine ls`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--dry-run` | Resolve the machine and its peer and print the change, without writing |  |
| `--mgmt-url <MGMT_URL>` | Override the NetBird mgmt URL (else env / derived from the profile) |  |

## `goat machine grant`

Set (create or adjust) a custodian's machine enrollment quota. Operator-only. This is the management path for the enrollment grant that onboarding seeds: `goat machine grant <custodian> --quota 5`. Include `--affiliation` the FIRST time a custodian is granted (a machine inherits its custodian's unit); omit it to only change the quota of a grant that already exists

**Arguments**

| Argument | Description |
|---|---|
| `<CUSTODIAN>` | Custodian handle (or slot uuid) to grant quota to |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--quota <QUOTA>` | How many machines the custodian may have enrolled at once |  |
| `--affiliation <AFFILIATION>` | Affiliation/unit stamped on every machine the custodian enrolls — required to seed a brand-new custodian's grant; omit to adjust quota only |  |
| `--site <SITE>` | Optional site stamped on enrolled machines (only meaningful with --affiliation) |  |
| `--network-admin` | Grant (or, with --no-network-admin, revoke) the delegated network-admin entitlement: lets this custodian enroll NETWORK-class bearer gear (routers/switches/radios) for their OWN site without an operator (ADR 1113). Requires the grant to carry a site. Omit both flags to leave the entitlement exactly as it is |  |
| `--no-network-admin` | Revoke the delegated network-admin entitlement (ADR 1113) |  |
| `--json` | Emit the raw JSON |  |

## `goat audit`

Read the operator-action audit trail (`ops/audit/operator/**`) — the who-did-what record of backend admin actions, decrypted with your operator age identity

**Subcommands**

- [`goat audit emit`](#goat-audit-emit) — Emit ONE operator-action record on behalf of another surface
- [`goat audit ops`](#goat-audit-ops) — Read the operator-action audit trail (`ops/audit/operator/**`): query the rocksdb-backed store, decrypt each event with your operator age identity, and print a readable table. Samples not encrypted to your key render an `<undecryptable>` row rather than aborting the whole read

## `goat audit emit`

Emit ONE operator-action record on behalf of another surface.

This exists because the on-wire record is `age(protobuf(OperatorActionV1))` and this crate holds the only implementation of that encoding — the proto warns that any change must be mirrored field-for-field, tag-for-tag, or the wire breaks silently. A surface written in another language (herd, ADR 1131) would otherwise need a second copy of the format to be auditable at all, and two copies of a wire format is how one of them drifts.

`--actor` is the PERSON the action belongs to, NOT whoever runs this command. That is the whole point: a forwarder emitting herd's records must attribute them to the person who opened the session, or attribution collapses onto the forwarder and the audit answers nothing.

SAID PLAINLY: emitting on behalf of another surface is a TRUSTED operation. Anyone able to run this already holds a roster identity, and nothing here proves the named actor really did the thing. What the record does carry is `surface`, `host` and `src` — so a forged actor is still traceable to the box that emitted it. Do not extend this to accept a caller-supplied host or src.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--actor <ACTOR>` | The person this action belongs to (kebab operator handle) |  |
| `--surface <SURFACE>` | Emitting surface, e.g. `herd` |  |
| `--action <ACTION>` | Semantic verb, e.g. `gear.session.open` |  |
| `--target <TARGET>` | What was acted on — node id, host, topic | `` |
| `--env <ENV>` | goatnet the action landed in |  |
| `--outcome <OUTCOME>` | started \| ok \| failed |  |
| `--error <ERROR>` | Failure detail; use with `--outcome failed` |  |
| `--args-digest <ARGS_DIGEST>` | An ALREADY-REDACTED argument summary. Hashed before it goes on the wire; never pass raw secrets |  |

## `goat audit ops`

Read the operator-action audit trail (`ops/audit/operator/**`): query the rocksdb-backed store, decrypt each event with your operator age identity, and print a readable table. Samples not encrypted to your key render an `<undecryptable>` row rather than aborting the whole read

**Options**

| Flag | Description | Default |
|---|---|---|
| `--actor <ACTOR>` | Restrict to one actor (the `<actor>` sub-key). Default: all actors |  |
| `--watch` | Keep the subscription open and print events as they arrive (live tail) instead of a one-shot query of stored events |  |
| `--json` | Raw output: one JSON object per decoded event on stdout |  |

## `goat demo`

Run a fabric demo end-to-end: open the browser-side traffic generator and tail the resulting Zenoh traffic in this terminal. With no name it runs the demo service the goatnet publishes (`demo_service`)

**Arguments**

| Argument | Description |
|---|---|
| `<TARGET>` | The demo service to run: its host label and key prefix. Defaults to the goatnet's `demo_service` (written by `goat context refresh`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--url <URL>` | Override the demo page URL (e.g. http://localhost:8095 for a local run, or a different goatnet). The `/demo` path is appended if absent |  |
| `--site <SITE>` | Override the site_id used for the watch key `<name>/<site>/**`. Defaults to the site derived from the profile (or the --url host) |  |
| `--no-watch` | Open the browser only; don't tail the fabric in this terminal. Combined with --no-open it's a dry-run: print the resolved URL + watch command and exit without opening anything |  |
| `--no-open` | Tail the fabric only; don't open the browser |  |
| `--tui` | Use the interactive Tail lens (event table + rate sparkline) for the watch instead of the default line-oriented tail. Requires a terminal |  |

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
| `<KEY_PATTERN>` | Key expression to replay from the audit-tail (e.g. `acme/**`) |

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
| `--json` | One-shot only: print one JSON object for the beat — domain, scope, colour, age, sample count — or `"beat": false` with the reason when none arrived in the window. The machine contract the acceptance assay grades (docs/design/goatnet-acceptance-suite.md). Not for `--domain crypto`, whose beat has no colour |  |

## `goat flow`

Is data flowing, and when did it last? Per-topic freshness across whatever observers are reachable — panoscope, the audit-tail, and a bounded live listen — with every row labelled by the observer that answered it. Distinguishes "no observer could answer" from "an observer looked back over a stated horizon and found nothing", and never claims a topic has never published. A bare invocation is the one-frame scriptable snapshot; --json is the stable machine contract

**Arguments**

| Argument | Description |
|---|---|
| `<KEY_EXPR>` | Key expression to report on (e.g. `acme/**`). Default: every topic any engaged observer knows about |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--since <SINCE>` | Also read the audit-tail over this window: a duration (`90m`, `24h`, `7d`, `2h30m`) or an RFC3339 timestamp. This is the only tier that can prove a topic has NOT published — without it a silent topic reads `unknown`, not `no-record`. The portal serves at most 7 days and says so on the frame when it clamps you |  |
| `--live <LIVE>` | When to engage a bounded live subscribe as an observer | `auto` |
| `--live-window <LIVE_WINDOW>` | How long the live tier listens, in seconds | `3` |
| `--fresh <FRESH>` | A topic whose last publish is within this many seconds reads `flowing`; older reads `idle` | `60` |
| `--watch` | Repaint on an interval until Ctrl-C. On a terminal the frame repaints in place; under a pipe each frame is appended with a separator, so `goat flow --watch \| tee flow.log` keeps a history |  |
| `--interval <INTERVAL>` | Seconds between `--watch` repaints | `10` |
| `--json` | Machine output: one JSON object carrying the rows AND a `sources` array describing every tier, including the ones that were skipped — so a script can tell "the audit tail said no" from "the audit tail was never asked". The stable machine contract |  |
| `--snapshot-endpoint <SNAPSHOT_ENDPOINT>` | Override the panoscope snapshot endpoint. Default: derived from the profile's portal_endpoint (`portal.*` → `panoscope.*`), also honors `$GOAT_PANOSCOPE_ENDPOINT` |  |
| `--all` | Show every topic any engaged observer knows about — declared, idle, no-record, and unknown rows included. Default shows only what is actually `flowing` right now; the full honesty-graded universe (this flag) is what `--json` always carries regardless, so scripts never lose the distinction this flag hides from the human table |  |

## `goat names`

Name-fallback self-claims: publish this device's signed claim, or collect published claims for the get tier (ADR 1082 Amendment 2)

**Subcommands**

- [`goat names publish-claim`](#goat-names-publish-claim) — Sign and publish this device's own name→IP claim to the fabric
- [`goat names collect`](#goat-names-collect) — Collect published claims into a claims.json for the get tier
- [`goat names resolve`](#goat-names-resolve) — Resolve a mesh name through the fallback resolver and print the source (live / self-claim / snapshot / observed) — the diagnostic for the self-claims path

## `goat names publish-claim`

Sign and publish this device's own name→IP claim to the fabric

**Options**

| Flag | Description | Default |
|---|---|---|
| `--name <NAME>` | Claimed name. Default: <netbird peer name>.<snapshot zone> — the zone-shaped rendering of this device's mesh peer name |  |
| `--ip <IP>` | Claimed IP. Default: this device's netbird mesh IP |  |
| `--ttl <TTL>` | Claim TTL in seconds (claims exist for fast-moving IPs — keep short) | `3600` |
| `--out <OUT>` | Also write the envelope JSON to this file (manual courier / tests) |  |
| `--no-publish` | Build + print + (optionally) write the envelope without publishing |  |

## `goat names collect`

Collect published claims into a claims.json for the get tier

**Options**

| Flag | Description | Default |
|---|---|---|
| `--out <OUT>` | Output claims.json path (written atomically on every update) |  |
| `--listen <LISTEN>` | Listen window in seconds; 0 = follow until interrupted | `30` |

## `goat names resolve`

Resolve a mesh name through the fallback resolver and print the source (live / self-claim / snapshot / observed) — the diagnostic for the self-claims path

**Arguments**

| Argument | Description |
|---|---|
| `<NAME>` | The mesh hostname to resolve |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--port <PORT>` | Port to attach (cosmetic — resolution is name-only) | `443` |

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

## `goat microscope`

Per-entity behavior lens (ADR 1090): spine + capability + merged activity timeline + per-source honesty, rendered from the mesh-side microscope service. Plain one-frame output; --json is the stable machine contract

**Arguments**

| Argument | Description |
|---|---|
| `<SELECTOR>` | The entity: email \| handle \| write-anchor UUID \| slot alias |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--service <SERVICE>` | Microscope service origin (default: $GOAT_MICROSCOPE_URL) |  |
| `--json` | Raw view-model JSON (stable machine contract) |  |
| `--limit <LIMIT>` | Max timeline rows in the plain-text frame (JSON is never truncated) | `40` |

## `goat who`

Identity common operating picture (the top(1) of identities): a searchable roster browser — who is in the goatnet, what each may do, and what they're doing. Interactive screen is TTY-only; --once is the scriptable plain-text snapshot (ADR 1047)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--sort <SORT>` | Initial roster sort; `s` cycles it live. Default LIVE (activity-first; `/` search stays instant, so still searchable-first) | `live` |
| `--filter <FILTER>` | Start with the roster filtered. Space-separated terms are AND-ed; a bare term is a case-insensitive substring of the name/anchor/display-name, and the facet prefixes `live:on\|off` / `use:on\|off` narrow by live/producing state. In the interactive screen `/` edits this live (htop idiom) |  |
| `--site <SITE>` | Filter the presence roll-up to one site scope (matches the `<scope>` of `ops/presence/health/<scope>`) |  |
| `--once` | Batch snapshot (the `top -bn1` analog): collect live state for one bounded window, render ONE plain-text roster frame (name / live / usage / band / registry-membership) to stdout, then exit. Allowed on a non-TTY. Permissions are NOT included in `--once` (an N-call-per-identity fetch) — the scriptable single-identity capability path stays `goat access who` |  |

## `goat heal`

One-action mesh recovery: run the status probes, and when the mesh is down bounce the mesh client (netbird down → up, or goat-clientd disconnect/connect), wait for the router to come back, and re-probe — reports what it did and the verified end state, or explains why it couldn't heal

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit one JSON object (before / actions / after / outcome) instead of pretty lines |  |
| `--wait <WAIT>` | Seconds to wait for the mesh to come back after the bounce | `30` |
| `--netbird-bin <NETBIRD_BIN>` | Mesh client binary to bounce (PATH lookup unless a path is given) | `netbird` |

## `goat profile`

Manage the local goat profile (mTLS cert + portal token + endpoint set).

**Subcommands**

- [`goat profile init`](#goat-profile-init) — Establish a profile from a bundle.cbor — verifies signature, extracts mTLS cert/key/CA + portal token + endpoints, writes to the profile directory (mode 0700 / 0600), then suggests `goat doctor`
- [`goat profile show`](#goat-profile-show) — Show the active profile's endpoints, device id, publish anchor, portal token status, and cert/bundle expiry. Reads files only; does not contact the network
- [`goat profile rotate`](#goat-profile-rotate) — Rotate the profile with a new bundle. The previous profile is preserved at <dir>.bak/ for one rotation cycle
- [`goat profile path`](#goat-profile-path) — Print the resolved profile directory path and exit. Always works, even before `init` has been run

## `goat profile init`

Establish a profile from a bundle.cbor — verifies signature, extracts mTLS cert/key/CA + portal token + endpoints, writes to the profile directory (mode 0700 / 0600), then suggests `goat doctor`

**Arguments**

| Argument | Description |
|---|---|
| `<BUNDLE>` | Path to the operator bundle (CBOR) |

## `goat profile show`

Show the active profile's endpoints, device id, publish anchor, portal token status, and cert/bundle expiry. Reads files only; does not contact the network

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
- [`goat context refresh`](#goat-context-refresh) — Write this goatnet's own facts into the context's profile.toml

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

## `goat context refresh`

Write this goatnet's own facts into the context's profile.toml.

Reads `goatnet.toml` from the goatnet's download surface (`get.<site>/goatnet.toml`): its NetBird management URL and group prefix, Ory URL, router endpoint and the hosts' SSH user. Commands that need a per-goatnet fact read it from the profile; goat holds none.

**Arguments**

| Argument | Description |
|---|---|
| `<NAME>` | Defaults to the active context |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--from <FROM>` | Read the facts from this URL or local file instead of get.<site> |  |
| `--dry-run` | Print what would change without writing |  |

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
| `--topic <TOPIC>` | Write-precheck a specific topic key BEFORE publishing: writes a unique sentinel to a scratch sub-key under <KEY> and reads it back. A round-trip means you can write here; a silent miss is exactly what an ACL ingress-denial looks like to a publisher Uses only your own publish credential — no COP/admin access — so a denied org can self-diagnose. Replaces the local-readiness and fabric sweep; runs the probe and exits |  |

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

- [`goat topic ls`](#goat-topic-ls) — List topics. Optional prefix arg drilldown (e.g. `org11x` or `org11x/observations`); without it, lists across all orgs
- [`goat topic show`](#goat-topic-show) — Show one topic's full record (key, format, schema, last publish)
- [`goat topic create`](#goat-topic-create) — Create a new topic in the portal. Requires the profile's Bearer token to have org-prefix authority server-side
- [`goat topic delete`](#goat-topic-delete) — Delete a topic. Default is soft delete (portal-side semantics); `--hard` passes `?hard=true` and the portal may refuse based on audit-tail retention policy
- [`goat topic export`](#goat-topic-export) — Export an org's topics as a bulk file (the interactive state-as-code on-ramp). Prints to stdout; redirect to a file
- [`goat topic import`](#goat-topic-import) — Import topics from a bulk file (the shape `export` produces). Reads from a file or stdin. `--dry-run` previews the diff (create/update/unchanged/retire) without applying

## `goat topic ls`

List topics. Optional prefix arg drilldown (e.g. `org11x` or `org11x/observations`); without it, lists across all orgs

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

Show one topic's full record (key, format, schema, last publish)

**Arguments**

| Argument | Description |
|---|---|
| `<KEY>` | Topic key (e.g. `org11x/observations/v1`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit JSON instead of pretty |  |

## `goat topic create`

Create a new topic in the portal. Requires the profile's Bearer token to have org-prefix authority server-side

**Arguments**

| Argument | Description |
|---|---|
| `<KEY>` | Topic key (e.g. `org11x/observations/v1`). The leading slash segment is the org prefix the portal POST targets |

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
| `<KEY>` | Topic key (e.g. `org11x/observations/v1`) |

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
| `<KEY>` | Topic key to publish to (e.g. `acme/observations/v1`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--payload <PAYLOAD>` | Payload text, given inline |  |
| `--payload-file <PAYLOAD_FILE>` | Payload from a file (use --payload for inline text, or pipe stdin) |  |
| `--encoding <ENCODING>` | Stamp the sample's self-describing wire encoding. Shorthands: `json`, `cbor`, `text`, `protobuf`, `protobuf:<MessageName>` (the message name rides along so a consumer knows which descriptor to decode with). Any other value is passed through as a media type verbatim. Unset leaves the Zenoh default (`zenoh/bytes`) |  |
| `--confirm` | After publishing, read the value back from router storage to verify it landed |  |
| `--volatile-stream` | Publish with DROP congestion control for the loss-tolerant `volatile/<peer>/stream/**` firehose, so a high-rate producer never back-pressures the session |  |
| `--sealed` | Encrypt the payload to the release-set's granted audience before publishing (grant-keyed encryption, ADR 1094). The key you publish under must be your OWN producer anchor subtree; the DEK comes from the key bundle grant-keyd wraps to you. Needs the signer-trust flags below |  |
| `--key-service-bundle <KEY_SERVICE_BUNDLE>` | (with --sealed/--open) The key service's SPIRE trust bundle PEM — the root grant-keyd's bundle-signing SVID chains to. Fail-closed: an unverifiable signer is refused |  |
| `--trust-domain <TRUST_DOMAIN>` | (with --sealed/--open) The goatnet's SPIFFE trust domain (e.g. zagros.zagros.example) |  |
| `--key-service-id <KEY_SERVICE_ID>` | (with --sealed/--open) The key service's SPIFFE id. Defaults to spiffe://<trust-domain>/grant-keyd |  |

## `goat sub`

Subscribe to a topic key expression.

**Arguments**

| Argument | Description |
|---|---|
| `<KEY_PATTERN>` | Zenoh key expression to subscribe to (e.g. `acme/**`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--follow` | Accepted and ignored: streaming until you stop it IS the default. Kept so the many runbooks, demo pages and muscle-memory invocations that spell `--follow`/`--watch` keep working unchanged |  |
| `--decode` | Decode payload bytes against the topic's portal-registered schema |  |
| `--tui` | Interactive Tail lens for live tail (default is line-oriented): event table, rate sparkline, on-demand decoded/hex viewer. Requires a terminal (refuses on a non-TTY) |  |
| `--count <COUNT>` | Exit after this many events. Omitted, `goat sub` streams until you stop it — a live tail is what an operator almost always wants, and the old exit-after-one default made a working subscription look like a failure. `--take` is the same flag (the spelling every health runbook already uses) |  |
| `--keys-only` | Print only each event's key, never the payload. The prefix-census form: `goat sub '**' --keys-only --take 200 \| sort -u` |  |
| `--for <FOR_>` | Stop after this long (`30s`, `2m`, `1h30m`) whether or not `--take` was satisfied. A bounded tail otherwise waits forever for events that may never come, so a runbook acceptance check hangs instead of failing. With `--take`, an unmet count at the deadline exits non-zero; without it, reaching the deadline is simply the end of the tail |  |
| `--since <SINCE>` | Replay past events from the audit-tail instead of subscribing live. A duration (`1h`, `90m`) or an RFC3339 timestamp |  |
| `--open` | Decrypt each received sample with the grant-keyed DEK you hold (grant-keyed encryption, ADR 1094). Query-history-then-subscribe: you get retained samples plus the live tail. A sample you have no key for prints DENIED — the read cut, enforced by the crypto, not the router |  |
| `--key-service-bundle <KEY_SERVICE_BUNDLE>` | (with --open) The key service's SPIRE trust bundle PEM |  |
| `--trust-domain <TRUST_DOMAIN>` | (with --open) The goatnet's SPIFFE trust domain |  |
| `--key-service-id <KEY_SERVICE_ID>` | (with --open) The key service's SPIFFE id (default spiffe://<td>/grant-keyd) |  |
| `--literal` | Subscribe on the key exactly as written — skip slot-alias resolution of the leading segment. For a namespace that is not a slot anchor on a slot-pool goatnet: a seeded per-vendor prefix such as `airdefenders/` (seeded beside the slot pool), where the resolver would refuse the segment as an unknown alias. Read-only, so a typo costs an empty tail, never a phantom write prefix (the F-177 case the fail-closed resolver exists for). `goat demo` sets this for its generated key |  |

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
| `<TARGET>` | Optional target to pre-select on open (a topic key, e.g. `org11x/observations/v1`) |

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

Interpret an error: a code (`E_OFF_MESH`, `GB-007`, `X-SSH-001`) or any error text, goat's own or a tool's words it carried — what happened, why, what to do, where to read.

**Arguments**

| Argument | Description |
|---|---|
| `<INPUT>` | A code (`E_PROFILE_MISSING`, `profile-missing`, `GB-007`, `X-SSH-001`) or the words of an error. `-` reads the text from stdin |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--report` | Also write the text and its interpretation to ~/.goat/error-reports/ — the file to share when the catalog does not know the error |  |
| `--list` | List every code the catalog knows |  |
| `--json` | Machine output: the entry, or the interpretation, as JSON |  |

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
- [`goat admin ssh-key`](#goat-admin-ssh-key) — Host-shell SSH keys: submit one for somebody, review the queue, decide. The CLI face of the console's /ssh-key + /onboarding/ssh-keys pages — same store, same gate, same audit (docs/design/ssh-key-self-service.md)
- [`goat admin invites`](#goat-admin-invites) — Manage existing onboarding invites: list the queue or revoke an open one. (The singular `invite` command mints a new one.)
- [`goat admin role`](#goat-admin-role) — Change an EXISTING person's login access — set their Ory groups, or flip their activation state — through the console's machine API. The CLI face of the web UI's user admin: the server runs the same last-admin guard, operator audit, and role-change email, so a `goat admin role` change and a console one are identical. For a NEW person use `invite`; this never mints an identity. See docs/design/onboarding-notifications.md
- [`goat admin operator-key`](#goat-admin-operator-key) — Operator API keys — the per-person `op_` key that makes a Portal admin action attributable to a named person rather than to a shared credential
- [`goat admin onboard`](#goat-admin-onboard) — REVOKE an operator / org by handle — the live offboarding path (ADR 1031)
- [`goat admin rotate`](#goat-admin-rotate) — Re-issue (rotate) a peer's cert in place — same anchor, fresh leaf. The "rotate" sibling of `--revoke`: use it for expiry renewal, post-compromise rotation, or retrofitting a new URI-SAN scheme onto an already-issued cert, all WITHOUT changing the peer's anchor / alias / write namespace / ACL. Pure-portal POST /api/orgs/{anchor}/rotate, operator-gated server-side. The selector is an anchor (32-hex slot_id) or its slot alias; the alias is resolved to the anchor before the call. Prints the fresh one-time bundle URL — fetch it, swap the cert+key on the peer, and restart it. See docs/design/credential-rotation.md
- [`goat admin sponsorship`](#goat-admin-sponsorship) — Grant or revoke an identity's sponsorship authorization (ADR 1071) — whether it may mint candidate-onboarding invites and vouch at the stage-2 gate, and up to which tier. The case-by-case lever for identities that didn't get sponsorship by tier default at onboarding (iam-admin / platform-admin get it automatically). Pure-portal POST /api/orgs/{id}/sponsorship, operator-gated server-side; accepts an anchor or the human handle
- [`goat admin lan`](#goat-admin-lan) — LAN troubleshooting toolkit for relocated goat mgmt hosts that don't appear on the WAN: `scan` finds them on the local LAN by MAC even when DHCP moved their IP; `doctor` runs a layered L0–L4 triage. Depends on NO goatnet/mesh service — works when the mesh does not. See docs/design/goat-cli.md §"goat admin lan"
- [`goat admin client`](#goat-admin-client) — goat-client device lifecycle (admin-gated). Today: bundle minting via `goat admin client onboard <handle>` which reads the per-device manifest at `ops/devices/<handle>.yml` and translates it into a `bundle-create` invocation. Per-site fleet config (CA-id, mode, age-identity / piv-slot) comes from CLI flags + env vars, NOT the per-device manifest
- [`goat admin peer`](#goat-admin-peer) — Mesh-peer lifecycle — the authoritative mesh-first cut. `ls` lists mesh peers; `cut` ejects one (deletes its NetBird peer, so its WireGuard tunnel drops). On a brokerless slot-pool fabric the Zenoh write-ACL is pre-baked for the whole pool, so deleting an org record does NOT stop a flooder — removing the *peer* does (a cert is inert without a mesh path). This is the one-command emergency cut: `goat health` finds the hog, `goat admin peer cut` ejects it. Needs the NetBird admin token in the environment ($<CONTEXT>_NB_API_TOKEN, e.g. ZAGROS_NB_API_TOKEN, or $GOAT_NB_API_TOKEN)
- [`goat admin platform-admin`](#goat-admin-platform-admin) — Make someone a platform admin on a goatnet, or stop them being one, on every plane at once: Ory login group, NetBird dashboard role, their peers' mesh groups, host SSH keys, operator key, sponsorship, and the 1Password Platform group. `status` reads; `grant` / `revoke` plan and, with --apply, execute and re-read. Revoke demotes to developer by default; --full offboards. Canon: docs/design/platform-admin-lifecycle.md
- [`goat admin members`](#goat-admin-members) — Membership transfer engine (ADR 1088): move entities between canonical and satellite groups (probation, quarantine, exercise sets) — audited, coverage-gated, mirror-first, idempotent; dry-run by default
- [`goat admin operator`](#goat-admin-operator) — Backend-OPERATOR tier provisioning (admin-gated). Distinct from `onboard` (the participant tier): `operator onboard <handle>` wraps `ops/ansible/operator-tier-onboard.yml` to mint the operator's `operator-trial-peer` Zenoh leaf (writes `ops/**`), install their host SSH key, grant the Ory admin group on their existing identity, render the operator bundle, and PRINT the two blast-radius additions (portal key, age recipient — never auto-edited). The manifest must carry an `operator:` block. See docs/operations/admin-operator-guide.md
- [`goat admin pqc`](#goat-admin-pqc) — Toggle PQC on an ALREADY-ENROLLED peer at BOTH layers — the Zenoh kx-mode state file (F-259 Tier 1 kill-switch, `/var/lib/goat/pqc-mode`; new processes latch it, long-running surfaces flip on restart) and intra-mesh Rosenpass (ADR 0602) — no setup key, no re-mint. The rosenpass leg runs `netbird down` then `netbird up` with the explicit flag (`--enable-rosenpass --rosenpass-permissive` for `on`, `--enable-rosenpass=false` for `off`). Both the `down` and the explicit-`false` matter: a bare `up` no-ops on a connected peer AND persists the prior setting, so it silently changes nothing. The toggle briefly drops + re-establishes the tunnel. Permissive means `on` never breaks connectivity to not-yet-enabled peers, and PQC only actually engages between two Rosenpass-enabled peers — so a mixed fleet is safe
- [`goat admin storm`](#goat-admin-storm) — Control-plane storm resilience (ADR 1081). Today: `storm brake` — the MANUAL operator brake fired when the relay-health host-net band shows a signaling storm (futility ratio ≥0.5 at ≥100 msgs/min). It flushes the signal container, tightens L2 limits (if deployed), sheds declared non-essential publishers, and records one audited operator action. Reaches the mgmt host over SSH (host+user from the goat context). NO automation / timers — auto-engage is a separately-gated follow-on. See docs/operations/storm-brake-runbook.md

## `goat admin org`

Org lifecycle (admin-gated).

Named `org` since the ADR 1052 vendor→org rename; `vendor` stays a hidden alias so existing scripts keep working.

**Subcommands**

- [`goat admin org create`](#goat-admin-org-create) — Create a new org in the portal. Admin-gated server-side
- [`goat admin org delete`](#goat-admin-org-delete) — Revoke an org. Portal refuses with 409 if the org still has non-retired topics — retire them first via `goat topic delete --hard <key>`
- [`goat admin org tree`](#goat-admin-org-tree) — Render the org registry as a tree (roots, children indented), with kind/nation/aliases/status. Read-only
- [`goat admin org suggest`](#goat-admin-org-suggest) — Fuzzy-match a typed org name against the registry (ADR 1112 §7). NEVER auto-merges — similar spelling is not the same org; this ranks candidates for a human to judge. 404 = matching not enabled here
- [`goat admin org register`](#goat-admin-org-register) — Register an org in the registry (deliberate act; audited). Use `--despite <candidate>` for every suggest candidate you reviewed and judged DISTINCT — recorded as negative pairs so that pairing is never re-suggested as a top candidate
- [`goat admin org set-parent`](#goat-admin-org-set-parent) — Set (or clear) an org's parent — an effective-dated containment edge; history survives, cycles are refused at write time
- [`goat admin org update`](#goat-admin-org-update) — Correct an existing org's facts (kind / nation / composition / display name) in place — audited; never touches identity, grants, or keys (org ids are immutable and off every wire anchor, ADR 1112 §3). The verb the `origin` kind mislabel lacked
- [`goat admin org alias`](#goat-admin-org-alias) — Record a name-variant/abbreviation alias for an org (accretes matcher recall; aliases share one namespace with org ids)
- [`goat admin org merge`](#goat-admin-org-merge) — Alias-absorb merge (ADR 1112 §8): LOSER's id becomes a permanent alias of --into; everything keyed to it keeps resolving; children re-parent effective-dated; reversible via unmerge. A merge is ALSO a governance union — review both orgs' authority sets (govern admins, grants-as-grantee, multipub pools: `goat access show`, `goat access co-publish list-topics`) and pass --acknowledge-governance-union
- [`goat admin org unmerge`](#goat-admin-org-unmerge) — Reactivate an absorbed org (the merge reversal). Parent edges are NOT auto-reverted — effective-dated history is never rewritten; re-attach with set-parent deliberately

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

## `goat admin org tree`

Render the org registry as a tree (roots, children indented), with kind/nation/aliases/status. Read-only

## `goat admin org suggest`

Fuzzy-match a typed org name against the registry (ADR 1112 §7). NEVER auto-merges — similar spelling is not the same org; this ranks candidates for a human to judge. 404 = matching not enabled here

**Arguments**

| Argument | Description |
|---|---|
| `<NAME>` | The name as typed (any language/spelling — normalization folds case, diacritics, punctuation) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--nation <NATION>` | Scope candidates to a nation (ISO alpha-3 / NATO / EU) |  |

## `goat admin org register`

Register an org in the registry (deliberate act; audited). Use `--despite <candidate>` for every suggest candidate you reviewed and judged DISTINCT — recorded as negative pairs so that pairing is never re-suggested as a top candidate

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Kebab org id (immutable — never renamed; a rename is a merge) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--display-name <DISPLAY_NAME>` |  |  |
| `--kind <KIND>` | vendor \| organic \| partner \| civil |  |
| `--nation <NATION>` |  |  |
| `--parent <PARENT>` | Parent org id (must already exist — chains build top-down) |  |
| `--provenance <PROVENANCE>` | authoritative \| operator-verified \| open-source \| self-asserted (default operator-verified for a CLI act) |  |
| `--justification <JUSTIFICATION>` |  |  |
| `--despite <DESPITE>` | Candidate org id you reviewed and judged distinct (repeatable) |  |

## `goat admin org set-parent`

Set (or clear) an org's parent — an effective-dated containment edge; history survives, cycles are refused at write time

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--parent <PARENT>` | The new parent org id-or-alias. Omit with --clear to detach |  |
| `--clear` | Detach: the org becomes a root |  |
| `--justification <JUSTIFICATION>` |  |  |

## `goat admin org update`

Correct an existing org's facts (kind / nation / composition / display name) in place — audited; never touches identity, grants, or keys (org ids are immutable and off every wire anchor, ADR 1112 §3). The verb the `origin` kind mislabel lacked

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--kind <KIND>` | vendor \| organic \| partner \| civil |  |
| `--nation <NATION>` | ISO 3166-1 alpha-3 (e.g. LVA) or a marker (NATO, EU) |  |
| `--display-name <DISPLAY_NAME>` |  |  |
| `--composition <COMPOSITION>` | national \| multinational — is this organisation one sovereign's, or constituted by a body of nations? (ADR 1171). Pass an empty string to clear it back to not-yet-declared, which kind and nation refuse |  |
| `--justification <JUSTIFICATION>` |  |  |

## `goat admin org alias`

Record a name-variant/abbreviation alias for an org (accretes matcher recall; aliases share one namespace with org ids)

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` |  |
| `<ALIAS>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--kind <KIND>` | name-variant \| abbreviation (absorbed-id is minted only by merge) |  |

## `goat admin org merge`

Alias-absorb merge (ADR 1112 §8): LOSER's id becomes a permanent alias of --into; everything keyed to it keeps resolving; children re-parent effective-dated; reversible via unmerge. A merge is ALSO a governance union — review both orgs' authority sets (govern admins, grants-as-grantee, multipub pools: `goat access show`, `goat access co-publish list-topics`) and pass --acknowledge-governance-union

**Arguments**

| Argument | Description |
|---|---|
| `<LOSER>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--into <INTO>` |  |  |
| `--justification <JUSTIFICATION>` |  |  |
| `--acknowledge-governance-union` | Assert you reviewed the combined authority set. Fail-closed without it (server-side too) |  |

## `goat admin org unmerge`

Reactivate an absorbed org (the merge reversal). Parent edges are NOT auto-reverted — effective-dated history is never rewritten; re-attach with set-parent deliberately

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` |  |

## `goat admin invite`

Sponsor a candidate: mint a one-time onboarding invite (ADR 1062).

Every field is OPTIONAL (minimal-burden): mint a bare link and let the enrollee fill their own details. Read a spec from a YAML file with `--file`, tweak it in your `$EDITOR` before firing with `--edit`, or pass fields as flags. Prints the one-time nomination link to hand the candidate. Escalated admin tiers (`iam-admin`/`platform-admin`) may be offered here — sponsor-only — and are flagged for scrutiny.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--file <FILE>` | Read the invite spec from a YAML file. Recognized keys (all optional): `env`, `tier`, `affiliation`, `site`, `machine_quota`, `ttl_hours`, `note`, `collab`. Flags below override matching file keys |  |
| `--edit` | Open the spec in `$EDITOR` before firing (like `git commit`). Starts from `--file` if given, else a commented template. On empty/unchanged save it still fires the (possibly bare) invite |  |
| `--env <ENV>` | Target goatnet (e.g. `zagros`) |  |
| `--tier <TIER>` | Access tier: `developer` \| `browse` \| `iam-admin` \| `platform-admin`. The two admin tiers are ESCALATED (scrutinized). Omit to let the enrollee choose (non-escalated only) |  |
| `--affiliation <AFFILIATION>` | Affiliation kind: `vendor` \| `organic` \| `partner` \| `civil` |  |
| `--site <SITE>` | Site (defaults to the environment) |  |
| `--machine-quota <MACHINE_QUOTA>` | Machine-enrollment quota the person may use later |  |
| `--ttl-hours <TTL_HOURS>` | One-time link lifetime in hours (default 72, max 720) |  |
| `--note <NOTE>` | Free-text note for your own reference |  |
| `--collab <COLLAB>` | Include the person in the collaboration environment (Outline / file sharing / Zulip — cloud SSO, separate from the mesh). Defaults to ON, matching the console's invite page; pass `--collab false` to exclude them |  |
| `--email <EMAIL>` | Email the one-time nomination link to this candidate right after minting (via the console SES mailer) — so the automated mint is self-contained. Omit to just print the link for manual hand-off |  |
| `--json` | Emit the raw JSON mint response instead of the friendly summary |  |

## `goat admin queue`

Review + act on the onboarding queue from the CLI (the admin's stage-3 gate — mirrors /admin/onboard-queue.html): list proposals, approve (→ the runner mints, or run the emitted command), or reject

**Subcommands**

- [`goat admin queue ls`](#goat-admin-queue-ls) — List proposals. Default `sponsor_approved` (the ones ready for the admin)
- [`goat admin queue approve`](#goat-admin-queue-approve) — Approve a sponsor-approved proposal → the runner mints it (or run the emitted command). Refused unless the sponsor has already vouched
- [`goat admin queue reject`](#goat-admin-queue-reject) — Reject a proposal (at either the sponsor or admin stage)
- [`goat admin queue sponsor-approve`](#goat-admin-queue-sponsor-approve) — Sponsor-vouch a proposed candidate (stage 2) → moves it to `sponsor_approved` for the admin's stage-3. Refused on an incomplete draft
- [`goat admin queue sponsor-reject`](#goat-admin-queue-sponsor-reject) — Sponsor-decline a proposed candidate (stage 2) → `rejected`

## `goat admin queue ls`

List proposals. Default `sponsor_approved` (the ones ready for the admin)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--status <STATUS>` | Status filter: `proposed` \| `sponsor_approved` \| `approved` \| `rejected` \| `onboarded` \| `failed` \| `offboarded` \| `onboarding` \| `enrolled` \| `archived` \| `offboard-requested` \| `offboarding` \| `offboard-failed` \| `all`. Default `sponsor_approved`, or `all` when `--email` is given (an explicit `--status` always wins) |  |
| `--env <ENV>` | Filter to one goatnet (e.g. `zagros`) |  |
| `--email <EMAIL>` | Show only this address's proposals (exact match, ASCII case-insensitive — the same fold as the store's collision key) — the lookup for a duplicate-email 409, which names the proposal blocking a submit; this finds it whatever state it is in, so it defaults to `--status all`. An explicit `--status` or `--env` can hide the blocking proposal. A non-escalated sponsor sees only their own invitees (the server scopes the list), so an existing proposal can still come back empty here |  |
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

## `goat admin queue sponsor-approve`

Sponsor-vouch a proposed candidate (stage 2) → moves it to `sponsor_approved` for the admin's stage-3. Refused on an incomplete draft

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Proposal id (`prop-…`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit raw JSON ack instead of the summary |  |

## `goat admin queue sponsor-reject`

Sponsor-decline a proposed candidate (stage 2) → `rejected`

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Proposal id (`prop-…`) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--note <NOTE>` | Reason, recorded on the proposal |  |

## `goat admin ssh-key`

Host-shell SSH keys: submit one for somebody, review the queue, decide. The CLI face of the console's /ssh-key + /onboarding/ssh-keys pages — same store, same gate, same audit (docs/design/ssh-key-self-service.md).

SUBMITTING GRANTS NOTHING. An admin approves, then the installer places the key on the hosts that person's ROLE may reach. `approved` is a decision; `installed` is a fact about a host.

**Subcommands**

- [`goat admin ssh-key ls`](#goat-admin-ssh-key-ls) — List submitted keys. Default `submitted` — what is waiting on a decision
- [`goat admin ssh-key submit`](#goat-admin-ssh-key-submit) — Record a public key FOR SOMEBODY ELSE — the "send it to an admin over chat" path, for people who would rather not use the console
- [`goat admin ssh-key approve`](#goat-admin-ssh-key-approve) — Approve a submitted key. This is a DECISION, not an installation: the installer still has to place it, and until then the person has no shell
- [`goat admin ssh-key reject`](#goat-admin-ssh-key-reject) — Reject a submitted key. Terminal — it cannot be approved afterwards
- [`goat admin ssh-key revoke`](#goat-admin-ssh-key-revoke) — Revoke a key. Works from every live state, because revocation must never be the operation refused on a technicality
- [`goat admin ssh-key install`](#goat-admin-ssh-key-install) — Install every approved host-shell key for a goatnet. Wraps `operator-ssh-key-install.yml` via `ops/scripts/ansible-preflight.sh`, resolving the console URL and operator API key from local config

## `goat admin ssh-key ls`

List submitted keys. Default `submitted` — what is waiting on a decision

**Options**

| Flag | Description | Default |
|---|---|---|
| `--status <STATUS>` | Status filter: `submitted` \| `approved` \| `installed` \| `rejected` \| `revoked` \| `all` | `submitted` |
| `--env <ENV>` | Filter to one goatnet (e.g. `zagros-prod`) |  |
| `--json` | Emit raw JSON instead of the table |  |

## `goat admin ssh-key submit`

Record a public key FOR SOMEBODY ELSE — the "send it to an admin over chat" path, for people who would rather not use the console.

Takes a FILE, not a pasted string: it is the difference between `~/.ssh/id_ed25519.pub` and `~/.ssh/id_ed25519`, and a pasted private key is the one input that must never reach the store.

**Arguments**

| Argument | Description |
|---|---|
| `<HANDLE>` | The person's handle, as onboarding knows them |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--pubkey-file <PUBKEY_FILE>` | Path to their PUBLIC key file (`*.pub`) |  |
| `--env <ENV>` | Goatnet this key is for |  |
| `--note <NOTE>` | Note for the reviewer — e.g. which machine this is | `` |
| `--email <EMAIL>` | Login email for this admin (for the display gate and legacy backfill) |  |

## `goat admin ssh-key approve`

Approve a submitted key. This is a DECISION, not an installation: the installer still has to place it, and until then the person has no shell

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Key id (from `ls`) |

## `goat admin ssh-key reject`

Reject a submitted key. Terminal — it cannot be approved afterwards

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Key id (from `ls`) |

## `goat admin ssh-key revoke`

Revoke a key. Works from every live state, because revocation must never be the operation refused on a technicality

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Key id (from `ls`) |

## `goat admin ssh-key install`

Install every approved host-shell key for a goatnet. Wraps `operator-ssh-key-install.yml` via `ops/scripts/ansible-preflight.sh`, resolving the console URL and operator API key from local config.

Default behaviour: fetch + preview the approved queue and ask for confirmation before running the installer. `--check` stops after the preview, and `--yes` skips the confirmation prompt.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | Target goatnet slug (e.g. `zagros`, `zagros-prod`). Required |  |
| `--check` | Preview-only: print what WOULD be installed and change nothing |  |
| `--yes` | Skip the interactive confirmation prompt for a real install |  |

## `goat admin invites`

Manage existing onboarding invites: list the queue or revoke an open one. (The singular `invite` command mints a new one.)

**Subcommands**

- [`goat admin invites ls`](#goat-admin-invites-ls) — List invites. Default `invited` (the still-open ones)
- [`goat admin invites revoke`](#goat-admin-invites-revoke) — Revoke an open invite before it is consumed (fails closed on use)

## `goat admin invites ls`

List invites. Default `invited` (the still-open ones)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--status <STATUS>` | Status filter: `invited` \| `submitted` \| `expired` \| `revoked` \| `all` | `invited` |
| `--env <ENV>` | Filter to one goatnet (e.g. `zagros`) |  |
| `--json` | Emit raw JSON instead of the table |  |

## `goat admin invites revoke`

Revoke an open invite before it is consumed (fails closed on use)

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Invite id |

## `goat admin role`

Change an EXISTING person's login access — set their Ory groups, or flip their activation state — through the console's machine API. The CLI face of the web UI's user admin: the server runs the same last-admin guard, operator audit, and role-change email, so a `goat admin role` change and a console one are identical. For a NEW person use `invite`; this never mints an identity. See docs/design/onboarding-notifications.md

**Subcommands**

- [`goat admin role set`](#goat-admin-role-set) — Set a person's Ory login groups to EXACTLY `--groups` (the full desired set; a person keeping IAM-ADMIN and gaining nothing else still passes the whole list). Escalated admin groups (PLATFORM-ADMIN / IAM-ADMIN) are allowed on this operator-key path and fully attributed in the audit
- [`goat admin role reactivate`](#goat-admin-role-reactivate) — Reactivate a deactivated identity (state → active) — the pre-step before granting a returning principal
- [`goat admin role deactivate`](#goat-admin-role-deactivate) — Deactivate an identity (state → inactive; reversible with `reactivate`). Refused if it would remove the last platform-admin

## `goat admin role set`

Set a person's Ory login groups to EXACTLY `--groups` (the full desired set; a person keeping IAM-ADMIN and gaining nothing else still passes the whole list). Escalated admin groups (PLATFORM-ADMIN / IAM-ADMIN) are allowed on this operator-key path and fully attributed in the audit

**Options**

| Flag | Description | Default |
|---|---|---|
| `--email <EMAIL>` | The person's email (the dedup key; the console resolves it to their Kratos identity) |  |
| `--groups <GROUPS>` | Comma-separated full desired group set, e.g. `--groups PLATFORM-ADMIN` or `--groups IAM-ADMIN,GRAFANA-READONLY`. Pass `--groups ""` to clear all login groups. Required so an omitted flag can never silently clear a person's groups |  |
| `--revoke-sessions` | Also force-log-out the person's live sessions (use on a reduction so old claims don't linger) |  |
| `--json` | Emit raw JSON instead of the human line |  |

## `goat admin role reactivate`

Reactivate a deactivated identity (state → active) — the pre-step before granting a returning principal

**Options**

| Flag | Description | Default |
|---|---|---|
| `--email <EMAIL>` | The person's email |  |
| `--reinstate` | Explicitly append a revocation-ledger reinstatement via privileged SSH |  |
| `--reason <REASON>` | Reason recorded in both console audit and the immutable ledger |  |
| `--env <ENV>` | Goatnet for the privileged management-host recovery path |  |
| `--json` |  |  |

## `goat admin role deactivate`

Deactivate an identity (state → inactive; reversible with `reactivate`). Refused if it would remove the last platform-admin

**Options**

| Flag | Description | Default |
|---|---|---|
| `--email <EMAIL>` | The person's email |  |
| `--json` |  |  |

## `goat admin operator-key`

Operator API keys — the per-person `op_` key that makes a Portal admin action attributable to a named person rather than to a shared credential.

Issuing one used to mean editing a gitignored envrc and redeploying the portal. It no longer does: approving writes the key file the portal reads, and the portal re-reads that directory while running, so the key is live within seconds. Revoking removes the file, the same way round.

Same engine as the console pages at `/operator-key` and `/onboarding/operator-keys` — docs/design/operator-action-audit.md.

**Subcommands**

- [`goat admin operator-key request`](#goat-admin-operator-key-request) — Record a request. Grants nothing — a DIFFERENT admin still approves
- [`goat admin operator-key list`](#goat-admin-operator-key-list) — List requests: requested (default) \| active \| revoked \| rejected \| all
- [`goat admin operator-key approve`](#goat-admin-operator-key-approve) — Approve a request — mints the key AND writes the portal's key file
- [`goat admin operator-key reject`](#goat-admin-operator-key-reject) — Decline a request without minting anything
- [`goat admin operator-key revoke`](#goat-admin-operator-key-revoke) — Revoke a live key — removes the file first, then the record
- [`goat admin operator-key collect`](#goat-admin-operator-key-collect) — Collect the key value. Works EXACTLY ONCE, for its owner
- [`goat admin operator-key reconcile`](#goat-admin-operator-key-reconcile) — Compare the record against the directory the portal actually reads. Dry by default

## `goat admin operator-key request`

Record a request. Grants nothing — a DIFFERENT admin still approves.

With no handle the request is for YOU: the console binds it to the person this profile's token belongs to, so a first-time admin needs no operator key to ask for one (#1373). Only a platform admin holds an operator key (ADR 1182), and a self request needs --reason naming the admin task. With a handle it is filed on that person's behalf, which takes your operator key (--context <env>-op).

**Arguments**

| Argument | Description |
|---|---|
| `<HANDLE>` | The person's handle, as the onboarding roster knows them. Omit it to file for yourself |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--email <EMAIL>` | Their email, so the queue shows who the request is for | `` |
| `--env <ENV>` | Target goatnet. Defaults to the console's own | `` |
| `--reason <REASON>` | What they need it for. The admin decides on the person and the work | `` |
| `--json` |  |  |

## `goat admin operator-key list`

List requests: requested (default) | active | revoked | rejected | all

**Options**

| Flag | Description | Default |
|---|---|---|
| `--status <STATUS>` |  | `requested` |
| `--json` |  |  |

## `goat admin operator-key approve`

Approve a request — mints the key AND writes the portal's key file

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` |  |  |

## `goat admin operator-key reject`

Decline a request without minting anything

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--note <NOTE>` |  | `` |
| `--json` |  |  |

## `goat admin operator-key revoke`

Revoke a live key — removes the file first, then the record

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--note <NOTE>` |  | `` |
| `--json` |  |  |

## `goat admin operator-key collect`

Collect the key value. Works EXACTLY ONCE, for its owner

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` |  |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--handle <HANDLE>` | The handle the key belongs to; defaults to the person this profile's token belongs to. Collection is bound to the owner either way |  |
| `--json` |  |  |

## `goat admin operator-key reconcile`

Compare the record against the directory the portal actually reads. Dry by default

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` |  | `` |
| `--apply` | Remove grants no record explains. Never invents a MISSING key: the secret is deliberately not retained, and manufacturing one would hand somebody a credential nobody approved |  |
| `--json` |  |  |

## `goat admin onboard`

REVOKE an operator / org by handle — the live offboarding path (ADR 1031).

The ENROLL direction is RETIRED (July 2026, ADR 1062 cutover) and refuses. It minted every surface from a hand-authored manifest and wrote an out-of-band bundle carrying the person's private key; the live path never puts a key in an operator's hands. Bring people on with `goat admin invite --env <goatnet>` and the three-stage candidate flow, then reconcile the roster with ops/scripts/sync-onboarded-manifests.py. The refusal names the override if you genuinely need it.

Two implementations underneath, both fenced for enroll and both live for revoke: a native fast path (portal + NetBird API + Ory over SSH, seconds) which is the DEFAULT when the manifest is native-serviceable, and `ops/scripts/ansible-preflight.sh operator-onboard.yml` otherwise. `--native` / `--ansible` force one. Streams child output line-by-line; SIGINT propagates via the process group (tokio::process default).

**Arguments**

| Argument | Description |
|---|---|
| `<HANDLE>` | Handle (kebab-case slug). Manifest must exist at `ops/operators/<handle>.yml` (flat / legacy) or, when `--env <env>` is passed, `ops/operators/<env>/<handle>.yml` in the discovered repo root. Omit when using `--org` |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--org <ORG>` | ORG-scope revoke (cascade). Instead of one handle, revoke EVERY active member of the affiliation org `<id>` (the `affiliation.id`, e.g. `acme`) — persons + systems, the whole unit. Resolves the members from the declared registry (`docs/operators/roster.json`, the affiliation-grouped roster) and runs the per-handle revoke for each, after printing the blast radius and asking to confirm (skip the prompt with `--yes`). Requires `--revoke`; mutually exclusive with a positional handle. Identity grammar B.4 / ADR 1031 org-cascade (`docs/design/credential-revocation-and-offboarding.md`) |  |
| `--yes` | Skip the interactive confirmation prompt for `--org` cascades. For scripted/non-interactive use; the blast radius is still printed |  |
| `--env <ENV>` | Environment / goatnet the manifest belongs to (e.g. `zagros`, `zagros-prod`). Selects the per-env manifest subfolder `ops/operators/<env>/<handle>.yml` and is passed to the playbook as `-e operator_env=<env>` so the per-site maps (NetBird mgmt URL, Ory Kratos admin host, Zenoh portal) resolve to that goatnet. Required on every live goatnet: with no env the native legs resolve to no config and defer |  |
| `--revoke` | Revoke instead of enroll. Cuts every provisioned surface — NetBird peer(s), Ory login, the Zenoh slot (guarded when teammates share it) — and flips the manifest to `status: revoked`. Reversible by default (Ory deactivate, registry deprecate): a returning principal is a re-onboard, not lost. Takes the native fast path (~seconds) when the env/manifest is native-serviceable; `--ansible` forces the playbook |  |
| `--purge` | With `--revoke`: terminally DELETE the Ory identity instead of the default reversible deactivate (state=inactive). Irreversible — use for a hard offboard where the login must not be recoverable. The Zenoh slot + NetBird peer are cut either way. Mirrors the playbook's `-e revoke_purge=true` (Phase R2.3) |  |
| `--from <FROM>` | With `--revoke`: batch-revoke every handle listed in FILE (one kebab-case handle per line; `#` comments and blanks ignored). Each is revoked natively and independently — a per-handle failure is reported, never aborts the batch. The generalized cohort-revoke path (the sweep tool). Mutually exclusive with a positional handle and `--org` |  |
| `--repair` | Re-mint a partial bundle in place: run the (hardened) revoke leg then the enroll leg as one operation, so every surface is freshly minted and rendered into one complete bundle. The all-or-nothing render means a half-provisioned bundle can't be completed by a plain re-run — this is the supported repair. The revoke leg frees the surfaces but leaves the manifest `status: active` (via `-e onboard_repair=true`) so the enroll leg re-mints onto it. See P1.5 in docs/design/onboard-rotate-revoke-streamlining.md. Mutually exclusive with --revoke |  |
| `--reissue-handset-key` | Revoke every live HANDSET key this person holds on the goatnet and mint a fresh one (a lost or replaced phone, ADR 1175). Touches nothing else: the device key, the Ory login and the Zenoh slot stay as they are, and no bundle is rendered. Native only. Refuses when the manifest says `netbird.handset_limit: 0` |  |
| `--reissue-device-key` | Mint a fresh DEVICE setup key for an already-onboarded person whose key has expired or been used up, and revoke any device key of theirs that is still live, so the person holds exactly one. Touches nothing else: enrolled peers, the handset key, the Ory login and the Zenoh slot stay as they are, and no bundle is rendered. Groups and the key name come from the manifest; with no manifest on this machine they come from the person's most recent device key on the goatnet (what they were actually granted), and the command says which it used. Records an operator-action audit event naming the operator, the person and the key. Native only. Console: "Reissue device key" |  |
| `--uses <USES>` | With `--reissue-device-key`: how many enrollments the fresh key allows (1-100). Default: the manifest's `netbird.usage_limit`, else 5 |  |
| `--requested-by <REQUESTED_BY>` | With either reissue flag: who asked for the key (the console user's email when the onboard-runner runs it on their behalf). Recorded on the audit event beside the operator who ran the command |  |
| `--native` | Force the native fast path (the DEFAULT for a plain enroll — provisions all three identity surfaces directly, no Ansible, seconds not minutes; mirrors `goat admin rotate`). Mints the Zenoh wire-layer (`POST /api/orgs` → slot/cert/key), the NetBird mesh setup key, AND the Ory Kratos login identity (over an SSH shell-out to the site Thor's Kratos admin API) — P2.1/P2.2/P2.4. Native config covers zagros/zagros-prod/zagros-dev; other envs defer that leg to Ansible with a clear note. The Ory leg runs last — a failure there leaves the Zenoh/NetBird bundle usable, marked `partial`, never silently dropped. The Ory operator-IdP surface stays on Ansible. As of goat-cli 0.4.13 the native path is the DEFAULT for BOTH enroll and revoke; this flag now only FORCES it (erroring if the env/manifest isn't native-serviceable) — pass `--ansible` for the legacy playbook. Not valid with --repair (which sequences a revoke+enroll through the playbook). See docs/design/onboard-rotate-revoke-streamlining.md |  |
| `--defer-credential` | Onboard the Zenoh slot WITHOUT minting its fabric credential (mint-on-collect, docs/design/fabric-credential-collection.md). The slot, the person linkage and the org row are created; no api_key and no cert are. The person mints their own from the console's fabric-credential page once they can sign in — which is the only way the key reaches them rather than passing through an operator and an email, since the plaintext can never be shown a second time |  |
| `--ansible` | Force the legacy Ansible playbook (`operator-onboard.yml`) instead of the native fast path — for enroll OR revoke. Use it when you need a surface native doesn't do (Ory operator-IdP, an env with no native config, observer/prod Zenoh flavours) or to fall back deliberately. Repair always uses the playbook regardless |  |
| `--repo-root <REPO_ROOT>` | Override repo root (defaults to searching upward from cwd for a `.git` directory) |  |
| `--collab <COLLAB>` | With `--revoke`: record what happens to this person's CLOUD COLLABORATION account (Outline / files / chat behind Dex+Kanidm) — `keep` or `revoke`. That plane is a separate identity store the mesh revoke deliberately does not cut, so the choice is a decision this command RECORDS (receipt line + audit event), not a cut it performs: goat-cli holds no collab credentials, and minting a second client + credential path for them is exactly what the provisioner seam avoids. `--collab revoke` prints the one command that does cut it (`onboard-runner -collab revoke`). Omit the flag and the receipt says `undecided` — honest, and the reconcile will keep surfacing the person until someone chooses. Native-path only. See F-274 + docs/design/credential-revocation-and-offboarding.md |  |
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
| `--org <ORG>` | ORG-scope rotation (cascade). Instead of one selector, re-issue the leaf cert of EVERY active member of the affiliation org `<id>` (the `affiliation.id`) in place — the rotate sibling of the `onboard --org --revoke` cascade, at the same granularity. Resolves the members from the declared registry (`docs/operators/roster.json`) and resolves each to its slot anchor via the alias map; members with no resolvable anchor are reported and skipped (fail-visible, never silent). Prints the blast radius and asks to confirm (skip with `--yes`). Mutually exclusive with a positional selector. Identity grammar / ADR 1044 (`docs/design/credential-rotation.md`) |  |
| `--yes` | Skip the interactive confirmation prompt for `--org` cascades. The blast radius is still printed |  |
| `--repo-root <REPO_ROOT>` | Repo root holding `docs/operators/roster.json` for `--org` resolution. Defaults to the nearest `.git` ancestor of the cwd |  |
| `--dry-run` | Don't actually POST; resolve the selector(s) + print the intended request(s) + exit 0 |  |
| `--key-only` | Reissue ONLY the portal key, leaving the mTLS cert untouched (goat#1193). The lighter, safer path for the recurring case: someone lost / mistyped / stale-pasted their portal key but their cert is fine and they are still publishing. Swaps the key in place with zero cert churn — no bundle, no download, no peer restart. Without it, `rotate` re-mints the cert too. Single-anchor only, not the `--org` cascade |  |

## `goat admin sponsorship`

Grant or revoke an identity's sponsorship authorization (ADR 1071) — whether it may mint candidate-onboarding invites and vouch at the stage-2 gate, and up to which tier. The case-by-case lever for identities that didn't get sponsorship by tier default at onboarding (iam-admin / platform-admin get it automatically). Pure-portal POST /api/orgs/{id}/sponsorship, operator-gated server-side; accepts an anchor or the human handle

**Arguments**

| Argument | Description |
|---|---|
| `<ID>` | Anchor (slot_id) or human handle of the identity |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--max-tier <MAX_TIER>` | Grant sponsorship up to this tier (browse \| developer \| iam-admin \| platform-admin) |  |
| `--email <EMAIL>` | Bind the sponsor's Ory login email so they can use the id-admin-console onboarding page (sponsor-scoped: they see and vouch only their own invitees there). Optional — without it the sponsor still mints/vouches via CLI/API with their own portal key |  |
| `--revoke` | Revoke sponsorship authorization instead of granting it |  |

## `goat admin lan`

LAN troubleshooting toolkit for relocated goat mgmt hosts that don't appear on the WAN: `scan` finds them on the local LAN by MAC even when DHCP moved their IP; `doctor` runs a layered L0–L4 triage. Depends on NO goatnet/mesh service — works when the mesh does not. See docs/design/goat-cli.md §"goat admin lan"

**Subcommands**

- [`goat admin lan scan`](#goat-admin-lan-scan) — Find goat mgmt hosts on the local LAN even when DHCP moved their IP: sweep each local /24 (TCP 22+443, 200 ms) to warm the ARP cache, then match ARP-table MACs against the host registry (~/.goat/lan-hosts.yaml)
- [`goat admin lan doctor`](#goat-admin-lan-doctor) — Layered triage of a relocated host: L0 local subnet plausibility, L1 ping + TCP reachability, L2 sshd banner health (wedged-sshd detection), L3 mesh-free service-plane probes (443 + :9090 /metrics), L4 on-host checks over one BatchMode ssh (clock / chrony-presync / default route / WAN / netbird / docker)

## `goat admin lan scan`

Find goat mgmt hosts on the local LAN even when DHCP moved their IP: sweep each local /24 (TCP 22+443, 200 ms) to warm the ARP cache, then match ARP-table MACs against the host registry (~/.goat/lan-hosts.yaml)

**Options**

| Flag | Description | Default |
|---|---|---|
| `--json` | Emit one JSON object instead of the pretty table |  |

## `goat admin lan doctor`

Layered triage of a relocated host: L0 local subnet plausibility, L1 ping + TCP reachability, L2 sshd banner health (wedged-sshd detection), L3 mesh-free service-plane probes (443 + :9090 /metrics), L4 on-host checks over one BatchMode ssh (clock / chrony-presync / default route / WAN / netbird / docker)

**Arguments**

| Argument | Description |
|---|---|
| `<TARGET>` | Registry host name (e.g. `mgmt-a-01`) or a literal IPv4 address. Omit to triage every registry host `scan` finds |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--ssh-user <SSH_USER>` | SSH user for the L4 on-host probe (default: the registry entry's ssh_user, else the context's `mgmt_ssh_user`) |  |
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

Mesh-peer lifecycle — the authoritative mesh-first cut. `ls` lists mesh peers; `cut` ejects one (deletes its NetBird peer, so its WireGuard tunnel drops). On a brokerless slot-pool fabric the Zenoh write-ACL is pre-baked for the whole pool, so deleting an org record does NOT stop a flooder — removing the *peer* does (a cert is inert without a mesh path). This is the one-command emergency cut: `goat health` finds the hog, `goat admin peer cut` ejects it. Needs the NetBird admin token in the environment ($<CONTEXT>_NB_API_TOKEN, e.g. ZAGROS_NB_API_TOKEN, or $GOAT_NB_API_TOKEN)

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

## `goat admin platform-admin`

Make someone a platform admin on a goatnet, or stop them being one, on every plane at once: Ory login group, NetBird dashboard role, their peers' mesh groups, host SSH keys, operator key, sponsorship, and the 1Password Platform group. `status` reads; `grant` / `revoke` plan and, with --apply, execute and re-read. Revoke demotes to developer by default; --full offboards. Canon: docs/design/platform-admin-lifecycle.md

**Subcommands**

- [`goat admin platform-admin status`](#goat-admin-platform-admin-status) — Read-only: what platform-admin access this person holds on this goatnet, plane by plane. Changes nothing
- [`goat admin platform-admin grant`](#goat-admin-platform-admin-grant) — Make the person a platform admin on this goatnet: Ory PLATFORM-ADMIN, NetBird dashboard admin, peers in <PREFIX>-PLATFORM-ADMIN, an operator key (requested and approved on their behalf), sponsorship up to platform-admin, their recorded SSH keys on the admin hosts, the 1Password Platform group. The person must already have an identity (bring new people on with `goat admin invite`). Dry-run unless --apply
- [`goat admin platform-admin revoke`](#goat-admin-platform-admin-revoke) — Stop the person being a platform admin on this goatnet. Default is a DEMOTE to developer, which keeps their login and fabric access and is reversible with `grant`. Removes: SSH keys from every host, operator key(s), sponsorship, PLATFORM-ADMIN from peers, Ory login group and NetBird dashboard role, 1Password Platform group. Dry-run unless --apply

## `goat admin platform-admin status`

Read-only: what platform-admin access this person holds on this goatnet, plane by plane. Changes nothing

**Arguments**

| Argument | Description |
|---|---|
| `<EMAIL>` | The person's login email |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | Goatnet (e.g. `zagros`, `zagros-prod`). Must be the goatnet the active context belongs to — run once per goatnet with `--context <env>-op` |  |
| `--peer <PEERS>` | Extra peer selector for the mesh leg (peer id, mesh IP, or name substring) when a device cannot be matched to the person automatically. Repeatable |  |
| `--no-1password` | Skip the 1Password plane (it is team-wide, not per goatnet — skip it on the second and later goatnet of a sweep, or when `op` is not signed in) |  |
| `--json` | Emit the report as JSON |  |

## `goat admin platform-admin grant`

Make the person a platform admin on this goatnet: Ory PLATFORM-ADMIN, NetBird dashboard admin, peers in <PREFIX>-PLATFORM-ADMIN, an operator key (requested and approved on their behalf), sponsorship up to platform-admin, their recorded SSH keys on the admin hosts, the 1Password Platform group. The person must already have an identity (bring new people on with `goat admin invite`). Dry-run unless --apply

**Arguments**

| Argument | Description |
|---|---|
| `<EMAIL>` | The person's login email |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | Goatnet (e.g. `zagros`, `zagros-prod`). Must be the goatnet the active context belongs to — run once per goatnet with `--context <env>-op` |  |
| `--peer <PEERS>` | Extra peer selector for the mesh leg (peer id, mesh IP, or name substring) when a device cannot be matched to the person automatically. Repeatable |  |
| `--no-1password` | Skip the 1Password plane (it is team-wide, not per goatnet — skip it on the second and later goatnet of a sweep, or when `op` is not signed in) |  |
| `--json` | Emit the report as JSON |  |
| `--pubkey-file <PUBKEY_FILES>` | Also record this public key file for the person and install it. Repeatable. Without it, only keys the person already submitted in the console are installed |  |
| `--apply` | Execute. Without it: plan only, nothing touched |  |
| `--write-entry` | On a goatnet governed by warrant, write the mesh leg's entries into `ops/warrant/membership/<env>/` (in place where a peer already has an entry, else `people.yaml`) for a PR. Needs a checkout |  |

## `goat admin platform-admin revoke`

Stop the person being a platform admin on this goatnet. Default is a DEMOTE to developer, which keeps their login and fabric access and is reversible with `grant`. Removes: SSH keys from every host, operator key(s), sponsorship, PLATFORM-ADMIN from peers, Ory login group and NetBird dashboard role, 1Password Platform group. Dry-run unless --apply

**Arguments**

| Argument | Description |
|---|---|
| `<EMAIL>` | The person's login email |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env <ENV>` | Goatnet (e.g. `zagros`, `zagros-prod`). Must be the goatnet the active context belongs to — run once per goatnet with `--context <env>-op` |  |
| `--peer <PEERS>` | Extra peer selector for the mesh leg (peer id, mesh IP, or name substring) when a device cannot be matched to the person automatically. Repeatable |  |
| `--no-1password` | Skip the 1Password plane (it is team-wide, not per goatnet — skip it on the second and later goatnet of a sweep, or when `op` is not signed in) |  |
| `--json` | Emit the report as JSON |  |
| `--full` | Offboard instead of demote: also deactivate the Ory login, block the NetBird user, and strip the person's peers from every group except `All`. Still reversible (reactivate + re-add); nothing is deleted |  |
| `--pubkey-file <PUBKEY_FILES>` | Also remove this public key file's key from the hosts. Repeatable |  |
| `--github <GITHUB>` | GitHub login whose current public keys must be removed from hosts |  |
| `--no-github-keys` | Do not also remove the public keys GitHub lists for the person's recorded GitHub account. Included by default because hand-installed keys predate the console and are often exactly those keys |  |
| `--no-ory` | Leave the Ory plane to a surrounding lifecycle command. Intended for offboard.sh, which runs onboard --revoke without notification and verifies Ory independently afterwards |  |
| `--apply` | Execute. Without it: plan only, nothing touched |  |
| `--write-entry` | On a goatnet governed by warrant, write the mesh leg's entries into `ops/warrant/membership/<env>/` (in place where a peer already has an entry, else `people.yaml`) for a PR. Needs a checkout |  |

## `goat admin members`

Membership transfer engine (ADR 1088): move entities between canonical and satellite groups (probation, quarantine, exercise sets) — audited, coverage-gated, mirror-first, idempotent; dry-run by default

**Subcommands**

- [`goat admin members move`](#goat-admin-members-move) — Move members between mesh groups (shed / absorb) — dry-run by default

## `goat admin members move`

Move members between mesh groups (shed / absorb) — dry-run by default.

Selects the peers currently in --from (all of them, or the subset named by --handle / --list / --filter), mirror-adds them to --to, verifies, then removes them from --from. Group names are suffix-resolved against the target goatnet's prefix (`DEVELOPER` → `ZAGROS-DEVELOPER`); a full group name is accepted verbatim.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--from <FROM>` | Source group (suffix or full name) |  |
| `--to <TO>` | Destination group (suffix or full name). Must be covered by at least one enabled policy (fail-closed) — stamp the matrix rule first if it isn't |  |
| `--handle <HANDLE>` | Select a single member: peer id, mesh IP, or a name/hostname substring (same matching as `goat admin peer cut`). Also the handle used for the Ory-plane leg when --planes includes ory or a marker flag is set |  |
| `--list <LIST>` | File with one selector per line (blank lines + `#` comments ok) |  |
| `--filter <FILTER>` | Comma-joined predicates over the --from members: `last_seen>14d` (dormant longer than N days; never-seen counts as dormant), `connected=true\|false` |  |
| `--planes <PLANES>` | Which plane(s) to touch: `mesh` (default), `ory`, `both`. The ory plane swaps --from/--to VERBATIM inside the identity's Ory `traits.groups` — Ory vocabulary only; mesh role names never enter Ory traits | `mesh` |
| `--strip-others` | After the mirror-add verifies, also remove the movers from EVERY other group (except the auto `All` and the destination). The quarantine-entry posture (ADR 1091): strip roles, P2P, labels |  |
| `--set-marker <SET_MARKER>` | Set a boolean Ory trait marker (e.g. `quarantine`, `probation`) on the selected identities. Requires the Kratos schema to carry the trait — fails loudly naming the schema if it doesn't |  |
| `--clear-marker <CLEAR_MARKER>` | Clear (remove) a boolean Ory trait marker from the selected identities |  |
| `--apply` | Execute the plan. Without this flag the run is a dry-run: plan printed, nothing touched, no audit emitted |  |
| `--env <ENV>` | Target goatnet (defaults to the active profile context, e.g. `zagros`). Site prefix for suffix resolution = uppercased env |  |
| `--mgmt-url <MGMT_URL>` | Override the NetBird mgmt URL (else the profile's) |  |
| `--json` | Emit the plan/result as JSON |  |
| `--write-entry` | On a scope governed by warrant (its `ops/warrant/membership/<scope>/_scope.yaml` says `converge: enforce`), add the entry that expresses this move to the scope directory instead of changing the mesh: edited in place where the peer already has an entry, else into `holds.yaml` / `services.yaml` / `people.yaml` by kind. Commit it and open a PR; the converger applies it |  |
| `--owner <OWNER>` | `owner:` of the written/printed entry (default: your operator handle) |  |
| `--reason <REASON>` | `reason:` of the written/printed entry (≤200 chars) |  |
| `--expires <EXPIRES>` | `expires:` (YYYY-MM-DD) of the written/printed entry. Default: 7 days for a quarantine hold, 90 days for P2P / PLATFORM-ADMIN / BREAKGLASS / EXTERNAL-INFRA, none otherwise |  |

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
| `--env <ENV>` | Goatnet the manifest belongs to (e.g. `zagros`). Passed to the playbook as `-e operator_env=<env>` |  |
| `--repo-root <REPO_ROOT>` | Override repo root (defaults to the nearest `.git` ancestor of cwd) |  |
| `--dry-run` | Don't run ansible — print the command + exit 0 |  |

## `goat admin pqc`

Toggle PQC on an ALREADY-ENROLLED peer at BOTH layers — the Zenoh kx-mode state file (F-259 Tier 1 kill-switch, `/var/lib/goat/pqc-mode`; new processes latch it, long-running surfaces flip on restart) and intra-mesh Rosenpass (ADR 0602) — no setup key, no re-mint. The rosenpass leg runs `netbird down` then `netbird up` with the explicit flag (`--enable-rosenpass --rosenpass-permissive` for `on`, `--enable-rosenpass=false` for `off`). Both the `down` and the explicit-`false` matter: a bare `up` no-ops on a connected peer AND persists the prior setting, so it silently changes nothing. The toggle briefly drops + re-establishes the tunnel. Permissive means `on` never breaks connectivity to not-yet-enabled peers, and PQC only actually engages between two Rosenpass-enabled peers — so a mixed fleet is safe.

This is the OTHER-peer surface — admin-gated (needs the NetBird API token plus SSH). To toggle THIS host, use `goat client pqc` instead (self-service, no admin token). Here you MUST pass `--peer` or `--fleet`. With `--peer <sel>` it SSHes to a mesh peer (the one-command "PQC-ify everything hosted on a mgmt-host" — e.g. Thor's own peer PQ-secures every service riding it). With `--fleet` it prints the per-peer plan for the whole mesh (each peer holds its OWN setting, so a fleet flip is a per-peer re-up, not an instant switch; the durable path is the `rosenpass_enabled` ansible group_var + re-onboard). See docs/design/rosenpass-intra-mesh-pqc.md.

**Arguments**

| Argument | Description |
|---|---|
| `<STATE>` | `on` (enable Rosenpass) or `off` (revert to classical WireGuard) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--peer <PEER>` | Toggle a remote mesh peer over SSH instead of this host. Selector is an exact peer id, exact mesh IP, or a case-insensitive name/hostname substring (as shown by `goat admin peer ls`). Refuses if it matches more than one peer. SSHes `<ssh-user>@<mesh-ip>` and runs the netbird command with `sudo` |  |
| `--fleet` | Print the per-peer plan for every mesh peer (the honest "sweep": PQC is per-peer, so this emits the command to run on each rather than fanning SSH into heterogeneous peers). Combine with `--json` for machine-readable output |  |
| `--ssh-user <SSH_USER>` | SSH user for `--peer` (default: the context's `mgmt_ssh_user`, written by `goat context refresh`) |  |
| `--mgmt-url <MGMT_URL>` | Override the NetBird mgmt URL for peer resolution (else env / profile) |  |
| `--yes, -y` | Skip the confirmation prompt (`--peer` mutates the remote mesh client) |  |
| `--dry-run` | Resolve + print what WOULD run, change nothing, exit 0 |  |
| `--json` | Emit JSON instead of the human summary |  |

## `goat admin storm`

Control-plane storm resilience (ADR 1081). Today: `storm brake` — the MANUAL operator brake fired when the relay-health host-net band shows a signaling storm (futility ratio ≥0.5 at ≥100 msgs/min). It flushes the signal container, tightens L2 limits (if deployed), sheds declared non-essential publishers, and records one audited operator action. Reaches the mgmt host over SSH (host+user from the goat context). NO automation / timers — auto-engage is a separately-gated follow-on. See docs/operations/storm-brake-runbook.md

**Subcommands**

- [`goat admin storm brake`](#goat-admin-storm-brake) — Fire the manual control-plane storm brake (ADR 1081 Layer 4): flush the signal container, tighten L2 limits (if deployed), shed declared non-essential publishers, and record one audited operator action

## `goat admin storm brake`

Fire the manual control-plane storm brake (ADR 1081 Layer 4): flush the signal container, tighten L2 limits (if deployed), shed declared non-essential publishers, and record one audited operator action.

Fire it when the relay-health host-net band shows a signaling futility ratio ≥0.5 at ≥100 msgs/min (and/or uplink loss ≥5% sustained). The goatnet is the active context (`--context` / `$GOAT_CONTEXT`). Preview with `--dry-run`; skip the confirmation with `--yes`.

NOT a cure on its own — M-085: a signal flush alone does not restore the mesh. Layers 1-2 (client backoff + signal shedding) are the fix; the brake buys recovery time. See docs/operations/storm-brake-runbook.md.

**Options**

| Flag | Description | Default |
|---|---|---|
| `--dry-run` | Print the plan + trigger-signal guidance and exit WITHOUT executing anything. Safe to run any time to learn the sequence |  |
| `--yes` | Skip the interactive confirmation prompt (for scripted/non-interactive use). The plan is still printed first |  |
| `--signal-container <SIGNAL_CONTAINER>` | Signal container / compose service name on the mgmt host | `signal-a` |
| `--shed-unit <SHED_UNIT>` | A systemd unit to pause during SHED (repeatable). Augments the `storm_brake_shed_units` list in the context profile.toml |  |
| `--mgmt-host <MGMT_HOST>` | Override the mgmt-host SSH host. Precedence: this flag > `$GOAT_MGMT_SSH_HOST` > `$<CONTEXT>_MGMT_SSH_HOST` > `mgmt_ssh_host` in the context profile.toml |  |
| `--mgmt-user <MGMT_USER>` | Override the mgmt-host SSH user. Precedence: this flag > `$GOAT_MGMT_SSH_USER` > `mgmt_ssh_user` in profile.toml (goatnet.toml) |  |
| `--compose-dir <COMPOSE_DIR>` | mgmt-host compose directory (holds docker-compose.yml + .env) the TIGHTEN recreate runs from | `/opt/mgmt-host` |
| `--tighten-window-min <TIGHTEN_WINDOW_MIN>` | Advisory bounded-window length (minutes) shown for the TIGHTEN step. Manual phase has no timer — the operator reverts (the step prints how) | `30` |

## `goat status`

Health synthesis: router port reachability + a real mTLS Zenoh session open + portal /api/health + local cert expiry + name, PQC and device state, one glyph line per check or one --json object.

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
- [`goat client pqc`](#goat-client-pqc) — Toggle post-quantum protection on THIS host at BOTH layers — self-service, no admin token (you're just reconfiguring your own host). Writes the Zenoh kx-mode state file (F-259 Tier 1 kill-switch: new goat/zenoh processes latch it; restart long-running surfaces), then flips Rosenpass (ADR 0602): `on` runs `netbird down` then re-ups with the Rosenpass flags; `off` re-ups with `--enable-rosenpass=false` (a bare re-up persists the prior setting, so the disable must be explicit). Briefly bounces the tunnel; reads back the verified `netbird status` line. To toggle OTHER peers, use `goat admin pqc --peer\|--fleet`

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

## `goat client pqc`

Toggle post-quantum protection on THIS host at BOTH layers — self-service, no admin token (you're just reconfiguring your own host). Writes the Zenoh kx-mode state file (F-259 Tier 1 kill-switch: new goat/zenoh processes latch it; restart long-running surfaces), then flips Rosenpass (ADR 0602): `on` runs `netbird down` then re-ups with the Rosenpass flags; `off` re-ups with `--enable-rosenpass=false` (a bare re-up persists the prior setting, so the disable must be explicit). Briefly bounces the tunnel; reads back the verified `netbird status` line. To toggle OTHER peers, use `goat admin pqc --peer|--fleet`

**Arguments**

| Argument | Description |
|---|---|
| `<STATE>` | `on` (enable Rosenpass) or `off` (revert to classical WireGuard) |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--netbird-bin <NETBIRD_BIN>` | netbird binary for the local toggle (PATH lookup or absolute) | `netbird` |
| `--yes, -y` | Skip the confirmation prompt (the toggle briefly bounces the tunnel) |  |
| `--dry-run` | Resolve + print what WOULD run, change nothing, exit 0 |  |
| `--json` | Emit JSON instead of the human summary |  |

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

