package api_test

import (
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
)

const slotSalt = "galaxy-onboard-test"

// newSlotPoolServer wires a per-uuid-slot-pool (ADR 1025) portal: a 3-slot pool
// baked from slotSalt, fake cert-sign, and an ACL config file the test asserts
// is never touched (per-uuid onboarding does no ACL render). Optional mutators
// adjust the config before the server is built (e.g. PortalPublicURL).
func newSlotPoolServer(t *testing.T, aclPath string, mutate ...func(*config.Config)) (*httptest.Server, *registry.Registry) {
	t.Helper()
	dir := t.TempDir()
	cfg := &config.Config{
		OperatorAPIKey:    "dev-operator-key",
		IdentityModel:     config.IdentityModelPerUUID,
		SlotPoolSalt:      slotSalt,
		SlotPoolSize:      3,
		ZenohRestFake:     true,
		ACLConfigPath:     aclPath,
		ACLReloadFake:     true,
		CertSignFake:      true,
		VendorBundleDir:   filepath.Join(dir, "bundles"),
		SchemaDir:         filepath.Join(dir, "schemas"),
		CertSignURIPrefix: "urn:goat:galaxy:org=",
		// Configured so /api/budget-projection gets past its config guard and the
		// tests can exercise the sanity floor behind it.
		TopicBudgetSelfBlock: 100,
	}
	for _, m := range mutate {
		m(cfg)
	}
	reg, err := registry.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("registry: %v", err)
	}
	t.Cleanup(func() { _ = reg.Close() })
	// Boot-time pool init (main.go does this for the real binary).
	if n, err := reg.InitializeSlotPool(t.Context(), cfg.SlotPoolSalt, cfg.SlotPoolSize); err != nil || n != 3 {
		t.Fatalf("InitializeSlotPool = (%d,%v), want (3,nil)", n, err)
	}

	zp := zenohproxy.New("", true)
	minter := &certmint.Minter{
		Fake:            true,
		VendorBundleDir: cfg.VendorBundleDir,
		URIPrefix:       cfg.CertSignURIPrefix,
	}
	aclR := &acl.Renderer{ConfigPath: cfg.ACLConfigPath, ReloadFake: true}
	schemas, err := schemastore.New(cfg.SchemaDir)
	if err != nil {
		t.Fatalf("schemastore: %v", err)
	}
	srv := api.New(cfg, reg, zp, minter, aclR, schemas, nil, nil)
	ts := httptest.NewServer(srv.Routes())
	t.Cleanup(ts.Close)
	return ts, reg
}

