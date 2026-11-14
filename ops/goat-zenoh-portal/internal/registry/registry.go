// Package registry holds the vendor + topic registry for goat-zenoh-portal.
//
// Backed by Postgres through jmoiron/sqlx over pgx's database/sql driver
// (ADR 1165 step A: the tables are the SQLite build's, unchanged). All SQL is
// raw, no ORM, per ADR 0413.
//
// Concurrency: a pool of connections. A method that reads and then writes on
// the strength of what it read runs in a transaction that first takes
// registryWriteLock (beginLocked), so those methods serialize against each
// other as they did when the single SQLite connection was the lock.
package registry

import (
	"context"
	_ "embed"
	"errors"
	"fmt"
	"regexp"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgstore"
	"github.com/jmoiron/sqlx"
)

//go:embed schema.sql
var schemaSQL string

// Vendor matches the vendors table.
type Vendor struct {
	Prefix     string `db:"prefix"          json:"prefix"`
	APIKeyHash string `db:"api_key_hash"    json:"-"`
	// APIKeySHA256 is hex(sha256(plaintext token)) — the INDEXED lookup key
	// that makes authentication O(1). The bcrypt APIKeyHash above is
	// kept for rows minted before this column existed; a row backfills its
	// sha256 the first time its holder authenticates. Empty = not yet
	// backfilled, which is the only state that still costs a bcrypt.
	//
	// Storing a bare sha256 of an API key is not the password-hashing mistake
	// it looks like: these tokens are 32 bytes from crypto/rand (see
	// generateAPIKey), so there is no dictionary to attack and a work factor
	// buys nothing. bcrypt's cost is meaningful only against guessable
	// secrets; here it only spends CPU on every request.
	APIKeySHA256   string  `db:"api_key_sha256"  json:"-"`
	DisplayName    string  `db:"display_name"    json:"display_name"`
	ApprovedAt     *int64  `db:"approved_at"     json:"approved_at,omitempty"`
	Approver       *string `db:"approver"        json:"approver,omitempty"`
	ACLRenderedAt  *int64  `db:"acl_rendered_at" json:"acl_rendered_at,omitempty"`
	CreatedAt      int64   `db:"created_at"      json:"created_at"`
	BundleToken    *string `db:"bundle_token"    json:"-"`
	BundlePath     *string `db:"bundle_path"     json:"-"`
	BundleMintedAt *int64  `db:"bundle_minted_at" json:"bundle_minted_at,omitempty"`
	// BundleDownloadedAt is the FIRST time the tarball was actually fetched,
	// and BundleDownloadCount how many fetches in total. Minted is not fetched:
	// a bundle can be minted and never collected off the wire, which is exactly
	// the state that stalls an onboarding silently. NULL means no download has
	// been recorded — for rows predating this column that is "unknown", not
	// "never", and the surface must not claim otherwise.
	BundleDownloadedAt  *int64 `db:"bundle_downloaded_at"    json:"bundle_downloaded_at,omitempty"`
	BundleDownloadCount int64  `db:"bundle_download_count"   json:"bundle_download_count"`
	// Synthetic flags operator-managed demo vendors so the UI and
	// panoscope can badge them and (in panoscope) filter them out via
	// the "hide synthetic" toggle. Bootstrap script passes synthetic=true
	// for caracal / community / homemade-sensors / pangolin. Real
	// partner vendors register with synthetic=false.
	Synthetic bool `db:"synthetic" json:"synthetic"`
	// Sponsorship authorization (ADR 1071): whether this identity may mint
	// candidate-onboarding invites and vouch at the stage-2 gate, and the
	// highest tier it may sponsor (at most its own tier). Decided at
	// onboarding: default-on for the escalated admin tiers, case-by-case
	// annotation for everyone else. Operators (backend super-user keys)
	// are sponsorship-authorized by definition and never consult these.
	SponsorAuthorized bool   `db:"sponsor_authorized" json:"sponsor_authorized"`
	SponsorMaxTier    string `db:"sponsor_max_tier"   json:"sponsor_max_tier,omitempty"`
	// SponsorEmail binds the human's Ory login email to this sponsorship
	// annotation so web surfaces that authenticate the person via Ory (the
	// id-admin-console sponsor mode, ADR 1072) can resolve the session back
	// to this registry row. Set with the grant, cleared with the revoke.
	// Empty is valid — the sponsor can still mint/vouch via CLI/API with
	// their own portal key; only the console binding needs it. Never emitted
	// on list surfaces (the gated resolve endpoints only).
	SponsorEmail string `db:"sponsor_email" json:"-"`
	// Producer data labels (ADR 1075), topic-namespace grain — one label set
	// per producer namespace, NEVER per message. fidelity/context are advisory
	// + display; access is the authz-relevant lane (community | org |
	// controlled). Empty = unlabeled, rendered honestly, never fabricated; an
	// empty access lane resolves to the per-goatnet data_label_access_default at
	// read time (SetVendorLabels validates the closed enums and the
	// justification-on-restriction gate). LabelJustification is the recorded
	// why REQUIRED when access is org|controlled (the NAVADMIN 060/23 pattern).
	LabelFidelity      string `db:"label_fidelity"      json:"label_fidelity,omitempty"`
	LabelContext       string `db:"label_context"       json:"label_context,omitempty"`
	LabelAccess        string `db:"label_access"        json:"label_access,omitempty"`
	LabelJustification string `db:"label_justification" json:"label_justification,omitempty"`
}

