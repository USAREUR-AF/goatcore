package onboardstore

import (
	"context"
	"errors"
	"testing"
)

// GHSA-q7qx: the portal lets a key under a service label name another person
// in X-Goat-Operator-Actor, so the console must never file or approve a
// person's key under one.
func TestRequestOperatorKey_refusesReservedServiceHandles(t *testing.T) {
	ctx := context.Background()
	s := opStore(t)
	for _, h := range []string{"operator", "id-console", "onboard-api", " Operator ", "ID-Console", "ONBOARD-API"} {
		if row, err := s.RequestOperatorKey(ctx, &OperatorKey{Handle: h, Env: "zagros"}); !errors.Is(err, ErrOperatorKeyReservedHandle) {
			t.Errorf("request for %q: row=%v err=%v, want ErrOperatorKeyReservedHandle", h, row, err)
		}
	}
	rows, err := s.ListOperatorKeys(ctx, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 0 {
		t.Fatalf("refused requests left %d rows", len(rows))
	}
	// Handles that merely contain a label are people, not services.
	for _, h := range []string{"jane-operator", "operator2", "id-console-admin"} {
		if _, err := s.RequestOperatorKey(ctx, &OperatorKey{Handle: h, Env: "zagros"}); err != nil {
			t.Errorf("request for %q: %v", h, err)
		}
	}
}

// A row filed under a service label before the request-time rule existed is
// not approvable: no secret is minted and the row stays requested.
func TestApproveOperatorKey_refusesLegacyReservedRow(t *testing.T) {
	ctx := context.Background()
	s := opStore(t)
	if _, err := s.db.ExecContext(ctx, `
		INSERT INTO operator_keys (id, created_at, handle, email, env, reason, status)
		VALUES ($1, $2, $3, $4, $5, $6, $7)`,
		"legacy-op", int64(1), "operator", "jane@example.org", "zagros", "", OperatorKeyRequested); err != nil {
		t.Fatalf("seed legacy row: %v", err)
	}
	if _, err := s.ApproveOperatorKey(ctx, "legacy-op", "bob@example.org", nil); !errors.Is(err, ErrOperatorKeyReservedHandle) {
		t.Fatalf("approve legacy reserved row: err=%v, want ErrOperatorKeyReservedHandle", err)
	}
	row, err := s.OperatorKey(ctx, "legacy-op")
	if err != nil {
		t.Fatal(err)
	}
	if row.Status != OperatorKeyRequested || row.Fingerprint != "" {
		t.Fatalf("legacy row after refused approve: status=%q fingerprint=%q, want requested and no key", row.Status, row.Fingerprint)
	}
}
