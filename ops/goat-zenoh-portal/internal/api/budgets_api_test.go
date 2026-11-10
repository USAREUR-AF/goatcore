package api_test

// Topic-budget grant surface + projection endpoint (ADR 1075 fold-in) —
// end-to-end over the real HTTP handlers.

import (
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

// newBudgetTestServer mirrors newTestServer but sets the self-block (0 in
// selfBlock exercises the fail-closed 503 leg).
func newBudgetTestServer(t *testing.T, selfBlock int) *httptest.Server {
	t.Helper()
	dir := t.TempDir()
	cfg := &config.Config{
		OperatorAPIKey:       "dev-operator-key",
		ZenohRestFake:        true,
		ACLConfigPath:        filepath.Join(dir, "config.json5"),
		ACLStorageDirPrefix:  filepath.Join(dir, "rocksdb"),
		ACLReloadFake:        true,
		CertSignFake:         true,
		VendorBundleDir:      filepath.Join(dir, "bundles"),
		SchemaDir:            filepath.Join(dir, "schemas"),
		CertSignURIPrefix:    "urn:goat:galaxy:org=",
		TopicBudgetSelfBlock: selfBlock,
	}
	if err := os.WriteFile(cfg.ACLConfigPath, []byte(aclFixture), 0o644); err != nil {
		t.Fatalf("write acl fixture: %v", err)
	}
	reg, err := registry.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("registry: %v", err)
	}
	t.Cleanup(func() { _ = reg.Close() })
	staticFS, err := fs.Sub(web.Static, "static")
	if err != nil {
		t.Fatalf("web.Sub: %v", err)
	}
	schemas, err := schemastore.New(cfg.SchemaDir)
	if err != nil {
		t.Fatalf("schemastore: %v", err)
	}
	srv := api.New(cfg, reg, zenohproxy.New("", true),
		&certmint.Minter{Fake: true, VendorBundleDir: cfg.VendorBundleDir, URIPrefix: cfg.CertSignURIPrefix},
		&acl.Renderer{ConfigPath: cfg.ACLConfigPath, StorageDirPrefix: cfg.ACLStorageDirPrefix, ReloadFake: true},
		schemas, nil, staticFS)
	ts := httptest.NewServer(srv.Routes())
	t.Cleanup(ts.Close)
	return ts
}

func TestBudgetGrantsAndProjection(t *testing.T) {
	ts := newBudgetTestServer(t, 100)
	c := &http.Client{}

	// Operator-only: unauthenticated projection refused.
	r, _ := http.NewRequest("GET", ts.URL+"/api/budget-projection", nil)
	resp, err := c.Do(r)
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusUnauthorized {
		t.Fatalf("unauth projection: %d, want 401", resp.StatusCode)
	}

	// Upsert a grant with prefixes, then one without.
	resp = doJSON(t, c, "PUT", ts.URL+"/api/budget-grants/aabb", "dev-operator-key",
		map[string]any{"granted": 400, "prefixes": []string{"video", "presence"}, "note": "platform-admin test"})
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("put grant: %d", resp.StatusCode)
	}
	resp = doJSON(t, c, "PUT", ts.URL+"/api/budget-grants/ops", "dev-operator-key",
		map[string]any{"granted": 2000, "note": "infra"})
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("put ops grant: %d", resp.StatusCode)
	}
	// granted <= 0 refused.
	resp = doJSON(t, c, "PUT", ts.URL+"/api/budget-grants/zz", "dev-operator-key",
		map[string]any{"granted": 0})
	if resp.StatusCode != http.StatusBadRequest {
		t.Fatalf("granted=0: %d, want 400", resp.StatusCode)
	}

	// The projection carries the crawler contract.
	resp = doJSON(t, c, "GET", ts.URL+"/api/budget-projection", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("projection: %d", resp.StatusCode)
	}
	body := mustReadJSON(t, resp)
	if sb, _ := body["self_service_block"].(float64); sb != 100 {
		t.Fatalf("self_service_block = %v", body["self_service_block"])
	}
	if _, ok := body["auto_self_service_anchors"].([]any); !ok {
		t.Fatalf("auto_self_service_anchors missing: %#v", body)
	}
	grants, ok := body["grants"].(map[string]any)
	if !ok || len(grants) != 2 {
		t.Fatalf("grants = %#v", body["grants"])
	}
	aabb := grants["aabb"].(map[string]any)
	if g, _ := aabb["granted"].(float64); g != 400 {
		t.Fatalf("aabb granted = %v", aabb)
	}
	if ps, _ := aabb["prefixes"].([]any); len(ps) != 2 {
		t.Fatalf("aabb prefixes = %v", aabb["prefixes"])
	}

	// Delete drops it from the projection.
	req, _ := http.NewRequest("DELETE", ts.URL+"/api/budget-grants/aabb", nil)
	req.Header.Set("Authorization", "Bearer dev-operator-key")
	resp, err = c.Do(req)
	if err != nil || resp.StatusCode != http.StatusOK {
		t.Fatalf("delete: %v %d", err, resp.StatusCode)
	}
	resp = doJSON(t, c, "GET", ts.URL+"/api/budget-projection", "dev-operator-key", nil)
	body = mustReadJSON(t, resp)
	if grants := body["grants"].(map[string]any); len(grants) != 1 {
		t.Fatalf("after delete grants = %#v", grants)
	}
}

func TestBudgetProjectionFailClosedWhenSelfBlockUnset(t *testing.T) {
	ts := newBudgetTestServer(t, 0)
	c := &http.Client{}
	resp := doJSON(t, c, "GET", ts.URL+"/api/budget-projection", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusServiceUnavailable {
		t.Fatalf("projection with self-block unset: %d, want 503", resp.StatusCode)
	}
}
