package api_test

import (
	"net/http"
	"testing"
)

// ADR 1131 — the Herd gear console gates every interactive bearer-gear session
// on this entitlement, so the read has to distinguish three states and never
// blur them: no grant at all, a grant without the entitlement, and a grant with
// it. The first two are both "denied", but they are different things to debug.

func TestGetMachineGrant_NoGrantIs404(t *testing.T) {
	ts, _ := newMachineServer(t)
	c := &http.Client{}

	r := doJSON(t, c, "GET", ts.URL+"/api/machines/grant/nobody", "dev-operator-key", nil)
	if r.StatusCode != http.StatusNotFound {
		t.Fatalf("custodian with no grant row: got %d, want 404 — a caller must be able to tell "+
			"'no grant' from 'grant without the entitlement': %s", r.StatusCode, mustReadBody(t, r))
	}
	errObj, _ := mustReadJSON(t, r)["error"].(map[string]any)
	if code, _ := errObj["code"].(string); code != "no_grant" {
		t.Errorf("error code = %q, want no_grant", code)
	}
}

func TestGetMachineGrant_ReadsBackTheEntitlement(t *testing.T) {
	ts, _ := newMachineServer(t)
	c := &http.Client{}

	// A grant WITHOUT the entitlement reads network_admin false — the fail-closed
	// default that every legacy row carries.
	r := doJSON(t, c, "POST", ts.URL+"/api/machines/grant", "dev-operator-key", map[string]any{
		"custodian": "morgan", "quota": 5, "affiliation": "usareur-af", "site": "harbor",
	})
	if r.StatusCode != http.StatusOK {
		t.Fatalf("seed grant: %d %s", r.StatusCode, mustReadBody(t, r))
	}

	r = doJSON(t, c, "GET", ts.URL+"/api/machines/grant/morgan", "dev-operator-key", nil)
	if r.StatusCode != http.StatusOK {
		t.Fatalf("read grant: %d %s", r.StatusCode, mustReadBody(t, r))
	}
	g := mustReadJSON(t, r)
	if na, _ := g["network_admin"].(bool); na {
		t.Error("network_admin = true on a grant that never set it — the default must be fail-closed")
	}

	// Granting the entitlement is visible to the reader.
	r = doJSON(t, c, "POST", ts.URL+"/api/machines/grant", "dev-operator-key", map[string]any{
		"custodian": "morgan", "quota": 5, "network_admin": true,
	})
	if r.StatusCode != http.StatusOK {
		t.Fatalf("grant network_admin: %d %s", r.StatusCode, mustReadBody(t, r))
	}

	r = doJSON(t, c, "GET", ts.URL+"/api/machines/grant/morgan", "dev-operator-key", nil)
	if r.StatusCode != http.StatusOK {
		t.Fatalf("re-read grant: %d %s", r.StatusCode, mustReadBody(t, r))
	}
	g = mustReadJSON(t, r)
	if na, _ := g["network_admin"].(bool); !na {
		t.Error("network_admin = false after it was granted — the read is not seeing the write")
	}

	// The scope travels with the answer. ADR 1113 §3 bounds the entitlement to a
	// site; an answer without the site would be an over-broad yes.
	if site, _ := g["site"].(string); site == "" {
		t.Error("site is empty — the entitlement is site-bounded and the caller cannot scope without it")
	}
}

// The route must not be open. It reports who may administer bearer gear, which
// is a map of where to attack the estate.
func TestGetMachineGrant_RequiresAuth(t *testing.T) {
	ts, _ := newMachineServer(t)
	c := &http.Client{}

	r := doJSON(t, c, "GET", ts.URL+"/api/machines/grant/morgan", "", nil)
	if r.StatusCode == http.StatusOK {
		t.Fatal("unauthenticated read succeeded — this endpoint names the estate's network admins")
	}
}
