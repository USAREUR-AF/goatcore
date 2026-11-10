package api_test

import (
	"bytes"
	"encoding/json"
	"io"
	"io/fs"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
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

// aclFixture is the minimal config.json5 shape with the four BEGIN/END
// markers the renderer edits between. Mirrors the fixture in
// internal/acl/acl_test.go but kept self-contained so the api_test
// package doesn't reach into the acl package's internals.
const aclFixture = `{
  mode: "router",
  access_control: {
    enabled: true,
    default_permission: "deny",
    rules: [
      // BEGIN: per-vendor write rules
      // END: per-vendor write rules
    ],
    subjects: [
      // BEGIN: per-vendor subjects
      // END: per-vendor subjects
    ],
    policies: [
      // BEGIN: per-vendor policies
      // END: per-vendor policies
    ],
  },
  plugins: {
    storage_manager: {
      storages: {
        // BEGIN: per-vendor storages
        // END: per-vendor storages
      },
    },
  },
}
`

// newTestServer wires the full Server with fakes for every external
// dependency so we exercise the real HTTP handlers end-to-end.
func newTestServer(t *testing.T) *httptest.Server {
	t.Helper()
	dir := t.TempDir()
	cfg := &config.Config{
		OperatorAPIKey:           "dev-operator-key",
		ZenohRestFake:            true,
		ACLConfigPath:            filepath.Join(dir, "config.json5"),
		ACLStorageDirPrefix:      filepath.Join(dir, "rocksdb"),
		ACLReloadFake:            true,
		CertSignFake:             true,
		VendorBundleDir:          filepath.Join(dir, "bundles"),
		SchemaDir:                filepath.Join(dir, "schemas"),
		CertSignURIPrefix:        "urn:goat:galaxy:org=",
		OperatorApprovalRequired: false,
	}
	// Seed a fixture config.json5 with the four markers so renderACL
	// has something to edit. Without this, vendor creation works but
	// the ACL apply leg returns "missing marker" and the response
	// carries the warning.
	if err := os.WriteFile(cfg.ACLConfigPath, []byte(aclFixture), 0o644); err != nil {
		t.Fatalf("write acl fixture: %v", err)
	}
	reg, err := registry.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("registry: %v", err)
	}
	t.Cleanup(func() { _ = reg.Close() })

	zp := zenohproxy.New("", true)
	minter := &certmint.Minter{
		Fake:            true,
		VendorBundleDir: cfg.VendorBundleDir,
		URIPrefix:       cfg.CertSignURIPrefix,
	}
	aclR := &acl.Renderer{
		ConfigPath:       cfg.ACLConfigPath,
		StorageDirPrefix: cfg.ACLStorageDirPrefix,
		ReloadFake:       true,
	}
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

func TestSmoke_VendorRegisterTopicLifecycle(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}

	// 1. unauth list
	r, _ := http.NewRequest("GET", ts.URL+"/api/vendors", nil)
	resp, err := c.Do(r)
	if err != nil {
		t.Fatalf("unauth list: %v", err)
	}
	if resp.StatusCode != http.StatusUnauthorized {
		t.Errorf("unauth list expected 401, got %d", resp.StatusCode)
	}

	// 2. operator list (empty)
	resp = doJSON(t, c, "GET", ts.URL+"/api/vendors", "dev-operator-key", nil)
	body := mustReadJSON(t, resp)
	if vs, ok := body["vendors"].([]any); !ok || len(vs) != 0 {
		t.Errorf("expected empty vendors list, got %#v", body["vendors"])
	}

	// 3. operator registers vendor
	resp = doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key",
		map[string]any{"prefix": "community", "display_name": "Community", "auto_approve": true})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("register: status %d", resp.StatusCode)
	}
	regResp := mustReadJSON(t, resp)
	vendorKey, _ := regResp["api_key"].(string)
	if vendorKey == "" {
		t.Fatalf("no api_key in response: %#v", regResp)
	}
	if !strings.HasPrefix(vendorKey, "vp_community_") {
		t.Errorf("api_key %q has unexpected prefix", vendorKey)
	}
	if uriSAN, _ := regResp["uri_san"].(string); uriSAN != "urn:goat:galaxy:org=community" {
		t.Errorf("uri_san = %q", uriSAN)
	}

	// 4. vendor lists topics (empty)
	resp = doJSON(t, c, "GET", ts.URL+"/api/vendors/community/topics", vendorKey, nil)
	body = mustReadJSON(t, resp)
	if ts2, _ := body["topics"].([]any); len(ts2) != 0 {
		t.Errorf("expected 0 topics, got %d", len(ts2))
	}

	// 5. vendor creates topic
	resp = doJSON(t, c, "POST", ts.URL+"/api/vendors/community/topics", vendorKey, map[string]any{
		"key_pattern": "community/jane-test/foo/v1",
		"format":      "json",
		"description": "hello",
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("topic create: status %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}

	// 6. cross-vendor write rejected
	resp = doJSON(t, c, "POST", ts.URL+"/api/vendors/acme/topics", vendorKey, map[string]any{
		"key_pattern": "acme/foo/v1",
		"format":      "json",
	})
	if resp.StatusCode != http.StatusForbidden {
		t.Errorf("cross-vendor write: expected 403, got %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}

	// 7. operator can act on any vendor
	resp = doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{
		"prefix":       "acme",
		"display_name": "Acme",
		"auto_approve": true,
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("operator register acme: %d", resp.StatusCode)
	}
	resp = doJSON(t, c, "POST", ts.URL+"/api/vendors/acme/topics", "dev-operator-key", map[string]any{
		"key_pattern": "acme/lattice/entity/track/v1",
		"format":      "json",
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("operator create acme topic: %d", resp.StatusCode)
	}

	// 8. inspector latest (fake zenoh)
	resp = doJSON(t, c, "GET",
		ts.URL+"/api/topics/latest?key=community%2Fdene-test%2Ffoo%2Fv1", vendorKey, nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("inspector latest: %d", resp.StatusCode)
	}
	body = mustReadJSON(t, resp)
	samples, _ := body["samples"].([]any)
	if len(samples) == 0 {
		t.Errorf("expected at least one fake sample, got none")
	}

	// 9. duplicate vendor 409
	resp = doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{
		"prefix": "community", "auto_approve": true,
	})
	if resp.StatusCode != http.StatusConflict {
		t.Errorf("duplicate vendor: expected 409, got %d", resp.StatusCode)
	}

	// 10. bundle download — vendor's tarball reachable via the URL returned
	//     at register time. Multi-use by design: siblings at the same
	//     vendor share one cert+api_key (operator-onboard-utilities §83E),
	//     so the URL keeps serving across downloads. Token rotation is an
	//     explicit operator action, not implicit on first read.
	bundleURL, _ := regResp["bundle_url"].(string)
	for i := 0; i < 3; i++ {
		resp, err = c.Get(ts.URL + bundleURL)
		if err != nil {
			t.Fatalf("bundle GET #%d: %v", i+1, err)
		}
		if resp.StatusCode != http.StatusOK {
			t.Errorf("bundle download #%d: status %d", i+1, resp.StatusCode)
		}
	}
}

func TestSmoke_HealthEndpointNoAuth(t *testing.T) {
	ts := newTestServer(t)
	resp, err := http.Get(ts.URL + "/api/health")
	if err != nil {
		t.Fatalf("health: %v", err)
	}
	if resp.StatusCode != http.StatusOK {
		t.Errorf("health: expected 200, got %d", resp.StatusCode)
	}
	body := mustReadJSON(t, resp)
	if body["status"] != "ok" {
		t.Errorf("health status = %v", body["status"])
	}
}

// --- helpers ---------------------------------------------------------

func doJSON(t *testing.T, c *http.Client, method, url, bearer string, payload any) *http.Response {
	t.Helper()
	var body io.Reader
	if payload != nil {
		buf, _ := json.Marshal(payload)
		body = bytes.NewReader(buf)
	}
	req, err := http.NewRequest(method, url, body)
	if err != nil {
		t.Fatalf("new req: %v", err)
	}
	if bearer != "" {
		req.Header.Set("Authorization", "Bearer "+bearer)
	}
	if payload != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := c.Do(req)
	if err != nil {
		t.Fatalf("do: %v", err)
	}
	return resp
}

func mustReadJSON(t *testing.T, resp *http.Response) map[string]any {
	t.Helper()
	defer resp.Body.Close()
	var m map[string]any
	if err := json.NewDecoder(resp.Body).Decode(&m); err != nil {
		t.Fatalf("decode: %v", err)
	}
	return m
}

func mustReadBody(t *testing.T, resp *http.Response) string {
	t.Helper()
	defer resp.Body.Close()
	b, _ := io.ReadAll(resp.Body)
	return string(b)
}

// TestSmoke_DeleteVendor exercises the operator-only DELETE /api/vendors/{v}.
// Three contracts: 409 when the vendor still owns live topics, 204 once those
// topics are retired (the cascade cleans up retired-topic rows + their FKs),
// and the operator-key gate (vendor's own key cannot prune the registry).
func TestSmoke_DeleteVendor(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}

	// 1. Register vendor as operator.
	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{
		"prefix": "doomed-vendor",
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("register: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	regResp := mustReadJSON(t, resp)
	vendorKey, _ := regResp["api_key"].(string)
	if vendorKey == "" {
		t.Fatal("register: missing api_key")
	}

	// 2. Vendor adds a topic.
	resp = doJSON(t, c, "POST", ts.URL+"/api/vendors/doomed-vendor/topics", vendorKey, map[string]any{
		"key_pattern": "doomed-vendor/scratch/x/v1",
		"format":      "json",
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("create topic: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	topic := mustReadJSON(t, resp)
	topicID, _ := topic["id"].(float64)

	// 3. Operator delete refuses with 409 because vendor still owns 1 active topic.
	resp = doJSON(t, c, "DELETE", ts.URL+"/api/vendors/doomed-vendor", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusConflict {
		t.Fatalf("delete-with-active-topic: want 409, got %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}

	// 4. Vendor-key holder cannot delete its own vendor entry (operator-only).
	resp = doJSON(t, c, "DELETE", ts.URL+"/api/vendors/doomed-vendor", vendorKey, nil)
	if resp.StatusCode != http.StatusForbidden && resp.StatusCode != http.StatusUnauthorized {
		t.Fatalf("delete-with-vendor-key: want 401/403, got %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}

	// 5. Retire the topic, then re-attempt delete — 204 expected.
	url := ts.URL + "/api/vendors/doomed-vendor/topics/" + strconv.Itoa(int(topicID))
	resp = doJSON(t, c, "DELETE", url, vendorKey, nil)
	if resp.StatusCode != http.StatusNoContent {
		t.Fatalf("retire topic: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}

	resp = doJSON(t, c, "DELETE", ts.URL+"/api/vendors/doomed-vendor", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusNoContent {
		t.Fatalf("delete-after-retire: want 204, got %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}

	// 6. Subsequent delete is 404 — the row is gone.
	resp = doJSON(t, c, "DELETE", ts.URL+"/api/vendors/doomed-vendor", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusNotFound {
		t.Fatalf("delete-twice: want 404, got %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
}
