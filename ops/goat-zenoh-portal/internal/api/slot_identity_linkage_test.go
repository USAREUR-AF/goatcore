package api_test

// The recorded person→slot linkage (identity fragility audit §1.2):
// the onboard flow holds the Kratos identity UUID and used to discard it, so
// every consumer re-derived ownership by parsing names. These tests pin the
// linkage being written at onboard, echoed back, served read-side, and cleared
// when a slot is re-claimed by someone else.

import (
	"net/http"
	"path/filepath"
	"testing"
)

const testIdentityUUID = "3f2a9c1e-7b44-4d2f-9a10-c8b6e5d40712"

func TestSlotOnboard_RecordsIdentityLinkage(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{
		"prefix":      "fra-land-bde",
		"identity_id": testIdentityUUID,
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard: status %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	body := mustReadJSON(t, resp)
	anchor, _ := body["slot_id"].(string)
	if anchor == "" {
		t.Fatalf("no slot_id in onboard response: %#v", body)
	}
	if got, _ := body["identity_id"].(string); got != testIdentityUUID {
		t.Errorf("response identity_id = %q, want %q", got, testIdentityUUID)
	}

	// Recorded on the slot row itself.
	aliases, err := reg.ListAliases(t.Context())
	if err != nil {
		t.Fatalf("ListAliases: %v", err)
	}
	var found bool
	for _, a := range aliases {
		if a.SlotID != anchor {
			continue
		}
		found = true
		if a.IdentityID != testIdentityUUID {
			t.Errorf("slot %s identity_id = %q, want %q", anchor, a.IdentityID, testIdentityUUID)
		}
	}
	if !found {
		t.Fatalf("anchor %s missing from ListAliases", anchor)
	}

	// And served read-side, where slot ownership is read.
	resp = doJSON(t, c, "GET", ts.URL+"/api/aliases", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("GET /api/aliases: status %d", resp.StatusCode)
	}
	listed := mustReadJSON(t, resp)
	rows, _ := listed["aliases"].([]any)
	if len(rows) != 1 {
		t.Fatalf("aliases = %#v, want 1 row", listed["aliases"])
	}
	row, _ := rows[0].(map[string]any)
	if got, _ := row["identity_id"].(string); got != testIdentityUUID {
		t.Errorf("/api/aliases identity_id = %q, want %q", got, testIdentityUUID)
	}
}

// A slot onboarded with no identity_id keeps an honest NULL rather than an empty
// string or a fabricated value — the read side must be able to tell "no recorded
// linkage" from "linked to nobody".
func TestSlotOnboard_NoIdentityLeavesLinkageEmpty(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key",
		map[string]any{"prefix": "isr-recon-det"})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard: status %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	if got, ok := mustReadJSON(t, resp)["identity_id"]; ok {
		t.Errorf("identity_id present on an unlinked onboard: %#v", got)
	}
	aliases, err := reg.ListAliases(t.Context())
	if err != nil {
		t.Fatalf("ListAliases: %v", err)
	}
	if len(aliases) != 1 || aliases[0].IdentityID != "" {
		t.Errorf("aliases = %#v, want one row with an empty identity_id", aliases)
	}
}

// A handle or an email in this column would re-create the mutable-key join the
// column exists to retire, so a non-UUID is refused before a slot is spent.
func TestSlotOnboard_RejectsNonUUIDIdentity(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}

	for _, bad := range []string{"jane", "jane@example.test", "not-a-uuid"} {
		resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{
			"prefix":      "alpha-fires-bn",
			"identity_id": bad,
		})
		if resp.StatusCode != http.StatusBadRequest {
			t.Fatalf("identity_id=%q: status %d, want 400 (body=%s)", bad, resp.StatusCode, mustReadBody(t, resp))
		}
	}
	// No slot was consumed by any of the refusals.
	if _, assigned, err := reg.SlotPoolCounts(t.Context()); err != nil || assigned != 0 {
		t.Errorf("SlotPoolCounts assigned = (%d,%v), want (0,nil) — a rejected onboard burned a slot", assigned, err)
	}
}

