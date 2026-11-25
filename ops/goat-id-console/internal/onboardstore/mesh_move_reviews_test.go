package onboardstore

import (
	"errors"
	"fmt"
	"sync/atomic"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/audit"
	"github.com/usareur-af/goatcore/ops/goat-id-console/internal/pgtest"
)

func TestMeshMoveReviewsConcurrentReplayAndExpiry(t *testing.T) {
	dsn := pgtest.DSN(t)
	s, err := Open(dsn)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	ctx := t.Context()
	now := time.Now().UTC().Truncate(time.Second)
	row := MeshMoveReview{
		ID: "move-fixture", ActorID: "fixture-actor", Realm: "tenant", Identity: "fixture-member",
		Site: "zagros", PeerID: "fixture-peer", Status: 200, SavedAt: now, ExpiresAt: now.Add(7 * 24 * time.Hour),
	}
	var counter, created atomic.Int32
	results := make(chan MeshMoveReview, racers)
	wins, errs := race(t, func() error {
		candidate := row
		attempt := fmt.Sprint(counter.Add(1))
		candidate.Result = fmt.Sprintf(`{"attempt":%q}`, attempt)
		event := audit.Event{Action: "mesh_move_declaration_review", RequestID: attempt, Result: "ok"}
		stored, inserted, err := s.SaveMeshMoveReview(ctx, candidate, event)
		if inserted {
			created.Add(1)
		}
		if err == nil {
			results <- stored
		}
		return err
	})
	close(results)
	if wins != racers || len(errs) != 0 || created.Load() != 1 {
		t.Fatalf("handled %d of %d confirmations and created %d operations: %v", wins, racers, created.Load(), errs)
	}
	stored, err := s.GetMeshMoveReview(ctx, row.ID, now)
	if err != nil {
		t.Fatal(err)
	}
	for result := range results {
		if result.ID != row.ID || result.Result != stored.Result || !result.ExpiresAt.Equal(row.ExpiresAt) {
			t.Fatalf("duplicate returned a different operation: %+v", result)
		}
	}
	for _, tc := range []struct {
		actor string
		at    time.Time
		want  int
	}{
		{row.ActorID, row.ExpiresAt.Add(-time.Second), 1},
		{"other-actor", now, 0},
		{row.ActorID, row.ExpiresAt, 0},
	} {
		rows, err := s.ListMeshMoveReviews(ctx, tc.actor, row.Realm, row.Identity, row.Site, tc.at)
		if err != nil || len(rows) != tc.want {
			t.Fatalf("list for %s at %s: %d rows, want %d; error %v", tc.actor, tc.at, len(rows), tc.want, err)
		}
	}
	if err := s.DeleteExpiredMeshMoveReviews(ctx, row.ExpiresAt); err != nil {
		t.Fatal(err)
	}
	if _, err := s.GetMeshMoveReview(ctx, row.ID, now); !errors.Is(err, ErrNotFound) {
		t.Fatalf("expired row still exists: %v", err)
	}
	reopened, err := Open(dsn)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = reopened.Close() })
	events, err := reopened.RecentMeshMoveAudit(ctx, 10)
	if err != nil || len(events) != 1 {
		t.Fatalf("want one durable audit event after expiry: %+v, %v", events, err)
	}
	if stored.Result != fmt.Sprintf(`{"attempt":%q}`, events[0].RequestID) || !events[0].TS.Equal(now) {
		t.Fatalf("audit event does not match the winning review: %+v", events[0])
	}
}

func TestMeshMoveReviewAuditFailureRollsBackResult(t *testing.T) {
	s := openTest(t)
	ctx := t.Context()
	now := time.Now().UTC()
	row := MeshMoveReview{
		ID: "move-refused", ActorID: "fixture-actor", Realm: "tenant", Identity: "fixture-member",
		Site: "zagros", PeerID: "fixture-peer", Status: 502, Result: `{"error":"fixture failure"}`,
		SavedAt: now, ExpiresAt: now.Add(time.Hour),
	}
	event := audit.Event{Action: "mesh_move_declaration_review", Result: "error"}
	if _, err := s.db.ExecContext(ctx, `ALTER TABLE mesh_move_review_audit ADD CONSTRAINT refuse_audit CHECK (operation_id <> 'move-refused')`); err != nil {
		t.Fatal(err)
	}
	if _, created, err := s.SaveMeshMoveReview(ctx, row, event); created || err == nil {
		t.Fatalf("audit insert failure returned a saved result: created=%v, err=%v", created, err)
	}
	var count int
	if err := s.db.GetContext(ctx, &count, "SELECT count(*) FROM mesh_move_reviews WHERE id = $1", row.ID); err != nil || count != 0 {
		t.Fatalf("failed audit insert left %d reviews: %v", count, err)
	}
	if _, err := s.db.ExecContext(ctx, `ALTER TABLE mesh_move_review_audit DROP CONSTRAINT refuse_audit`); err != nil {
		t.Fatal(err)
	}
	if _, created, err := s.SaveMeshMoveReview(ctx, row, event); !created || err != nil {
		t.Fatalf("retry could not save review and audit: created=%v, err=%v", created, err)
	}
	events, err := s.RecentMeshMoveAudit(ctx, 10)
	if err != nil || len(events) != 1 || events[0].Result != "error" {
		t.Fatalf("want the failed preparation's durable audit record: %+v, %v", events, err)
	}
}
