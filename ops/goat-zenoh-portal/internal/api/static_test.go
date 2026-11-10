package api

import (
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"testing/fstest"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
)

const tokenPage = `<title>__LEX_PRODUCT_NAME__ — Portal</title><h1>__LEX_PRODUCT_TITLE__</h1>` +
	`<a href="/vendor.html">__LEX_ORGS_TITLE__</a><p>register an __LEX_ORG__</p>`

func getBody(t *testing.T, s *Server, path string) (int, string) {
	t.Helper()
	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, path, nil)
	s.staticHandler().ServeHTTP(rec, req)
	b, _ := io.ReadAll(rec.Result().Body)
	return rec.Code, string(b)
}

// Default (site-A) lexicon renders the historical wording and leaves no tokens.
func TestStaticHandler_DefaultRendersBuiltInLexicon(t *testing.T) {
	cfg, err := config.LoadFromFlags(nil) // normalizes the empty lexicon to the built-in defaults
	if err != nil {
		t.Fatalf("LoadFromFlags: %v", err)
	}
	s := &Server{cfg: cfg, staticFS: fstest.MapFS{"index.html": {Data: []byte(tokenPage)}}}

	code, body := getBody(t, s, "/")
	if code != http.StatusOK {
		t.Fatalf("status = %d", code)
	}
	for _, want := range []string{
		"Zenoh Trial — Portal",
		"<h1>Zenoh Trial — Vendor Portal</h1>",
		">Vendors</a>",
		"register an vendor",
	} {
		if !strings.Contains(body, want) {
			t.Errorf("default render missing %q\nbody: %s", want, body)
		}
	}
	if strings.Contains(body, "__LEX_") {
		t.Errorf("unresolved token leaked into default render:\n%s", body)
	}
}

// GALAXY override renders the org wording; still no tokens left behind.
func TestStaticHandler_GoatnetOverride(t *testing.T) {
	cfg, err := config.LoadFromFlags([]string{
		"--lex-product-name", "GALAXY Fabric",
		"--lex-product-title", "GALAXY Fabric Portal",
		"--lex-org", "org",
	})
	if err != nil {
		t.Fatalf("LoadFromFlags: %v", err)
	}
	s := &Server{cfg: cfg, staticFS: fstest.MapFS{"index.html": {Data: []byte(tokenPage)}}}

	code, body := getBody(t, s, "/")
	if code != http.StatusOK {
		t.Fatalf("status = %d", code)
	}
	for _, want := range []string{
		"GALAXY Fabric — Portal",
		"<h1>GALAXY Fabric Portal</h1>",
		">Orgs</a>",
		"register an org",
	} {
		if !strings.Contains(body, want) {
			t.Errorf("GALAXY render missing %q\nbody: %s", want, body)
		}
	}
	if strings.Contains(body, "__LEX_") {
		t.Errorf("unresolved token leaked into GALAXY render:\n%s", body)
	}
}

// Non-HTML assets pass through untouched (no substitution, stock file server).
func TestStaticHandler_AssetsPassThrough(t *testing.T) {
	cfg, _ := config.LoadFromFlags(nil)
	s := &Server{cfg: cfg, staticFS: fstest.MapFS{
		"index.html": {Data: []byte(tokenPage)},
		"styles.css": {Data: []byte("body{color:red}")},
	}}
	code, body := getBody(t, s, "/styles.css")
	if code != http.StatusOK || !strings.Contains(body, "color:red") {
		t.Errorf("asset passthrough failed: code=%d body=%q", code, body)
	}
}

// The API document is YAML, which Go's mime table does not know; it must not
// leave as application/octet-stream (RFC 9512).
func TestStaticHandler_YAMLContentType(t *testing.T) {
	cfg, _ := config.LoadFromFlags(nil)
	s := &Server{cfg: cfg, staticFS: fstest.MapFS{
		"index.html":   {Data: []byte(tokenPage)},
		"openapi.yaml": {Data: []byte("openapi: 3.1.0\n")},
	}}
	rec := httptest.NewRecorder()
	s.staticHandler().ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/openapi.yaml", nil))
	if ct := rec.Header().Get("Content-Type"); ct != "application/yaml; charset=utf-8" {
		t.Errorf("openapi.yaml Content-Type = %q", ct)
	}
}
