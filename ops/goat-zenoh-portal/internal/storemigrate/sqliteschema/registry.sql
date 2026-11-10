-- goat-zenoh-portal SQLite schema.
--
-- Idempotent: every statement is CREATE ... IF NOT EXISTS. Re-runnable
-- at every startup. Schema evolution lands as additive new statements
-- ("ALTER TABLE ... ADD COLUMN") with NULL-friendly defaults; never
-- destructive in trial scope.
--
-- ADR 0413: raw SQL, no ORM.

CREATE TABLE IF NOT EXISTS vendors (
    -- Vendor prefix is the primary key. It IS the identity ("acme",
    -- "example-co", "community"). Lowercase ASCII + digits + '-' only,
    -- enforced at the application layer.
    prefix              TEXT PRIMARY KEY,
    -- bcrypt hash of the portal API key. Token presented at vendor mint
    -- time, never stored as plaintext. The Zenoh-wire credential is
    -- the per-vendor X.509 cert, NOT this key.
    --
    -- LOOKUP DOES NOT GO THROUGH THIS COLUMN. bcrypt is not searchable, so
    -- authenticating against it means walking every row and hashing each.
    -- api_key_sha256 below is the indexed lookup key; this stays only to
    -- authenticate rows minted before it existed, one row at a time.
    api_key_hash        TEXT NOT NULL,
    -- hex(sha256(plaintext token)) — indexed, exact-match, O(1). The tokens
    -- are 32 crypto/rand bytes, so a plain digest is the right primitive:
    -- nothing to brute-force, nothing for a work factor to slow down.
    -- Empty on rows minted before this column; those backfill on first use.
    api_key_sha256      TEXT NOT NULL DEFAULT '',
    -- Human-readable display name (free text); falls back to prefix.
    display_name        TEXT NOT NULL DEFAULT '',
    -- Per design's open-decision #2: operator-approved cohort discipline.
    -- approved_at = NULL means pending. Approver is the operator user
    -- identity that flipped the bit.
    approved_at         INTEGER,
    approver            TEXT,
    -- When the per-vendor write rule was last rendered into the ACL.
    -- NULL until the first ACL render after approval. Updated whenever
    -- the registry → ACL reconciliation fires.
    acl_rendered_at     INTEGER,
    -- Audit fields.
    created_at          INTEGER NOT NULL,
    -- One-time secret consumed by GET /api/vendor-bundle/<v>/<token>.tar.gz.
    -- NULLed after the tarball is fetched. Bundle file path is derived.
    bundle_token        TEXT,
    bundle_path         TEXT,
    bundle_minted_at    INTEGER,
    -- Synthetic = operator-managed demo/illustrative vendor (e.g. the
    -- synth-vendor demo publishers under caracal / community /
    -- homemade-sensors / pangolin). UI badges these clearly so they
    -- aren't confused with real partner traffic; panoscope exposes a
    -- "hide synthetic" toggle that filters these out of the graph
    -- render. Real vendors land with synthetic = 0.
    synthetic           INTEGER NOT NULL DEFAULT 0
);

-- Additive migration for databases created before the synthetic column.
-- Wrapped in a defensive idempotent CREATE … ALTER pattern; modernc sqlite
-- accepts ALTER TABLE ADD COLUMN with no IF NOT EXISTS support, so the
-- registry layer does a PRAGMA-driven check before applying.

CREATE TABLE IF NOT EXISTS topics (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    vendor_prefix       TEXT NOT NULL REFERENCES vendors(prefix) ON DELETE CASCADE,
    -- Exact key expression the producer publishes to, e.g.
    -- "acme/sensor/entity/track/v1". Validated at insert: must start
    -- with "<vendor_prefix>/<system>/..." (i.e. at least one '/' past
    -- the vendor prefix).
    key_pattern         TEXT NOT NULL,
    -- One of: json, cbor, msgpack, text, raw, or "protobuf:<msg-name>".
    format              TEXT NOT NULL,
    -- Pointer into attest/schema/<vendor>/<system>/.../v<N>. Optional.
    schema_ref          TEXT NOT NULL DEFAULT '',
    -- Free text. UI surfaces this on topic-detail screens.
    description         TEXT NOT NULL DEFAULT '',
    -- Whether this topic should be picked up by zenohd's storage_manager
    -- for replay. Today informational only; storage policy lands as a
    -- follow-on when the storage_manager config is wired.
    publish_to_storage  INTEGER NOT NULL DEFAULT 0,
    -- Counters updated by the audit-logger plugin or via PATCH /api/...
    -- (audit-logger integration is a follow-on; today these are 0/NULL
    -- until ACL-reload integration lands).
    last_seen_at        INTEGER,
    publish_count       INTEGER NOT NULL DEFAULT 0,
    created_at          INTEGER NOT NULL,
    retired_at          INTEGER,
    -- Tombstone: retired topics stay in the registry for audit; the
    -- API surface filters them out by default.
    UNIQUE(vendor_prefix, key_pattern)
);

