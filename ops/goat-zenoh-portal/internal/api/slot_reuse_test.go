package api_test

import (
	"net/http"
	"path/filepath"
	"strings"
	"testing"
)

// #1576: a person whose slot outlived their onboarding record (archived, then
// restored; or a mint that failed after the slot leg) comes back through a
// deferred onboard with the same handle. Before the fix the portal answered
// alias_taken and the restore landed in `failed`. A deferred onboard returns no
// credential, so handing back the existing slot is safe; these tests fence that
// it happens ONLY for a deferred, non-partner request, and never relinks.

const otherIdentity = "22222222-2222-4222-8222-222222222222"

func deferredOnboard(t *testing.T, c *http.Client, url string, body map[string]any) (*http.Response, map[string]any) {
	t.Helper()
	resp := doJSON(t, c, "POST", url+"/api/orgs", "dev-operator-key", body)
	if resp.StatusCode >= 300 {
		return resp, nil
	}
	return resp, mustReadJSON(t, resp)
}

func TestSlotReuse_DeferredAliasTakenReturnsExistingSlot(t *testing.T) {
	ts, reg := newSlotPoolServer(t, filepath.Join(t.TempDir(), "config.json5"))
	c := &http.Client{}

	first, ob := deferredOnboard(t, c, ts.URL, map[string]any{
		"prefix": "mlee", "identity_id": collectorIdentity, "defer_credential": true,
	})
	if first.StatusCode != http.StatusCreated {
		t.Fatalf("first deferred onboard: %d", first.StatusCode)
	}
	anchor, _ := ob["slot_id"].(string)

	again, body := deferredOnboard(t, c, ts.URL, map[string]any{
		"prefix": "mlee", "identity_id": collectorIdentity, "defer_credential": true,
	})
	if again.StatusCode != http.StatusOK {
		t.Fatalf("re-onboard of a held handle: want 200 reused, got %d %s", again.StatusCode, mustReadBody(t, again))
	}
	if got, _ := body["slot_id"].(string); got != anchor {
		t.Errorf("reused slot_id = %q, want the existing %q", got, anchor)
	}
	if reused, _ := body["reused"].(bool); !reused {
		t.Errorf("reused flag missing: %#v", body)
	}
	if got, _ := body["identity_id"].(string); got != collectorIdentity {
		t.Errorf("identity_id = %q, want %q", got, collectorIdentity)
	}
	for _, k := range []string{"api_key", "bundle_url", "bundle_token"} {
		if v, _ := body[k].(string); v != "" {
			t.Errorf("reuse returned %s=%q; it must mint nothing", k, v)
		}
	}
	total, assigned, err := reg.SlotPoolCounts(t.Context())
	if err != nil || assigned != 1 {
		t.Errorf("pool after reuse: total=%d assigned=%d err=%v, want exactly one assigned slot", total, assigned, err)
	}

	// No identity on the request (the native onboard's order) still reuses, and
	// reports the slot's recorded identity so the caller can check it.
	_, body = deferredOnboard(t, c, ts.URL, map[string]any{"prefix": "mlee", "defer_credential": true})
	if got, _ := body["identity_id"].(string); got != collectorIdentity {
		t.Errorf("identity-less reuse reported identity_id %q, want the recorded %q", got, collectorIdentity)
	}
}

func TestSlotReuse_NonDeferredStillConflicts(t *testing.T) {
	ts, _ := newSlotPoolServer(t, filepath.Join(t.TempDir(), "config.json5"))
	c := &http.Client{}
	if r, _ := deferredOnboard(t, c, ts.URL, map[string]any{"prefix": "echo", "defer_credential": true}); r.StatusCode != http.StatusCreated {
		t.Fatalf("seed: %d", r.StatusCode)
	}
	resp := doJSON(t, c, "POST", ts.URL+"/api/orgs", "dev-operator-key", map[string]any{"prefix": "echo"})
	if resp.StatusCode != http.StatusConflict {
		t.Fatalf("non-deferred re-onboard: want 409 alias_taken, got %d", resp.StatusCode)
	}
}

func TestSlotReuse_IdentityMismatchRefused(t *testing.T) {
	ts, _ := newSlotPoolServer(t, filepath.Join(t.TempDir(), "config.json5"))
	c := &http.Client{}
	if r, _ := deferredOnboard(t, c, ts.URL, map[string]any{
		"prefix": "echo", "identity_id": collectorIdentity, "defer_credential": true,
	}); r.StatusCode != http.StatusCreated {
		t.Fatalf("seed: %d", r.StatusCode)
	}
	resp, _ := deferredOnboard(t, c, ts.URL, map[string]any{
		"prefix": "echo", "identity_id": otherIdentity, "defer_credential": true,
	})
	if resp.StatusCode != http.StatusConflict {
		t.Fatalf("reuse for a different identity: want 409 identity_mismatch, got %d", resp.StatusCode)
	}
	if b := mustReadBody(t, resp); !strings.Contains(b, "identity_mismatch") {
		t.Errorf("refusal does not name identity_mismatch: %s", b)
	}
}

func TestSlotReuse_RevokedSlotIsNotReused(t *testing.T) {
	ts, _ := newSlotPoolServer(t, filepath.Join(t.TempDir(), "config.json5"))
	c := &http.Client{}
	if r, _ := deferredOnboard(t, c, ts.URL, map[string]any{"prefix": "echo", "defer_credential": true}); r.StatusCode != http.StatusCreated {
		t.Fatalf("seed: %d", r.StatusCode)
	}
	if resp := doJSON(t, c, "DELETE", ts.URL+"/api/orgs/echo", "dev-operator-key", nil); resp.StatusCode != http.StatusNoContent {
		t.Fatalf("revoke: %d", resp.StatusCode)
	}
	resp, body := deferredOnboard(t, c, ts.URL, map[string]any{"prefix": "echo", "defer_credential": true})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard after revoke: want a fresh 201 allocation, got %d", resp.StatusCode)
	}
	if reused, _ := body["reused"].(bool); reused {
		t.Errorf("a revoked slot was reused: %#v", body)
	}
}
