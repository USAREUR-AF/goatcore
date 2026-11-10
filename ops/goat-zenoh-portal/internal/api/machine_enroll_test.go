package api_test

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

// newMachineServer is a slot-pool portal with a scratch ACL file, for the ADR
// 1096 machine-enrollment endpoint.
func newMachineServer(t *testing.T) (*httptest.Server, *registry.Registry) {
	t.Helper()
	aclPath := filepath.Join(t.TempDir(), "config.json5")
	if err := os.WriteFile(aclPath, []byte("{}"), 0o644); err != nil {
		t.Fatalf("seed acl: %v", err)
	}
	return newSlotPoolServer(t, aclPath)
}

// onboardVendorBearer onboards a slot-system as the operator and returns its
// (prefix, api_key) so a test can act as that vendor-scoped principal.
func onboardVendorBearer(t *testing.T, ts *httptest.Server, handle string) (string, string) {
	t.Helper()
	c := &http.Client{}
	resp := doJSON(t, c, "POST", ts.URL+"/api/vendors", "dev-operator-key", map[string]any{"prefix": handle})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("onboard %s bearer: status=%d body=%s", handle, resp.StatusCode, mustReadBody(t, resp))
	}
	b := mustReadJSON(t, resp)
	prefix, _ := b["prefix"].(string)
	key, _ := b["api_key"].(string)
	if prefix == "" || key == "" {
		t.Fatalf("onboard %s: empty prefix/api_key", handle)
	}
	return prefix, key
}

func TestMachineEnroll_OperatorOnBehalf_HappyPath(t *testing.T) {
	ts, reg := newMachineServer(t)
	ctx := t.Context()
	if err := reg.SetMachineGrant(ctx, "alice", 2, "usareur-af", "", "op"); err != nil {
		t.Fatalf("SetMachineGrant: %v", err)
	}
	c := &http.Client{}
	resp := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "alice-sensor-1", "node_class": "SENSOR-FEED", "autonomy": "supervised", "custodian": "alice",
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("enroll: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	b := mustReadJSON(t, resp)
	if b["custodian"] != "alice" || b["node_class"] != "SENSOR-FEED" || b["autonomy"] != "supervised" || b["affiliation"] != "usareur-af" {
		t.Fatalf("response stamp wrong: %+v", b)
	}
	if got, _ := b["quota_remaining"].(float64); got != 1 {
		t.Fatalf("quota_remaining=%v, want 1", b["quota_remaining"])
	}
	if s, _ := b["slot_id"].(string); s == "" {
		t.Fatal("empty slot_id")
	}
	if k, _ := b["api_key"].(string); k == "" {
		t.Fatal("empty api_key (machine's portal bearer)")
	}
	// Registry reflects the custody edge + quota consumption.
	q, _ := reg.MachineQuotaFor(ctx, "alice")
	if q.Used != 1 || q.Remaining != 1 {
		t.Fatalf("quota after enroll = %+v, want used 1 remaining 1", q)
	}
	fleet, _ := reg.MachinesForCustodian(ctx, "alice")
	if len(fleet) != 1 || fleet[0].NodeClass == nil || *fleet[0].NodeClass != "SENSOR-FEED" {
		t.Fatalf("custody read model wrong: %+v", fleet)
	}
	// ADR 1116 §9 — the mesh groups are DERIVED from the class, not typed. With
	// mint disabled (the default in these tests) they are still reported, so the
	// operator assigns exactly them and nothing else.
	mg, _ := b["mesh_groups"].([]any)
	if len(mg) != 1 || mg[0] != "SENSOR-FEED" {
		t.Fatalf("mesh_groups = %v, want [SENSOR-FEED] derived from the node-class", b["mesh_groups"])
	}
	if k, _ := b["mesh_setup_key"].(string); k != "" {
		t.Fatalf("mesh_setup_key should be empty when minting is disabled, got %q", k)
	}
	if n, _ := b["mesh_note"].(string); !strings.Contains(n, "NOT minted") {
		t.Fatalf("mesh_note should say the key was not minted, got %q", n)
	}
}

func TestMachineEnroll_QuotaExhausted(t *testing.T) {
	ts, reg := newMachineServer(t)
	_ = reg.SetMachineGrant(t.Context(), "bob", 1, "unit-b", "", "op")
	c := &http.Client{}
	first := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "bob-1", "node_class": "SENSOR-FEED", "autonomy": "manual", "custodian": "bob",
	})
	if first.StatusCode != http.StatusCreated {
		t.Fatalf("first enroll: status=%d body=%s", first.StatusCode, mustReadBody(t, first))
	}
	second := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "bob-2", "node_class": "SENSOR-FEED", "autonomy": "manual", "custodian": "bob",
	})
	if second.StatusCode != http.StatusConflict {
		t.Fatalf("over-quota enroll: status=%d body=%s, want 409", second.StatusCode, mustReadBody(t, second))
	}
	if c := errCode(t, second); c != "quota_exhausted" {
		t.Fatalf("error code = %q, want quota_exhausted", c)
	}
}

