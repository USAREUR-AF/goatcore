package api_test

// Tests for the machine revoke leg (DELETE /api/machines/{anchor}) — the exit
// the mint never had. The cases that matter are the ORDER and the failure
// path: a revoke that reports success while the peer is still on the mesh is
// worse than one that refuses, because the registry then says retired.

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
)

// mgmtFleet is a fake netbird mgmt server that actually HOLDS state: minted
// setup keys and joined peers, both deletable. fakeMgmt (machine_mesh_bulk_test)
// only ever mints, which is enough for the forward direction and nothing for
// this one.
type mgmtFleet struct {
	mu sync.Mutex
	// keys/peers are id -> record.
	keys  map[string]map[string]any
	peers map[string]map[string]any
	// order records every mutating call in sequence, so a test can assert that
	// the key was shut before the peer was evicted.
	order []string
	// failDelete makes every DELETE return 500 — the mgmt-unreachable case.
	failDelete bool
	n          atomic.Int64
}

func newMgmtFleet(t *testing.T, groupNames ...string) (*httptest.Server, *mgmtFleet) {
	t.Helper()
	f := &mgmtFleet{keys: map[string]map[string]any{}, peers: map[string]map[string]any{}}
	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/groups", func(w http.ResponseWriter, r *http.Request) {
		out := make([]map[string]string, 0, len(groupNames))
		for i, name := range groupNames {
			out = append(out, map[string]string{"id": fmt.Sprintf("gid-%d", i), "name": name})
		}
		_ = json.NewEncoder(w).Encode(out)
	})
	mux.HandleFunc("POST /api/setup-keys", func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		_ = json.NewDecoder(r.Body).Decode(&body)
		id := fmt.Sprintf("key-%d", f.n.Add(1))
		f.mu.Lock()
		f.keys[id] = map[string]any{"id": id, "name": body["name"], "used_times": 0, "revoked": false}
		f.mu.Unlock()
		w.WriteHeader(http.StatusCreated)
		_ = json.NewEncoder(w).Encode(map[string]string{"id": id, "key": "KEY-" + id})
	})
	mux.HandleFunc("GET /api/setup-keys", func(w http.ResponseWriter, r *http.Request) {
		f.mu.Lock()
		defer f.mu.Unlock()
		out := make([]map[string]any, 0, len(f.keys))
		for _, k := range f.keys {
			out = append(out, k)
		}
		_ = json.NewEncoder(w).Encode(out)
	})
	mux.HandleFunc("DELETE /api/setup-keys/{id}", func(w http.ResponseWriter, r *http.Request) {
		f.mu.Lock()
		defer f.mu.Unlock()
		if f.failDelete {
			w.WriteHeader(http.StatusInternalServerError)
			return
		}
		f.order = append(f.order, "del-key:"+r.PathValue("id"))
		delete(f.keys, r.PathValue("id"))
		w.WriteHeader(http.StatusNoContent)
	})
	mux.HandleFunc("GET /api/peers", func(w http.ResponseWriter, r *http.Request) {
		f.mu.Lock()
		defer f.mu.Unlock()
		out := make([]map[string]any, 0, len(f.peers))
		for _, p := range f.peers {
			out = append(out, p)
		}
		_ = json.NewEncoder(w).Encode(out)
	})
	mux.HandleFunc("DELETE /api/peers/{id}", func(w http.ResponseWriter, r *http.Request) {
		f.mu.Lock()
		defer f.mu.Unlock()
		if f.failDelete {
			w.WriteHeader(http.StatusInternalServerError)
			return
		}
		f.order = append(f.order, "del-peer:"+r.PathValue("id"))
		delete(f.peers, r.PathValue("id"))
		w.WriteHeader(http.StatusOK)
	})
	ts := httptest.NewServer(mux)
	t.Cleanup(ts.Close)
	return ts, f
}