// Topic matches the topics table.
type Topic struct {
	ID               int64  `db:"id"                  json:"id"`
	VendorPrefix     string `db:"vendor_prefix"       json:"vendor_prefix"`
	KeyPattern       string `db:"key_pattern"         json:"key_pattern"`
	Format           string `db:"format"              json:"format"`
	SchemaRef        string `db:"schema_ref"          json:"schema_ref"`
	Description      string `db:"description"         json:"description"`
	PublishToStorage bool   `db:"publish_to_storage"  json:"publish_to_storage"`
	LastSeenAt       *int64 `db:"last_seen_at"        json:"last_seen_at,omitempty"`
	PublishCount     int64  `db:"publish_count"       json:"publish_count"`
	CreatedAt        int64  `db:"created_at"          json:"created_at"`
	RetiredAt        *int64 `db:"retired_at"          json:"retired_at,omitempty"`
}

// Registry wraps the Postgres connection pool.
type Registry struct {
	db *sqlx.DB
}

// migrationTimeout bounds connecting and applying the schema at startup.
const migrationTimeout = 2 * time.Minute

// migrationCtx returns the bounded context the startup steps run under.
func migrationCtx() (context.Context, context.CancelFunc) {
	return context.WithTimeout(context.Background(), migrationTimeout)
}

// Migrations is the registry's versioned schema. Version 1 is the SQLite
// schema with the tables and columns unchanged (ADR 1165 step A); later
// versions are step B.
var Migrations = []pgstore.Migration{{Version: 1, SQL: schemaSQL}}

// Tables are the registry's tables in version 1, in foreign-key order: a table
// comes after every table it references. store-migrate copies them in this
// order.
var Tables = []string{
	"vendors",
	"topics",
	"audit_events",
	"topic_schemas",
	"topic_code_examples",
	"vendor_progress",
	"vendor_topic_activity",
	"audit_replay_cursor",
	"trashgoat_specs",
	"slots",
	"machine_grants",
	"envelopes",
	"grants",
	"govern_admins",
	"org_multipub_pools",
	"multipub_topics",
	"govern_meta",
	"data_access_requests",
	"probation",
	"topic_budgets",
}

// Open connects to the Postgres database named by dsn (a libpq URL or keyword
// string) and applies any schema migration not yet recorded. Safe to call
// repeatedly and from several processes at once: migrations run under an
// advisory lock.
func Open(dsn string) (*Registry, error) {
	ctx, cancel := migrationCtx()
	defer cancel()
	db, err := pgstore.Open(ctx, dsn)
	if err != nil {
		return nil, err
	}
	if err := pgstore.Migrate(ctx, db, Migrations); err != nil {
		_ = db.Close()
		return nil, fmt.Errorf("apply schema: %w", err)
	}
	if err := pgstore.RequireTables(ctx, db, Tables...); err != nil {
		_ = db.Close()
		return nil, err
	}
	// ADR 1075 — a producer flagged synthetic:true (the pre-labels
	// self-declaration) is fidelity:notional in the label lanes. Only fills an
	// UNLABELED fidelity lane, never overwrites an explicit one; idempotent.
	if _, err := db.ExecContext(ctx,
		`UPDATE vendors SET label_fidelity = 'notional' WHERE synthetic = 1 AND label_fidelity = ''`); err != nil {
		_ = db.Close()
		return nil, fmt.Errorf("migrate synthetic->fidelity:notional: %w", err)
	}
	// One-shot normalization of the polymorphic custodian key (the
	// identity fragility audit §1.2: machine_grants.custodian took an anchor OR
	// a handle OR free text and the quota join keyed on it, so a grant under one
	// spelling and an enrollment under another silently read quota 0 or a
	// doubled quota). Idempotent — a second boot finds nothing left to rewrite.
	if err := normalizeMachineCustodians(ctx, db); err != nil {
		_ = db.Close()
		return nil, fmt.Errorf("normalize machine_grants.custodian: %w", err)
	}
	return &Registry{db: db}, nil
}

// OpenExisting connects to a registry database that a portal has already
// migrated and changes nothing: it applies no schema and runs no
// normalization. It refuses a database whose schema is not at the version this
// build knows. For read-only tools (pubsub-export) that must never be the
// process that creates or migrates the schema.
func OpenExisting(dsn string) (*Registry, error) {
	ctx, cancel := migrationCtx()
	defer cancel()
	db, err := pgstore.Open(ctx, dsn)
	if err != nil {
		return nil, err
	}
	v, err := pgstore.SchemaVersion(ctx, db)
	if err != nil {
		_ = db.Close()
		return nil, fmt.Errorf("read schema version: %w", err)
	}
	if want := Migrations[len(Migrations)-1].Version; v != want {
		_ = db.Close()
		return nil, fmt.Errorf("registry schema is at version %d, this build expects %d: start the portal against this database first", v, want)
	}
	if err := pgstore.RequireTables(ctx, db, Tables...); err != nil {
		_ = db.Close()
		return nil, err
	}
	return &Registry{db: db}, nil
}

