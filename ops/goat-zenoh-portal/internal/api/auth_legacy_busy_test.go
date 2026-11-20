package api

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// GHSA-fgm6 review round 2. When the legacy gate refuses to check a key
// (queue full) the answer must say "retry", not "wrong key": a client does
// not retry a 401, so a 401 here would deny every not-yet-backfilled vendor
// for as long as a flood lasts.

// withGate swaps the process-wide gate for g for the duration of the test.
func withGate(t *testing.T, g *legacyBcryptGate) {
	t.Helper()
	prev := legacyBcrypt
	legacyBcrypt = g
	t.Cleanup(func() { legacyBcrypt = prev })
}

// fullGate returns a gate whose queue is already full, so any compare that
// is not answered from the cache is refused at once.
func fullGate() *legacyBcryptGate {
	g := newLegacyBcryptGate()
	g.pending = cap(g.slots) + g.maxQueue
	return g
}

func authCall(t *testing.T, s *Server, token string) *httptest.ResponseRecorder {
	t.Helper()
	h := s.AuthMiddleware(true, func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(http.StatusNoContent) })
	req := httptest.NewRequest(http.MethodGet, "/api/vendors", nil)
	req.Header.Set("Authorization", "Bearer "+token)
	rec := httptest.NewRecorder()
	h(rec, req)
	return rec
}

func problemCode(t *testing.T, rec *httptest.ResponseRecorder) string {
	t.Helper()
	if ct := rec.Header().Get("Content-Type"); !strings.HasPrefix(ct, "application/problem+json") {
		t.Fatalf("content-type %q; want a problem", ct)
	}
	var p Problem
	if err := json.Unmarshal(rec.Body.Bytes(), &p); err != nil {
		t.Fatalf("decode problem: %v (%s)", err, rec.Body)
	}
	if p.Status != rec.Code || p.Error.Code != p.Code {
		t.Fatalf("inconsistent problem %+v for status %d", p, rec.Code)
	}
	return p.Code
}

// A full queue answers 503 auth_busy with Retry-After, for the correct key
// too, and counts the refusal; the key is then accepted once a place frees.
func TestAuth_LegacyQueueFullIs503RetryAfter(t *testing.T) {
	s, reg := newAuthTestServer(t)
	key := legacyVendor(t, reg, "busy-victim")
	withGate(t, fullGate())

	before := APIKeyBcryptRefused()
	rec := authCall(t, s, key)
	if rec.Code != http.StatusServiceUnavailable {
		t.Fatalf("correct legacy key with the queue full: status %d; want 503 (%s)", rec.Code, rec.Body)
	}
	if ra := rec.Header().Get("Retry-After"); ra != "1" {
		t.Fatalf("Retry-After %q; want 1", ra)
	}
	if code := problemCode(t, rec); code != "auth_busy" {
		t.Fatalf("problem code %q; want auth_busy", code)
	}
	if APIKeyBcryptRefused()-before != 1 {
		t.Fatalf("refusal counted %d times; want 1", APIKeyBcryptRefused()-before)
	}

	// Junk for the same legacy prefix is refused the same way: the portal
	// cannot know it is junk without the compare it is declining to run.
	junk := "vp_busy-victim_" + strings.Repeat("ab", 32)
	if rec := authCall(t, s, junk); rec.Code != http.StatusServiceUnavailable {
		t.Fatalf("unchecked junk key: status %d; want 503", rec.Code)
	}

	// Queue drains: the retry succeeds.
	withGate(t, newLegacyBcryptGate())
	if rec := authCall(t, s, key); rec.Code != http.StatusNoContent {
		t.Fatalf("retry after the flood: status %d; want 204 (%s)", rec.Code, rec.Body)
	}
}

// A key that was checked and is wrong is still 401, never 503; including
// when it is answered from the cache while the queue is full.
func TestAuth_LegacyWrongKeyIs401(t *testing.T) {
	s, reg := newAuthTestServer(t)
	legacyVendor(t, reg, "wrong-key-org")
	g := newLegacyBcryptGate()
	withGate(t, g)
	wrong := "vp_wrong-key-org_" + strings.Repeat("cd", 32)

	rec := authCall(t, s, wrong)
	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("wrong legacy key: status %d; want 401 (%s)", rec.Code, rec.Body)
	}
	if code := problemCode(t, rec); code != "auth" {
		t.Fatalf("problem code %q; want auth", code)
	}
	if rec.Header().Get("Retry-After") != "" {
		t.Fatal("a 401 carries Retry-After")
	}

	g.mu.Lock()
	g.pending = cap(g.slots) + g.maxQueue
	g.mu.Unlock()
	if rec := authCall(t, s, wrong); rec.Code != http.StatusUnauthorized {
		t.Fatalf("cached wrong key with the queue full: status %d; want 401", rec.Code)
	}
	// Tokens that never reach the gate stay 401 as well.
	for _, tok := range []string{"not-a-token", "vp_no-such-org_" + strings.Repeat("ef", 32)} {
		if rec := authCall(t, s, tok); rec.Code != http.StatusUnauthorized {
			t.Fatalf("%q with the queue full: status %d; want 401", tok, rec.Code)
		}
	}
}