// join simulates a box coming up on a minted key: the peer registers under the
// machine handle (peer name == machine handle is the provenance join key) and
// the one-off key is spent.
func (f *mgmtFleet) join(peerName, keyID string) string {
	f.mu.Lock()
	defer f.mu.Unlock()
	id := fmt.Sprintf("peer-%d", f.n.Add(1))
	f.peers[id] = map[string]any{"id": id, "name": peerName}
	if k, ok := f.keys[keyID]; ok {
		k["used_times"] = 1
	}
	return id
}

func (f *mgmtFleet) counts() (keys, peers int) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return len(f.keys), len(f.peers)
}

func (f *mgmtFleet) callOrder() []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]string(nil), f.order...)
}

func enrollForRevoke(t *testing.T, ts *httptest.Server, handle, custodian string) (anchor string) {
	t.Helper()
	resp := doJSON(t, &http.Client{}, "POST", ts.URL+"/api/machines/enroll", "dev-operator-key", map[string]any{
		"machine_handle": handle, "node_class": "SENSOR-FEED", "autonomy": "remote", "custodian": custodian,
	})
	if resp.StatusCode != http.StatusCreated {
		t.Fatalf("enroll %s: status=%d body=%s", handle, resp.StatusCode, mustReadBody(t, resp))
	}
	anchor, _ = mustReadJSON(t, resp)["slot_id"].(string)
	return anchor
}

