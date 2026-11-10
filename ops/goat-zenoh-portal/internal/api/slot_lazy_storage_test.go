package api_test

import (
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/acl"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/api"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/certmint"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/schemastore"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/zenohproxy"
)

// fakeAdminRouter records storage PUTs and answers ZID discovery + the
// verify-via-GET the F-156 AddStorage does. Minimal mirror of the acl-package
// fake (that one is unexported).
type fakeAdminRouter struct {
	zid      string
	mu       sync.Mutex
	stored   map[string]json.RawMessage // storage name -> body
	putNames []string
}

func (f *fakeAdminRouter) handler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		path := strings.TrimLeft(r.URL.Path, "/")
		f.mu.Lock()
		defer f.mu.Unlock()
		// ZID discovery.
		if r.Method == http.MethodGet && (path == "@/*/router" || path == "@/"+f.zid+"/router") {
			_ = json.NewEncoder(w).Encode([]map[string]any{{"key": "@/" + f.zid + "/router"}})
			return
		}
		cfgPrefix := "@/" + f.zid + "/router/config/plugins/storage_manager/storages/"
		stPrefix := "@/" + f.zid + "/router/status/plugins/storage_manager/storages/"
		if r.Method == http.MethodPut && strings.HasPrefix(path, cfgPrefix) {
			name := strings.TrimSuffix(strings.TrimPrefix(path, cfgPrefix), "/")
			body, _ := io.ReadAll(r.Body)
			f.stored[name] = json.RawMessage(body)
			f.putNames = append(f.putNames, name)
			w.WriteHeader(http.StatusOK)
			return
		}
		// verify-via-GET: present iff we stored it.
		if r.Method == http.MethodGet && strings.HasPrefix(path, stPrefix) {
			name := strings.TrimSuffix(strings.TrimPrefix(path, stPrefix), "/")
			if _, ok := f.stored[name]; ok {
				_ = json.NewEncoder(w).Encode([]map[string]any{{"key": path, "value": map[string]any{}}})
				return
			}
			_ = json.NewEncoder(w).Encode([]map[string]any{})
			return
		}
		w.WriteHeader(http.StatusOK)
	})
}

// TestSlotOnboard_LazyStorageLiveAdd: with LazySlotStorage on + AdminREST wired,
// onboarding a slot live-adds its <anchor>/** rocksdb store via the admin REST
// API.
func TestSlotOnboard_LazyStorageLiveAdd(t *testing.T) {
	dir := t.TempDir()
	fr := &fakeAdminRouter{zid: "testzid001", stored: map[string]json.RawMessage{}}
	router := httptest.NewServer(fr.handler())
	t.Cleanup(router.Close)

	cfg := &config.Config{
		OperatorAPIKey:    "dev-operator-key",
		IdentityModel:     config.IdentityModelPerUUID,
		SlotPoolSalt:      slotSalt,
		SlotPoolSize:      3,
		ZenohRestFake:     true,
		ACLConfigPath:     filepath.Join(dir, "config.json5"),
		ACLReloadFake:     true,
		CertSignFake:      true,
		VendorBundleDir:   filepath.Join(dir, "bundles"),
		SchemaDir:         filepath.Join(dir, "schemas"),
		CertSignURIPrefix: "urn:goat:galaxy:org=",
		LazySlotStorage:   true,
	}
	reg, err := registry.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("registry: %v", err)
	}
	t.Cleanup(func() { _ = reg.Close() })
	if _, err := reg.InitializeSlotPool(t.Context(), cfg.SlotPoolSalt, cfg.SlotPoolSize); err != nil {
		t.Fatalf("InitializeSlotPool: %v", err)
	}
	minter := &certmint.Minter{Fake: true, VendorBundleDir: cfg.VendorBundleDir, URIPrefix: cfg.CertSignURIPrefix}
	aclR := &acl.Renderer{
		ConfigPath:       cfg.ACLConfigPath,
		StorageDirPrefix: filepath.Join(dir, "rocksdb"),
		ReloadFake:       true,
		AdminREST:        &acl.AdminRESTClient{BaseURL: router.URL},
	}
	schemas, err := schemastore.New(cfg.SchemaDir)
	if err != nil {
		t.Fatalf("schemastore: %v", err)
	}
	srv := api.New(cfg, reg, zenohproxy.New("", true), minter, aclR, schemas, nil, nil)
	ts := httptest.NewServer(srv.Routes())
	t.Cleanup(ts.Close)

	c := ts.Client()
	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key",
		map[string]any{"prefix": "navy-recon"})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	var body struct {
		SlotID string `json:"slot_id"`
	}
	if err := json.Unmarshal([]byte(mustReadBody(t, resp)), &body); err != nil {
		t.Fatalf("decode: %v", err)
	}
	anchor := body.SlotID
	if anchor == "" {
		t.Fatal("no slot_id in onboard response")
	}

	fr.mu.Lock()
	defer fr.mu.Unlock()
	stored, ok := fr.stored[anchor]
	if !ok {
		t.Fatalf("lazy storage: no store PUT for anchor %q; puts=%v", anchor, fr.putNames)
	}
	// The store must key on <anchor>/** (matches the pre-cut config exactly).
	var got map[string]any
	if err := json.Unmarshal(stored, &got); err != nil {
		t.Fatalf("stored body not JSON: %v", err)
	}
	if ke, _ := got["key_expr"].(string); ke != anchor+"/**" {
		t.Errorf("key_expr = %q, want %q", ke, anchor+"/**")
	}
}
