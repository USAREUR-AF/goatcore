package api_test

import (
	"net/http"
	"testing"
)

// GHSA-rr3p review r3 F4: a revoked machine's slot stays in reuse quarantine
// while its leaf is valid, and that slot counts against the custodian's
// machine quota. Enrolling and revoking in a loop therefore cannot retire pool
// slots beyond the custodian's quota (the reviewer's pool-drain probe drained
// a pool of three with quota 1).
func TestMachineQuota_quarantinedSlotsCountAgainstTheCustodian(t *testing.T) {
	ts, reg := newMachineServer(t) // pool of 3
	ctx := t.Context()
	prefix, key := onboardVendorBearer(t, ts, "rr3p-custodian") // takes one slot
	if err := reg.SetMachineGrant(ctx, prefix, 1, "unit-rr3p", "", "op"); err != nil {
		t.Fatal(err)
	}
	c := &http.Client{}
	enrol := func(h string) *http.Response {
		return doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", key, map[string]any{
			"machine_handle": h, "node_class": "FUSION", "autonomy": "autonomous",
		})
	}
	r := enrol("rr3p-m-a")
	if r.StatusCode != http.StatusCreated {
		t.Fatalf("first enrol: %d %s", r.StatusCode, mustReadBody(t, r))
	}
	anchor, _ := mustReadJSON(t, r)["slot_id"].(string)
	rv := doJSON(t, c, "DELETE", ts.URL+"/api/machines/"+anchor, key, nil)
	if rv.StatusCode != http.StatusOK {
		t.Fatalf("revoke: %d %s", rv.StatusCode, mustReadBody(t, rv))
	}
	if q := mustReadJSON(t, rv)["quota_remaining"]; q != float64(0) {
		t.Fatalf("quota_remaining after revoke = %v, want 0 (the quarantined slot is still the custodian's)", q)
	}
	q, err := reg.MachineQuotaFor(ctx, prefix)
	if err != nil || q.Quarantined != 1 || q.Remaining != 0 {
		t.Fatalf("quota view = %+v (%v), want quarantined 1, remaining 0", q, err)
	}
	r = enrol("rr3p-m-b")
	if r.StatusCode != http.StatusConflict || errCode(t, r) != "quota_exhausted" {
		t.Fatalf("second enrol inside the quarantine: %d, want 409 quota_exhausted", r.StatusCode)
	}
	if free, err := reg.GetFreeSlot(ctx); err != nil || free == nil {
		t.Fatalf("the pool must keep a free slot: %v, %v", free, err)
	}
	// Raising the quota is the operator's relief for a genuine replacement.
	if err := reg.SetMachineGrant(ctx, prefix, 2, "unit-rr3p", "", "op"); err != nil {
		t.Fatal(err)
	}
	if r := enrol("rr3p-m-c"); r.StatusCode != http.StatusCreated {
		t.Fatalf("enrol after the quota was raised: %d %s", r.StatusCode, mustReadBody(t, r))
	}
	// Once the quarantined leaf expires it no longer counts.
	pastReuseQuarantine(t)
	if q, _ := reg.MachineQuotaFor(ctx, prefix); q.Quarantined != 0 {
		t.Fatalf("an expired leaf still counts: %+v", q)
	}
}