CREATE INDEX IF NOT EXISTS topics_vendor_idx       ON topics(vendor_prefix);
CREATE INDEX IF NOT EXISTS topics_key_pattern_idx  ON topics(key_pattern);

-- Operator audit trail. Append-only.
CREATE TABLE IF NOT EXISTS audit_events (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at         INTEGER NOT NULL,
    actor               TEXT NOT NULL,    -- "operator" or vendor prefix
    action              TEXT NOT NULL,    -- "vendor-register", "topic-create", etc.
    target              TEXT NOT NULL,    -- vendor prefix or key_pattern
    details             TEXT NOT NULL DEFAULT '{}'  -- JSON blob
);

CREATE INDEX IF NOT EXISTS audit_actor_idx   ON audit_events(actor);
CREATE INDEX IF NOT EXISTS audit_action_idx  ON audit_events(action);

-- Schema bodies uploaded for a topic. v1 (Block 85): portal-local file
-- storage. The pointer-string `topics.schema_ref` is the user-facing
-- conceptual pointer (typically of the form attest/schema/<vendor>/...);
-- the actual bytes live on disk under <SchemaDir>/<topic_id>/<version>.<ext>
-- and are surfaced via GET /api/schemas/<key...>.
--
-- Per the design memo at docs/design/portal-schema-publishing.md, this is
-- a trial-fabric expedient — long-term, the bytes migrate to attest/schema
-- on the Zenoh fabric and the vendor signs the publish directly. Until
-- then, the portal is the durable copy and the broker.
CREATE TABLE IF NOT EXISTS topic_schemas (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    topic_id        INTEGER NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
    -- Vendor-supplied version label. Enforced ^v\d+$ at the API layer.
    version         TEXT NOT NULL,
    -- Vendor-declared compatibility against the prior version.
    -- Enum: 'backward' | 'forward' | 'none'. v1 doesn't validate the
    -- claim; the portal records what the vendor asserted.
    compatibility   TEXT NOT NULL DEFAULT 'none',
    -- Schema-document format. v1 ships protobuf-fds only; future entries
    -- include json-schema, asyncapi, openapi.
    format          TEXT NOT NULL DEFAULT 'protobuf-fds',
    -- Relative path under SchemaDir. Constructed as <topic_id>/<version>.<ext>
    -- where ext is fds / json / yaml depending on format.
    file_path       TEXT NOT NULL,
    size_bytes      INTEGER NOT NULL,
    -- Hex-encoded sha256 of the file content.
    sha256          TEXT NOT NULL,
    uploaded_at     INTEGER NOT NULL,
    -- Actor that uploaded (vendor prefix or "operator").
    uploaded_by     TEXT NOT NULL,
    UNIQUE(topic_id, version)
);

CREATE INDEX IF NOT EXISTS topic_schemas_topic_idx ON topic_schemas(topic_id);

