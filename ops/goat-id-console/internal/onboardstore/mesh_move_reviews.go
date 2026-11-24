package onboardstore

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/audit"
)

const meshMoveReviewSchema = `
CREATE TABLE IF NOT EXISTS mesh_move_reviews (
    id TEXT COLLATE "C" PRIMARY KEY,
    actor_id TEXT COLLATE "C" NOT NULL,
    realm TEXT COLLATE "C" NOT NULL,
    identity_id TEXT COLLATE "C" NOT NULL,
    site TEXT COLLATE "C" NOT NULL,
    peer_id TEXT COLLATE "C" NOT NULL,
    http_status INTEGER NOT NULL,
    result_json TEXT COLLATE "C" NOT NULL,
    saved_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS mesh_move_reviews_scope_idx
    ON mesh_move_reviews (actor_id, realm, identity_id, site, peer_id, saved_at DESC);
CREATE INDEX IF NOT EXISTS mesh_move_reviews_expiry_idx ON mesh_move_reviews (expires_at);
`

// Audit records outlive the short-lived handoff and have their own retention.
const meshMoveReviewAuditSchema = `
CREATE TABLE IF NOT EXISTS mesh_move_review_audit (
    operation_id TEXT COLLATE "C" PRIMARY KEY,
    recorded_at TIMESTAMPTZ NOT NULL,
    event_json JSONB NOT NULL
);
CREATE INDEX IF NOT EXISTS mesh_move_review_audit_recent_idx
    ON mesh_move_review_audit (recorded_at DESC, operation_id DESC);
`

// MeshMoveReview is the immutable outcome of a confirmed declaration handoff.
type MeshMoveReview struct {
	ID        string    `db:"id"`
	ActorID   string    `db:"actor_id"`
	Realm     string    `db:"realm"`
	Identity  string    `db:"identity_id"`
	Site      string    `db:"site"`
	PeerID    string    `db:"peer_id"`
	Status    int       `db:"http_status"`
	Result    string    `db:"result_json"`
	SavedAt   time.Time `db:"saved_at"`
	ExpiresAt time.Time `db:"expires_at"`
}

const meshMoveReviewColumns = "id, actor_id, realm, identity_id, site, peer_id, http_status, result_json, saved_at, expires_at"

// Older reviews without a durable record can expire normally but cannot be
// handed off again. Their original audit outcome cannot be reconstructed.
const auditedMeshMoveReview = "EXISTS (SELECT 1 FROM mesh_move_review_audit WHERE operation_id = mesh_move_reviews.id)"

func (s *Store) GetMeshMoveReview(ctx context.Context, id string, now time.Time) (MeshMoveReview, error) {
	var row MeshMoveReview
	err := s.db.GetContext(ctx, &row, "SELECT "+meshMoveReviewColumns+" FROM mesh_move_reviews WHERE id = $1 AND expires_at > $2 AND "+auditedMeshMoveReview, id, now)
	if errors.Is(err, sql.ErrNoRows) {
		err = ErrNotFound
	}
	return row, err
}

// SaveMeshMoveReview commits the result and its audit event in one SQL
// statement. Retrying a confirmation returns the result of the winning save.
func (s *Store) SaveMeshMoveReview(ctx context.Context, row MeshMoveReview, event audit.Event) (MeshMoveReview, bool, error) {
	var stored MeshMoveReview
	event.TS = row.SavedAt
	payload, err := json.Marshal(event)
	if err != nil {
		return stored, false, err
	}
	err = s.db.GetContext(ctx, &stored, `WITH saved AS (
		INSERT INTO mesh_move_reviews (`+meshMoveReviewColumns+`)
		VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
		ON CONFLICT (id) DO NOTHING
		RETURNING `+meshMoveReviewColumns+`
	), audited AS (
		INSERT INTO mesh_move_review_audit (operation_id, recorded_at, event_json)
		SELECT id, saved_at, $11::jsonb FROM saved
		RETURNING operation_id
	)
	SELECT `+meshMoveReviewColumns+` FROM saved JOIN audited ON saved.id = audited.operation_id`,
		row.ID, row.ActorID, row.Realm, row.Identity, row.Site, row.PeerID, row.Status, row.Result, row.SavedAt, row.ExpiresAt, string(payload))
	if errors.Is(err, sql.ErrNoRows) {
		stored, err = s.GetMeshMoveReview(ctx, row.ID, row.SavedAt)
		return stored, false, err
	}
	return stored, err == nil, err
}

func (s *Store) RecentMeshMoveAudit(ctx context.Context, limit int) ([]audit.Event, error) {
	var payloads []string
	err := s.db.SelectContext(ctx, &payloads, `SELECT event_json::text FROM mesh_move_review_audit
		ORDER BY recorded_at DESC, operation_id DESC LIMIT $1`, limit)
	if err != nil {
		return nil, err
	}
	events := make([]audit.Event, 0, len(payloads))
	for _, payload := range payloads {
		var event audit.Event
		if err := json.Unmarshal([]byte(payload), &event); err != nil {
			return nil, err
		}
		events = append(events, event)
	}
	return events, nil
}

func (s *Store) ListMeshMoveReviews(ctx context.Context, actor, realm, identity, site string, now time.Time) ([]MeshMoveReview, error) {
	var rows []MeshMoveReview
	err := s.db.SelectContext(ctx, &rows, `SELECT DISTINCT ON (peer_id) `+meshMoveReviewColumns+`
		FROM mesh_move_reviews
		WHERE actor_id = $1 AND realm = $2 AND identity_id = $3 AND site = $4 AND expires_at > $5 AND `+auditedMeshMoveReview+`
		ORDER BY peer_id, saved_at DESC, id DESC`, actor, realm, identity, site, now)
	return rows, err
}

func (s *Store) DeleteExpiredMeshMoveReviews(ctx context.Context, now time.Time) error {
	_, err := s.db.ExecContext(ctx, "DELETE FROM mesh_move_reviews WHERE expires_at <= $1", now)
	return err
}