// Close releases the connection pool.
func (r *Registry) Close() error {
	return r.db.Close()
}

// reader returns the read path: the pool, with a deadline on every query.
func (r *Registry) reader() readOnly {
	return readOnly{db: r.db, ttl: readTimeout}
}

// Ping is the cheap liveness probe behind GET /api/health: one 1-row read against
// a real table. It catches the failure modes a static "ok" cannot — the
// database unreachable, the credentials refused, the table missing — which is
// the whole point (the identity fragility audit §1.3 violation 5:
// portal death was observable only through two ≥30-minute proxies). An EMPTY
// registry is healthy, so the probe must not depend on any row existing.
func (r *Registry) Ping(ctx context.Context) error {
	var one int
	if err := r.reader().GetContext(ctx, &one, `SELECT COUNT(*) FROM (SELECT 1 FROM vendors LIMIT 1) AS probe`); err != nil {
		return fmt.Errorf("registry probe: %w", err)
	}
	return nil
}

// DB exposes the underlying handle for tests and out-of-band queries. Not
// part of the stable surface.
func (r *Registry) DB() *sqlx.DB { return r.db }

// vendorPrefixRe matches the allowed vendor-prefix syntax: lowercase ASCII
// letters / digits / '-', 1-32 chars, no leading or trailing '-'.
var vendorPrefixRe = regexp.MustCompile(`^[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?$`)

// ValidVendorPrefix returns nil when the prefix is allowed, error otherwise.
// Exposed for handlers that want to validate before hitting the DB.
func ValidVendorPrefix(p string) error {
	if !vendorPrefixRe.MatchString(p) {
		return fmt.Errorf("org prefix %q: must be 1-32 chars [a-z0-9-], no leading/trailing '-'", p)
	}
	// Block reserved top-level prefixes. `ops` is the canonical operator/
	// control namespace (ADR 0844 / ADR 1035); `attest` is schema decls;
	// `system` is the deprecated operator predecessor, still reserved during
	// the migration window so a vendor can't claim it (ADR 1035).
	for _, reserved := range []string{"ops", "attest", "system"} {
		if p == reserved {
			return fmt.Errorf("org prefix %q is reserved", p)
		}
	}
	return nil
}

// aliasHexRe matches a 32-char lowercase-hex string — the exact shape of a slot
// anchor (SlotAnchor = sha256(...)[:32]). An alias must NOT be hex-shaped so a
// resolver can classify any token as alias-or-UUID by shape alone (ADR 1039 —
// the disjointness that stops an alias masquerading as an anchor).
var aliasHexRe = regexp.MustCompile(`^[0-9a-f]{32}$`)

// ErrAliasTaken means the requested slot alias (human handle) is already
// assigned to a live (non-revoked) slot. Aliases are unique per goatnet (ADR 1039).
var ErrAliasTaken = errors.New("alias already in use")

// ValidAlias validates a slot alias — the human handle a slot is onboarded with
// (ADR 1039). Stricter than ValidVendorPrefix: same charset, but additionally
// rejects the 32-hex anchor shape (so an alias is always shape-disjoint from a
// UUID slot anchor) and the namespace-class reserved words (so an alias can
// never collide with a top-level namespace the resolver rewrites into).
func ValidAlias(a string) error {
	if !vendorPrefixRe.MatchString(a) {
		return fmt.Errorf("alias %q: must be 1-32 chars [a-z0-9-], no leading/trailing '-'", a)
	}
	if aliasHexRe.MatchString(a) {
		return fmt.Errorf("alias %q: must not be a 32-hex string (that is the slot-anchor shape; ADR 1039)", a)
	}
	for _, reserved := range []string{"ops", "attest", "system", "volatile", "release", "community", "loadtest"} {
		if a == reserved {
			return fmt.Errorf("alias %q is reserved (namespace class)", a)
		}
	}
	return nil
}

// identityUUIDRe is the canonical Kratos identity id shape (8-4-4-4-12 hex).
var identityUUIDRe = regexp.MustCompile(`^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$`)

// ValidIdentityID checks the recorded person linkage is a UUID and nothing else.
// The principal-key discipline the govern_admins table already follows: an email
// or a handle in this column would re-create the very name join the column exists
// to retire, and both are mutable keys that silently de-link on a change.
func ValidIdentityID(id string) error {
	if !identityUUIDRe.MatchString(id) {
		return fmt.Errorf("identity_id %q: must be a Kratos identity UUID (8-4-4-4-12 hex) — never an email or a handle", id)
	}
	return nil
}

// Alias is one slot's UUID↔handle mapping for the display layer (ADR 1039):
// SlotID is the canonical machine key, Alias the human name. Read-only
// projection — the alias is never written into any machine path.
type Alias struct {
	SlotID string `db:"slot_id"     json:"slot_id"`
	Alias  string `db:"assigned_to" json:"alias"`
	// IdentityID is the Kratos identity UUID recorded on the slot at onboard
	// (slots.identity_id). Empty = no recorded linkage — either the slot predates
	// the column or it is a machine slot; consumers then fall back to the name
	// joins (alias↔handle, tag) and must say which they used, never present the
	// guess as the recorded owner.
	IdentityID string `db:"identity_id" json:"identity_id,omitempty"`
}

