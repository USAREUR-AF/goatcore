// Package onboardstore is the persistent store for the three-stage delegated
// candidate onboarding (ADR 1062/1071), extracted out of the data-fabric portal
// (goat-zenoh-portal) into the identity plane (goat-id-console) per
// docs/design/onboarding-extraction-plan.md.
//
// It owns the console's durable workflow tables: onboarding proposals and
// invites, credential requests, follow-up actions, and the separate probation
// reactivation-request lane. The org/slot/cert registry stays elsewhere; this
// package never mutates mesh membership.
//
// Backed by jmoiron/sqlx over Postgres through pgx's database/sql driver (ADR
// 1165 step A: the SQLite tables moved unchanged). All SQL is raw, no ORM.
//
// Concurrency: single statements are atomic, and every read-then-write path
// runs in one transaction that locks what it read (SELECT ... FOR UPDATE, or an
// advisory lock where no row exists yet), so two console processes, or two
// requests in one, cannot interleave. On SQLite one open connection
// (SetMaxOpenConns(1)) and the file lock gave the same guarantee.
package onboardstore

import (
	"context"
	_ "embed"
	"fmt"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/pgstore"
	"github.com/jmoiron/sqlx"
)

//go:embed schema.sql
var schemaSQL string

// Migrations is the store's versioned schema. Version 1 is the SQLite schema,
// tables and columns unchanged.
var Migrations = []pgstore.Migration{{Version: 1, SQL: schemaSQL}}

// Tables are the store's tables in version 1, in foreign-key order (a table
// comes after every table it references). cmd/store-migrate copies them in
// this order.
var Tables = []string{"onboard_proposals", "onboard_invites", "ssh_keys", "operator_keys", "onboard_actions", "probation_reactivation_requests"}

// Store wraps the Postgres handle for the onboarding tables.
type Store struct {
	db *sqlx.DB
}

