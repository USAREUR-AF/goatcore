package api_test

// Tests for GET /api/machines/{anchor} (machine-mesh-alignment D6a): the
// single-machine registry read bundle-create's --from-machine consumes. The
// derivation needs no mgmt server, so both the mesh-enabled harness
// (machine_mesh_bulk_test.go) and the mint-disabled one
// (machine_enroll_test.go) are exercised.

import (
	"net/http"
	"strings"
	"testing"
)

func TestGetMachine_DerivedGroupsSiteAndExerciseScope(t *testing.T) {
	mgmt := fakeMgmt(t, "GALAXY-SENSOR-FEED")
	ts, reg := newMeshMintServer(t, mgmt.URL)
	_ = reg.SetMachineGrant(t.Context(), "alice", 1, "usareur-af", "", "op")
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "alice-feed-1", "node_class": "SENSOR-FEED", "autonomy": "supervised", "custodian": "alice",
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("enroll: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	anchor, _ := mustReadJSON(t, resp)["slot_id"].(string)

	// Site scope: full names under the goatnet prefix, derivable.
	get := doJSON(t, c, "GET", ts.URL+"/api/machines/"+anchor, "dev-operator-key", nil)
	if get.StatusCode != http.StatusOK {
		t.Fatalf("get: status=%d body=%s", get.StatusCode, mustReadBody(t, get))
	}
	b := mustReadJSON(t, get)
	if b["node_class"] != "SENSOR-FEED" || b["custodian"] != "alice" || b["system_handle"] != "alice-feed-1" {
		t.Fatalf("listItem fields wrong: %+v", b)
	}
	if d, _ := b["mesh_derivable"].(bool); !d {
		t.Fatalf("mesh_derivable = %v, want true", b["mesh_derivable"])
	}
	mg, _ := b["mesh_groups"].([]any)
	if len(mg) != 1 || mg[0] != "GALAXY-SENSOR-FEED" {
		t.Fatalf("mesh_groups = %v, want [GALAXY-SENSOR-FEED]", b["mesh_groups"])
	}

	// Exercise scope: same suffix derivation under the EX prefix.
	ex := doJSON(t, c, "GET", ts.URL+"/api/machines/"+anchor+"?exercise=drill1", "dev-operator-key", nil)
	if ex.StatusCode != http.StatusOK {
		t.Fatalf("get exercise: status=%d body=%s", ex.StatusCode, mustReadBody(t, ex))
	}
	eb := mustReadJSON(t, ex)
	emg, _ := eb["mesh_groups"].([]any)
	if len(emg) != 1 || emg[0] != "EX-DRILL1-SENSOR-FEED" {
		t.Fatalf("exercise mesh_groups = %v, want [EX-DRILL1-SENSOR-FEED]", eb["mesh_groups"])
	}
	if eb["mesh_scope"] != "drill1" {
		t.Fatalf("mesh_scope = %v, want drill1", eb["mesh_scope"])
	}

	// Unknown exercise is refused with the known names (same contract as the
	// mint doors), before any registry read matters.
	unk := doJSON(t, c, "GET", ts.URL+"/api/machines/"+anchor+"?exercise=nosuch", "dev-operator-key", nil)
	if unk.StatusCode != http.StatusBadRequest || errCode(t, unk) != "unknown_exercise" {
		t.Fatalf("unknown exercise: status=%d code=%s, want 400 unknown_exercise", unk.StatusCode, errCode(t, unk))
	}

	// Unknown anchor → 404, not a guess.
	nf := doJSON(t, c, "GET", ts.URL+"/api/machines/no-such-anchor", "dev-operator-key", nil)
	if nf.StatusCode != http.StatusNotFound {
		t.Fatalf("unknown anchor: status=%d, want 404", nf.StatusCode)
	}
}

func TestGetMachine_LegacyClassReads200NotRefusal(t *testing.T) {
	mgmt := fakeMgmt(t, "GALAXY-SENSOR-FEED")
	ts, reg := newMeshMintServer(t, mgmt.URL)
	ctx := t.Context()
	_ = reg.SetMachineGrant(ctx, "alice", 1, "usareur-af", "", "op")

	// A legacy born-as class (whole-system SENSOR, retro-stamped) has no
	// derivation. The GET is a read: 200 + mesh_derivable:false + the fix
	// named, where the re-mint would 409.
	slot, err := reg.AllocateSlot(ctx, "legacy-sensor")
	if err != nil {
		t.Fatalf("allocate: %v", err)
	}
	if err := reg.RecordMachineEnrollment(ctx, slot.SlotID, "alice", "SENSOR", "remote", "usareur-af", nil); err != nil {
		t.Fatalf("record legacy: %v", err)
	}

	c := &http.Client{}
	get := doJSON(t, c, "GET", ts.URL+"/api/machines/"+slot.SlotID, "dev-operator-key", nil)
	if get.StatusCode != http.StatusOK {
		t.Fatalf("legacy get: status=%d body=%s, want 200 (a read, not a refusal)", get.StatusCode, mustReadBody(t, get))
	}
	b := mustReadJSON(t, get)
	if d, _ := b["mesh_derivable"].(bool); d {
		t.Fatalf("mesh_derivable = true for legacy SENSOR, want false")
	}
	if _, present := b["mesh_groups"]; present {
		t.Fatalf("mesh_groups present for an underivable class: %v", b["mesh_groups"])
	}
	note, _ := b["mesh_note"].(string)
	if !strings.Contains(note, "Re-stamp") || !strings.Contains(note, "PUT /api/slots/{anchor}/custody") {
		t.Fatalf("mesh_note should name the re-stamp fix, got %q", note)
	}
}

func TestGetMachine_VendorScopedOwnCustodyOnly(t *testing.T) {
	mgmt := fakeMgmt(t, "GALAXY-KID")
	ts, reg := newMeshMintServer(t, mgmt.URL)
	c := &http.Client{}
	// Onboard the bearers FIRST: the grant + custody edge normalize a handle
	// to its slot anchor once the slot exists, and the principal's Vendor is
	// that anchor.
	_, frankKey := onboardVendorBearer(t, ts, "frank")
	_, malloryKey := onboardVendorBearer(t, ts, "mallory")
	if err := reg.SetMachineGrant(t.Context(), "frank", 1, "unit-f", "", "op"); err != nil {
		t.Fatalf("SetMachineGrant: %v", err)
	}

	resp := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "frank-kid-1", "node_class": "KID", "autonomy": "remote", "custodian": "frank",
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("enroll: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	anchor, _ := mustReadJSON(t, resp)["slot_id"].(string)

	own := doJSON(t, c, "GET", ts.URL+"/api/machines/"+anchor, frankKey, nil)
	if own.StatusCode != http.StatusOK {
		t.Fatalf("custodian's own read: status=%d body=%s, want 200", own.StatusCode, mustReadBody(t, own))
	}
	other := doJSON(t, c, "GET", ts.URL+"/api/machines/"+anchor, malloryKey, nil)
	if other.StatusCode != http.StatusForbidden {
		t.Fatalf("cross-custodian read: status=%d, want 403", other.StatusCode)
	}
}

func TestGetMachine_MintDisabledStillDerives(t *testing.T) {
	// The derivation needs no mgmt server: the mint-disabled harness has no
	// netbird URL and no site prefix, so the groups come back as bare
	// suffixes — exactly what the enroll response does on the same harness.
	ts, reg := newMachineServer(t)
	_ = reg.SetMachineGrant(t.Context(), "grace", 1, "unit-g", "", "op")
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "grace-feed-1", "node_class": "SENSOR-FEED", "autonomy": "manual", "custodian": "grace",
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("enroll: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	anchor, _ := mustReadJSON(t, resp)["slot_id"].(string)

	get := doJSON(t, c, "GET", ts.URL+"/api/machines/"+anchor, "dev-operator-key", nil)
	if get.StatusCode != http.StatusOK {
		t.Fatalf("get: status=%d body=%s", get.StatusCode, mustReadBody(t, get))
	}
	b := mustReadJSON(t, get)
	if d, _ := b["mesh_derivable"].(bool); !d {
		t.Fatalf("mesh_derivable = %v, want true on a mint-disabled portal", b["mesh_derivable"])
	}
	mg, _ := b["mesh_groups"].([]any)
	if len(mg) != 1 || mg[0] != "SENSOR-FEED" {
		t.Fatalf("mesh_groups = %v, want bare [SENSOR-FEED] with no site prefix configured", b["mesh_groups"])
	}
}