// ListAliases returns the slot_id↔alias map for every live (assigned,
// non-revoked) slot. The display layer caches this; it reveals names only, not
// access. Empty slice (not nil) when the goatnet is per-vendor (no slots).
func (r *Registry) ListAliases(ctx context.Context) ([]Alias, error) {
	out := []Alias{}
	err := r.reader().SelectContext(ctx, &out,
		`SELECT slot_id, assigned_to, COALESCE(identity_id, '') AS identity_id
		 FROM slots WHERE assigned_to IS NOT NULL AND revoked_at IS NULL ORDER BY assigned_to`)
	if err != nil {
		return nil, fmt.Errorf("list aliases: %w", err)
	}
	return out, nil
}

// CreateVendor inserts a new vendor. apiKeyHash should be bcrypt(plaintext).
// Returns ErrVendorExists if the prefix is already taken.
func (r *Registry) CreateVendor(ctx context.Context, v *Vendor) error {
	if err := ValidVendorPrefix(v.Prefix); err != nil {
		return err
	}
	v.CreatedAt = time.Now().Unix()
	_, err := r.db.ExecContext(ctx,
		`INSERT INTO vendors (prefix, api_key_hash, api_key_sha256, display_name, approved_at, approver, created_at, bundle_token, bundle_path, bundle_minted_at, synthetic, sponsor_authorized, sponsor_max_tier)
		 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)`,
		v.Prefix, v.APIKeyHash, v.APIKeySHA256, v.DisplayName, v.ApprovedAt, v.Approver, v.CreatedAt, v.BundleToken, v.BundlePath, v.BundleMintedAt, boolInt(v.Synthetic), boolInt(v.SponsorAuthorized), v.SponsorMaxTier)
	if err != nil {
		if pgstore.IsUniqueViolation(err) {
			return ErrVendorExists
		}
		return fmt.Errorf("insert vendor: %w", err)
	}
	return nil
}

// GetVendor returns a vendor by prefix, or ErrNotFound.
func (r *Registry) GetVendor(ctx context.Context, prefix string) (*Vendor, error) {
	var v Vendor
	err := r.reader().GetContext(ctx, &v, `SELECT * FROM vendors WHERE prefix = $1`, prefix)
	if errors.Is(err, sqlErrNoRows()) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, err
	}
	return &v, nil
}

// VendorBySponsorEmail resolves a sponsorship-authorized identity by its
// bound Ory login email (the id-admin-console sponsor-mode join, ADR 1072).
// Matching is on the stored lowercased value. Fail-closed contract:
// ErrNotFound when no annotated row carries the email, ErrAmbiguous when more
// than one does (an operator must fix the annotations — the console cannot
// guess which identity a session is).
func (r *Registry) VendorBySponsorEmail(ctx context.Context, email string) (*Vendor, error) {
	var vs []Vendor
	err := r.reader().SelectContext(ctx, &vs,
		`SELECT * FROM vendors WHERE sponsor_authorized = 1 AND sponsor_email = $1 LIMIT 2`,
		strings.ToLower(strings.TrimSpace(email)))
	if err != nil {
		return nil, err
	}
	switch len(vs) {
	case 0:
		return nil, ErrNotFound
	case 1:
		return &vs[0], nil
	default:
		return nil, ErrAmbiguous
	}
}

// VendorByAPIKeySHA256 resolves the vendor holding a presented token, by exact
// match on the indexed digest. This is THE authentication lookup: one
// index probe, no key-derivation, cost independent of how many vendors exist.
//
// Returns ErrNotFound when nothing matches — which includes every row that has
// not backfilled its digest yet, so callers must keep the bcrypt leg for those.
// An empty digest never matches: un-backfilled rows all carry ” and looking
// one up would otherwise resolve an arbitrary vendor for an empty token.
func (r *Registry) VendorByAPIKeySHA256(ctx context.Context, sha256hex string) (*Vendor, error) {
	if sha256hex == "" {
		return nil, ErrNotFound
	}
	var vs []Vendor
	if err := r.reader().SelectContext(ctx, &vs,
		`SELECT * FROM vendors WHERE api_key_sha256 = $1 LIMIT 2`, sha256hex); err != nil {
		return nil, err
	}
	switch len(vs) {
	case 0:
		return nil, ErrNotFound
	case 1:
		return &vs[0], nil
	default:
		// Two rows claiming one token is a credential-identity collision, not a
		// pick-one situation. Fail closed rather than authenticate a coin flip.
		return nil, ErrAmbiguous
	}
}

// SetVendorAPIKeySHA256 backfills the indexed digest for a row that
// authenticated on its legacy bcrypt hash. Guarded on the column still being
// empty so a concurrent rotation that already wrote a NEW key's digest is not
// overwritten by a request that was mid-flight with the old one.
func (r *Registry) SetVendorAPIKeySHA256(ctx context.Context, prefix, sha256hex string) error {
	if sha256hex == "" {
		return fmt.Errorf("refusing to backfill an empty api_key_sha256 for %q", prefix)
	}
	_, err := r.db.ExecContext(ctx,
		`UPDATE vendors SET api_key_sha256 = $1 WHERE prefix = $2 AND api_key_sha256 = ''`,
		sha256hex, prefix)
	return err
}

