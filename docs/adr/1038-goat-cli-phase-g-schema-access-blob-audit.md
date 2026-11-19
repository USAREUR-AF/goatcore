# ADR 1038 — goat-cli Phase G: close the declare / diagnose / bulk-transfer gaps (schema, access-trace, blob, audit-tail read)

**Status:** Proposed (2026-06-09)
**Repo scope:** goat core (`goat-cli` crate: new `schema` / `access` / `blob` command groups, `pub --volatile-stream`, `topic export|import`, the audit-tail read path that unblocks `narrate` / `sub --since` / `replay`)
**Related:** [ADR 1019](1019-goat-cli-unified-operator-binary.md) (the unified operator binary this extends — Phase G follows F) · [ADR 1026](1026-data-access-common-operating-picture.md) (the Data-Access COP whose `goat access` CLI half this builds) · [ADR 1025](1025-zenoh-dynamic-tenant-authz-slot-pool.md) / F-181 (the write-slot model `goat access why` traces) · [ADR 1022](1022-zenoh-denial-observability.md) / F-177 (the silent-denial problem `goat access` + the write-precheck answer operator-side) · [ADR 1032](1032-zenoh-volatile-non-persisted-namespace-class.md) (the `volatile/` class `goat blob` + `pub --volatile-stream` make ergonomic) · [ADR 0811](0811-audit-log-integrity.md) (the Object-Lock audit bucket the read path reaches) · `goat-cli.md` §"Phase G" (the design) · [`pubsub-best-practices.md`](../design/pubsub-best-practices.md) (the schema + chunking doctrine being lifted into the CLI).

## Plain-language summary

A review of everything a participant must do to send and receive data on the fabric — register a schema,
declare a topic, publish, confirm the write landed, subscribe, decode, and monitor health — against the
`goat` binary found the **read / observe** side well covered (`doctor`, `health`, `inspect`,
`pub --confirm`) but the **declare** side and the **why-did-this-fail** side under-exposed relative to what
the portal and the design docs already support. Specifically: schema handling is a second-class citizen
folded into one `topic create --schema-file` flag (the portal has full versioned schema CRUD the CLI never
reaches, and nobody compiles `.proto` → `.fds` for the operator); there is no CLI for the Data-Access COP
(`goat access who|topic|why`) even though it is the direct answer to the fabric's defining footgun — writes
fail *silently*; the `volatile/` blob-transfer chunk/manifest/sha256 dance is hand-rolled in every
publisher; and `goat narrate` standalone, `goat sub --since`, and a future `goat replay` are all blocked on
the same missing piece — the operator profile carries no way to read the audit-tail. This ADR takes on
**Phase G** to close those gaps, and settles the one genuinely architectural question among them: **how an
operator reads the audit-tail** — portal-minted presigned URLs, not MinIO credentials in the bundle.

## Context

`goat-cli` shipped Phases A–F ([ADR 1019](1019-goat-cli-unified-operator-binary.md)): profile, doctor,
vendor/topic browse + CRUD, pub/sub, compose, narrate (inline), inspect, status, services, client, admin,
plugin, plus `health` (the fabric-health consumer from [ADR 1034](1034-fabric-health-surface.md)). What an
operator can already do well is **get on, diagnose reachability, browse, publish-with-confirm, subscribe,
and watch fabric weather**.

The gaps, each backed by a server-side surface that already exists:

1. **Schema is second-class.** The portal exposes full versioned schema CRUD — `POST/GET/DELETE
   /api/vendors/{v}/topics/{id}/schemas`, with `version`, `compatibility`, `sha256`, `size_bytes` — but the
   only CLI path is `topic create --schema-file`. There is no way to upload a new schema version to an
   existing topic, list versions, view a body, or diff two versions from the CLI. Worse, the portal wants a
   `.fds` (protobuf `FileDescriptorSet`) and operators hold `.proto`; **nobody runs `protoc`** for them, so
   even the one supported path has a manual compile step.