func TestMachineEnroll_ValidationAndNoGrant(t *testing.T) {
	ts, reg := newMachineServer(t)
	_ = reg.SetMachineGrant(t.Context(), "carol", 2, "unit-c", "", "op")
	c := &http.Client{}

	// bad node-class
	if r := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "x1", "node_class": "ROUTER", "autonomy": "manual", "custodian": "carol",
	}); r.StatusCode != http.StatusBadRequest {
		t.Fatalf("bad node_class: status=%d, want 400", r.StatusCode)
	}
	// bad autonomy
	if r := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "x2", "node_class": "SENSOR-FEED", "autonomy": "auto", "custodian": "carol",
	}); r.StatusCode != http.StatusBadRequest {
		t.Fatalf("bad autonomy: status=%d, want 400", r.StatusCode)
	}
	// custodian with no grant → fail-closed 403
	r := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "x3", "node_class": "SENSOR-FEED", "autonomy": "manual", "custodian": "nobody",
	})
	if r.StatusCode != http.StatusForbidden {
		t.Fatalf("no-grant: status=%d body=%s, want 403", r.StatusCode, mustReadBody(t, r))
	}
	if c := errCode(t, r); c != "no_enrollment_grant" {
		t.Fatalf("error = %q, want no_enrollment_grant", c)
	}
}

func TestMachineEnroll_InfraGate(t *testing.T) {
	ts, reg := newMachineServer(t)
	ctx := t.Context()
	// A vendor-scoped custodian with a grant.
	prefix, key := onboardVendorBearer(t, ts, "site-custodian")
	_ = reg.SetMachineGrant(ctx, prefix, 3, "unit-i", "", "op")
	c := &http.Client{}

	// self-service caller tries an INFRA class → 403 (operator-gated).
	r := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", key, map[string]any{
		"machine_handle": "mgmt-1", "node_class": "MGMT-HOST", "autonomy": "manual",
	})
	if r.StatusCode != http.StatusForbidden {
		t.Fatalf("vendor infra enroll: status=%d body=%s, want 403", r.StatusCode, mustReadBody(t, r))
	}
	if c := errCode(t, r); c != "infra_requires_operator" {
		t.Fatalf("error = %q, want infra_requires_operator", c)
	}
	// operator CAN enroll the infra node (naming the custodian).
	r2 := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "mgmt-1", "node_class": "MGMT-HOST", "autonomy": "manual", "custodian": prefix,
	})
	if r2.StatusCode != http.StatusCreated {
		t.Fatalf("operator infra enroll: status=%d body=%s, want 201", r2.StatusCode, mustReadBody(t, r2))
	}
}

func TestMachineEnroll_SelfServiceAndMismatch(t *testing.T) {
	ts, reg := newMachineServer(t)
	ctx := t.Context()
	prefix, key := onboardVendorBearer(t, ts, "dev-custodian")
	_ = reg.SetMachineGrant(ctx, prefix, 2, "unit-s", "", "op")
	c := &http.Client{}

	// self-service function enroll: no custodian in body → resolves to own identity.
	r := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", key, map[string]any{
		"machine_handle": "proc-1", "node_class": "FUSION", "autonomy": "autonomous",
	})
	if r.StatusCode != http.StatusCreated {
		t.Fatalf("self-service enroll: status=%d body=%s, want 201", r.StatusCode, mustReadBody(t, r))
	}
	if b := mustReadJSON(t, r); b["custodian"] != prefix {
		t.Fatalf("custodian = %v, want own prefix %q", b["custodian"], prefix)
	}
	// naming a DIFFERENT custodian as a non-operator → 403 mismatch.
	r2 := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", key, map[string]any{
		"machine_handle": "proc-2", "node_class": "FUSION", "autonomy": "autonomous", "custodian": "someone-else",
	})
	if r2.StatusCode != http.StatusForbidden {
		t.Fatalf("mismatch enroll: status=%d body=%s, want 403", r2.StatusCode, mustReadBody(t, r2))
	}
	if c := errCode(t, r2); c != "custodian_mismatch" {
		t.Fatalf("error = %q, want custodian_mismatch", c)
	}
}

