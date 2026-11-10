package api_test

// Tests for the machine-mesh-alignment additions: the quota-neutral mesh-key
// re-mint (D1), the bulk enrollment act (D7) and the exercise key scope (D5),
// all against a mesh-mint-ENABLED portal talking to a fake netbird mgmt server
// (the pre-existing machine_enroll tests cover the mint-disabled report-only
// default).

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/acl"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/api"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/certmint"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/schemastore"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/zenohproxy"
)

// fakeMgmt serves just enough of the netbird mgmt API for the mint: a fixed
// group list and setup keys whose value counts up so every mint is
// distinguishable.
func fakeMgmt(t *testing.T, groupNames ...string) *httptest.Server {
	t.Helper()
	var n atomic.Int64
	mux := http.NewServeMux()
	mux.HandleFunc("/api/groups", func(w http.ResponseWriter, r *http.Request) {
		out := make([]map[string]string, 0, len(groupNames))
		for i, name := range groupNames {
			out = append(out, map[string]string{"id": fmt.Sprintf("gid-%d", i), "name": name})
		}
		_ = json.NewEncoder(w).Encode(out)
	})
	mux.HandleFunc("/api/setup-keys", func(w http.ResponseWriter, r *http.Request) {
		k := n.Add(1)
		w.WriteHeader(http.StatusCreated)
		_ = json.NewEncoder(w).Encode(map[string]string{
			"id": fmt.Sprintf("key-%d", k), "key": fmt.Sprintf("KEY-%04d", k),
		})
	})
	ts := httptest.NewServer(mux)
	t.Cleanup(ts.Close)
	return ts
}

// newMeshMintServer is newSlotPoolServer with the ADR 1116 §9 mesh leg ON,
// pointed at a fake mgmt server, with an exercise map configured.
func newMeshMintServer(t *testing.T, mgmtURL string) (*httptest.Server, *registry.Registry) {
	t.Helper()
	dir := t.TempDir()
	aclPath := filepath.Join(dir, "config.json5")
	if err := os.WriteFile(aclPath, []byte("{}"), 0o644); err != nil {
		t.Fatalf("seed acl: %v", err)
	}
	cfg := &config.Config{
		OperatorAPIKey:       "dev-operator-key",
		IdentityModel:        config.IdentityModelPerUUID,
		SlotPoolSalt:         "mesh-mint-test-salt",
		SlotPoolSize:         4,
		ZenohRestFake:        true,
		ACLConfigPath:        aclPath,
		ACLReloadFake:        true,
		CertSignFake:         true,
		VendorBundleDir:      filepath.Join(dir, "bundles"),
		SchemaDir:            filepath.Join(dir, "schemas"),
		CertSignURIPrefix:    "urn:goat:galaxy:org=",
		NetbirdMgmtURL:       mgmtURL,
		NetbirdAPIToken:      "fake-pat",
		MeshSitePrefix:       "GALAXY",
		MeshKeyMintEnabled:   true,
		MeshKeyTTL:           "720h",
		MeshExercisePrefixes: "drill1=EX-DRILL1, drill2=EX-DRILL2",
	}
	reg, err := registry.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("registry: %v", err)
	}
	t.Cleanup(func() { _ = reg.Close() })
	if _, err := reg.InitializeSlotPool(t.Context(), cfg.SlotPoolSalt, cfg.SlotPoolSize); err != nil {
		t.Fatalf("InitializeSlotPool: %v", err)
	}
	zp := zenohproxy.New("", true)
	minter := &certmint.Minter{Fake: true, VendorBundleDir: cfg.VendorBundleDir, URIPrefix: cfg.CertSignURIPrefix}
	aclR := &acl.Renderer{ConfigPath: cfg.ACLConfigPath, ReloadFake: true}
	schemas, err := schemastore.New(cfg.SchemaDir)
	if err != nil {
		t.Fatalf("schemastore: %v", err)
	}
	srv := api.New(cfg, reg, zp, minter, aclR, schemas, nil, nil)
	ts := httptest.NewServer(srv.Routes())
	t.Cleanup(ts.Close)
	return ts, reg
}

