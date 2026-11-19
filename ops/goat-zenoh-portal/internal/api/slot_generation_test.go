package api_test

// GHSA-rr3p / GHSA-fghv: every slot change advances the slot's generation, the
// leaf minted for it carries the new generation in its CN and is recorded, and
// the router's slot subjects are re-rendered from the registry so a free slot
// names no CN and an assigned slot names exactly its current one.

import (
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/acl"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

const slotFragmentIdentity = "7a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d"

// newFragmentServer is a slot-pool server whose router config carries the
// slot-fragment markers, as the router role renders it.
func newFragmentServer(t *testing.T) (string, func() string, *registry.Registry, string) {
	t.Helper()
	cfgPath := filepath.Join(t.TempDir(), "config.json5")
	seed := "{\n  access_control: {\n    subjects: [\n" + acl.SlotFragmentBegin + "\n" + acl.SlotFragmentEnd + "\n    ],\n  },\n}\n"
	if err := os.WriteFile(cfgPath, []byte(seed), 0o644); err != nil {
		t.Fatal(err)
	}
	ts, reg := newSlotPoolServer(t, cfgPath)
	read := func() string {
		t.Helper()
		b, err := os.ReadFile(cfgPath)
		if err != nil {
			t.Fatal(err)
		}
		return string(b)
	}
	return ts.URL, read, reg, cfgPath
}

// subjectCNs returns the cert_common_names line of slot-<anchor> in cfg.
func subjectCNs(t *testing.T, cfg, anchor string) string {
	t.Helper()
	_, rest, ok := strings.Cut(cfg, `id: "slot-`+anchor+`",`)
	if !ok {
		t.Fatalf("no subject slot-%s in\n%s", anchor, cfg)
	}
	line, _, _ := strings.Cut(strings.TrimSpace(rest), "\n")
	return line
}

func TestSlotGeneration_onboardRotateRevoke(t *testing.T) {
	url, cfg, reg, _ := newFragmentServer(t)
	c := &http.Client{}

	resp := doJSON(t, c, "POST", url+"/api/vendors", "dev-operator-key",
		map[string]any{"prefix": "alpha", "identity_id": slotFragmentIdentity})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	b := mustReadJSON(t, resp)
	anchor, _ := b["slot_id"].(string)
	if got, _ := b["leaf_cn"].(string); got != anchor+".g1" {
		t.Fatalf("onboard leaf_cn = %q, want %s.g1", got, anchor)
	}
	if got := subjectCNs(t, cfg(), anchor); got != `cert_common_names: ["`+anchor+`.g1"],` {
		t.Fatalf("after onboard the subject is %s", got)
	}
	// The other two pool slots are free and name no CN.
	for k := 1; k < 3; k++ {
		if a := registry.SlotAnchor(slotSalt, k); subjectCNs(t, cfg(), a) != `cert_common_names: ["`+a+`.unassigned"],` {
			got := subjectCNs(t, cfg(), a)
			t.Fatalf("free slot %d subject = %s", k, got)
		}
	}
	if strings.Count(cfg(), `"`+anchor+`.g`)+strings.Count(cfg(), `"`+anchor+`"`) != 1 {
		t.Fatalf("the anchor appears as a leaf CN more than once:\n%s", cfg())
	}
	if _, err := acl.SlotFragmentDigest(fragmentOf(t, cfg())); err != nil {
		t.Fatalf("live fragment digest: %v", err)
	}

	// Rotate: generation 2, the generation-1 CN is gone in the same render.
	resp = doJSON(t, c, "POST", url+"/api/vendors/"+anchor+"/rotate", "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("rotate: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	if got, _ := mustReadJSON(t, resp)["leaf_cn"].(string); got != anchor+".g2" {
		t.Fatalf("rotate leaf_cn = %q, want %s.g2", got, anchor)
	}
	if got := subjectCNs(t, cfg(), anchor); got != `cert_common_names: ["`+anchor+`.g2"],` {
		t.Fatalf("after rotate the subject is %s", got)
	}
	leaves, err := reg.SlotLeaves(t.Context(), anchor)
	if err != nil || len(leaves) != 2 {
		t.Fatalf("leaves after onboard + rotate = %d (%v), want 2", len(leaves), err)
	}
	if leaves[0].Generation != 1 || leaves[0].SupersededAt == nil || leaves[1].Generation != 2 || leaves[1].SupersededAt != nil {
		t.Fatalf("leaf records wrong: %+v", leaves)
	}
	if leaves[1].Serial == "" || leaves[1].NotAfter <= leaves[1].IssuedAt || leaves[1].IssuedForIdentity != slotFragmentIdentity {
		t.Fatalf("leaf record lacks serial, validity or identity: %+v", leaves[1])
	}

	// Offboard: the slot names no CN, and the slot is not reissued.
	resp = doJSON(t, c, "DELETE", url+"/api/vendors/"+anchor, "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("delete: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	if b := mustReadJSON(t, resp); b["fabric_revocation"] != "unknown" || b["revoked_cn"] != anchor+".g2" {
		t.Fatalf("delete with no reload record = %+v, want fabric_revocation unknown for %s.g2", b, anchor)
	}
	if got := subjectCNs(t, cfg(), anchor); got != `cert_common_names: ["`+anchor+`.unassigned"],` {
		t.Fatalf("after offboard the subject is %s", got)
	}
	if strings.Contains(cfg(), `"`+anchor+`.g`) || strings.Contains(cfg(), `["`+anchor+`"]`) {
		t.Fatalf("the offboarded slot still names a CN:\n%s", cfg())
	}
	resp = doJSON(t, c, "POST", url+"/api/vendors", "dev-operator-key", map[string]any{"prefix": "bravo"})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard bravo: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	if got, _ := mustReadJSON(t, resp)["slot_id"].(string); got == anchor {
		t.Fatal("the offboarded slot was reissued while its leaves are valid")
	}
}

// A deferred onboard renders the slot's generation-1 CN, which no leaf has
// yet; collection mints at that generation instead of advancing it again.
func TestSlotGeneration_collect(t *testing.T) {
	url, cfg, reg, _ := newFragmentServer(t)
	c := &http.Client{}
	resp := doJSON(t, c, "POST", url+"/api/vendors", "dev-operator-key",
		map[string]any{"prefix": "charlie", "identity_id": slotFragmentIdentity, "defer_credential": true})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("deferred onboard: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	anchor, _ := mustReadJSON(t, resp)["slot_id"].(string)
	if got := subjectCNs(t, cfg(), anchor); got != `cert_common_names: ["`+anchor+`.g1"],` {
		t.Fatalf("after deferred onboard the subject is %s", got)
	}
	resp = doJSON(t, c, "POST", url+"/api/orgs/collect", "dev-operator-key", map[string]any{"identity_id": slotFragmentIdentity})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("collect: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	b := mustReadJSON(t, resp)
	if got, _ := b["leaf_cn"].(string); got != anchor+".g1" {
		t.Fatalf("collect leaf_cn = %q, want %s.g1 (the leafless generation the assignment made)", got, anchor)
	}
	if got, _ := b["leaf_status"].(string); got != "unknown" {
		t.Fatalf("leaf_status with no reload record = %q, want unknown", got)
	}
	if leaves, _ := reg.SlotLeaves(t.Context(), anchor); len(leaves) != 1 || leaves[0].Generation != 1 {
		t.Fatalf("collect leaf record: %+v", leaves)
	}
}

// Where the router host records what the router loaded, collection holds the
// credential (mints nothing) until the router admits the slot's current CN,
// and a slot still at generation 0 is advanced first.
func TestSlotGeneration_collectHeldUntilLive(t *testing.T) {
	url, _, reg, cfgPath := newFragmentServer(t)
	c := &http.Client{}
	anchor := seedUncollectedSlot(t, reg, "delta", slotFragmentIdentity)
	// Model a pre-change slot: assigned at generation 0.
	if _, err := reg.DB().ExecContext(t.Context(), `UPDATE slots SET generation = 0 WHERE slot_id = $1`, anchor); err != nil {
		t.Fatal(err)
	}
	loaded := filepath.Join(filepath.Dir(cfgPath), acl.SlotFragmentLoadedFile)
	writeLoaded := func(cn string) {
		t.Helper()
		frag, _, err := acl.RenderSlotFragment([]acl.SlotBinding{{Anchor: anchor, CN: cn}})
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(loaded, []byte(frag), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	writeLoaded(anchor) // the router loaded generation 0

	resp := doJSON(t, c, "POST", url+"/api/orgs/collect", "dev-operator-key", map[string]any{"identity_id": slotFragmentIdentity})
	if resp.StatusCode != http.StatusServiceUnavailable {
		t.Fatalf("collect before the router loaded the new CN: %d %s, want 503", resp.StatusCode, mustReadBody(t, resp))
	}
	if code := errCode(t, resp); code != "fabric_pending" {
		t.Fatalf("code = %q, want fabric_pending", code)
	}
	if leaves, _ := reg.SlotLeaves(t.Context(), anchor); len(leaves) != 0 {
		t.Fatalf("a pending collect minted: %+v", leaves)
	}
	// A retry before the reload stays pending and does not advance again.
	resp = doJSON(t, c, "POST", url+"/api/orgs/collect", "dev-operator-key", map[string]any{"identity_id": slotFragmentIdentity})
	if resp.StatusCode != http.StatusServiceUnavailable {
		t.Fatalf("retry before reload: %d", resp.StatusCode)
	}
	var gen int64
	if err := reg.DB().GetContext(t.Context(), &gen, `SELECT generation FROM slots WHERE slot_id = $1`, anchor); err != nil || gen != 1 {
		t.Fatalf("generation after two pending collects = %d (%v), want 1", gen, err)
	}

	writeLoaded(anchor + ".g1") // the watcher reloaded the router
	resp = doJSON(t, c, "POST", url+"/api/orgs/collect", "dev-operator-key", map[string]any{"identity_id": slotFragmentIdentity})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("collect after reload: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	b := mustReadJSON(t, resp)
	if b["leaf_cn"] != anchor+".g1" || b["leaf_status"] != "live" {
		t.Fatalf("collect after reload = leaf_cn %v status %v", b["leaf_cn"], b["leaf_status"])
	}
}

// A machine enrolment mints at the assignment's generation and records it;
// the machine revoke frees the slot and its CN goes.
func TestSlotGeneration_machineEnrollAndRevoke(t *testing.T) {
	url, cfg, reg, _ := newFragmentServer(t)
	if err := reg.SetMachineGrant(t.Context(), "alice", 2, "usareur-af", "", "op"); err != nil {
		t.Fatal(err)
	}
	c := &http.Client{}
	resp := doJSON(t, c, "POST", url+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "alice-sensor-1", "node_class": "SENSOR-FEED", "autonomy": "supervised", "custodian": "alice",
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("enroll: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	b := mustReadJSON(t, resp)
	anchor, _ := b["slot_id"].(string)
	if got, _ := b["leaf_cn"].(string); got != anchor+".g1" {
		t.Fatalf("machine leaf_cn = %q, want %s.g1", got, anchor)
	}
	if got := subjectCNs(t, cfg(), anchor); got != `cert_common_names: ["`+anchor+`.g1"],` {
		t.Fatalf("after enroll the subject is %s", got)
	}
	if leaves, _ := reg.SlotLeaves(t.Context(), anchor); len(leaves) != 1 || leaves[0].Generation != 1 {
		t.Fatalf("machine leaf record: %+v", leaves)
	}
	resp = doJSON(t, c, "DELETE", url+"/api/machines/"+anchor, "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("machine revoke: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	if got := subjectCNs(t, cfg(), anchor); got != `cert_common_names: ["`+anchor+`.unassigned"],` {
		t.Fatalf("after machine revoke the subject is %s", got)
	}
}

func fragmentOf(t *testing.T, cfg string) string {
	t.Helper()
	_, rest, ok := strings.Cut(cfg, acl.SlotFragmentBegin+"\n")
	if !ok {
		t.Fatal("no fragment begin marker")
	}
	body, _, ok := strings.Cut(rest, acl.SlotFragmentEnd)
	if !ok {
		t.Fatal("no fragment end marker")
	}
	return body
}

// An offboard says whether the router still admits the revoked leaf: pending
// while the loaded slot subjects name it, live once they do not. The audit
// event records the same.
func TestSlotRevocation_reportsPendingThenLive(t *testing.T) {
	url, _, reg, cfgPath := newFragmentServer(t)
	c := &http.Client{}
	resp := doJSON(t, c, "POST", url+"/api/vendors", "dev-operator-key", map[string]any{"prefix": "echo"})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	anchor, _ := mustReadJSON(t, resp)["slot_id"].(string)
	loaded := filepath.Join(filepath.Dir(cfgPath), acl.SlotFragmentLoadedFile)
	frag, _, err := acl.RenderSlotFragment([]acl.SlotBinding{{Anchor: anchor, CN: anchor + ".g1"}})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(loaded, []byte(frag), 0o644); err != nil { // the router loaded the holder's leaf
		t.Fatal(err)
	}
	resp = doJSON(t, c, "DELETE", url+"/api/vendors/"+anchor, "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("delete: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	b := mustReadJSON(t, resp)
	if b["fabric_revocation"] != "pending_router_reload" || b["slot_freed"] != true {
		t.Fatalf("delete before the router reload = %+v, want pending_router_reload", b)
	}
	if note, _ := b["note"].(string); !strings.Contains(note, "not complete") {
		t.Fatalf("a pending revocation must not read as complete: %q", note)
	}
	var details string
	if err := reg.DB().GetContext(t.Context(), &details,
		`SELECT details FROM audit_events WHERE action = 'vendor-delete' AND target = $1 ORDER BY id DESC LIMIT 1`, anchor); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(details, `"fabric_revocation":"pending_router_reload"`) {
		t.Fatalf("audit event does not record the pending revocation: %s", details)
	}
	// The watcher reloads: the loaded subjects no longer name any leaf. An
	// offboard whose leaf the router does not admit reports live.
	empty, _, _ := acl.RenderSlotFragment([]acl.SlotBinding{{Anchor: anchor}})
	if err := os.WriteFile(loaded, []byte(empty), 0o644); err != nil {
		t.Fatal(err)
	}
	resp = doJSON(t, c, "POST", url+"/api/vendors", "dev-operator-key", map[string]any{"prefix": "foxtrot"})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard foxtrot: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	other, _ := mustReadJSON(t, resp)["slot_id"].(string)
	resp = doJSON(t, c, "DELETE", url+"/api/vendors/"+other, "dev-operator-key", nil)
	if b := mustReadJSON(t, resp); b["fabric_revocation"] != "live" {
		t.Fatalf("delete of a leaf the router does not admit = %+v, want live", b)
	}
}

// GHSA-rr3p review r4 F1: relinking a collected slot to another identity hands
// it over: the generation moves (the previous person's leaf leaves the
// rendered subjects) and the new person can collect.
func TestSlotIdentityRelink_handsOver(t *testing.T) {
	url, cfg, reg, _ := newFragmentServer(t)
	c := &http.Client{}
	const other = "3e4f5a6b-7c8d-4e9f-8a0b-1c2d3e4f5a6b"
	resp := doJSON(t, c, "POST", url+"/api/vendors", "dev-operator-key",
		map[string]any{"prefix": "golf", "identity_id": slotFragmentIdentity, "defer_credential": true})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	anchor, _ := mustReadJSON(t, resp)["slot_id"].(string)
	resp = doJSON(t, c, "POST", url+"/api/orgs/collect", "dev-operator-key", map[string]any{"identity_id": slotFragmentIdentity})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("collect: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	keyA, _ := mustReadJSON(t, resp)["api_key"].(string)
	if r := doJSON(t, c, "GET", url+"/api/orgs/"+anchor+"/labels", keyA, nil); r.StatusCode != http.StatusOK {
		t.Fatalf("A's key before the hand-over: %d", r.StatusCode)
	}
	before := subjectCNs(t, cfg(), anchor)
	resp = doJSON(t, c, "PUT", url+"/api/slots/"+anchor+"/identity", "dev-operator-key", map[string]any{"identity_id": other})
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("relink: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	if b := mustReadJSON(t, resp); b["handed_over"] != true || b["revoked_cn"] != anchor+".g1" || b["fabric_revocation"] != "unknown" {
		t.Fatalf("relink of a collected slot = %+v, want handed_over, revoked_cn %s.g1, fabric_revocation unknown", b, anchor)
	}
	// A's portal key no longer authenticates as the anchor (r5 F2).
	if r := doJSON(t, c, "GET", url+"/api/orgs/"+anchor+"/labels", keyA, nil); r.StatusCode != http.StatusUnauthorized {
		t.Fatalf("A's key after the hand-over: %d, want 401", r.StatusCode)
	}
	var details string
	if err := reg.DB().GetContext(t.Context(), &details,
		`SELECT details FROM audit_events WHERE action = 'slot.identity.set' AND target = $1 ORDER BY id DESC LIMIT 1`, anchor); err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{`"previous_identity_id":"` + slotFragmentIdentity + `"`, `"generation_from":1`, `"generation_to":2`, `"handed_over":true`} {
		if !strings.Contains(details, want) {
			t.Fatalf("audit row %s lacks %s", details, want)
		}
	}
	if after := subjectCNs(t, cfg(), anchor); after == before || !strings.Contains(after, anchor+".g2") {
		t.Fatalf("subject after hand-over = %s (before %s), want the next generation", after, before)
	}
	if leaves, _ := reg.SlotLeaves(t.Context(), anchor); len(leaves) != 1 || leaves[0].SupersededAt == nil {
		t.Fatalf("the previous person's leaf is not superseded: %+v", leaves)
	}
	resp = doJSON(t, c, "POST", url+"/api/orgs/collect", "dev-operator-key", map[string]any{"identity_id": other})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("the new identity's collect: %d %s", resp.StatusCode, mustReadBody(t, resp))
	}
	if got, _ := mustReadJSON(t, resp)["leaf_cn"].(string); got != anchor+".g2" {
		t.Fatalf("new identity's leaf_cn = %q, want %s.g2", got, anchor)
	}
}