// The happy path for the population the endpoint was built for: a machine that
// was minted and never joined. The unconsumed key must not survive the machine
// — a standing invitation carrying gear groups is the exact shape ADR 1116 §9
// exists to eliminate.
func TestMachineRevoke_DeletesUnconsumedKeyAndReturnsQuota(t *testing.T) {
	mgmt, fleet := newMgmtFleet(t, "GALAXY-SENSOR-FEED")
	ts, reg := newMeshMintServer(t, mgmt.URL)
	ctx := t.Context()
	_ = reg.SetMachineGrant(ctx, "alice", 1, "usareur-af", "", "op")
	anchor := enrollForRevoke(t, ts, "mma-probe-1", "alice")

	if k, _ := fleet.counts(); k != 1 {
		t.Fatalf("after enroll the mgmt server holds %d keys, want 1", k)
	}
	q, _ := reg.MachineQuotaFor(ctx, "alice")
	if q.Remaining != 0 {
		t.Fatalf("quota before revoke = %+v, want 0 remaining", q)
	}

	resp := doJSON(t, &http.Client{}, "DELETE", ts.URL+"/api/machines/"+anchor, "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("revoke: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	b := mustReadJSON(t, resp)
	if b["mesh_cut"] != true {
		t.Fatalf("mesh_cut = %v, want true (the portal has a mgmt credential)", b["mesh_cut"])
	}
	if kd, _ := b["keys_deleted"].([]any); len(kd) != 1 {
		t.Fatalf("keys_deleted = %v, want exactly the one unconsumed key", b["keys_deleted"])
	}
	if k, p := fleet.counts(); k != 0 || p != 0 {
		t.Fatalf("mgmt server left with %d keys / %d peers, want 0/0", k, p)
	}

	// Quota returns BY CONSTRUCTION — nothing incremented a counter, the
	// derived count simply stopped seeing a live enrolled slot.
	q, _ = reg.MachineQuotaFor(ctx, "alice")
	if q.Used != 0 || q.Remaining != 1 {
		t.Fatalf("quota after revoke = %+v, want 0 used / 1 remaining", q)
	}
	if _, err := reg.MachineByAnchor(ctx, anchor); err == nil {
		t.Fatal("machine still readable after revoke")
	}
	// Re-enrolling the handle must work — the whole point of returning quota.
	if again := enrollForRevoke(t, ts, "mma-probe-1", "alice"); again == "" {
		t.Fatal("re-enroll after revoke produced no anchor")
	}
}

// A machine that actually joined: the peer is the cut that matters, and the
// spent key is left behind on purpose as the record of how the peer got there.
func TestMachineRevoke_CutsPeerFirstShutsKeyBeforeEvicting(t *testing.T) {
	mgmt, fleet := newMgmtFleet(t, "GALAXY-SENSOR-FEED", "GALAXY-KID")
	ts, reg := newMeshMintServer(t, mgmt.URL)
	ctx := t.Context()
	_ = reg.SetMachineGrant(ctx, "alice", 2, "usareur-af", "", "op")

	joined := enrollForRevoke(t, ts, "alice-kid-1", "alice")
	fleet.join("alice-kid-1", "key-1") // the box comes up; the one-off is spent

	// A second, still-unconsumed key for the same machine (the D1 re-mint
	// case: the first window lapsed, a fresh key was cut).
	re := doJSON(t, &http.Client{}, "POST", ts.URL+"/api/machines/"+joined+"/mesh-key", "dev-operator-key", nil)
	if re.StatusCode != http.StatusOK {
		t.Fatalf("re-mint: status=%d body=%s", re.StatusCode, mustReadBody(t, re))
	}

	resp := doJSON(t, &http.Client{}, "DELETE", ts.URL+"/api/machines/"+joined, "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("revoke: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	b := mustReadJSON(t, resp)
	if pd, _ := b["peers_deleted"].([]any); len(pd) != 1 {
		t.Fatalf("peers_deleted = %v, want the one joined peer", b["peers_deleted"])
	}
	if kd, _ := b["keys_deleted"].([]any); len(kd) != 1 {
		t.Fatalf("keys_deleted = %v, want the one UNCONSUMED key", b["keys_deleted"])
	}
	if ks, _ := b["keys_already_spent"].([]any); len(ks) != 1 {
		t.Fatalf("keys_already_spent = %v, want the spent key named, not deleted", b["keys_already_spent"])
	}

	// Order is the property: shut the door, then evict. Deleting the peer
	// first leaves a live single-use key a box could re-join on in the gap.
	order := fleet.callOrder()
	var firstPeer, lastKey = -1, -1
	for i, c := range order {
		if strings.HasPrefix(c, "del-peer:") && firstPeer < 0 {
			firstPeer = i
		}
		if strings.HasPrefix(c, "del-key:") {
			lastKey = i
		}
	}
	if firstPeer < 0 || lastKey < 0 || lastKey > firstPeer {
		t.Fatalf("call order %v — every setup-key delete must precede the first peer delete", order)
	}
}

// The failure that matters: the mgmt server refuses the cut. The registry must
// be exactly as it was, so a retry is a retry and not a second half-revoke.
func TestMachineRevoke_MgmtFailureLeavesRegistryUntouched(t *testing.T) {
	mgmt, fleet := newMgmtFleet(t, "GALAXY-SENSOR-FEED")
	ts, reg := newMeshMintServer(t, mgmt.URL)
	ctx := t.Context()
	_ = reg.SetMachineGrant(ctx, "alice", 1, "usareur-af", "", "op")
	anchor := enrollForRevoke(t, ts, "alice-feed-9", "alice")

	fleet.mu.Lock()
	fleet.failDelete = true
	fleet.mu.Unlock()

	resp := doJSON(t, &http.Client{}, "DELETE", ts.URL+"/api/machines/"+anchor, "dev-operator-key", nil)
	if resp.StatusCode != http.StatusBadGateway {
		t.Fatalf("revoke against a failing mgmt server: status=%d, want 502", resp.StatusCode)
	}
	if m, err := reg.MachineByAnchor(ctx, anchor); err != nil || m == nil {
		t.Fatalf("machine gone after a FAILED mesh cut (err=%v) — the registry must be untouched", err)
	}
	q, _ := reg.MachineQuotaFor(ctx, "alice")
	if q.Used != 1 {
		t.Fatalf("quota returned on a failed revoke: %+v", q)
	}
}

// A portal with no mgmt credential (report-only, or a goatnet that never wired
// the mint) must still be able to retire a registry row — enrollment works
// there, so the exit has to as well. What it must NOT do is imply a cut it did
// not make.
func TestMachineRevoke_NoMgmtCredentialSaysSoAndStillRetires(t *testing.T) {
	ts, reg := newSlotPoolServer(t, "")
	ctx := t.Context()
	_ = reg.SetMachineGrant(ctx, "alice", 1, "usareur-af", "", "op")
	anchor := enrollForRevoke(t, ts, "reportonly-feed-1", "alice")

	resp := doJSON(t, &http.Client{}, "DELETE", ts.URL+"/api/machines/"+anchor, "dev-operator-key", nil)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("revoke on a mint-off portal: status=%d body=%s", resp.StatusCode, mustReadBody(t, resp))
	}
	b := mustReadJSON(t, resp)
	if b["mesh_cut"] != false {
		t.Fatalf("mesh_cut = %v, want false — this portal cannot reach the mgmt server", b["mesh_cut"])
	}
	note, _ := b["mesh_note"].(string)
	if !strings.Contains(note, "by hand") || !strings.Contains(note, "reportonly-feed-1") {
		t.Fatalf("mesh_note = %q; it must name the manual step and the peer name", note)
	}
	if _, err := reg.MachineByAnchor(ctx, anchor); err == nil {
		t.Fatal("registry row survived a revoke on a mint-off portal")
	}
	q, _ := reg.MachineQuotaFor(ctx, "alice")
	if q.Remaining != 1 {
		t.Fatalf("quota after mint-off revoke = %+v, want 1 remaining", q)
	}
}

// Gate + selector. Same custody model as the re-mint, and the handle the
// enroll issued is accepted alongside the anchor.
func TestMachineRevoke_GateNotFoundAndHandleSelector(t *testing.T) {
	mgmt, _ := newMgmtFleet(t, "GALAXY-SENSOR-FEED")
	ts, reg := newMeshMintServer(t, mgmt.URL)
	ctx := t.Context()
	_ = reg.SetMachineGrant(ctx, "alice", 2, "usareur-af", "", "op")
	anchor := enrollForRevoke(t, ts, "alice-feed-3", "alice")
	c := &http.Client{}

	_, malloryKey := onboardVendorBearer(t, ts, "mallory")
	forbidden := doJSON(t, c, "DELETE", ts.URL+"/api/machines/"+anchor, malloryKey, nil)
	if forbidden.StatusCode != http.StatusForbidden {
		t.Fatalf("cross-custodian revoke: status=%d, want 403", forbidden.StatusCode)
	}

	nf := doJSON(t, c, "DELETE", ts.URL+"/api/machines/no-such-anchor", "dev-operator-key", nil)
	if nf.StatusCode != http.StatusNotFound {
		t.Fatalf("unknown anchor: status=%d, want 404", nf.StatusCode)
	}

	// By handle, not anchor.
	byHandle := doJSON(t, c, "DELETE", ts.URL+"/api/machines/alice-feed-3", "dev-operator-key", nil)
	if byHandle.StatusCode != http.StatusOK {
		t.Fatalf("revoke by handle: status=%d body=%s", byHandle.StatusCode, mustReadBody(t, byHandle))
	}
	if _, err := reg.MachineByAnchor(ctx, anchor); err == nil {
		t.Fatal("revoke by handle did not retire the machine")
	}

	// Second revoke of the same machine is a 404, not a silent success — the
	// caller should learn the first one already happened.
	again := doJSON(t, c, "DELETE", ts.URL+"/api/machines/"+anchor, "dev-operator-key", nil)
	if again.StatusCode != http.StatusNotFound {
		t.Fatalf("double revoke: status=%d, want 404", again.StatusCode)
	}
}