// CountVendorsAwaitingAPIKeyDigest reports how many credential-bearing rows
// still authenticate the slow way. It is the convergence signal for F-371:
// when this reaches zero on every goatnet, the bcrypt leg in resolvePrincipal
// can be deleted outright. Surfaced on /api/health so it is observable rather
// than something someone has to remember to go and query.
func (r *Registry) CountVendorsAwaitingAPIKeyDigest(ctx context.Context) (int, error) {
	var n int
	err := r.reader().GetContext(ctx, &n,
		`SELECT COUNT(*) FROM vendors WHERE api_key_sha256 = '' AND api_key_hash != ''`)
	return n, err
}

// ListVendors returns all vendors ordered by prefix.
func (r *Registry) ListVendors(ctx context.Context) ([]Vendor, error) {
	var vs []Vendor
	if err := r.reader().SelectContext(ctx, &vs, `SELECT * FROM vendors ORDER BY prefix`); err != nil {
		return nil, err
	}
	return vs, nil
}

// CountVendors returns how many vendor rows exist. The frontend header shows
// this count on every page; going through ListVendors to get it materialized
// every row and serialized ~63 KB of JSON to render one integer.
func (r *Registry) CountVendors(ctx context.Context) (int, error) {
	var n int
	if err := r.reader().GetContext(ctx, &n, `SELECT COUNT(*) FROM vendors`); err != nil {
		return 0, err
	}
	return n, nil
}