func TestMachineEnroll_PodMoonExactAutoGroupsAndInvalidCompositionsDoNotMint(t *testing.T) {
	setupKeyRequests := make(chan []string, 8)
	mux := http.NewServeMux()
	mux.HandleFunc("/api/groups", func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewEncoder(w).Encode([]map[string]string{
			{"id": "gid-pod", "name": "GALAXY-POD"},
			{"id": "gid-moon", "name": "GALAXY-MOON"},
			{"id": "gid-kid", "name": "GALAXY-KID"},
		})
	})
	mux.HandleFunc("/api/setup-keys", func(w http.ResponseWriter, r *http.Request) {
		var body struct {
			AutoGroups []string `json:"auto_groups"`
		}
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Errorf("decode setup-key request: %v", err)
			http.Error(w, "bad request", http.StatusBadRequest)
			return
		}
		setupKeyRequests <- body.AutoGroups
		w.WriteHeader(http.StatusCreated)
		_ = json.NewEncoder(w).Encode(map[string]string{"id": "key", "key": "KEY"})
	})
	mgmt := httptest.NewServer(mux)
	t.Cleanup(mgmt.Close)

	ts, reg := newMeshMintServer(t, mgmt.URL)
	ctx := t.Context()
	_ = reg.SetMachineGrant(ctx, "boundary-op", 5, "unit-b", "", "op")
	c := &http.Client{}

	for _, tc := range []struct {
		class   string
		groupID string
	}{
		{class: "POD", groupID: "gid-pod"},
		{class: "MOON", groupID: "gid-moon"},
	} {
		r := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
			"machine_handle": strings.ToLower(tc.class) + "-1", "node_class": tc.class,
			"autonomy": "remote", "custodian": "boundary-op",
		})
		if r.StatusCode != http.StatusCreated {
			t.Fatalf("standalone %s: status=%d body=%s", tc.class, r.StatusCode, mustReadBody(t, r))
		}
		got := <-setupKeyRequests
		if len(got) != 1 || got[0] != tc.groupID {
			t.Fatalf("standalone %s auto_groups=%v, want exactly [%s]", tc.class, got, tc.groupID)
		}
	}

	_, assignedBefore, err := reg.SlotPoolCounts(ctx)
	if err != nil {
		t.Fatalf("SlotPoolCounts before invalid requests: %v", err)
	}
	for i, workloads := range [][]string{{"POD"}, {"MOON"}, {"MOON", "POD"}} {
		r := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
			"machine_handle": fmt.Sprintf("platform-%d", i), "node_class": "PLATFORM",
			"autonomy": "remote", "custodian": "boundary-op", "workloads": workloads,
		})
		status := r.StatusCode
		code := errCode(t, r)
		if status != http.StatusBadRequest || code != "invalid_workloads" {
			t.Fatalf("PLATFORM/%v: status=%d code=%s, want 400 invalid_workloads", workloads, status, code)
		}
	}
	_, assignedAfter, err := reg.SlotPoolCounts(ctx)
	if err != nil {
		t.Fatalf("SlotPoolCounts after invalid requests: %v", err)
	}
	if assignedAfter != assignedBefore {
		t.Fatalf("invalid compositions allocated slots: before=%d after=%d", assignedBefore, assignedAfter)
	}
	q, _ := reg.MachineQuotaFor(ctx, "boundary-op")
	if q.Used != 2 || q.Remaining != 3 {
		t.Fatalf("invalid compositions changed quota after two valid enrollments: %+v", q)
	}
	if got := len(setupKeyRequests); got != 0 {
		t.Fatalf("invalid compositions minted %d setup keys", got)
	}
}

