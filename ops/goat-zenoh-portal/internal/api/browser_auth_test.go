package api

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
)

func TestBrowserAuthenticationRevocationAndBearerPrecedence(t *testing.T) {
	server := &Server{cfg: &config.Config{OperatorAPIKey: "synthetic-browser-key"}}
	login := sessionRequest(http.MethodPost)
	login.Header.Set("Authorization", "Bearer synthetic-browser-key")
	created := httptest.NewRecorder()
	server.handleBrowserSession(created, login)
	if created.Code != http.StatusNoContent {
		t.Fatalf("login: %d", created.Code)
	}
	cookie := created.Result().Cookies()[0]
	handler := server.AuthMiddleware(true, func(w http.ResponseWriter, r *http.Request) {
		p := PrincipalFromContext(r.Context())
		if p == nil || !p.Operator {
			t.Error("lost operator principal")
		}
		w.WriteHeader(http.StatusNoContent)
	})
	read := sessionRequest(http.MethodGet)
	read.AddCookie(cookie)
	ok := httptest.NewRecorder()
	handler(ok, read)
	if ok.Code != http.StatusNoContent {
		t.Fatalf("cookie auth: %d", ok.Code)
	}
	read.Header.Set("Authorization", "Bearer invalid")
	refused := httptest.NewRecorder()
	handler(refused, read)
	if refused.Code != http.StatusUnauthorized {
		t.Fatal("invalid explicit bearer fell back to cookie")
	}
	read.Header.Del("Authorization")
	server.cfg.OperatorAPIKey = "rotated-browser-key"
	revoked := httptest.NewRecorder()
	handler(revoked, read)
	if revoked.Code != http.StatusUnauthorized {
		t.Fatal("revoked credential remained valid through session")
	}
}

func TestBrowserLoginRefusesCrossOriginAndInvalidCredentials(t *testing.T) {
	server := &Server{cfg: &config.Config{OperatorAPIKey: "synthetic-browser-key"}}
	for _, origin := range []string{"https://attacker.example.test", ""} {
		r := sessionRequest(http.MethodPost)
		r.Header.Set("Origin", origin)
		r.Header.Set("Authorization", "Bearer synthetic-browser-key")
		w := httptest.NewRecorder()
		server.handleBrowserSession(w, r)
		if w.Code != http.StatusForbidden || len(w.Result().Cookies()) != 0 {
			t.Fatal("cross-origin login issued a session")
		}
	}
	r := sessionRequest(http.MethodPost)
	r.Header.Set("Authorization", "Bearer invalid")
	w := httptest.NewRecorder()
	server.handleBrowserSession(w, r)
	if w.Code != http.StatusUnauthorized || len(w.Result().Cookies()) != 0 {
		t.Fatal("invalid credential issued a session")
	}
}