// UpdateVendorDisplayName changes the human-readable display name on
// an existing vendor row. Used when the operator's initial mint baked
// the wrong string into display_name — typically when an older manifest
// shape conflated the PERSON name (Ory traits.name) with the VENDOR
// display_name (portal display_name). Idempotent; returns ErrNotFound
// when the prefix doesn't exist.
func (r *Registry) UpdateVendorDisplayName(ctx context.Context, prefix, displayName string) error {
	res, err := r.db.ExecContext(ctx,
		`UPDATE vendors SET display_name = $1 WHERE prefix = $2`,
		displayName, prefix)
	if err != nil {
		return err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return err
	}
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// UpdateVendorSponsorship sets or revokes the sponsorship annotation on an
// existing identity (ADR 1071): the case-by-case grant lever for identities
// that didn't get it by tier default at onboarding, and the revoke lever for
// ones that should lose it. Idempotent; ErrNotFound on unknown prefix.
func (r *Registry) UpdateVendorSponsorship(ctx context.Context, prefix string, authorized bool, maxTier, email string) error {
	res, err := r.db.ExecContext(ctx,
		`UPDATE vendors SET sponsor_authorized = $1, sponsor_max_tier = $2, sponsor_email = $3 WHERE prefix = $4`,
		boolInt(authorized), maxTier, email, prefix)
	if err != nil {
		return err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return err
	}
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// UpdateVendorAPIKey replaces the stored API-key credential on an existing vendor
// row, leaving every identity-bearing field (prefix/anchor, display name, slot
// assignment, approval) untouched. The rotate path calls it: re-issuing a cert
// in place also mints a fresh portal token (the old plaintext is unrecoverable
// — only its hash was kept — and a rotation/compromise event wants fresh
// credentials anyway). Idempotent shape; returns ErrNotFound when the prefix
// doesn't exist.
// Both credential columns move together — a rotation that swapped only the
// bcrypt hash would leave the OLD key's sha256 indexed and still resolving,
// which is a revoked credential that keeps working.
func (r *Registry) UpdateVendorAPIKey(ctx context.Context, prefix, apiKeyHash, apiKeySHA256 string) error {
	res, err := r.db.ExecContext(ctx,
		`UPDATE vendors SET api_key_hash = $1, api_key_sha256 = $2 WHERE prefix = $3`,
		apiKeyHash, apiKeySHA256, prefix)
	if err != nil {
		return err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return err
	}
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// ApproveVendor flips approved_at + approver. Idempotent; no-op when already
// approved.
func (r *Registry) ApproveVendor(ctx context.Context, prefix, approver string) error {
	now := time.Now().Unix()
	_, err := r.db.ExecContext(ctx,
		`UPDATE vendors SET approved_at = COALESCE(approved_at, $1), approver = COALESCE(approver, $2) WHERE prefix = $3`,
		now, approver, prefix)
	return err
}

// MarkBundle stores the one-time bundle token + path. Token is consumed at
// download (cleared in ConsumeBundleToken).
func (r *Registry) MarkBundle(ctx context.Context, prefix, token, path string) error {
	now := time.Now().Unix()
	_, err := r.db.ExecContext(ctx,
		`UPDATE vendors SET bundle_token = $1, bundle_path = $2, bundle_minted_at = $3 WHERE prefix = $4`,
		token, path, now, prefix)
	return err
}

// RecordBundleDownload stamps a successful bundle fetch: first-download time
// (sticky — COALESCE keeps the original) and a running count. Called only
// AFTER the file has been served successfully, so the record means "the
// tarball left the portal", not "someone asked for it".
//
// This is deliberately separate from token consumption. The token stays valid
// by design (siblings at one org share a bundle), so it can never be the
// download signal; before this existed the only evidence a bundle had ever
// been fetched was atime on the mgmt-host filesystem.
func (r *Registry) RecordBundleDownload(ctx context.Context, prefix string) error {
	now := time.Now().Unix()
	res, err := r.db.ExecContext(ctx,
		`UPDATE vendors
		    SET bundle_downloaded_at = COALESCE(bundle_downloaded_at, $1),
		        bundle_download_count = bundle_download_count + 1
		  WHERE prefix = $2`, now, prefix)
	if err != nil {
		return err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return err
	}
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// ConsumeBundleToken returns the bundle path for a matching token + vendor.
// Returns ErrNotFound when the token doesn't match. The token is NOT cleared
// on read — siblings at the same vendor share one cert+api_key by design
// (see docs/design/operator-onboard-utilities.md §83E), so the bundle URL
// must remain valid across multiple downloads. Token rotation happens at
// operator discretion (an UPDATE) rather than implicitly on first read.
func (r *Registry) ConsumeBundleToken(ctx context.Context, prefix, token string) (string, error) {
	var path string
	err := r.reader().GetContext(ctx, &path,
		`SELECT bundle_path FROM vendors WHERE prefix = $1 AND bundle_token = $2`, prefix, token)
	if errors.Is(err, sqlErrNoRows()) {
		return "", ErrNotFound
	}
	if err != nil {
		return "", err
	}
	return path, nil
}

// CreateTopic inserts a new topic row. Returns ErrTopicExists when the
// (vendor, key_pattern) pair already exists.
func (r *Registry) CreateTopic(ctx context.Context, t *Topic) (int64, error) {
	if err := validateTopic(t); err != nil {
		return 0, invalid(err)
	}
	t.CreatedAt = time.Now().Unix()
	var id int64
	err := r.db.GetContext(ctx, &id,
		`INSERT INTO topics (vendor_prefix, key_pattern, format, schema_ref, description, publish_to_storage, created_at)
		 VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id`,
		t.VendorPrefix, t.KeyPattern, t.Format, t.SchemaRef, t.Description, boolInt(t.PublishToStorage), t.CreatedAt)
	if err != nil {
		if pgstore.IsUniqueViolation(err) {
			return 0, ErrTopicExists
		}
		return 0, fmt.Errorf("insert topic: %w", err)
	}
	t.ID = id
	return id, nil
}

// GetTopic returns a topic by id, or ErrNotFound.
func (r *Registry) GetTopic(ctx context.Context, id int64) (*Topic, error) {
	var t Topic
	err := r.reader().GetContext(ctx, &t, `SELECT * FROM topics WHERE id = $1`, id)
	if errors.Is(err, sqlErrNoRows()) {
		return nil, ErrNotFound
	}
	if err != nil {
		return nil, err
	}
	return &t, nil
}

// ListVendorTopics returns active (non-retired) topics for a vendor.
func (r *Registry) ListVendorTopics(ctx context.Context, vendor string) ([]Topic, error) {
	var ts []Topic
	err := r.reader().SelectContext(ctx, &ts,
		`SELECT * FROM topics WHERE vendor_prefix = $1 AND retired_at IS NULL ORDER BY key_pattern`,
		vendor)
	return ts, err
}

// UpdateTopic updates format / schema_ref / description / publish_to_storage
// in place. Other fields are immutable; the caller should retire + re-create
// for a different key_pattern.
func (r *Registry) UpdateTopic(ctx context.Context, t *Topic) error {
	if err := validateTopic(t); err != nil {
		return invalid(err)
	}
	_, err := r.db.ExecContext(ctx,
		`UPDATE topics SET format = $1, schema_ref = $2, description = $3, publish_to_storage = $4 WHERE id = $5`,
		t.Format, t.SchemaRef, t.Description, boolInt(t.PublishToStorage), t.ID)
	return err
}

// RetireTopic flips retired_at to now. Idempotent. The tombstone row
// stays in place — the UNIQUE(vendor_prefix, key_pattern) slot remains
// occupied, so re-registering the same key_pattern returns
// ErrTopicExists. Audit-preserving. Callers that need to free the slot
// for reuse should use DeleteTopic instead.
func (r *Registry) RetireTopic(ctx context.Context, id int64) error {
	now := time.Now().Unix()
	_, err := r.db.ExecContext(ctx,
		`UPDATE topics SET retired_at = COALESCE(retired_at, $1) WHERE id = $2`, now, id)
	return err
}

// DeleteTopic removes a topic row outright. FK CASCADE on topic_schemas
// and topic_code_examples wipes the dependents. Frees the
// UNIQUE(vendor_prefix, key_pattern) slot — re-registering the same
// key_pattern after a DeleteTopic succeeds. Returns ErrNotFound if the
// row is already gone. Audit history is NOT preserved on the row itself;
// callers should LogAuditEvent before invoking so the action is
// reconstructible from audit_events.
func (r *Registry) DeleteTopic(ctx context.Context, id int64) error {
	res, err := r.db.ExecContext(ctx, `DELETE FROM topics WHERE id = $1`, id)
	if err != nil {
		return err
	}
	n, err := res.RowsAffected()
	if err != nil {
		return err
	}
	if n == 0 {
		return ErrNotFound
	}
	return nil
}

// CountActiveTopics returns the number of non-retired topics owned by the
// vendor. Used by DeleteVendor's pre-check to refuse vendor removal while
// the vendor still owns live topics.
func (r *Registry) CountActiveTopics(ctx context.Context, vendor string) (int, error) {
	var n int
	err := r.reader().GetContext(ctx, &n,
		`SELECT COUNT(*) FROM topics WHERE vendor_prefix = $1 AND retired_at IS NULL`, vendor)
	return n, err
}

// DeclaredTopicCounts returns the live (non-retired) topic-row count per
// vendor prefix in one query, for list surfaces that must not fan out a
// CountActiveTopics per row. Prefixes with no live rows are absent from the
// map; callers read absence as zero.
func (r *Registry) DeclaredTopicCounts(ctx context.Context) (map[string]int64, error) {
	var rows []struct {
		VendorPrefix string `db:"vendor_prefix"`
		N            int64  `db:"n"`
	}
	if err := r.reader().SelectContext(ctx, &rows,
		`SELECT vendor_prefix, COUNT(*) AS n FROM topics WHERE retired_at IS NULL GROUP BY vendor_prefix`); err != nil {
		return nil, fmt.Errorf("declared topic counts: %w", err)
	}
	out := make(map[string]int64, len(rows))
	for _, row := range rows {
		out[row.VendorPrefix] = row.N
	}
	return out, nil
}

// DeleteVendor removes a vendor row. The vendor must have zero non-retired
// topics; the caller is expected to retire its topics first via RetireTopic.
// Returns ErrNotFound if the vendor doesn't exist. The schema's FK
// `ON DELETE CASCADE` would otherwise wipe retired topics + their schemas +
// code-example rows — which is the intended cleanup of a fully-tombstoned
// vendor.
func (r *Registry) DeleteVendor(ctx context.Context, prefix string) error {
	if _, err := r.GetVendor(ctx, prefix); err != nil {
		return err
	}
	n, err := r.CountActiveTopics(ctx, prefix)
	if err != nil {
		return err
	}
	if n > 0 {
		return fmt.Errorf("vendor %q has %d non-retired topic(s); retire them first", prefix, n)
	}
	_, err = r.db.ExecContext(ctx, `DELETE FROM vendors WHERE prefix = $1`, prefix)
	return err
}

// LogAuditEvent appends an audit row.
func (r *Registry) LogAuditEvent(ctx context.Context, actor, action, target, details string) error {
	if details == "" {
		details = "{}"
	}
	_, err := r.db.ExecContext(ctx,
		`INSERT INTO audit_events (occurred_at, actor, action, target, details)
		 VALUES ($1, $2, $3, $4, $5)`,
		time.Now().Unix(), actor, action, target, details)
	return err
}

// AuditEvent is one durable audit row, read back for the operator-gated
// audit-read API (the id-console's onboarding audit view reads THIS record —
// its own ring buffer is per-container and dies on redeploy; ADR 0860 interim).
type AuditEvent struct {
	ID         int64  `json:"id"`
	OccurredAt int64  `json:"occurred_at"`
	Actor      string `json:"actor"`
	Action     string `json:"action"`
	Target     string `json:"target"`
	Details    string `json:"details"`
}

// AuditEvents returns the newest audit rows, optionally filtered to an action
// prefix (e.g. "onboard." for the onboarding trail). Newest first.
func (r *Registry) AuditEvents(ctx context.Context, actionPrefix string, limit int) ([]AuditEvent, error) {
	if limit <= 0 || limit > 1000 {
		limit = 200
	}
	q := `SELECT id, occurred_at, actor, action, target, details FROM audit_events`
	args := []any{}
	if actionPrefix != "" {
		q += ` WHERE action LIKE ? ESCAPE '\'`
		args = append(args, escapeLike(actionPrefix)+"%")
	}
	q += ` ORDER BY id DESC LIMIT ?`
	args = append(args, limit)
	q = sqlx.Rebind(sqlx.DOLLAR, q)
	rows, cancel, err := r.reader().QueryContext(ctx, q, args...)
	if err != nil {
		return nil, err
	}
	defer cancel()
	defer rows.Close()
	out := []AuditEvent{}
	for rows.Next() {
		var e AuditEvent
		if err := rows.Scan(&e.ID, &e.OccurredAt, &e.Actor, &e.Action, &e.Target, &e.Details); err != nil {
			return nil, err
		}
		out = append(out, e)
	}
	return out, rows.Err()
}

// escapeLike escapes SQL LIKE wildcards in a caller-supplied prefix.
func escapeLike(s string) string {
	s = strings.ReplaceAll(s, `\`, `\\`)
	s = strings.ReplaceAll(s, `%`, `\%`)
	return strings.ReplaceAll(s, `_`, `\_`)
}

// Allowed format set. Two schema-bound formats take a colon-suffix shape:
// "protobuf:<msg-name>" (message inside an uploaded .fds) and
// "xml:<root-element>" (top-level element declared in an uploaded .xsd).
// Bare "xml" is the opaque-XML sibling of "raw"/"text": XML bytes carried
// with no schema binding (e.g. Cursor-on-Target).
var allowedFormats = map[string]bool{
	"json": true, "cbor": true, "msgpack": true, "text": true, "raw": true,
	"xml": true,
}

// validateTopic checks key_pattern / format invariants. Vendor-prefix match
// is enforced here in addition to the API-layer scope check, so DB-direct
// inserts can't sneak past.
func validateTopic(t *Topic) error {
	if err := ValidVendorPrefix(t.VendorPrefix); err != nil {
		return err
	}
	parts := strings.Split(t.KeyPattern, "/")
	if len(parts) < 3 {
		return fmt.Errorf("key_pattern %q: must be at least <vendor>/<system>/...; got %d segments", t.KeyPattern, len(parts))
	}
	if parts[0] != t.VendorPrefix {
		return fmt.Errorf("key_pattern %q: first segment %q must equal org prefix %q", t.KeyPattern, parts[0], t.VendorPrefix)
	}
	for i, p := range parts {
		if p == "" {
			return fmt.Errorf("key_pattern %q: empty segment", t.KeyPattern)
		}
		// `**` (multi-segment wildcard) is never registrable: it makes the
		// pattern's key population unbounded and defeats schema binding.
		if strings.Contains(p, "**") {
			return fmt.Errorf("key_pattern %q: segment %q: '**' is not registrable", t.KeyPattern, p)
		}
		// A single `*` is allowed as a COMPLETE mid-pattern segment (the
		// bounded-selectable-dim shape of pubsub-best-practices §1b, e.g.
		// adsb/aircraft/*/v1) — never the first segment (that is the org
		// prefix, the write-ACL anchor) and never embedded in a segment.
		// Concrete keys published under the pattern resolve to this topic's
		// schema via keyexpr intersection (GetTopicByVendorKey fallback).
		if p == "*" {
			if i == 0 {
				return fmt.Errorf("key_pattern %q: first segment must be the org prefix, not a wildcard", t.KeyPattern)
			}
			continue
		}
		// Zenoh key expressions forbid '?' '#' '$' '*' inside segments; a
		// concrete registered segment may not carry them ('*' only as the
		// whole segment, handled above).
		if strings.ContainsAny(p, "?#$*") {
			return fmt.Errorf("key_pattern %q: segment %q contains reserved char ?#$* ('*' is only valid as a complete segment)", t.KeyPattern, p)
		}
	}
	if !allowedFormats[t.Format] &&
		!strings.HasPrefix(t.Format, "protobuf:") &&
		!strings.HasPrefix(t.Format, "xml:") {
		return fmt.Errorf("format %q: must be one of json/cbor/msgpack/text/raw/xml or protobuf:<msg-name> or xml:<root-element>", t.Format)
	}
	if strings.HasPrefix(t.Format, "protobuf:") && len(t.Format) <= len("protobuf:") {
		return fmt.Errorf("format %q: protobuf format requires a message name (protobuf:<msg-name>)", t.Format)
	}
	if strings.HasPrefix(t.Format, "xml:") && len(t.Format) <= len("xml:") {
		return fmt.Errorf("format %q: xml format requires a root element name (xml:<root-element>)", t.Format)
	}
	return nil
}

// ApprovedVendorPrefixes returns the prefixes of vendors with approved_at
// not null. Used by the ACL renderer.
func (r *Registry) ApprovedVendorPrefixes(ctx context.Context) ([]string, error) {
	var ps []string
	err := r.reader().SelectContext(ctx, &ps,
		`SELECT prefix FROM vendors WHERE approved_at IS NOT NULL ORDER BY prefix`)
	return ps, err
}

// registryWriteLock is the transaction-scoped advisory lock every
// read-then-write transaction in this package takes first (beginLocked).
// Arbitrary but fixed; it differs from pgstore's migration lock.
const registryWriteLock int64 = 0x676f61745f726567 // "goat_reg"

// beginLocked starts a transaction and takes registryWriteLock inside it, so
// the transaction's reads and its writes see no other locked writer in
// between. This is what the single SQLite connection gave every transaction;
// the lock is released at commit or rollback.
func (r *Registry) beginLocked(ctx context.Context) (*sqlx.Tx, error) {
	tx, err := r.db.BeginTxx(ctx, nil)
	if err != nil {
		return nil, err
	}
	if _, err := tx.ExecContext(ctx, `SELECT pg_advisory_xact_lock($1)`, registryWriteLock); err != nil {
		_ = tx.Rollback()
		return nil, fmt.Errorf("registry write lock: %w", err)
	}
	return tx, nil
}

// boolInt is how a Go bool is stored in the registry's 0/1 BIGINT columns
// (synthetic, sponsor_authorized, publish_to_storage, persist, network_admin).
// The SQLite driver converted a bool argument implicitly; pgx does not encode a
// bool into an int8 parameter. Reads need nothing: database/sql scans 0/1 into
// a bool.
func boolInt(b bool) int64 {
	if b {
		return 1
	}
	return 0
}