func TestMachineMeshKey_RemintIsQuotaNeutral(t *testing.T) {
	mgmt := fakeMgmt(t, "GALAXY-SENSOR-FEED", "GALAXY-KID")
	ts, reg := newMeshMintServer(t, mgmt.URL)
	ctx := t.Context()
	_ = reg.SetMachineGrant(ctx, "alice", 1, "usareur-af", "", "op")
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "alice-feed-1", "node_class": "SENSOR-FEED", "autonomy": "supervised", "custodian": "alice",
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("enroll: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	b := mustReadJSON(t, resp)
	anchor, _ := b["slot_id"].(string)
	firstKey, _ := b["mesh_setup_key"].(string)
	if firstKey == "" {
		t.Fatal("mint-enabled enroll returned no mesh_setup_key")
	}

	// Join window lapsed; the custodian re-mints. Quota must not move.
	re := doJSON(t, c, "POST", ts.URL+"/api/machines/"+anchor+"/mesh-key", "dev-operator-key", nil)
	if re.StatusCode != http.StatusOK {
		t.Fatalf("re-mint: status=%d body=%s", re.StatusCode, mustReadBody(t, re))
	}
	rb := mustReadJSON(t, re)
	newKey, _ := rb["mesh_setup_key"].(string)
	if newKey == "" || newKey == firstKey {
		t.Fatalf("re-mint key = %q (first %q); want a fresh non-empty key", newKey, firstKey)
	}
	mg, _ := rb["mesh_groups"].([]any)
	if len(mg) != 1 || mg[0] != "GALAXY-SENSOR-FEED" {
		t.Fatalf("re-mint mesh_groups = %v, want the born-as derivation [GALAXY-SENSOR-FEED]", rb["mesh_groups"])
	}
	q, _ := reg.MachineQuotaFor(ctx, "alice")
	if q.Used != 1 || q.Remaining != 0 {
		t.Fatalf("quota after re-mint = %+v; a re-mint is a join window, not a second machine", q)
	}
}

// An empty or whitespace-only body still means "defaults" now that the body
// goes through the shared reader (#1296): the decoder's io.EOF is not an
// error here.
func TestMachineMeshKey_WhitespaceBodyMeansDefaults(t *testing.T) {
	mgmt := fakeMgmt(t, "GALAXY-SENSOR-FEED", "GALAXY-KID")
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

	req, _ := http.NewRequest("POST", ts.URL+"/api/machines/"+anchor+"/mesh-key", strings.NewReader("  \n"))
	req.Header.Set("Authorization", "Bearer dev-operator-key")
	req.Header.Set("Content-Type", "application/json")
	re, err := c.Do(req)
	if err != nil {
		t.Fatalf("re-mint: %v", err)
	}
	if re.StatusCode != http.StatusOK {
		t.Fatalf("re-mint with a whitespace body: status=%d body=%s", re.StatusCode, mustReadBody(t, re))
	}
	re.Body.Close()
}

func TestMachineMeshKey_GateNotFoundAndLegacyClass(t *testing.T) {
	mgmt := fakeMgmt(t, "GALAXY-SENSOR-FEED")
	ts, reg := newMeshMintServer(t, mgmt.URL)
	ctx := t.Context()
	_ = reg.SetMachineGrant(ctx, "alice", 2, "usareur-af", "", "op")
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "alice-feed-2", "node_class": "SENSOR-FEED", "autonomy": "remote", "custodian": "alice",
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("enroll: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	anchor, _ := mustReadJSON(t, resp)["slot_id"].(string)

	// A different custodian's bearer may not re-mint alice's machine.
	_, malloryKey := onboardVendorBearer(t, ts, "mallory")
	forbidden := doJSON(t, c, "POST", ts.URL+"/api/machines/"+anchor+"/mesh-key", malloryKey, nil)
	if forbidden.StatusCode != http.StatusForbidden {
		t.Fatalf("cross-custodian re-mint: status=%d, want 403", forbidden.StatusCode)
	}

	// Unknown anchor → 404, not a guess.
	nf := doJSON(t, c, "POST", ts.URL+"/api/machines/no-such-anchor/mesh-key", "dev-operator-key", nil)
	if nf.StatusCode != http.StatusNotFound {
		t.Fatalf("unknown anchor: status=%d, want 404", nf.StatusCode)
	}

	// A legacy born-as class (whole-system SENSOR, retro-stamped) has no
	// derivation: the re-mint refuses and names the fix.
	slot, err := reg.AllocateSlot(ctx, "legacy-sensor")
	if err != nil {
		t.Fatalf("allocate: %v", err)
	}
	if err := reg.RecordMachineEnrollment(ctx, slot.SlotID, "alice", "SENSOR", "remote", "usareur-af", nil); err != nil {
		t.Fatalf("record legacy: %v", err)
	}
	legacy := doJSON(t, c, "POST", ts.URL+"/api/machines/"+slot.SlotID+"/mesh-key", "dev-operator-key", nil)
	if legacy.StatusCode != http.StatusConflict {
		t.Fatalf("legacy-class re-mint: status=%d body=%s, want 409", legacy.StatusCode, mustReadBody(t, legacy))
	}
	if code := errCode(t, legacy); code != "no_mesh_mapping" {
		t.Fatalf("legacy-class code = %q, want no_mesh_mapping", code)
	}
}

func TestBulkEnroll_OneActTwoMachines(t *testing.T) {
	mgmt := fakeMgmt(t, "GALAXY-SENSOR-FEED", "GALAXY-KID")
	ts, reg := newMeshMintServer(t, mgmt.URL)
	ctx := t.Context()
	_ = reg.SetMachineGrant(ctx, "bob", 2, "unit-b", "", "op")
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll-bulk", "dev-operator-key", map[string]any{
		"custodian": "bob",
		"machines": []map[string]any{
			{"machine_handle": "bob-feed-1", "node_class": "SENSOR-FEED", "autonomy": "supervised"},
			{"machine_handle": "bob-kid-1", "node_class": "KID", "autonomy": "remote"},
		},
	})
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("bulk: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	b := mustReadJSON(t, resp)
	if m, _ := b["minted"].(float64); m != 2 {
		t.Fatalf("minted=%v, want 2 (body %v)", b["minted"], b)
	}
	if f, _ := b["failed"].(float64); f != 0 {
		t.Fatalf("failed=%v, want 0", b["failed"])
	}
	entries, _ := b["entries"].([]any)
	keys := map[string]bool{}
	for _, e := range entries {
		em, _ := e.(map[string]any)
		res, _ := em["result"].(map[string]any)
		k, _ := res["mesh_setup_key"].(string)
		if k == "" {
			t.Fatalf("entry %v missing mesh_setup_key", em["machine_handle"])
		}
		keys[k] = true
	}
	// N single-use keys, never one key handed to N boxes.
	if len(keys) != 2 {
		t.Fatalf("expected 2 distinct setup keys, got %d", len(keys))
	}
	q, _ := reg.MachineQuotaFor(ctx, "bob")
	if q.Used != 2 || q.Remaining != 0 {
		t.Fatalf("quota after bulk = %+v, want used 2 remaining 0", q)
	}
}

func TestBulkEnroll_QuotaCeilingPartialFailure(t *testing.T) {
	mgmt := fakeMgmt(t, "GALAXY-SENSOR-FEED")
	ts, reg := newMeshMintServer(t, mgmt.URL)
	_ = reg.SetMachineGrant(t.Context(), "carol", 1, "unit-c", "", "op")
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll-bulk", "dev-operator-key", map[string]any{
		"custodian": "carol",
		"machines": []map[string]any{
			{"machine_handle": "carol-1", "node_class": "SENSOR-FEED", "autonomy": "manual"},
			{"machine_handle": "carol-2", "node_class": "SENSOR-FEED", "autonomy": "manual"},
		},
	})
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("bulk: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	b := mustReadJSON(t, resp)
	if m, _ := b["minted"].(float64); m != 1 {
		t.Fatalf("minted=%v, want 1", b["minted"])
	}
	if f, _ := b["failed"].(float64); f != 1 {
		t.Fatalf("failed=%v, want 1", b["failed"])
	}
	entries, _ := b["entries"].([]any)
	second, _ := entries[1].(map[string]any)
	if second["error_code"] != "quota_exhausted" {
		t.Fatalf("second entry error_code = %v, want quota_exhausted", second["error_code"])
	}
}