func TestSlotOnboard_AllocatesRouterEnforcedAnchors(t *testing.T) {
	dir := t.TempDir()
	aclPath := filepath.Join(dir, "config.json5")
	const aclMarker = "// per-uuid ACL is baked by ansible; portal must not touch this\n"
	if err := os.WriteFile(aclPath, []byte(aclMarker), 0o644); err != nil {
		t.Fatalf("seed acl: %v", err)
	}
	ts, _ := newSlotPoolServer(t, aclPath)
	c := &http.Client{}

	// Onboard two systems by human handle. Each must come back with the exact
	// deterministic slot anchor the router pre-baked — slot_id == prefix ==
	// SlotAnchor(salt, idx). This is the api-layer half of the parity locked
	// by registry.TestSlotAnchorParity.
	for idx, handle := range []string{"fra-land-bde", "ita-recon-coy"} {
		resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key",
			map[string]any{"prefix": handle})
		if resp.StatusCode != http.StatusCreated {
			t.Fatalf("onboard %s: status=%d body=%s", handle, resp.StatusCode, mustReadBody(t, resp))
		}
		body := mustReadJSON(t, resp)
		wantAnchor := registry.SlotAnchor(slotSalt, idx)
		if got, _ := body["slot_id"].(string); got != wantAnchor {
			t.Errorf("onboard %s slot_id=%q, want %q (router-enforced anchor for slot %d)", handle, got, wantAnchor, idx)
		}
		if got, _ := body["prefix"].(string); got != wantAnchor {
			t.Errorf("onboard %s prefix=%q, want anchor %q (anchor IS the portal prefix)", handle, got, wantAnchor)
		}
		if got, _ := body["system_handle"].(string); got != handle {
			t.Errorf("onboard %s system_handle=%q, want %q", handle, got, handle)
		}
		// prefix=anchor was passed to the minter, so the URI-SAN carries it.
		if got, _ := body["uri_san"].(string); got != "urn:goat:galaxy:org="+wantAnchor {
			t.Errorf("onboard %s uri_san=%q, want vendor=%s", handle, got, wantAnchor)
		}
	}

	// The system writes under its own anchor namespace; cross-anchor is denied.
	anchor0 := registry.SlotAnchor(slotSalt, 0)
	anchor1 := registry.SlotAnchor(slotSalt, 1)
	// Re-fetch anchor0's api_key by onboarding is one-time; use the operator key
	// (operator can act on any prefix) to prove the namespace shape works.
	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors/"+anchor0+"/topics", "dev-operator-key", map[string]any{
		"key_pattern": anchor0 + "/data/track/v1",
		"format":      "json",
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("topic under own anchor: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	// A topic whose key_pattern starts with a different anchor is rejected by
	// the registry's prefix-match validation.
	resp = doJSON(t, c, "POST", ts.URL+"/api/vendors/"+anchor0+"/topics", "dev-operator-key", map[string]any{
		"key_pattern": anchor1 + "/data/track/v1",
		"format":      "json",
	})
	if resp.StatusCode == http.StatusCreated {
		t.Errorf("topic under foreign anchor was accepted; want rejection")
	}

	// The ACL config file must be byte-identical — per-uuid onboarding never
	// renders the ACL (the slot rules are pre-baked by ansible).
	after, err := os.ReadFile(aclPath)
	if err != nil {
		t.Fatalf("read acl after: %v", err)
	}
	if string(after) != aclMarker {
		t.Errorf("ACL config was modified by per-uuid onboarding; want untouched.\ngot: %q", string(after))
	}
}

// TestSlotOnboard_DeleteFreesSlotAndAlias locks F-190: DELETE /api/vendors on a
// slot-pool vendor must free the slot — drop the alias from /api/aliases AND
// return the slot index to the pool for re-onboard. Before the fix, delete
// removed the registry row + ACL but left the slot assigned, leaking it.
func TestSlotOnboard_DeleteFreesSlotAndAlias(t *testing.T) {
	dir := t.TempDir()
	aclPath := filepath.Join(dir, "config.json5")
	if err := os.WriteFile(aclPath, []byte("{}"), 0o644); err != nil {
		t.Fatalf("seed acl: %v", err)
	}
	ts, _ := newSlotPoolServer(t, aclPath)
	c := &http.Client{}

	aliasCount := func() int {
		resp := doJSON(t, c, "GET", ts.URL+"/api/aliases", "dev-operator-key", nil)
		if resp.StatusCode != http.StatusOK {
			t.Fatalf("GET aliases: status=%d", resp.StatusCode)
		}
		body := mustReadJSON(t, resp)
		as, _ := body["aliases"].([]any)
		return len(as)
	}

	// Fill the 3-slot pool.
	for _, h := range []string{"alpha", "bravo", "charlie"} {
		resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{"prefix": h})
		if resp.StatusCode != http.StatusCreated {
			t.Fatalf("onboard %s: %d %s", h, resp.StatusCode, mustReadBody(t, resp))
		}
	}
	if got := aliasCount(); got != 3 {
		t.Fatalf("aliases after 3 onboards = %d, want 3", got)
	}
	// Pool is full — a 4th onboard 503s.
	if resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{"prefix": "delta"}); resp.StatusCode != http.StatusServiceUnavailable {
		t.Fatalf("4th onboard into full pool: want 503, got %d", resp.StatusCode)
	}

	// Delete alpha (slot 0). Must 204 and free the slot.
	anchor0 := registry.SlotAnchor(slotSalt, 0)
	if resp := doJSON(t, c, "DELETE", ts.URL+"/api/vendors/"+anchor0, "dev-operator-key", nil); resp.StatusCode != http.StatusNoContent {
		t.Fatalf("delete alpha: want 204, got %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	if got := aliasCount(); got != 2 {
		t.Errorf("aliases after delete = %d, want 2 (alpha's alias should be gone)", got)
	}

	// The freed slot is reclaimable — a new onboard succeeds and takes slot 0.
	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{"prefix": "echo"})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("re-onboard into freed slot: want 201, got %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	if got, _ := mustReadJSON(t, resp)["slot_id"].(string); got != anchor0 {
		t.Errorf("re-onboard slot_id=%q, want the freed slot %q (lowest free idx)", got, anchor0)
	}

	// Idempotent reconcile: deleting an already-absent vendor whose slot is now
	// reassigned still frees it (204); deleting a never-used anchor 404s.
	if resp := doJSON(t, c, "DELETE", ts.URL+"/api/vendors/"+anchor0, "dev-operator-key", nil); resp.StatusCode != http.StatusNoContent {
		t.Errorf("delete echo (slot 0): want 204, got %d", resp.StatusCode)
	}
	if resp := doJSON(t, c, "DELETE", ts.URL+"/api/vendors/"+registry.SlotAnchor(slotSalt, 0), "dev-operator-key", nil); resp.StatusCode != http.StatusNotFound {
		t.Errorf("delete now-free slot 0 again: want 404 (nothing to delete or free), got %d", resp.StatusCode)
	}
}

