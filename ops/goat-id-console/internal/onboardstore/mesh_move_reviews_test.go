package onboardstore

import (
	"errors"
	"fmt"
	"sync/atomic"
	"testing"
	"time"
)

func TestMeshMoveReviewsConcurrentReplayAndExpiry(t *testing.T) {
	s := openTest(t)
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
		candidate.Result = fmt.Sprintf(`{"attempt":%d}`, counter.Add(1))
		stored, inserted, err := s.SaveMeshMoveReview(ctx, candidate)
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
}