func TestBulkEnroll_InfraEntryStaysOperatorGated(t *testing.T) {
	mgmt := fakeMgmt(t, "GALAXY-SENSOR-FEED", "GALAXY-INFRA")
	ts, reg := newMeshMintServer(t, mgmt.URL)
	_ = reg.SetMachineGrant(t.Context(), "dave", 5, "unit-d", "", "op")
	c := &http.Client{}
	_, daveKey := onboardVendorBearer(t, ts, "dave")

	// A self-service caller smuggling an infra class inside a bulk act must be
	// refused whole — the act rides the strictest gate among its entries.
	resp := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll-bulk", daveKey, map[string]any{
		"machines": []map[string]any{
			{"machine_handle": "dave-feed-1", "node_class": "SENSOR-FEED", "autonomy": "manual"},
			{"machine_handle": "dave-mgmt-1", "node_class": "MGMT-HOST", "autonomy": "manual"},
		},
	})
	if resp.StatusCode != http.StatusForbidden {
		t.Fatalf("bulk with infra entry from non-operator: status=%d body=%s, want 403", resp.StatusCode, mustReadBody(t, resp))
	}
}

func TestEnroll_ExerciseScopeMintsEXGroups(t *testing.T) {
	// The exercise scope only exists where the exercise declared its gear
	// groups on the live mgmt server — EX-DRILL1-SENSOR-FEED is present,
	// drill2's is NOT, so drill1 mints and drill2 fail-closes.
	mgmt := fakeMgmt(t, "GALAXY-SENSOR-FEED", "EX-DRILL1-SENSOR-FEED")
	ts, reg := newMeshMintServer(t, mgmt.URL)
	_ = reg.SetMachineGrant(t.Context(), "erin", 3, "unit-e", "", "op")
	c := &http.Client{}

	resp := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "erin-ex-feed", "node_class": "SENSOR-FEED", "autonomy": "supervised",
		"custodian": "erin", "exercise": "drill1",
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("exercise enroll: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	b := mustReadJSON(t, resp)
	mg, _ := b["mesh_groups"].([]any)
	if len(mg) != 1 || mg[0] != "EX-DRILL1-SENSOR-FEED" {
		t.Fatalf("mesh_groups = %v, want [EX-DRILL1-SENSOR-FEED]", b["mesh_groups"])
	}
	if b["mesh_scope"] != "drill1" {
		t.Fatalf("mesh_scope = %v, want drill1", b["mesh_scope"])
	}
	if k, _ := b["mesh_setup_key"].(string); k == "" {
		t.Fatal("exercise enroll returned no mesh_setup_key")
	}

	// Unknown exercise: refused with the known names, before any allocation.
	unk := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "erin-ex-2", "node_class": "SENSOR-FEED", "autonomy": "supervised",
		"custodian": "erin", "exercise": "nosuch",
	})
	if unk.StatusCode != http.StatusBadRequest || errCode(t, unk) != "unknown_exercise" {
		t.Fatalf("unknown exercise: status=%d code=%s, want 400 unknown_exercise", unk.StatusCode, errCode(t, unk))
	}

	// Known exercise, gear groups not declared on the mgmt server: the whole
	// enrollment fail-closes and quota is returned.
	fc := doJSON(t, c, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": "erin-ex-3", "node_class": "SENSOR-FEED", "autonomy": "supervised",
		"custodian": "erin", "exercise": "drill2",
	})
	if fc.StatusCode != http.StatusBadGateway {
		t.Fatalf("undeclared exercise groups: status=%d body=%s, want 502", fc.StatusCode, mustReadBody(t, fc))
	}
	q, _ := reg.MachineQuotaFor(t.Context(), "erin")
	if q.Used != 1 {
		t.Fatalf("quota used = %d after one success + one rollback, want 1", q.Used)
	}
}
