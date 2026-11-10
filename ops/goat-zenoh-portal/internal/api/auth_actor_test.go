package api

import (
	"context"
	"net/http/httptest"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// authTestServer builds a minimal Server with a real empty registry so the
// vendor-walk leg of resolvePrincipal has something to query.
func authTestServer(t *testing.T, cfg *config.Config) *Server {
	t.Helper()
	reg, err := registry.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("registry: %v", err)
	}
	t.Cleanup(func() { _ = reg.Close() })
	return &Server{cfg: cfg, reg: reg}
}

// A per-person operator key resolves to its handle in Principal.Actor.
func TestResolvePrincipal_NamedOperatorKeySetsActor(t *testing.T) {
	s := authTestServer(t, &config.Config{
		OperatorAPIKeys: map[string]string{"jane-operator": "k1", "alice-op": "k2"},
	})
	p, err := s.resolvePrincipal(context.Background(), "k2")
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if p == nil || !p.Operator {
		t.Fatalf("expected operator principal, got %#v", p)
	}
	if p.Actor != "alice-op" {
		t.Errorf("Actor = %q, want alice-op", p.Actor)
	}
}

// The legacy single key resolves to the default "operator" handle.
func TestResolvePrincipal_LegacyKeyDefaultsActor(t *testing.T) {
	s := authTestServer(t, &config.Config{OperatorAPIKey: "legacy"})
	p, err := s.resolvePrincipal(context.Background(), "legacy")
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if p == nil || !p.Operator {
		t.Fatalf("expected operator principal, got %#v", p)
	}
	if p.Actor != defaultOperatorHandle {
		t.Errorf("Actor = %q, want %q", p.Actor, defaultOperatorHandle)
	}
}

// An unknown token resolves to no principal (not an operator).
func TestResolvePrincipal_UnknownTokenIsNil(t *testing.T) {
	s := authTestServer(t, &config.Config{
		OperatorAPIKey:  "legacy",
		OperatorAPIKeys: map[string]string{"jane-operator": "k1"},
	})
	p, err := s.resolvePrincipal(context.Background(), "nope")
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if p != nil {
		t.Errorf("expected nil principal for unknown token, got %#v", p)
	}
}

// Named keys win over the legacy single key when their values collide on a
// presented token: the named handle is attributed, not "operator".
func TestResolvePrincipal_NamedKeyWinsOnCollision(t *testing.T) {
	s := authTestServer(t, &config.Config{
		OperatorAPIKey:  "shared",
		OperatorAPIKeys: map[string]string{"jane-operator": "shared"},
	})
	p, err := s.resolvePrincipal(context.Background(), "shared")
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if p == nil || p.Actor != "jane-operator" {
		t.Fatalf("expected jane-operator actor, got %#v", p)
	}
}

// actorOf returns the operator's Actor, and falls back to "operator" when no
// principal is attached.
func TestActorOf(t *testing.T) {
	r := httptest.NewRequest("GET", "/api/progress", nil)
	if got := actorOf(r); got != defaultOperatorHandle {
		t.Errorf("actorOf with no principal = %q, want %q", got, defaultOperatorHandle)
	}

	ctx := context.WithValue(r.Context(), principalKey, &Principal{Operator: true, Actor: "jane-operator"})
	r2 := r.WithContext(ctx)
	if got := actorOf(r2); got != "jane-operator" {
		t.Errorf("actorOf with operator principal = %q, want jane-operator", got)
	}
}