-- Per-topic, per-language org-provided example code (Block 85 PR-D
-- follow-on). When a row exists for a (topic_id, language) pair, the
-- /api/schemas/render endpoint returns it with source="org" (ADR 1052
-- Phase 3; was "vendor"); the viewer renders a green "org-provided"
-- badge and suppresses the "notional, not validated" disclaimer for
-- that language. When no row
-- exists, the renderer falls back to portal-templated autogen and
-- emits source="autogen".
--
-- Storage is inline (TEXT, not file-backed) — examples are small
-- (~kilobytes), text-only, and the on-the-fly autogen path keeps the
-- on-disk surface narrow.
CREATE TABLE IF NOT EXISTS topic_code_examples (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    topic_id        INTEGER NOT NULL REFERENCES topics(id) ON DELETE CASCADE,
    -- Language tag (e.g. "rust", "go", "python", "typescript"). Lowercased
    -- at the API layer. UNIQUE per topic so each language has at most one
    -- vendor override; UPSERT semantics replace prior content.
    language        TEXT NOT NULL,
    -- The vendor-supplied source code, verbatim. No syntax validation —
    -- the portal does not compile the snippet. Capped to 64 KiB at the
    -- API layer so vendors can't dump megabytes into the registry.
    code            TEXT NOT NULL,
    updated_at      INTEGER NOT NULL,
    -- Actor that supplied this row (vendor prefix or "operator"); same
    -- shape as topic_schemas.uploaded_by + audit_events.actor.
    updated_by      TEXT NOT NULL,
    UNIQUE(topic_id, language)
);

CREATE INDEX IF NOT EXISTS topic_code_examples_topic_idx ON topic_code_examples(topic_id);

-- Vendor progress swimlane (Block 88). One row per vendor; sticky
-- L2/L3/L4 timestamps. L1 is NOT stored here — it's derived from
-- vendors.bundle_minted_at, which is already canonical for "bundle
-- minted". Storing it twice would create a drift surface. L2 written
-- by the netbird-mgmt poller; L3/L4 written by the audit tailer
-- (cursor below). Monotonic — once a level is set, COALESCE on every
-- write keeps it set.
CREATE TABLE IF NOT EXISTS vendor_progress (
    prefix          TEXT PRIMARY KEY REFERENCES vendors(prefix) ON DELETE CASCADE,
    l2_at           INTEGER,
    l3_at           INTEGER,
    l4_at           INTEGER,
    last_publish_at INTEGER,
    peer_count      INTEGER NOT NULL DEFAULT 0,
    updated_at      INTEGER NOT NULL
);

-- Per-vendor, per-topic activity rollup (Block 88 activity column).
-- Upserted once per audit event the tailer observes, from the same
-- event stream that drives L3/L4 — no extra subscriber. Lets the
-- swimlane summarize WHAT a vendor has actually done (how many topics,
-- how many publishes, what they touched most recently) rather than just
-- WHICH onboarding level they reached. Deliberately summary-grade, not
-- rate/payload telemetry — panoscope still owns the live-traffic view.
-- last_seen_at is monotonic-max so a replayed-out-of-order older event
-- never rewinds it; publish_count increments once per observed event
-- (the audit cursor guarantees each event is processed once, so steady
-- state never double-counts).
CREATE TABLE IF NOT EXISTS vendor_topic_activity (
    prefix          TEXT NOT NULL REFERENCES vendors(prefix) ON DELETE CASCADE,
    topic           TEXT NOT NULL,
    publish_count   INTEGER NOT NULL DEFAULT 0,
    first_seen_at   INTEGER NOT NULL,
    last_seen_at    INTEGER NOT NULL,
    PRIMARY KEY (prefix, topic)
);

CREATE INDEX IF NOT EXISTS vendor_topic_activity_prefix_idx
    ON vendor_topic_activity(prefix);

-- Cursor for the MinIO audit tailer. One row per audit_subscriber_id
-- (plural-ready). last_key is the highest object
-- key processed; ListObjectsV2 StartAfter consumes the right side of
-- this. Lex order matches the audit-subscriber's key shape
-- '{id}/YYYY/MM/DD/HH/{ms}-{n}.ndjson'.
CREATE TABLE IF NOT EXISTS audit_replay_cursor (
    subscriber_id   TEXT PRIMARY KEY,
    last_key        TEXT NOT NULL DEFAULT '',
    last_seen_at    INTEGER NOT NULL
);

