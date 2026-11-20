package api_test

// Loud-failure tests for the two surfaces the identity fragility
// audit found could not report their own failure:
//
//   - GET /api/budget-projection served a VALID, EMPTY projection off a reset
//     registry, which the crawler believed — every owner silently reclassified
//     unmeasured while the source-health metric read green (§1.3 violation 1).
//   - GET /api/health was a constant "ok" that no failure could change, so
//     portal death surfaced only via two unrelated ≥30-minute proxies
//     (§1.3 violation 5).
//
// Each test drives the failure the fix exists for, not just the happy path.

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
)

// TestBudgetProjection_RefusesResetRegistry points the projection at a pool that
// is baked but has nothing assigned and no grants — the shape a restored or
// emptied registry takes — and asserts it 503s with a reason instead of serving
// an empty projection the crawler would consume as truth.
func TestBudgetProjection_RefusesResetRegistry(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}

	// The pool is initialized (3 slots) with nothing onboarded: assigned == 0,
	// grants == 0, slots > 0.
	if total, assigned, err := reg.SlotPoolCounts(t.Context()); err != nil || total != 3 || assigned != 0 {
		t.Fatalf("SlotPoolCounts = (%d,%d,%v), want (3,0,nil)", total, assigned, err)
	}

	resp := doJSON(t, c, "GET", ts.URL+"/api/budget-projection", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusServiceUnavailable {
		t.Fatalf("empty projection on a populated pool: status %d, want 503 (body=%s)",
			resp.StatusCode, mustReadBody(t, resp))
	}
	body := mustReadJSON(t, resp)
	errObj, _ := body["error"].(map[string]any)
	if errObj == nil {
		t.Fatalf("503 body carries no error object: %#v", body)
	}
	if code, _ := errObj["code"].(string); code != "registry_reset" {
		t.Errorf("error code = %q, want registry_reset", code)
	}
	if msg, _ := errObj["message"].(string); msg == "" {
		t.Error("503 carries no reason — the crawler operator has to guess why")
	}
}

// TestBudgetProjection_ServesOnceASlotIsAssigned is the other half: the floor
// must not block a legitimately-quiet-but-live goatnet. One onboard is enough to
// make the projection plausible again.
func TestBudgetProjection_ServesOnceASlotIsAssigned(t *testing.T) {
	dir := t.TempDir()
	ts, _ := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key",
		map[string]any{"prefix": "fra-land-bde"})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard: status %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}

	resp = doJSON(t, c, "GET", ts.URL+"/api/budget-projection", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("projection with one assigned slot: status %d body=%s",
			resp.StatusCode, mustReadBody(t, resp))
	}
	body := mustReadJSON(t, resp)
	anchors, _ := body["auto_self_service_anchors"].([]any)
	if len(anchors) != 1 {
		t.Errorf("auto_self_service_anchors = %#v, want exactly the one onboarded anchor", body["auto_self_service_anchors"])
	}
}

// TestBudgetProjection_EmptyIsFineOnAPerVendorGoatnet: with no slot pool at all
// (per-vendor mode, nova-a), an empty projection is the honest answer and the floor
// must stay out of the way.
func TestBudgetProjection_EmptyIsFineOnAPerVendorGoatnet(t *testing.T) {
	ts := newTestServer(t)
	c := &http.Client{}

	resp := doJSON(t, c, "GET", ts.URL+"/api/budget-projection", "dev-operator-key", nil)
	// The per-vendor test server leaves PORTAL_TOPIC_BUDGET_SELF_BLOCK unset, so
	// the pre-existing config guard answers first — the point being that the new
	// floor never turns an honest empty answer into a 503 for the wrong reason.
	if resp.StatusCode == http.StatusServiceUnavailable {
		body := mustReadJSON(t, resp)
		errObj, _ := body["error"].(map[string]any)
		if code, _ := errObj["code"].(string); code == "registry_reset" {
			t.Errorf("per-vendor goatnet (no slot pool) refused as a reset registry: %#v", errObj)
		}
	}
}