2. **No `goat access`.** [ADR 1026](1026-data-access-common-operating-picture.md) designed the Data-Access
   COP with a `goat access` CLI half; the web half exists, the CLI does not (`access` appears only as a
   string in `tutor.rs`; there is no `cmd/access.rs`). This is the direct operator answer to F-177: a `put`
   succeeds locally even when the router denies it at ingress, and today the only feedback is
   `pub --confirm` *after the fact*. `goat access why me <key>` would trace the allow/deny across mesh → Ory
   → Zenoh ACL → release-set, and a `doctor --topic <key>` write-precheck would catch the denial *before*
   publishing.
3. **Volatile blob transfer is hand-rolled.** [ADR 1032](1032-zenoh-volatile-non-persisted-namespace-class.md)
   reserved `volatile/<peer>/blob/**` (chunked ≤100 MiB, BLOCK congestion, manifest + sha256) and
   `volatile/<peer>/stream/**` (DROP congestion, fire-and-forget). Both are implemented entirely in each
   publisher's own code — the exact chunk/reassemble/verify logic that produces the "12 MB published clean
   but `get` returns nothing" trap documented in [`pubsub-best-practices.md`](../design/pubsub-best-practices.md).
4. **The audit-tail is unreadable from the CLI.** `goat narrate` (standalone), `goat sub --since`, and a
   future `goat replay` all need to read the Object-Lock audit bucket ([ADR 0811](0811-audit-log-integrity.md)).
   The operator profile carries an mTLS cert + a portal token but **no MinIO read path**. This is the open
   "D.D" decision named in `goat-cli.md`: (a) MinIO creds in the operator bundle,
   (b) portal proxies S3 GETs, (c) portal mints presigned URLs on demand. Until it is settled, three
   features stay stubbed behind a "D.D-gated" error.

A fifth, smaller gap: the portal's `/topics/export` + `/topics/import` YAML endpoints exist but have no CLI
wrapper, so the only state-as-code on-ramp is the separate `pubsub-apply` Go tool. And the
`goat-cli.md` command tree has drifted — it predates the shipped `health`, `init`,
`tutor`, `hello`, `ui`, `docs`, `status`, `services`, `client`, and `plugin` commands.

## Decision

**Take on Phase G** as the close-out of the send/receive lifecycle gaps, in six independently-shippable
increments (detailed in `goat-cli.md` §"Phase G"):

- **G.1 — doc reconcile.** Bring the `goat-cli.md` command tree + status line back in sync with the binary.
  Done first so Phase G is added to an accurate baseline. Pure docs.
- **G.2 — `goat topic export | import`.** Thin wrappers over the existing portal `/topics/export` +
  `/topics/import` YAML endpoints. The cheap state-as-code on-ramp; complements, does not replace,
  `pubsub-apply` (which stays the canonical declarative reconciler).
- **G.3 — `goat schema {upload, ls, show, diff}`.** First-class schema lifecycle over the existing
  portal schema CRUD. `upload` posts a new version to an existing topic (a `.proto` source or a `.fds`,
  selected by extension or `--format`); `ls`/`show`/`diff` read versions, bodies, and compatibility.
  `topic create --schema-file` keeps working as the create-time on-ramp.

  **Amendment (2026-06-09, at build time).** The original draft listed a fifth subcommand, `compile`, to
  run `protoc`/`prost` client-side so the operator never shells out. Dropped during G.3 implementation:
  the portal's `uploadSchema` endpoint already compiles `.proto` source in-process when
  `format=protobuf-source` (bufbuild/protocompile, closing F-158). So `goat schema upload foo.proto`
  ships the source and the portal builds the `FileDescriptorSet` — embedding a protobuf compiler in the
  Rust binary would duplicate the server-side path for no gain and add a heavy build dependency. The
  multi-file vendor-source case keeps the existing `build-fds.sh` → `.fds` path. (One client-side dependency
  change did land: `panoscope-core`'s `reqwest` gains the `multipart` feature, since upload is a multipart
  POST.)