// Open connects to the Postgres database at dsn and applies any migration not
// yet recorded. Safe to call repeatedly and from more than one process.
func Open(dsn string) (*Store, error) {
	ctx := context.Background()
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
	// Additive columns, kept from the SQLite build where they brought an older
	// database up to the current shape. Version 1 already has every one of them,
	// so on Postgres these are checks that find the column and do nothing.
	for _, m := range []struct{ table, column, decl string }{
		{"onboard_proposals", "escalated", "BIGINT NOT NULL DEFAULT 0"},
		{"onboard_proposals", "complete", "BIGINT NOT NULL DEFAULT 1"},
		{"onboard_proposals", "petitions", `TEXT COLLATE "C" NOT NULL DEFAULT '[]'`},
		{"onboard_proposals", "tier", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "setup_key", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "handset_key", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "bundle_path", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "repo_grants", `TEXT COLLATE "C" NOT NULL DEFAULT '[]'`},
		{"onboard_proposals", "repo_overrides", `TEXT COLLATE "C" NOT NULL DEFAULT '{}'`},
		{"onboard_proposals", "sponsor_name", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "sponsor_approved_by", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "sponsor_approved_at", "BIGINT NOT NULL DEFAULT 0"},
		{"onboard_proposals", "retroactive", "BIGINT NOT NULL DEFAULT 0"},
		{"onboard_proposals", "collab_account", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "collab_activated", "BIGINT NOT NULL DEFAULT 0"},
		{"onboard_proposals", "org_source", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "restored_by", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "restored_at", "BIGINT NOT NULL DEFAULT 0"},
		{"onboard_proposals", "offboard_requested_by", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "offboard_requested_at", "BIGINT NOT NULL DEFAULT 0"},
		{"onboard_proposals", "offboard_requested_by_identity", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "offboard_operator_audit", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "offboard_reason_class", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "offboard_reason_text", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "offboard_effective", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_proposals", "claimed_at", "BIGINT NOT NULL DEFAULT 0"},
		{"onboard_invites", "sponsor_name", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_invites", "candidate_email", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_invites", "sent_at", "BIGINT"},
		{"onboard_invites", "sent_via", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_invites", "revoked_at", "BIGINT"},
		{"onboard_invites", "archived_at", "BIGINT"},
		{"onboard_actions", "params", `TEXT COLLATE "C" NOT NULL DEFAULT '{}'`},
		{"operator_keys", "holder_identity_id", `TEXT COLLATE "C" NOT NULL DEFAULT ''`}, // GHSA-mxf4
		{"onboard_actions", "target_env", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_actions", "target_handle", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"onboard_actions", "claim_token", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"probation_reactivation_requests", "decided_by", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"probation_reactivation_requests", "decided_at", "BIGINT NOT NULL DEFAULT 0"},
		{"probation_reactivation_requests", "decision_reason", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"ssh_keys", "skip_reason", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"ssh_keys", "skip_reported_by", `TEXT COLLATE "C" NOT NULL DEFAULT ''`},
		{"ssh_keys", "skip_reported_at", "BIGINT NOT NULL DEFAULT 0"},
	} {
		if err := pgstore.AddColumnIfMissing(ctx, db, m.table, m.column, m.decl); err != nil {
			_ = db.Close()
			return nil, fmt.Errorf("migrate %s.%s: %w", m.table, m.column, err)
		}
	}
	// GHSA-2w7v: who filed each operator-key request. Its own table, created
	// here for new and existing databases alike, and deliberately not one of
	// the tables storemigrate copies: a request that arrives without a row
	// here (filed before this existed, or imported from a SQLite checkpoint)
	// has no recorded filer, and the two-person rule refuses to decide it.
	if _, err := db.ExecContext(ctx, `CREATE TABLE IF NOT EXISTS operator_key_requesters (
		id           TEXT COLLATE "C" PRIMARY KEY,
		requested_by TEXT COLLATE "C" NOT NULL
	)`); err != nil {
		_ = db.Close()
		return nil, fmt.Errorf("create operator_key_requesters: %w", err)
	}
	if err := backfillOrgSource(ctx, db); err != nil {
		_ = db.Close()
		return nil, fmt.Errorf("backfill onboard_proposals.org_source: %w", err)
	}
	return &Store{db: db}, nil
}

// The proposals whose invite did, and did not, pin an org. The CASE fixes the
// evaluation order, so the jsonb cast never sees a malformed prebound_json:
// such a row matches neither query and its org_source stays empty. (SQLite:
// json_valid / json_extract(prebound_json, '$.org'); Postgres 16: IS JSON /
// prebound_json::jsonb->>'org'. Both give NULL for a missing key and for a
// document that is not an object.)
const (
	backfillOrgSourceSponsor = `UPDATE onboard_proposals SET org_source = $1
		WHERE org_source = '' AND id IN (
			SELECT proposal_id FROM onboard_invites
			WHERE proposal_id IS NOT NULL
			  AND CASE WHEN prebound_json IS JSON
			           THEN TRIM(COALESCE(prebound_json::jsonb->>'org', '')) != '' END)`
	backfillOrgSourceTyped = `UPDATE onboard_proposals SET org_source = $1
		WHERE org_source = '' AND id IN (
			SELECT proposal_id FROM onboard_invites
			WHERE proposal_id IS NOT NULL
			  AND CASE WHEN prebound_json IS JSON
			           THEN TRIM(COALESCE(prebound_json::jsonb->>'org', '')) = '' END)`
)

// backfillOrgSource recovers org_source for proposals written before the column
// existed. The assembled manifest keeps no trace of a sponsor's pre-binding, but
// the invite that produced the proposal does: onboard_invites.prebound_json.org
// is the org the sponsor pinned, and onboard_invites.proposal_id links the two.
// A pinned org gives "sponsor" and an invite without one gives "typed", the same
// rule the submit path applies. A proposal with no invite behind it (API and
// batch intakes) keeps an empty value, which the review page reads as a claim:
// the conservative default, left only where the truth cannot be recovered.
//
// Runs on every Open and only ever fills an empty value, so it is idempotent
// and never overwrites a recorded provenance.
func backfillOrgSource(ctx context.Context, db *sqlx.DB) error {
	if _, err := db.ExecContext(ctx, backfillOrgSourceSponsor, OrgSourceSponsor); err != nil {
		return fmt.Errorf("sponsor-pinned rows: %w", err)
	}
	if _, err := db.ExecContext(ctx, backfillOrgSourceTyped, OrgSourceTyped); err != nil {
		return fmt.Errorf("typed rows: %w", err)
	}
	return nil
}

// Close releases the database handle.
func (s *Store) Close() error { return s.db.Close() }

// DB exposes the underlying handle for tests and out-of-band queries. Not part
// of the stable surface.
func (s *Store) DB() *sqlx.DB { return s.db }
