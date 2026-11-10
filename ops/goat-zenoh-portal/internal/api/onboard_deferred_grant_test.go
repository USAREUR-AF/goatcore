package api_test

import (
	"errors"
	"net/http"
	"path/filepath"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// F-613: a deferred (mint-on-collect) onboard returned before the machine grant
// was seeded, so every person onboarded that way with a machine_quota held no
// grant and `goat machine enroll` answered 403. The grant is an authorization
// approved at intake and does not wait for the credential.

func TestDeferredOnboard_SeedsTheMachineGrant(t *testing.T) {
	ts, reg := newSlotPoolServer(t, filepath.Join(t.TempDir(), "config.json5"))
	c := &http.Client{}

	resp, ob := deferredOnboard(t, c, ts.URL, map[string]any{
		"prefix": "grant-holder", "identity_id": collectorIdentity, "defer_credential": true,
		"machine_quota": 5, "affiliation": "mdc-e-g-6",
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("deferred onboard: %d", resp.StatusCode)
	}
	anchor, _ := ob["slot_id"].(string)

	g, err := reg.MachineGrantFor(t.Context(), anchor)
	if err != nil {
		t.Fatalf("no machine grant after a deferred onboard with machine_quota: %v", err)
	}
	if g.Quota != 5 || g.Affiliation != "mdc-e-g-6" {
		t.Errorf("grant = quota %d affiliation %q, want 5 mdc-e-g-6", g.Quota, g.Affiliation)
	}
	// The handle resolves to the same grant, as `goat machine enroll` looks it up.
	if q, err := reg.MachineQuotaFor(t.Context(), "grant-holder"); err != nil || q.Quota != 5 || q.Remaining != 5 {
		t.Errorf("quota by handle = %+v err=%v, want 5 remaining", q, err)
	}
}

func TestDeferredOnboard_NoQuotaNoGrant(t *testing.T) {
	ts, reg := newSlotPoolServer(t, filepath.Join(t.TempDir(), "config.json5"))
	c := &http.Client{}

	for _, tc := range []struct {
		prefix string
		body   map[string]any
	}{
		{"no-quota", map[string]any{"affiliation": "mdc-e-g-6"}},
		{"no-affiliation", map[string]any{"machine_quota": 5}},
	} {
		tc.body["prefix"] = tc.prefix
		tc.body["defer_credential"] = true
		resp, ob := deferredOnboard(t, c, ts.URL, tc.body)
		if resp.StatusCode != http.StatusCreated {
			t.Fatalf("%s: deferred onboard: %d", tc.prefix, resp.StatusCode)
		}
		anchor, _ := ob["slot_id"].(string)
		if _, err := reg.MachineGrantFor(t.Context(), anchor); !errors.Is(err, registry.ErrNotFound) {
			t.Errorf("%s: want no grant, got err=%v", tc.prefix, err)
		}
	}
}