// TestHealth_ProbesTheRegistry asserts /api/health reports the registry's actual
// state: 200 while it is readable, 503 with a reason once it is not. Closing the
// registry handle is the in-process stand-in for the file being gone, locked, or
// corrupt — every one of which the old constant "ok" reported as healthy.
func TestHealth_ProbesTheRegistry(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}

	resp := doJSON(t, c, "GET", ts.URL+"/api/health", "", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("healthy portal: status %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	body := mustReadJSON(t, resp)
	if s, _ := body["status"].(string); s != "ok" {
		t.Errorf("status = %q, want ok", s)
	}
	if s, _ := body["registry"].(string); s != "ok" {
		t.Errorf("registry = %q, want ok", s)
	}

	// Break the registry underneath the running server.
	if err := reg.Close(); err != nil {
		t.Fatalf("close registry: %v", err)
	}

	resp = doJSON(t, c, "GET", ts.URL+"/api/health", "", nil)
	if resp.StatusCode != http.StatusServiceUnavailable {
		t.Fatalf("portal with an unreadable registry: status %d, want 503 (body=%s)",
			resp.StatusCode, mustReadBody(t, resp))
	}
	// The 503 is a problem body (ADR 1184): status is the HTTP status, the
	// health word moves to `health`, and the reason is a fixed sentence.
	if ct := resp.Header.Get("Content-Type"); ct != "application/problem+json; charset=utf-8" {
		t.Errorf("503 Content-Type = %q", ct)
	}
	body = mustReadJSON(t, resp)
	if s, _ := body["health"].(string); s != "unhealthy" {
		t.Errorf("health = %q, want unhealthy", s)
	}
	if s, _ := body["status"].(float64); s != http.StatusServiceUnavailable {
		t.Errorf("status = %v, want 503", body["status"])
	}
	if reason, _ := body["reason"].(string); reason == "" {
		t.Error("unhealthy response carries no reason — an operator learns nothing from it")
	}
}

// TestHealth_ReportsShadowedOperatorKeys: a directory key shadowed by the env
// tier cannot be revoked from the console. /api/health carries the count so the
// condition is visible without host access; the handles themselves stay in the
// host log, not on an unauthenticated endpoint.
func TestHealth_ReportsShadowedOperatorKeys(t *testing.T) {
	keyDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(keyDir, "ada.key"), []byte("ada:op_dir_ada\nbob:op_dir_bob\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	store, err := config.NewOperatorKeyStore(map[string]string{"ada": "op_env_ada"}, keyDir)
	if err != nil {
		t.Fatal(err)
	}
	ts, _ := newSlotPoolServer(t, filepath.Join(t.TempDir(), "config.json5"), func(c *config.Config) {
		c.OperatorKeys = store
	})
	resp := doJSON(t, &http.Client{}, "GET", ts.URL+"/api/health", "", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("status %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	body := mustReadJSON(t, resp)
	if n, _ := body["operator_keys_shadowed"].(float64); n != 1 {
		t.Errorf("operator_keys_shadowed = %v, want 1", body["operator_keys_shadowed"])
	}
	if strings.Contains(fmt.Sprint(body), "ada") {
		t.Errorf("health body names a handle: %v", body)
	}
}

func TestHealth_ShadowedOperatorKeysZeroWithoutStore(t *testing.T) {
	ts, _ := newSlotPoolServer(t, filepath.Join(t.TempDir(), "config.json5"))
	body := mustReadJSON(t, doJSON(t, &http.Client{}, "GET", ts.URL+"/api/health", "", nil))
	if n, ok := body["operator_keys_shadowed"].(float64); !ok || n != 0 {
		t.Errorf("operator_keys_shadowed = %v, want 0", body["operator_keys_shadowed"])
	}
}

// GHSA-2w7v: an env key refused by a console revocation tombstone is counted
// on /api/health, by number only.
func TestHealth_ReportsEnvRevokedOperatorKeys(t *testing.T) {
	keyDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(keyDir, config.TombstoneFileName("ada")), []byte("ada\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	store, err := config.NewOperatorKeyStore(map[string]string{"ada": "op_env_ada"}, keyDir)
	if err != nil {
		t.Fatal(err)
	}
	ts, _ := newSlotPoolServer(t, filepath.Join(t.TempDir(), "config.json5"), func(c *config.Config) {
		c.OperatorKeys = store
	})
	body := mustReadJSON(t, doJSON(t, &http.Client{}, "GET", ts.URL+"/api/health", "", nil))
	if n, _ := body["operator_keys_env_revoked"].(float64); n != 1 {
		t.Errorf("operator_keys_env_revoked = %v, want 1", body["operator_keys_env_revoked"])
	}
	if strings.Contains(fmt.Sprint(body), "ada") {
		t.Errorf("health body names a handle: %v", body)
	}
}