-- trashgoat specs (Block 92). Each row is one operator-defined chaos
-- experiment. Lifecycle: draft → active ⇄ paused → retired, plus an
-- orthogonal cleanse action that stamps last_cleansed_at without
-- changing state. The portal owns persistence; the runner (PR #3)
-- subscribes to ops/trashgoat/control/<id>/v1 and interprets
-- payload_spec + cadence_spec at publish time.
--
-- Design: docs/design/trashgoat.md.
CREATE TABLE IF NOT EXISTS trashgoat_specs (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    -- url-safe kebab-case experiment slug; first segment past the
    -- trashgoat/ vendor prefix in published keys. Unique.
    slug                TEXT NOT NULL UNIQUE,
    -- one-line operator-readable purpose
    description         TEXT NOT NULL DEFAULT '',
    -- full key the runner publishes under, e.g.
    -- "trashgoat/large-payload-12mb/v1" or
    -- "trashgoat/burst/v1/{uuid}". Templates ({seq}/{uuid}/{ts_ms})
    -- are resolved by the runner, not the portal.
    topic_pattern       TEXT NOT NULL,
    -- JSON-encoded payload generator spec; one of:
    --   {"kind":"bytes",          "size_bytes":N, "content":"random|zeros|repeating:..."}
    --   {"kind":"chunked",        "total_size_bytes":N, "chunk_size_bytes":N, "content":"..."}
    --   {"kind":"json_shape",     "template":{...}}
    --   {"kind":"protobuf_shape", "message_type":"...", "fields":{...}}
    payload_spec_json   TEXT NOT NULL,
    -- JSON-encoded cadence spec; one of:
    --   {"kind":"oneshot"}
    --   {"kind":"every",   "interval_ms":N}
    --   {"kind":"burst",   "count":N, "spacing_ms":N, "then_idle":bool}
    cadence_spec_json   TEXT NOT NULL,
    -- Lifecycle state. Enum: 'draft' | 'active' | 'paused' | 'retired'.
    -- Application layer enforces the valid transition graph.
    state               TEXT NOT NULL DEFAULT 'draft',
    -- Operator scratch — what was learned, when, by whom. Free text.
    notes               TEXT NOT NULL DEFAULT '',
    -- Audit + lifecycle timestamps.
    created_at          INTEGER NOT NULL,
    created_by          TEXT NOT NULL DEFAULT 'operator',
    last_activated_at   INTEGER,
    last_paused_at      INTEGER,
    last_cleansed_at    INTEGER,
    retired_at          INTEGER
);

CREATE INDEX IF NOT EXISTS trashgoat_specs_state_idx ON trashgoat_specs(state);

-- Per-uuid slot pool (ADR 1025). Present only in per-uuid-slot-pool goatnets;
-- empty/untouched in per-vendor goatnets. Pre-populated once at
-- portal boot from the per-goatnet salt: slot_id = sha256(salt:k)[:32] — the
-- SAME deterministic anchor the router ACL bakes (goat-zenoh
-- config.json5.j2, per-uuid mode), so a portal-minted cert (CN=slot_id) always
-- matches a pre-existing router rule with no ACL edit + no restart. Onboarding
-- a system = allocate a free slot + mint its cert; offboarding = free the slot.
CREATE TABLE IF NOT EXISTS slots (
    slot_id          TEXT PRIMARY KEY,   -- sha256(salt:k)[:32]; the cert CN AND write anchor
    idx              INTEGER NOT NULL,   -- slot index k (0..pool_size-1)
    assigned_to      TEXT,               -- system handle assigned; NULL = free
    assigned_at      INTEGER,
    cert_minted_at   INTEGER,            -- when the CN=slot_id cert was minted
    bundle_token     TEXT,               -- one-time tarball download token
    bundle_path      TEXT,
    bundle_minted_at INTEGER,
    revoked_at       INTEGER,            -- last offboard time (slot returns to free pool)
    -- ADR 1096 delegated machine enrollment — the machine-principal stamp on a
    -- slot that was enrolled by a custodian (vs a developer self-slot, which
    -- leaves these NULL). enrolled_by is the person→system CUSTODY edge
    -- (identity-grammar.md §2); node_class/autonomy/affiliation are the grammar
    -- born-as snapshot (ADR 1043; ADR 1051 cert=born-as / registry=currently-is)
    -- stamped into the cert URI-SAN at mint. affiliation is INHERITED from the
    -- custodian's unit, never a mint parameter. All NULL on a fresh/free slot.
    enrolled_by      TEXT,               -- custodian person handle (custody edge)
    node_class       TEXT,               -- grammar node-class (SENSOR/EFFECTOR/PROCESSOR/PLATFORM/infra)
    autonomy         TEXT,               -- grammar autonomy (manual/remote/supervised/autonomous)
    affiliation      TEXT,               -- inherited affiliation anchor snapshot
    workloads        TEXT,               -- ADR 1098: a PLATFORM's declared sub-workload composition (comma-joined function classes); the born-as declaration SPIRE registers SVIDs from. Empty for a simple system.
    enrolled_at      INTEGER,            -- when the custody enrollment was recorded
    -- The Kratos identity UUID of the person this slot was onboarded FOR, stamped
    -- at slot-onboard while the flow still holds it (the identity
    -- fragility audit §1.2: the linkage exists at that moment and was discarded,
    -- forcing every downstream consumer to re-derive ownership by parsing names —
    -- alias↔handle, tag suffix stripping, email local-part). Same principal-key
    -- discipline as govern_admins.identity_id: a UUID, never an email or a handle.
    -- NULL on a free slot, on a machine slot (machines are WireGuard identities,
    -- not Ory ones), and on every slot onboarded before this column existed —
    -- consumers fall back to the name joins and say so.
    identity_id      TEXT,
    -- Mint-on-collect (docs/design/fabric-credential-collection.md): when the
    -- PERSON who owns this slot collected their own fabric credential from the
    -- console, proving who they are first. NULL means uncollected — either the
    -- slot's credential was minted FOR them at onboard (every slot predating the
    -- change) or nobody has collected yet. A NULL here is a real signal, not a
    -- gap: it distinguishes "onboarded" from "holds a fabric credential".
    collected_at     INTEGER
);