- **G.4 — `goat access who | topic | why`** + **`goat doctor --topic <key>`** write-precheck. The CLI half
  of the Data-Access COP ([ADR 1026](1026-data-access-common-operating-picture.md), Phase 1: read-only) —
  forward query (what an identity may write/read), reverse query (who may touch a topic), and a single
  allow/deny trace across the four layers. The precheck reuses the `why` trace to answer "can I write here?"
  before a publish, turning F-177's silent denial into a pre-flight verdict.

  **Amendment (2026-06-09, at build time).** The Consequences section below called G.4 "CLI wiring over
  portal endpoints that already exist." That was wrong — no `/api/access` endpoints existed and the
  Data-Access COP was design-only server-side (F-187).
  G.4 is re-scoped as **Full COP Phase 1**: the portal-side graph assembly + `who/topic/why` query API
  (built over the existing `netbirdpoll` / `kratosadmin` / `registry` / `ops/operators` readers, joined on
  the vendor-prefix spine) *then* the Rust CLI on top — three increments: PR-A (graph + `who/topic` read
  API), PR-B (`why` trace), PR-C (`goat access` CLI + `doctor --topic`). Build plan in
  [`data-access-cop.md` §"Build plan (Phase 1 / G.4)"](../design/data-access-cop.md). The "wraps surfaces
  that already exist" premise holds for G.2/G.3-upload but **not** G.4 — check it per-increment.
- **G.5 — `goat blob send | get`** + **`goat pub --volatile-stream`.** `blob` owns the chunk (≤1.5 MiB) →
  BLOCK-publish → manifest → reassemble → sha256-verify cycle for `volatile/<peer>/blob/**`, capped at the
  ADR 1032 100 MiB ceiling (above it: refuse, point at the external-pointer pattern). `--volatile-stream`
  sets DROP congestion control for the loss-tolerant `volatile/<peer>/stream/**` firehose.
- **G.6 — audit-tail read unblock.** **The operator reads the audit-tail via portal-minted presigned URLs.**
  The portal gains a `GET /api/audit/presign?key=<expr>&since=<t>` endpoint that, authorized by the
  operator's existing Bearer token, returns short-TTL presigned S3 GET URLs for the matching audit objects;
  `goat` fetches the NDJSON directly from MinIO over those URLs via `panoscope_core::AuditTail`. This
  unblocks `goat narrate` (standalone), `goat sub --since`, and `goat replay` (a new G.6 surface) in one
  move.

Each increment lands its own PR, ADR-line in the `goat-cli.md` phasing table, and `in-flight.md` update.
The machine-readable contract rules from [ADR 1019](1019-goat-cli-unified-operator-binary.md) §Conventions
(stable `--json` schema, error codes, `--dry-run` on mutators, locale-invariant codes per
[ADR 1024](1024-goat-cli-internationalization.md)) apply to every new command.

### Why presigned URLs for audit read (G.6)

The audit-tail read shape is the one architectural choice here; the rest follow existing surfaces.

- **MinIO creds in the bundle** — rejected. Puts standing object-store credentials on every operator laptop,
  widens the blast radius of a lost bundle to the whole audit bucket, and contradicts the revocation model
  ([ADR 1031](1031-credential-revocation-mesh-first-offboarding.md)) where cutting a principal should not
  require rotating a shared MinIO key. The profile-storage posture (`goat-cli.md`)
  deliberately keeps the profile to cert + token at mode 0600; a long-lived S3 key is a different tier.
- **Portal proxies every S3 GET** — rejected as the default. Routes potentially large NDJSON payloads
  through the portal process, making it a throughput bottleneck and a memory-pressure surface for a job
  (bulk replay) that is inherently large. Acceptable as a fallback for environments where direct
  laptop→MinIO reachability is absent, but not the primary path.
- **Portal mints presigned URLs** — chosen. The operator already holds a portal Bearer token; the portal
  already knows the audit bucket and can scope a presign to exactly the requested key/time window with a
  short TTL. No standing object-store credential leaves the server, revocation stays mesh-first (revoke the
  token → no new presigns), and the bytes flow laptop→MinIO directly so the portal is not in the data path.
  It is the same "server authorizes, client fetches" shape as the rest of goat-cli.

## Metadata-only audit read clarification (2026-11-06)