func TestMachineEnroll_PodMoonSelfService(t *testing.T) {
	for _, nodeClass := range []string{"POD", "MOON"} {
		t.Run(nodeClass, func(t *testing.T) {
			ts, reg := newMachineServer(t)
			prefix, key := onboardVendorBearer(t, ts, strings.ToLower(nodeClass)+"-custodian")
			if err := reg.SetMachineGrant(t.Context(), prefix, 1, "unit-gear", "", "op"); err != nil {
				t.Fatalf("SetMachineGrant: %v", err)
			}
			c := &http.Client{}

			r := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", key, map[string]any{
				"machine_handle": strings.ToLower(nodeClass) + "-1", "node_class": nodeClass, "autonomy": "remote",
			})
			if r.StatusCode != http.StatusCreated {
				t.Fatalf("self-service %s: status=%d body=%s, want 201", nodeClass, r.StatusCode, mustReadBody(t, r))
			}
			b := mustReadJSON(t, r)
			if b["custodian"] != prefix || b["affiliation"] != "unit-gear" || b["node_class"] != nodeClass {
				t.Fatalf("response stamp wrong: %+v", b)
			}
			if got, _ := b["quota_remaining"].(float64); got != 0 {
				t.Fatalf("quota_remaining=%v, want 0", b["quota_remaining"])
			}
			groups, _ := b["mesh_groups"].([]any)
			if len(groups) != 1 || groups[0] != nodeClass {
				t.Fatalf("mesh_groups=%v, want [%s]", groups, nodeClass)
			}

			q, err := reg.MachineQuotaFor(t.Context(), prefix)
			if err != nil {
				t.Fatalf("MachineQuotaFor: %v", err)
			}
			if q.Used != 1 || q.Remaining != 0 {
				t.Fatalf("quota after enroll=%+v, want used 1 remaining 0", q)
			}

			overQuota := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", key, map[string]any{
				"machine_handle": strings.ToLower(nodeClass) + "-2", "node_class": nodeClass, "autonomy": "remote",
			})
			if overQuota.StatusCode != http.StatusConflict {
				t.Fatalf("over-quota %s: status=%d body=%s, want 409", nodeClass, overQuota.StatusCode, mustReadBody(t, overQuota))
			}
			if code := errCode(t, overQuota); code != "quota_exhausted" {
				t.Fatalf("over-quota error=%q, want quota_exhausted", code)
			}

			_, ungrantedKey := onboardVendorBearer(t, ts, strings.ToLower(nodeClass)+"-ungranted")
			ungranted := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", ungrantedKey, map[string]any{
				"machine_handle": strings.ToLower(nodeClass) + "-3", "node_class": nodeClass, "autonomy": "remote",
			})
			if ungranted.StatusCode != http.StatusForbidden {
				t.Fatalf("ungranted %s: status=%d body=%s, want 403", nodeClass, ungranted.StatusCode, mustReadBody(t, ungranted))
			}
			if code := errCode(t, ungranted); code != "no_enrollment_grant" {
				t.Fatalf("ungranted error=%q, want no_enrollment_grant", code)
			}
		})
	}
}

// errCode extracts the nested error.code from a portal error response
// ({"error":{"code":..,"message":..}}).
func errCode(t *testing.T, resp *http.Response) string {
	t.Helper()
	b := mustReadJSON(t, resp)
	e, _ := b["error"].(map[string]any)
	code, _ := e["code"].(string)
	return code
}

func TestMachineEnroll_PlatformWorkloads(t *testing.T) {
	ts, reg := newMachineServer(t)
	ctx := t.Context()
	_ = reg.SetMachineGrant(ctx, "uas-op", 3, "usareur-af", "", "op")
	c := &http.Client{}

	// PLATFORM with a declared workload composition → 201, echoed in the response.
	r := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "uas-1", "node_class": "PLATFORM", "autonomy": "supervised",
		"custodian": "uas-op", "workloads": []string{"EFFECTOR-CUE", "EFFECTOR-ARBITER", "SENSOR-TELEMETRY"},
	})
	if r.StatusCode != http.StatusCreated {
		t.Fatalf("platform enroll: %d %s", r.StatusCode, mustReadBody(t, r))
	}
	b := mustReadJSON(t, r)
	wl, _ := b["workloads"].([]any)
	if len(wl) != 3 {
		t.Fatalf("response workloads = %v, want 3", b["workloads"])
	}

	// the custody surface surfaces the composition.
	list := doJSON(t, c, "GET", ts.URL+"/api/machines?custodian=uas-op", "dev-operator-key", nil)
	lb := mustReadJSON(t, list)
	arr, _ := lb["machines"].([]any)
	m0, _ := arr[0].(map[string]any)
	if w, _ := m0["workloads"].([]any); len(w) != 3 {
		t.Fatalf("listed workloads = %v, want 3", m0["workloads"])
	}

	// workloads on a non-PLATFORM node-class → 400.
	bad := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "sensor-1", "node_class": "SENSOR-FEED", "autonomy": "manual",
		"custodian": "uas-op", "workloads": []string{"FUSION"},
	})
	if bad.StatusCode != http.StatusBadRequest {
		t.Fatalf("workloads on non-platform = %d, want 400", bad.StatusCode)
	}
	if errCode(t, bad) != "invalid_workloads" {
		t.Fatalf("want invalid_workloads")
	}
}