// The stamp-after-the-fact path: the native `goat admin onboard` mints the slot
// BEFORE its Ory leg runs, and pre-existing slots predate the column, so both
// need an audited write path rather than a hand-edited SQLite row.
func TestPutSlotIdentity_StampsCorrectsAndClears(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key",
		map[string]any{"prefix": "fra-land-bde"})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard: status %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	anchor, _ := mustReadJSON(t, resp)["slot_id"].(string)

	readBack := func() string {
		t.Helper()
		aliases, err := reg.ListAliases(t.Context())
		if err != nil {
			t.Fatalf("ListAliases: %v", err)
		}
		for _, a := range aliases {
			if a.SlotID == anchor {
				return a.IdentityID
			}
		}
		t.Fatalf("anchor %s not found", anchor)
		return ""
	}

	// Stamp.
	resp = doJSON(t, c, "PUT", ts.URL+"/api/slots/"+anchor+"/identity", "dev-operator-key",
		map[string]any{"identity_id": testIdentityUUID})
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("stamp: status %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	if got := readBack(); got != testIdentityUUID {
		t.Errorf("after stamp identity_id = %q, want %q", got, testIdentityUUID)
	}

	// A wrong stamp is withdrawn to "not recorded", never left as a confident
	// wrong owner.
	resp = doJSON(t, c, "PUT", ts.URL+"/api/slots/"+anchor+"/identity", "dev-operator-key",
		map[string]any{"identity_id": ""})
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("clear: status %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	if got := readBack(); got != "" {
		t.Errorf("after clear identity_id = %q, want empty", got)
	}

	// Same key discipline as the onboard path, and an unknown anchor 404s.
	resp = doJSON(t, c, "PUT", ts.URL+"/api/slots/"+anchor+"/identity", "dev-operator-key",
		map[string]any{"identity_id": "dana@example.test"})
	if resp.StatusCode != http.StatusBadRequest {
		t.Errorf("non-UUID stamp: status %d, want 400", resp.StatusCode)
	}
	resp = doJSON(t, c, "PUT", ts.URL+"/api/slots/deadbeef/identity", "dev-operator-key",
		map[string]any{"identity_id": testIdentityUUID})
	if resp.StatusCode != http.StatusNotFound {
		t.Errorf("unknown anchor: status %d, want 404", resp.StatusCode)
	}
}

// A re-claimed slot must not carry the prior tenant's linkage: attributing the
// new tenant's writes (and any revocation aimed at them) to the previous person
// is the mis-attribution class this whole item exists to close.
func TestSlotReclaim_ClearsPriorIdentityLinkage(t *testing.T) {
	dir := t.TempDir()
	ts, reg := newSlotPoolServer(t, filepath.Join(dir, "config.json5"))
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{
		"prefix":      "fra-land-bde",
		"identity_id": testIdentityUUID,
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("first onboard: status %d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	anchor, _ := mustReadJSON(t, resp)["slot_id"].(string)

	if err := reg.RevokeSlot(t.Context(), anchor); err != nil {
		t.Fatalf("RevokeSlot: %v", err)
	}
	slot, err := reg.AllocateSlot(t.Context(), "next-tenant")
	if err != nil {
		t.Fatalf("AllocateSlot: %v", err)
	}
	if slot.SlotID != anchor {
		t.Fatalf("re-allocated %s, wanted the freed slot %s", slot.SlotID, anchor)
	}
	if slot.IdentityID != nil {
		t.Errorf("re-claimed slot carries the prior tenant's identity_id %q", *slot.IdentityID)
	}
	aliases, err := reg.ListAliases(t.Context())
	if err != nil {
		t.Fatalf("ListAliases: %v", err)
	}
	for _, a := range aliases {
		if a.SlotID == anchor && a.IdentityID != "" {
			t.Errorf("re-claimed slot still reads identity_id %q in the registry", a.IdentityID)
		}
	}
}
