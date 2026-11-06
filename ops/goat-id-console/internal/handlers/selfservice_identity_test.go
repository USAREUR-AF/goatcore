package handlers

// F-309: the self-service pages answered "no signed-in identity on this
// request" to every visitor, including people who had just authenticated.
// "Ungated" had been implemented as no middleware at all, and the actor only
// reaches a handler through a middleware that stores it — so these routes read
// the zero Actor and refused everyone.
//
// The existing page tests render pageVars directly and stayed green throughout,
// which is exactly why it shipped. These go through Routes(), the only path
// that exercises the header -> actor -> handler chain that was broken.

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// selfServiceRoutes are the routes that identify a caller without authorizing
// one. Every entry must survive an authenticated request; a new self-service
// route added without selfServiceAuth fails here rather than in production.
var selfServiceRoutes = []string{"/identity", "/ssh-key", "/operator-key", "/fabric-credential"}

func TestSelfService_AuthenticatedRequestReachesTheHandler(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.DevBypassAuth = false // otherwise the actor is fabricated and proves nothing
	h := s.Routes()
	const email = "robin@example.test"

	for _, path := range selfServiceRoutes {
		r := httptest.NewRequestWithContext(context.Background(), http.MethodGet, path, nil)
		r.Header.Set("X-Forwarded-Email", email)
		w := httptest.NewRecorder()
		h.ServeHTTP(w, r)
		body := w.Body.String()

		if strings.Contains(body, "no signed-in identity") {
			t.Errorf("GET %s with a valid identity header still says %q (status %d) — "+
				"the route is not wrapped in selfServiceAuth", path, strings.TrimSpace(body), w.Code)
			continue
		}
		// The handler ran and saw WHO the caller is. With no onboarding record
		// behind the address it declines by naming that address back, which is
		// only possible if the identity arrived.
		if w.Code == http.StatusUnauthorized {
			t.Errorf("GET %s returned 401 for an authenticated caller: %s", path, body)
		}
		if !strings.Contains(body, email) {
			t.Errorf("GET %s never echoes the caller's identity (status %d): %s", path, w.Code, body)
		}
	}
}

// Authentication is still required: no identity means no page. A self-service
// route is un-AUTHORIZED, never un-authenticated.
func TestSelfService_AnonymousRequestRefused(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.DevBypassAuth = false
	h := s.Routes()

	for _, path := range selfServiceRoutes {
		r := httptest.NewRequestWithContext(context.Background(), http.MethodGet, path, nil)
		w := httptest.NewRecorder()
		h.ServeHTTP(w, r)
		if w.Code != http.StatusUnauthorized {
			t.Errorf("GET %s unauthenticated = %d, want 401; body=%s", path, w.Code, w.Body.String())
		}
	}
}

// The identity must come from the family oauth2-proxy overwrites. A
// browser-supplied X-Auth-Request-* is not stripped by the proxy, so trusting
// it would let anyone act as anyone — the forgery this console already fixed
// once, re-asserted here because selfServiceAuth is a new place to get it wrong.
func TestSelfService_IgnoresForgeableIdentityHeaders(t *testing.T) {
	s := newOnboardTestServer(t)
	s.Cfg.DevBypassAuth = false
	h := s.Routes()

	for _, path := range selfServiceRoutes {
		r := httptest.NewRequestWithContext(context.Background(), http.MethodGet, path, nil)
		r.Header.Set("X-Auth-Request-Email", "attacker@example.test")
		r.Header.Set("X-Auth-Request-Groups", "PLATFORM-ADMIN")
		w := httptest.NewRecorder()
		h.ServeHTTP(w, r)
		if w.Code != http.StatusUnauthorized {
			t.Errorf("GET %s with only forgeable headers = %d, want 401", path, w.Code)
		}
		if strings.Contains(w.Body.String(), "attacker@example.test") {
			t.Errorf("GET %s honored a client-supplied X-Auth-Request-Email", path)
		}
	}
}