func TestMachineEnroll_StandaloneOnlyClassesRejectedAsPlatformWorkloads(t *testing.T) {
	ts, reg := newMachineServer(t)
	ctx := t.Context()
	_ = reg.SetMachineGrant(ctx, "boundary-op", 3, "unit-b", "", "op")
	c := &http.Client{}

	_, assignedBefore, err := reg.SlotPoolCounts(ctx)
	if err != nil {
		t.Fatalf("SlotPoolCounts before: %v", err)
	}
	for i, workloads := range [][]string{{"POD"}, {"MOON"}, {"MOON", "POD"}} {
		r := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
			"machine_handle": fmt.Sprintf("boundary-%d", i), "node_class": "PLATFORM", "autonomy": "remote",
			"custodian": "boundary-op", "workloads": workloads,
		})
		status := r.StatusCode
		code := errCode(t, r)
		if status != http.StatusBadRequest || code != "invalid_workloads" {
			t.Fatalf("PLATFORM/%v: status=%d code=%s, want 400 invalid_workloads", workloads, status, code)
		}
	}
	_, assignedAfter, err := reg.SlotPoolCounts(ctx)
	if err != nil {
		t.Fatalf("SlotPoolCounts after: %v", err)
	}
	if assignedAfter != assignedBefore {
		t.Fatalf("invalid compositions allocated slots: before=%d after=%d", assignedBefore, assignedAfter)
	}
	q, _ := reg.MachineQuotaFor(ctx, "boundary-op")
	if q.Used != 0 || q.Remaining != 3 {
		t.Fatalf("invalid compositions consumed quota: %+v", q)
	}
}

// TestMachineEnroll_GearClassGate proves the ADR 1116 §9 mint-time refusals
// FIRE, which is the whole claim: "the setup key is the enforcement point, not
// this document." The Harbor cohort of put an effector ground
// station, a sensor gateway and a human laptop into one group in a single
// sitting because the mint had no opinion. Each case below is a shape that
// sitting would have produced.
func TestMachineEnroll_GearClassGate(t *testing.T) {
	ts, reg := newMachineServer(t)
	_ = reg.SetMachineGrant(t.Context(), "gina", 9, "unit-g", "", "op")
	c := &http.Client{}

	cases := []struct {
		name     string
		body     map[string]any
		wantCode string
		wantSays string // a fragment the refusal must name, so it is actionable
	}{
		{
			// The exact 1116 §3 trap: a ground station named for the whole system.
			name:     "reserved whole-system class refused",
			body:     map[string]any{"machine_handle": "g1", "node_class": "SENSOR", "autonomy": "manual", "custodian": "gina"},
			wantCode: "reserved_node_class",
			wantSays: "SENSOR-FEED",
		},
		{
			name:     "reserved EFFECTOR refused",
			body:     map[string]any{"machine_handle": "g2", "node_class": "EFFECTOR", "autonomy": "manual", "custodian": "gina"},
			wantCode: "reserved_node_class",
			wantSays: "EFFECTOR-CUE",
		},
		{
			// Intake-only alone is an uncovered peer: it could not reach the
			// router to receive the tasking the class exists for.
			name:     "intake-only class refused as a primary class",
			body:     map[string]any{"machine_handle": "g3", "node_class": "SENSOR-TASKING", "autonomy": "manual", "custodian": "gina"},
			wantCode: "intake_only_node_class",
			wantSays: "enumerated by hand",
		},
		{
			name:     "EFFECTOR-COMMAND refused as a primary class",
			body:     map[string]any{"machine_handle": "g4", "node_class": "EFFECTOR-COMMAND", "autonomy": "manual", "custodian": "gina"},
			wantCode: "intake_only_node_class",
			wantSays: "outbound half",
		},
		{
			// A legacy class with no NetBird group. Enrolling it produces a peer
			// holding nothing; the mint says so instead of minting anyway.
			name:     "class with no group derivation refused",
			body:     map[string]any{"machine_handle": "g5", "node_class": "PROCESSOR", "autonomy": "manual", "custodian": "gina"},
			wantCode: "no_mesh_mapping",
			wantSays: "FUSION",
		},
		{
			// A composition of nothing has no derivable reach.
			name:     "PLATFORM with no declared workloads refused",
			body:     map[string]any{"machine_handle": "g6", "node_class": "PLATFORM", "autonomy": "supervised", "custodian": "gina"},
			wantCode: "no_mesh_mapping",
			wantSays: "composition",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			r := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", tc.body)
			if r.StatusCode != http.StatusBadRequest {
				t.Fatalf("status=%d, want 400", r.StatusCode)
			}
			b := mustReadJSON(t, r)
			e, _ := b["error"].(map[string]any)
			code, _ := e["code"].(string)
			msg, _ := e["message"].(string)
			if code != tc.wantCode {
				t.Fatalf("error code = %q, want %q (message: %s)", code, tc.wantCode, msg)
			}
			if !strings.Contains(msg, tc.wantSays) {
				t.Fatalf("refusal does not name %q, so a custodian cannot act on it: %s", tc.wantSays, msg)
			}
		})
	}

	// And the mint still consumed NO quota for any of them: a refusal must not
	// cost the custodian a machine.
	q, _ := reg.MachineQuotaFor(t.Context(), "gina")
	if q.Used != 0 {
		t.Fatalf("refusals consumed quota: used=%d, want 0", q.Used)
	}
}

