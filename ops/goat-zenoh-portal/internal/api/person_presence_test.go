package api_test

import (
	"encoding/json"
	"io/fs"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/acl"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/api"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/certmint"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/schemastore"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/zenohproxy"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/web"
)

// goat#319 — GET /api/lookup/person-presence?id=<email|handle>.
//
// The id-console's offboard-verification view needs the per-person mesh +
// fabric slice, but the console holds the lookup service secret, not an
// operator key, and the operator-gated roster/audit endpoints rightly refuse
// it. This endpoint is the narrow door: ONE person's WhoResult, admitted by
// the same console↔portal secret that already gates the sponsorship lookup
// (lookupAuthorized), never the fleet roster.

// newPresenceTestServer mirrors newTestServer but sets the console lookup
// secret, which is the credential class under test.
func newPresenceTestServer(t *testing.T) *httptest.Server {
	t.Helper()
	dir := t.TempDir()
	cfg := &config.Config{
		OperatorAPIKey:      "dev-operator-key",
		ConsoleLookupToken:  "console-secret",
		ZenohRestFake:       true,
		ACLConfigPath:       filepath.Join(dir, "config.json5"),
		ACLStorageDirPrefix: filepath.Join(dir, "rocksdb"),
		ACLReloadFake:       true,
		CertSignFake:        true,
		VendorBundleDir:     filepath.Join(dir, "bundles"),
		SchemaDir:           filepath.Join(dir, "schemas"),
		CertSignURIPrefix:   "urn:goat:galaxy:org=",
	}
	if err := os.WriteFile(cfg.ACLConfigPath, []byte(aclFixture), 0o644); err != nil {
		t.Fatalf("write acl fixture: %v", err)
	}
	reg, err := registry.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("registry: %v", err)
	}
	t.Cleanup(func() { _ = reg.Close() })
	zp := zenohproxy.New("", true)
	minter := &certmint.Minter{Fake: true, VendorBundleDir: cfg.VendorBundleDir, URIPrefix: cfg.CertSignURIPrefix}
	aclR := &acl.Renderer{ConfigPath: cfg.ACLConfigPath, StorageDirPrefix: cfg.ACLStorageDirPrefix, ReloadFake: true}
	staticFS, err := fs.Sub(web.Static, "static")
	if err != nil {
		t.Fatalf("web.Sub: %v", err)
	}
	schemas, err := schemastore.New(cfg.SchemaDir)
	if err != nil {
		t.Fatalf("schemastore: %v", err)
	}
	srv := api.New(cfg, reg, zp, minter, aclR, schemas, nil, staticFS)
	ts := httptest.NewServer(srv.Routes())
	t.Cleanup(ts.Close)
	return ts
}

func presenceGet(t *testing.T, ts *httptest.Server, bearer, query string) *http.Response {
	t.Helper()
	req, _ := http.NewRequest("GET", ts.URL+"/api/lookup/person-presence"+query, nil)
	if bearer != "" {
		req.Header.Set("Authorization", "Bearer "+bearer)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatalf("GET person-presence: %v", err)
	}
	return resp
}

func TestPersonPresenceRefusesWithoutToken(t *testing.T) {
	ts := newPresenceTestServer(t)
	for name, bearer := range map[string]string{"no token": "", "garbage": "not-a-real-token"} {
		resp := presenceGet(t, ts, bearer, "?id=pat@partner.example")
		_ = resp.Body.Close()
		if resp.StatusCode != http.StatusUnauthorized {
			t.Errorf("%s: want 401, got %d", name, resp.StatusCode)
		}
	}
}

func TestPersonPresenceAdmitsConsoleSecretAndOperator(t *testing.T) {
	ts := newPresenceTestServer(t)
	for name, bearer := range map[string]string{
		"console lookup secret": "console-secret",
		"operator key":          "dev-operator-key",
	} {
		resp := presenceGet(t, ts, bearer, "?id=pat@partner.example")
		if resp.StatusCode != http.StatusOK {
			t.Errorf("%s: want 200, got %d", name, resp.StatusCode)
			_ = resp.Body.Close()
			continue
		}
		var body struct {
			Query    string `json:"query"`
			Resolved string `json:"resolved"`
			OnMesh   bool   `json:"on_mesh"`
			Writes   []any  `json:"writes"`
			Reads    []any  `json:"reads"`
		}
		if err := json.NewDecoder(resp.Body).Decode(&body); err != nil {
			t.Errorf("%s: decode: %v", name, err)
		}
		_ = resp.Body.Close()
		// An empty registry knows nobody: the honest answer is an unresolved
		// WhoResult, not an error — absence of evidence is a real finding for
		// the offboard verifier.
		if body.Query != "pat@partner.example" || body.Resolved != "" {
			t.Errorf("%s: want unresolved echo of the query, got query=%q resolved=%q", name, body.Query, body.Resolved)
		}
		if body.Writes == nil || body.Reads == nil {
			t.Errorf("%s: writes/reads must be arrays, never null", name)
		}
	}
}

func TestPersonPresenceRequiresID(t *testing.T) {
	ts := newPresenceTestServer(t)
	resp := presenceGet(t, ts, "console-secret", "")
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusBadRequest {
		t.Fatalf("want 400 without ?id=, got %d", resp.StatusCode)
	}
}
