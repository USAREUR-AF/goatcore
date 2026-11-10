package server

import (
	"bytes"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/snitch/internal/auth"
	"github.com/usareur-af/goatcore/ops/snitch/internal/compat"
)

func TestSatlinkRoutesServedUnderBothPaths(t *testing.T) {
	s := New(Config{Auth: auth.New(nil)})
	for _, base := range []string{"/api/satlink/", compat.SatlinkAPIPrefix} {
		for _, leaf := range []string{"latest", "trend", "pops", "probe"} {
			path := base + leaf
			_, pattern := s.mux.Handler(httptest.NewRequestWithContext(t.Context(), http.MethodGet, path, nil))
			if pattern != path {
				t.Errorf("%s: routed to %q, want its own route", path, pattern)
			}
		}
	}
	for _, page := range []string{"/satlink", compat.SatlinkPageRoute} {
		rec := httptest.NewRecorder()
		s.mux.ServeHTTP(rec, httptest.NewRequestWithContext(t.Context(), http.MethodGet, page, nil))
		if rec.Code != http.StatusOK || !bytes.Equal(rec.Body.Bytes(), satlinkHTML) {
			t.Errorf("%s: code=%d, want the satellite-link page", page, rec.Code)
		}
	}
}

func TestPagesFilledFromConfig(t *testing.T) {
	t.Setenv("SNITCH_GATEWATCH_URL", "")
	t.Setenv("SNITCH_PUBLIC_URL", "")

	s := New(Config{Auth: auth.New(nil)})
	if bytes.Contains(s.dashboard, []byte(gatewatchPlaceholder)) || bytes.Contains(s.install, []byte(publicURLPlaceholder)) {
		t.Fatal("placeholder left in a served page")
	}
	if !bytes.Contains(s.dashboard, []byte(compat.GatewatchURL+"/api/bridges")) {
		t.Error("dashboard: built-in gatewatch URL not used when unset")
	}
	if !bytes.Contains(s.install, []byte(`${SNITCH_COLLECTOR:-`+compat.InstallCollectorURL+`}`)) {
		t.Error("install.sh: built-in collector URL not used when unset")
	}

	t.Setenv("SNITCH_GATEWATCH_URL", "https://gatewatch.goat.example/")
	t.Setenv("SNITCH_PUBLIC_URL", "https://snitch.goat.example")
	s = New(Config{Auth: auth.New(nil)})
	if !bytes.Contains(s.dashboard, []byte(`href="https://gatewatch.goat.example/"`)) ||
		!bytes.Contains(s.dashboard, []byte(`'https://gatewatch.goat.example/api/bridges'`)) {
		t.Error("dashboard: SNITCH_GATEWATCH_URL not applied")
	}
	if !strings.Contains(string(s.install), `${SNITCH_COLLECTOR:-https://snitch.goat.example}`) {
		t.Error("install.sh: SNITCH_PUBLIC_URL not applied")
	}

	// Config wins over the environment; an unsafe value falls back.
	s = New(Config{Auth: auth.New(nil), PublicURL: `https://x.goat.example/"; rm -rf /`})
	if !strings.Contains(string(s.install), `${SNITCH_COLLECTOR:-https://snitch.goat.example}`) {
		t.Error("install.sh: unsafe PublicURL was not rejected")
	}
}