Ordinary audit reads use the metadata projection defined by ADR 0811's fabric
record scope and the [subscriber contract](../design/zenoh-audit-external-subscriber.md#metadata-and-replay-separation-2026-10-26).
This supersedes the direct raw-audit-object download described in G.6 above.
The operator-authenticated portal lists relative `/api/audit/object` references,
reads its fixed storage bucket over verified HTTPS, and returns allowlisted
metadata without message bodies. The laptop uses the portal's HTTPS endpoint;
direct access to the storage service on port 9000 is not required. This applies
the proxy fallback already allowed by this ADR to ordinary metadata reads.

The existing `replay` and historical `sub` commands select this path with
`--metadata`; standalone `narrate` and the audit tier of `flow` use metadata
directly. Payload replay remains gated pending its separate governance design.
Key filtering remains a client-side selection within operator-authorized audit
history, not a pod-owner authorization boundary. A record establishes that the
audit subscriber observed the sample; it does not establish processing by an
application or authenticated publisher identity. The `volatile/**` exclusion and
its lack of historical confirmation remain unchanged.

This is branch implementation and local validation, not live release evidence.
Compatible releases and the non-admin mesh acceptance check are documented in
the [cutover procedure](../operations/audit-replay-cutover.md).

## Consequences

- **The declare and diagnose halves of the lifecycle reach parity with the read/observe half.** Schema
  becomes a first-class noun; the silent-denial footgun gains both an after-the-fact (`pub --confirm`,
  already shipped) and a before-the-fact (`access why` / `doctor --topic`) answer.
- **G.2, G.3, G.4 are CLI-wiring work, not new substrate** — they wrap portal endpoints and live layers that
  already exist, so they carry low substrate risk and can land quickly.
- **G.6 adds one new portal endpoint** (`/api/audit/presign`) and the IAM policy to allow the portal's role
  to presign GETs on the audit bucket. This is the only server-side addition in Phase G and the only piece
  that touches the audit-integrity boundary; it is read-only and presign-scoped, so it cannot weaken the
  Object-Lock write/retention guarantees of [ADR 0811](0811-audit-log-integrity.md).
- **`goat blob` makes a 100 MiB transport ceiling a CLI-enforced contract** rather than a doctrine operators
  must remember — the failure mode (silent storage drop above ~2 MiB on persisted prefixes) is replaced by a
  loud refuse-with-remediation.
- **No change to the canonical declarative path.** `pubsub-apply` + `pubsubstate.yaml` remain the source of
  truth for state-as-code; `goat topic export|import` and `goat schema upload` are the *interactive* surface,
  same split [ADR 1019](1019-goat-cli-unified-operator-binary.md) already drew for `topic create`.
- **Forward-only.** Existing topics/schemas are not retroactively re-compiled or re-declared; Phase G changes
  what new operations *can* do, consistent with the goat-cli forward-only stance.

## Alternatives considered

- **Leave schema folded into `topic create`** — rejected. The portal already models schemas as versioned,
  compatibility-graded, content-addressed objects; collapsing that into one create-time flag throws away the
  version/diff/compatibility surface operators need when a wire format evolves (the `vN` bump doctrine).
- **Ship `goat access` as web-only** (status quo) — rejected. The COP web view answers "what does access
  look like"; the operator debugging a silently-dropped publish is in a shell, mid-incident, and needs the
  same trace at the prompt. The trace logic is server-readable from the same four layers either way.
- **A `goat-blob` PATH extension instead of a built-in** — rejected. Chunk/manifest/sha256 reassembly is
  core enough to the `volatile/` contract ([ADR 1032](1032-zenoh-volatile-non-persisted-namespace-class.md))
  that it should be uniform across operators, not a per-vendor binary; the kubectl-style extension surface is
  for vendor-specific add-ons, not core fabric operations.
- **Defer G.6 indefinitely** (keep narrate/replay stubbed) — rejected. The audit-tail is the substrate's most
  expensive investment ([ADR 0811](0811-audit-log-integrity.md) Object-Lock retention); leaving its
  reader-side features stubbed wastes that investment, and the presigned-URL shape is low-risk enough to
  settle now rather than carry as an open question through three more features.