CREATE INDEX IF NOT EXISTS slots_assigned_idx ON slots(assigned_to);
CREATE INDEX IF NOT EXISTS slots_idx_idx      ON slots(idx);
-- slots_enrolled_by_idx is created in registry.go AFTER the addColumnIfMissing
-- migration adds the enrolled_by column: on an existing pre-#1812 DB the
-- CREATE TABLE IF NOT EXISTS above is a no-op so enrolled_by is absent here, and
-- indexing it inline (before the Go column-add) fails with "no such column".

-- ADR 1096 delegated machine enrollment — the enrollment_grant
-- (onboard-tier-role-model.md §5): a custodian's STANDING authority + quota to
-- bring machines online later, late-bound. Seeded at person-onboard from the
-- manifest machine_quota; the portal registry is the authoritative record.
-- Remaining quota is derived, never stored: quota - COUNT(live enrolled slots),
-- so revoking a machine (slots.revoked_at set) returns its quota BY CONSTRUCTION
-- (resolves the machine-enrollment.md open question in the affirmative).
-- affiliation/site are the INHERITED grant attributes (onboard-tier-role-model.md
-- §5: "quota + inherited affiliation/echelon/site"): every machine this custodian
-- enrolls is stamped with the custodian's unit — never a mint parameter. site is
-- optional (a mobile custodian may have none).
CREATE TABLE IF NOT EXISTS machine_grants (
    custodian    TEXT PRIMARY KEY,            -- person handle holding the grant
    quota        INTEGER NOT NULL DEFAULT 0,  -- max machines this custodian may enroll
    affiliation  TEXT NOT NULL DEFAULT '',    -- inherited unit anchor stamped on every enrolled machine
    site         TEXT NOT NULL DEFAULT '',    -- inherited site scope (optional)
    -- ADR 1113: may this custodian enroll the delegable infrastructure classes
    -- (NETWORK — routers/switches/radios) without the goatnet operator? Bounded
    -- to `site` above; 0 (fail-closed) means infra stays operator-only for them.
    network_admin INTEGER NOT NULL DEFAULT 0,
    granted_by   TEXT NOT NULL DEFAULT '',    -- operator/onboard actor that set it
    granted_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

-- Per-actor throughput envelopes (ADR 1045 /
-- docs/design/qos-throughput-envelopes.md). One row per actor, keyed by the
-- write anchor (= vendor prefix in per-vendor mode, = slot_id in
-- per-uuid-slot-pool mode — the SAME string fabric-health footprints key on,
-- so `goat top` joins a live footprint to its envelope). {min, mean, high, max}
-- on two axes (msgs/s + bytes/s) = 8 numbers. source='default' is the rough
-- class-baseline placeholder assigned at registration (ADR 1045 §3 — a
-- placeholder to be corrected, not a model); 'edited' is set by the registering
-- party via PUT /api/vendors/{anchor}/envelope. DECLARED governance/display
-- metadata ONLY — never touches the wire, ACL, cert, or namespace (mirrors the
-- slot-alias layer, ADR 1039). The FK cascades the envelope away with its
-- vendor/anchor row.
CREATE TABLE IF NOT EXISTS envelopes (
    anchor            TEXT PRIMARY KEY REFERENCES vendors(prefix) ON DELETE CASCADE,
    min_msgs_per_s    REAL NOT NULL,
    mean_msgs_per_s   REAL NOT NULL,
    high_msgs_per_s   REAL NOT NULL,
    max_msgs_per_s    REAL NOT NULL,
    min_bytes_per_s   REAL NOT NULL,
    mean_bytes_per_s  REAL NOT NULL,
    high_bytes_per_s  REAL NOT NULL,
    max_bytes_per_s   REAL NOT NULL,
    -- 'default' (rough class-baseline placeholder) | 'edited' (party-set).
    source            TEXT NOT NULL DEFAULT 'default',
    -- Which per-class baseline a default came from (provenance/display only).
    class_key         TEXT NOT NULL DEFAULT '',
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

-- Producer data-access grant rows (ADR 1075). Each row is one enumerable
-- relation "grantee may VIEW producer's namespace", effective-dated and
-- deprecate-never-delete (revoke = end-date + status flip, never DELETE — the
-- before-the-fact audit-by-enumeration depends on the history staying intact).
-- The row shape is the SoT law (ops/permissions/data-access-roles.yaml,
-- `grants.row_shape`): grantee is org:<id> | principal:<identity-uuid>, split
-- here into (grantee_kind, grantee_id) so the resolver joins without parsing;
-- producer is a producer key (vendor prefix / slot anchor); granted_by is the
-- custodian actor; justification is the recorded why;
-- effective_from/effective_to bound the active window (to NULL = open-ended);
-- status is active | revoked. The data-roles resolver reads these rows live
-- (no parallel store — SoT no_third_shim); "who can see X" is a row read.
CREATE TABLE IF NOT EXISTS grants (
    id             TEXT PRIMARY KEY,                  -- grant-<hex>
    grantee_kind   TEXT NOT NULL,                     -- 'org' | 'principal'
    grantee_id     TEXT NOT NULL,                     -- org id (kebab) | identity UUID
    producer       TEXT NOT NULL,                     -- view: producer anchor. write (co-publish, ADR 1105): a multipub-topic key-base multipub/<pool>/<topic>
    action         TEXT NOT NULL DEFAULT 'view',      -- 'view' (ADR 1075) | 'write' (co-publish multi-publisher topic, ADR 1105)
    granted_by     TEXT NOT NULL,                     -- custodian actor handle
    justification  TEXT NOT NULL DEFAULT '',          -- the recorded why
    effective_from INTEGER NOT NULL,                  -- unix; active-window start
    effective_to   INTEGER,                           -- unix; NULL = open-ended
    status         TEXT NOT NULL DEFAULT 'active',    -- 'active' | 'revoked'
    created_at     INTEGER NOT NULL,
    revoked_at     INTEGER,
    revoked_by     TEXT
);

CREATE INDEX IF NOT EXISTS grants_producer_idx ON grants(producer);
CREATE INDEX IF NOT EXISTS grants_grantee_idx  ON grants(grantee_kind, grantee_id);
CREATE INDEX IF NOT EXISTS grants_status_idx   ON grants(status);

-- Governance-admin nominations (ADR 1102 decision 1, the U9 surface). The
-- bootstrap rule `member-of(O) ⇒ data.govern.org:O` holds until an org has at
-- least one ACTIVE row here; from then on the org is NARROWED — only its
-- nominated admins carry data.govern.org:<O> (an org self-administers its
-- admins, the at-most-own-scope pattern of ADR 1071/1072). Deprecate-never-
-- delete like grants: removal end-dates + flips status. Removing the LAST
-- active admin of an org is refused (monotonic narrowing; no silent fall-open
-- back to everyone-governs) — nominate a replacement first, or a platform
-- operator exercises proxy stewardship. Principal-key discipline: the nominee
-- is an identity UUID; email never appears here.
CREATE TABLE IF NOT EXISTS govern_admins (
    id             TEXT PRIMARY KEY,                  -- gadm-<hex>
    org            TEXT NOT NULL,                     -- org id (kebab)
    identity_id    TEXT NOT NULL,                     -- nominee identity UUID (person)
    nominated_by   TEXT NOT NULL,                     -- nominating actor
    justification  TEXT NOT NULL DEFAULT '',          -- the recorded why
    status         TEXT NOT NULL DEFAULT 'active',    -- 'active' | 'removed'
    created_at     INTEGER NOT NULL,
    removed_at     INTEGER,
    removed_by     TEXT
);

CREATE INDEX IF NOT EXISTS govern_admins_org_idx ON govern_admins(org, status);

-- Multi-publisher topics (ADR 1105, PUBSUB-ORG-ADMIN). The one-writer-
-- per-<uuid>/**-root rule (ADR 1025) is inviolable — provenance is free there.
-- A multi-publisher topic is the ONLY place more than one writer is allowed,
-- and it is a DELIBERATELY-created, marked, discoverable, org-scoped object in
-- a distinct namespace: multipub/<pool_handle>/<topic_id>/**. The reserved
-- `multipub/` segment announces "do not assume one writer here"; it is never a
-- single-owner root.
--
-- org_multipub_pools: a STABLE OPAQUE per-org handle, minted once. It is a
-- born-under marker (ADR 1051) — the org NAME is never frozen into a key (the
-- data-access-roles.md §1 / ADR 1043 anti-pattern: ownership is mutable, keys
-- are freeze-critical). `org` is currently-is; a reorg updates it, the handle
-- never changes, so no key ever migrates. The handle resolves to the org, and
-- the org to a display name via the alias layer (ADR 1039).
CREATE TABLE IF NOT EXISTS org_multipub_pools (
    pool_handle    TEXT PRIMARY KEY,                  -- stable opaque per-org handle (hex)
    org            TEXT NOT NULL UNIQUE,              -- owning org id (currently-is; reorg updates, handle stays)
    created_by     TEXT NOT NULL,                     -- minting actor
    created_at     INTEGER NOT NULL
);

-- multipub_topics: one deliberately-created multi-publisher topic. key-base =
-- multipub/<pool_handle>/<topic_id>. topic_id is stable opaque; display_name
-- is a human alias (display-only, ADR 1039), never a machine path.
-- Deprecate-never-delete (status flip). `persist` = render a rocksdb store
-- for it (history) vs routed-only. The authorized publisher set is NOT here —
-- it is write grant rows (grants.action='write', producer=this key-base), so
-- "who may publish here, since when, granted by whom" stays an
-- effective-dated, audited row read.
CREATE TABLE IF NOT EXISTS multipub_topics (
    topic_id       TEXT PRIMARY KEY,                  -- stable opaque topic id (hex)
    org            TEXT NOT NULL,                     -- owning org id (currently-is)
    pool_handle    TEXT NOT NULL,                     -- the org's multipub-pool handle (born-under)
    display_name   TEXT NOT NULL DEFAULT '',          -- human alias (display only)
    justification  TEXT NOT NULL DEFAULT '',          -- the recorded why (creation is deliberate)
    persist        INTEGER NOT NULL DEFAULT 1,        -- 1 = render a store (history); 0 = routed-only
    status         TEXT NOT NULL DEFAULT 'active',    -- 'active' | 'retired'
    created_by     TEXT NOT NULL,                     -- creating actor
    created_at     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS multipub_topics_org_idx ON multipub_topics(org, status);

-- govern_meta is a tiny key/value store for one-shot governance-cutover state
-- (ADR 1102). The only key today is `cutover_all_members_seeded` — the marker
-- the one-time existing-orgs all-members migration writes on success, so the
-- self-triggering startup migration runs exactly once and every later boot
-- skips it (going-forward new orgs are handled by the first-member reconcile,
-- not the migration). Kept separate from govern_admins so a marker row is never
-- mistaken for a nomination.
CREATE TABLE IF NOT EXISTS govern_meta (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL DEFAULT '',
    updated_at INTEGER NOT NULL
);

-- Discover -> request -> approve loop (ADR 1075 §"The loop"). A request is a
-- viewer's inert ask to read a producer's restricted namespace; the producer
-- custodian (or operator) approves, which MINTS a grant row (grant_id below)
-- and marks the request approved. Modeled on the ADR 1071/1072 sponsor-door
-- proposal shape. Preemptive grants (ADR 1075 §12 / follow-ons §0e.2) skip this
-- table entirely — the grant verb has no pending-request precondition — so a
-- request row is provenance for the request-driven path only, never a
-- precondition for a grant to exist.
CREATE TABLE IF NOT EXISTS data_access_requests (
    id             TEXT PRIMARY KEY,                  -- dar-<hex>
    requester_id   TEXT NOT NULL,                     -- requesting identity UUID (or handle when no UUID)
    requester_label TEXT NOT NULL DEFAULT '',         -- display only (email/handle), never a join key
    producer       TEXT NOT NULL,                     -- producer key requested
    grantee_kind   TEXT NOT NULL DEFAULT 'principal', -- requested grant scope: 'principal' | 'org'
    grantee_id     TEXT NOT NULL,                     -- self identity UUID, or an org id
    justification  TEXT NOT NULL DEFAULT '',          -- requester's stated need
    status         TEXT NOT NULL DEFAULT 'pending',   -- 'pending' | 'approved' | 'declined'
    created_at     INTEGER NOT NULL,
    decided_by     TEXT,                              -- custodian/operator actor
    decided_at     INTEGER,
    grant_id       TEXT,                              -- the grant minted on approval
    note           TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS data_access_requests_producer_idx ON data_access_requests(producer);
CREATE INDEX IF NOT EXISTS data_access_requests_status_idx   ON data_access_requests(status);

-- Anomaly probation list (ADR 1068). A holding area for things the COP
-- surfaces that don't reconcile — a stray first, later other anomaly kinds —
-- so they can be verified then repaired or purged instead of lingering or being
-- hastily cut. Held state lives HERE, separate from the declared registry (the
-- declared registry is for things decided to belong; this is for undecided
-- maybes). Non-load-bearing: nothing here is in an enforcement path.
CREATE TABLE IF NOT EXISTS probation (
    id           TEXT PRIMARY KEY,                   -- prob-<hex>
    ref          TEXT NOT NULL,                      -- stable ref to the held entity (identity id / slot anchor / recon key)
    kind         TEXT NOT NULL DEFAULT 'identity',   -- identity | topic | peer | inbound (v1: identity)
    label        TEXT NOT NULL DEFAULT '',           -- human display (alias/email/name), display-only
    reason       TEXT NOT NULL DEFAULT '',           -- why it was held (e.g. stray:unregistered-key)
    detected_at  INTEGER NOT NULL DEFAULT 0,         -- when first surfaced (0 = unknown)
    held_at      INTEGER NOT NULL,                   -- when it entered probation
    held_by      TEXT NOT NULL,                      -- operator who held it
    cure_by      INTEGER NOT NULL,                   -- deadline; on expiry the COP flags it — never auto-deletes
    disposition  TEXT NOT NULL DEFAULT 'probation',  -- probation | repaired | purged
    decided_at   INTEGER,                            -- when repaired/purged
    decided_by   TEXT NOT NULL DEFAULT '',           -- operator who decided
    note         TEXT NOT NULL DEFAULT ''
);
-- One OPEN probation row per ref (can't double-hold); historical repaired/purged rows are allowed.
CREATE UNIQUE INDEX IF NOT EXISTS probation_ref_open_idx    ON probation(ref) WHERE disposition = 'probation';
CREATE INDEX IF NOT EXISTS        probation_disposition_idx ON probation(disposition);

-- Topic-budget grant store (the registry home of the per-owner allowance
-- record; zenoh-cardinality-budget-and-lazy-crawler.md §2 layer 2b + §3
-- layer 3). One row per owner that holds an explicit allowance beyond the
-- automatic self-service block; onboarded slots with no row hold the block
-- automatically, other owners with no row read unmeasured. `owner` is the
-- crawler's owner grain — a slot anchor or an infra store name — not an FK.
CREATE TABLE IF NOT EXISTS topic_budgets (
    owner       TEXT PRIMARY KEY,        -- slot anchor or infra store name
    granted     INTEGER NOT NULL,        -- agreed distinct-stored-key ceiling (> 0)
    prefixes    TEXT NOT NULL DEFAULT '',-- JSON array of named channel prefixes ('' = no ACL subdivision); consumed by the router render at the next F-156 window
    note        TEXT NOT NULL DEFAULT '',-- approver + date + rationale (approval-tier record)
    updated_at  INTEGER NOT NULL,
    updated_by  TEXT NOT NULL DEFAULT ''
);
