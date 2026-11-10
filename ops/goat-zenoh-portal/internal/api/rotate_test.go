package api_test

import (
	"net/http"
	"os"
	"path/filepath"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// TestRotateVendor_ReissuesInPlace locks the core rotate contract (ADR 1031's
// sibling): POST /api/vendors/{anchor}/rotate re-mints a fresh leaf for the
// SAME anchor with a fresh portal api_key, consuming NO slot, touching NO ACL,
// and leaving the alias/handle in place. This is the non-destructive path that
// replaces delete+re-onboard for expiry/compromise/URI-SAN-scheme rotation.
func TestRotateVendor_ReissuesInPlace(t *testing.T) {
	dir := t.TempDir()
	aclPath := filepath.Join(dir, "config.json5")
	const aclMarker = "// pre-baked per-uuid ACL; rotate must not touch this\n"
	if err := os.WriteFile(aclPath, []byte(aclMarker), 0o644); err != nil {
		t.Fatalf("seed acl: %v", err)
	}
	ts, _ := newSlotPoolServer(t, aclPath)
	c := &http.Client{}

	// Onboard one system into a 3-slot pool.
	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key",
		map[string]any{"prefix": "fra-land-bde"})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	onboard := mustReadJSON(t, resp)
	anchor, _ := onboard["slot_id"].(string)
	if anchor == "" {
		t.Fatalf("onboard returned no slot_id")
	}
	wantAnchor := registry.SlotAnchor(slotSalt, 0)
	if anchor != wantAnchor {
		t.Fatalf("onboard slot_id=%q, want %q", anchor, wantAnchor)
	}
	oldKey, _ := onboard["api_key"].(string)
	oldToken, _ := onboard["bundle_token"].(string)

	aliasCount := func() int {
		r := doJSON(t, c, "GET", ts.URL+"/api/aliases", "dev-operator-key", nil)
		return len(mustReadJSON(t, r)["aliases"].([]any))
	}
	if got := aliasCount(); got != 1 {
		t.Fatalf("aliases after onboard = %d, want 1", got)
	}

	// Rotate the anchor.
	resp = doJSON(t, c, "POST", ts.URL+"/api/vendors/"+anchor+"/rotate", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("rotate: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	rot := mustReadJSON(t, resp)

	// Identity is unchanged: same anchor/prefix/slot_id and the same handle.
	if got, _ := rot["prefix"].(string); got != anchor {
		t.Errorf("rotate prefix=%q, want unchanged anchor %q", got, anchor)
	}
	if got, _ := rot["slot_id"].(string); got != anchor {
		t.Errorf("rotate slot_id=%q, want unchanged anchor %q", got, anchor)
	}
	if got, _ := rot["system_handle"].(string); got != "fra-land-bde" {
		t.Errorf("rotate system_handle=%q, want fra-land-bde", got)
	}
	// URI-SAN is re-derived from the current minter config (the scheme-retrofit
	// mechanism): always <prefix>+anchor for this server.
	if got, _ := rot["uri_san"].(string); got != "urn:goat:galaxy:org="+anchor {
		t.Errorf("rotate uri_san=%q, want urn:goat:galaxy:org=%s", got, anchor)
	}

	// Credentials are fresh: new api_key + new one-time bundle token.
	newKey, _ := rot["api_key"].(string)
	if newKey == "" || newKey == oldKey {
		t.Errorf("rotate api_key=%q, want a fresh non-empty key (old=%q)", newKey, oldKey)
	}
	if newToken, _ := rot["bundle_token"].(string); newToken == "" || newToken == oldToken {
		t.Errorf("rotate bundle_token=%q, want a fresh token (old=%q)", newToken, oldToken)
	}

	// The fresh portal key authenticates a vendor-scoped read.
	if r := doJSON(t, c, "GET", ts.URL+"/api/vendors/"+anchor+"/topics", newKey, nil); r.StatusCode != http.StatusOK {
		t.Errorf("read with rotated key: status=%d, want 200", r.StatusCode)
	}

	// No churn: alias count unchanged, ACL file byte-identical, and the slot
	// pool was NOT consumed — onboarding two MORE systems fills the remaining
	// 2 slots exactly (proving rotate cost zero slots).
	if got := aliasCount(); got != 1 {
		t.Errorf("aliases after rotate = %d, want 1 (no churn)", got)
	}
	if after, _ := os.ReadFile(aclPath); string(after) != aclMarker {
		t.Errorf("ACL modified by rotate; want untouched. got: %q", string(after))
	}
	for _, h := range []string{"ita-recon-coy", "deu-log-coy"} {
		if r := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{"prefix": h}); r.StatusCode != http.StatusCreated {
			t.Fatalf("post-rotate onboard %s: want 201, got %d %s", h, r.StatusCode, mustReadBody(t, r))
		}
	}
	if r := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{"prefix": "overflow"}); r.StatusCode != http.StatusServiceUnavailable {
		t.Errorf("4th onboard after rotate: want 503 (pool was 3, rotate consumed none), got %d", r.StatusCode)
	}
}

// TestRotateVendor_UnknownAnchor404 — rotation re-issues an existing cert; it
// never creates. An unknown anchor is a 404, not an implicit onboard.
func TestRotateVendor_UnknownAnchor404(t *testing.T) {
	dir := t.TempDir()
	aclPath := filepath.Join(dir, "config.json5")
	if err := os.WriteFile(aclPath, []byte("{}"), 0o644); err != nil {
		t.Fatalf("seed acl: %v", err)
	}
	ts, _ := newSlotPoolServer(t, aclPath)
	c := &http.Client{}

	unknown := registry.SlotAnchor(slotSalt, 0) // a valid anchor shape, never onboarded
	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors/"+unknown+"/rotate", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusNotFound {
		t.Fatalf("rotate unknown anchor: want 404, got %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
}

// TestRotateVendor_RequiresOperator — rotation is operator-gated; a vendor
// Bearer token (or none) must not be able to re-issue a cert.
func TestRotateVendor_RequiresOperator(t *testing.T) {
	dir := t.TempDir()
	aclPath := filepath.Join(dir, "config.json5")
	if err := os.WriteFile(aclPath, []byte("{}"), 0o644); err != nil {
		t.Fatalf("seed acl: %v", err)
	}
	ts, _ := newSlotPoolServer(t, aclPath)
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{"prefix": "alpha"})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	body := mustReadJSON(t, resp)
	anchor, _ := body["slot_id"].(string)
	vendorKey, _ := body["api_key"].(string)

	// The vendor's own Bearer token must be rejected at the operator gate.
	if r := doJSON(t, c, "POST", ts.URL+"/api/vendors/"+anchor+"/rotate", vendorKey, nil); r.StatusCode == http.StatusOK {
		t.Errorf("rotate with vendor key succeeded; want operator-only rejection")
	}
}