// TestMachineEnroll_PlatformDerivesFromWorkloads proves the composite case: a
// PLATFORM has no reach of its own, so its groups are the UNION of the functions
// it performs. ADR 1116's own worked example is an effector navigator box — one box,
// EFFECTOR-CUE and EFFECTOR-ARBITER.
func TestMachineEnroll_PlatformDerivesFromWorkloads(t *testing.T) {
	ts, reg := newMachineServer(t)
	_ = reg.SetMachineGrant(t.Context(), "falcon", 2, "usareur-af", "", "op")
	c := &http.Client{}

	r := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "navigator-1", "node_class": "PLATFORM", "autonomy": "supervised",
		"custodian": "falcon", "workloads": []string{"EFFECTOR-CUE", "EFFECTOR-ARBITER"},
	})
	if r.StatusCode != http.StatusCreated {
		t.Fatalf("platform enroll: %d %s", r.StatusCode, mustReadBody(t, r))
	}
	b := mustReadJSON(t, r)
	mg, _ := b["mesh_groups"].([]any)
	got := make([]string, 0, len(mg))
	for _, g := range mg {
		s, _ := g.(string)
		got = append(got, s)
	}
	sort.Strings(got)
	if len(got) != 2 || got[0] != "EFFECTOR-ARBITER" || got[1] != "EFFECTOR-CUE" {
		t.Fatalf("mesh_groups = %v, want the union of the declared workloads", got)
	}
}

// #1404: NETWORK-JUMP was enrollable but had no mesh-group derivation, so the
// operator's mint was refused with no_mesh_mapping before it reached the
// operator gate. It must mint for the operator, derive its own group (ADR 1131
// §2), and stay operator-only for everyone else.
func TestMachineEnroll_NetworkJump(t *testing.T) {
	ts, reg := newMachineServer(t)
	prefix, key := onboardVendorBearer(t, ts, "jump-custodian")
	_ = reg.SetMachineGrant(t.Context(), prefix, 3, "unit-j", "", "op")
	c := &http.Client{}

	r := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", key, map[string]any{
		"machine_handle": "jump-1", "node_class": "NETWORK-JUMP", "autonomy": "manual",
	})
	if r.StatusCode != http.StatusForbidden {
		t.Fatalf("self-service NETWORK-JUMP: status=%d body=%s, want 403", r.StatusCode, mustReadBody(t, r))
	}
	if code := errCode(t, r); code != "infra_requires_operator" {
		t.Fatalf("error = %q, want infra_requires_operator", code)
	}

	r2 := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "jump-1", "node_class": "NETWORK-JUMP", "autonomy": "manual", "custodian": prefix,
	})
	if r2.StatusCode != http.StatusCreated {
		t.Fatalf("operator NETWORK-JUMP: status=%d body=%s, want 201", r2.StatusCode, mustReadBody(t, r2))
	}
	mg, _ := mustReadJSON(t, r2)["mesh_groups"].([]any)
	if len(mg) != 1 || mg[0] != "NETWORK-JUMP" {
		t.Fatalf("mesh_groups = %v, want [NETWORK-JUMP]", mg)
	}
}