// TestSlotOnboard_DeleteByHandleResolvesAnchor locks the fix: a
// DELETE keyed by the human handle the onboard POST issued (not the uuid
// anchor) must resolve to the anchor, free the slot, and 204 — instead of
// 404ing and leaving an orphaned org row that 409s the next onboard.
func TestSlotOnboard_DeleteByHandleResolvesAnchor(t *testing.T) {
	dir := t.TempDir()
	aclPath := filepath.Join(dir, "config.json5")
	if err := os.WriteFile(aclPath, []byte("{}"), 0o644); err != nil {
		t.Fatalf("seed acl: %v", err)
	}
	ts, _ := newSlotPoolServer(t, aclPath)
	c := &http.Client{}

	const handle = "fra-land-bde"
	// Onboard by handle; the vendor row + slot are keyed by the anchor.
	if resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{"prefix": handle}); resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard %s: %d %s", handle, resp.StatusCode, mustReadBody(t, resp))
	}

	// DELETE by the HANDLE (the identifier the operator knows), not the anchor.
	// Pre-fix this 404s (delete matches the anchor-keyed row) and orphans the slot.
	if resp := doJSON(t, c, "DELETE", ts.URL+"/api/vendors/"+handle, "dev-operator-key", nil); resp.StatusCode != http.StatusNoContent {
		t.Fatalf("delete by handle %q: want 204, got %d %s", handle, resp.StatusCode, mustReadBody(t, resp))
	}

	// Proof the slot was actually freed (not orphaned): re-onboarding the same
	// handle succeeds — pre-fix this 409s ("alias already assigned").
	if resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{"prefix": handle}); resp.StatusCode != http.StatusCreated {
		t.Fatalf("re-onboard %s after delete-by-handle: want 201 (slot freed), got %d %s", handle, resp.StatusCode, mustReadBody(t, resp))
	}
}

func TestSlotOnboard_PoolExhaustionReturns503(t *testing.T) {
	dir := t.TempDir()
	aclPath := filepath.Join(dir, "config.json5")
	if err := os.WriteFile(aclPath, []byte("{}"), 0o644); err != nil {
		t.Fatalf("seed acl: %v", err)
	}
	ts, _ := newSlotPoolServer(t, aclPath)
	c := &http.Client{}

	// Pool size is 3 — onboard 3, the 4th must 503 (planned-restart message).
	for i, handle := range []string{"sys-a", "sys-b", "sys-c"} {
		resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key",
			map[string]any{"prefix": handle})
		if resp.StatusCode != http.StatusCreated {
			t.Fatalf("onboard #%d (%s): status=%d body=%s", i+1, handle, resp.StatusCode, mustReadBody(t, resp))
		}
	}
	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key",
		map[string]any{"prefix": "sys-d"})
	if resp.StatusCode != http.StatusServiceUnavailable {
		t.Fatalf("4th onboard into a 3-slot pool: want 503, got %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
}
