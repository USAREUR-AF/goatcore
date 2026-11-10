package netbirdpoll_test

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/netbirdpoll"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/pgtest"
	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/registry"
)

func openReg(t *testing.T) *registry.Registry {
	t.Helper()
	r, err := registry.Open(pgtest.DSN(t))
	if err != nil {
		t.Fatalf("open: %v", err)
	}
	t.Cleanup(func() { _ = r.Close() })
	return r
}

func TestDisabledClientIsNoop(t *testing.T) {
	c := netbirdpoll.New("", "")
	if c.Enabled() {
		t.Fatalf("empty client should be disabled")
	}
	n, err := c.Poll(context.Background(), nil)
	if err != nil || n != 0 {
		t.Fatalf("disabled Poll should be no-op (got n=%d err=%v)", n, err)
	}
}

func TestPollMatchesPeerNamesAndUpdatesProgress(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/peers" {
			http.NotFound(w, r)
			return
		}
		if got := r.Header.Get("Authorization"); got != "Bearer tk-test" {
			http.Error(w, "bad bearer "+got, http.StatusUnauthorized)
			return
		}
		peers := []netbirdpoll.Peer{
			{ID: "p1", Name: "peer-aardwolf-lab-2026-05-21-laptop"},
			{ID: "p2", Name: "peer-aardwolf-lab-2026-05-21-phone"},
			{ID: "p3", Name: "peer-otter-lab-2026-05-22-thinkpad"},
			{ID: "p4", Name: "some-other-peer"},
			{ID: "p5", Name: "peer-unknown-lab-2026-05-21-x"},
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(peers)
	}))
	defer srv.Close()

	r := openReg(t)
	ctx := context.Background()
	for _, p := range []string{"aardwolf", "otter"} {
		if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: p, APIKeyHash: "h"}); err != nil {
			t.Fatalf("create %s: %v", p, err)
		}
	}

	c := netbirdpoll.New(srv.URL, "tk-test")
	if !c.Enabled() {
		t.Fatalf("client should be enabled")
	}
	n, err := c.Poll(ctx, r)
	if err != nil {
		t.Fatalf("poll: %v", err)
	}
	if n != 2 {
		t.Fatalf("expected 2 vendors updated, got %d", n)
	}

	rows, err := r.ListProgress(ctx, true)
	if err != nil {
		t.Fatalf("list: %v", err)
	}
	byPrefix := map[string]registry.VendorProgressRow{}
	for _, row := range rows {
		byPrefix[row.Prefix] = row
	}
	if got := byPrefix["aardwolf"]; got.PeerCount != 2 {
		t.Errorf("aardwolf peer_count = %d, want 2", got.PeerCount)
	}
	if got := byPrefix["aardwolf"]; got.L2At == nil {
		t.Errorf("aardwolf L2At should be set")
	}
	if got := byPrefix["otter"]; got.PeerCount != 1 {
		t.Errorf("otter peer_count = %d, want 1", got.PeerCount)
	}
}

func TestPollPrefersLongestVendorMatch(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewEncoder(w).Encode([]netbirdpoll.Peer{
			{ID: "p1", Name: "peer-foobar-lab-2026-05-21-host"},
		})
	}))
	defer srv.Close()

	r := openReg(t)
	ctx := context.Background()
	for _, p := range []string{"foo", "foobar"} {
		if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: p, APIKeyHash: "h"}); err != nil {
			t.Fatalf("create %s: %v", p, err)
		}
	}

	c := netbirdpoll.New(srv.URL, "tk")
	if _, err := c.Poll(ctx, r); err != nil {
		t.Fatalf("poll: %v", err)
	}
	rows, _ := r.ListProgress(ctx, true)
	byPrefix := map[string]int64{}
	for _, row := range rows {
		byPrefix[row.Prefix] = row.PeerCount
	}
	if byPrefix["foobar"] != 1 {
		t.Errorf("foobar should own the peer; got %d", byPrefix["foobar"])
	}
	if byPrefix["foo"] != 0 {
		t.Errorf("foo should NOT match peer-foobar-…; got %d", byPrefix["foo"])
	}
}

func TestPollUpstreamError(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, "boom", http.StatusInternalServerError)
	}))
	defer srv.Close()

	r := openReg(t)
	ctx := context.Background()

	c := netbirdpoll.New(srv.URL, "tk")
	if _, err := c.Poll(ctx, r); err == nil {
		t.Fatalf("expected error on 500")
	}
}

// A slot-pool goatnet keys its org rows by a 32-hex anchor, while the peer name
// carries the PERSON's handle and no anchor at all. Offering only anchors as
// candidate keys made L2 unreachable for every such org — the swimlane read
// "no peers" for people who were demonstrably enrolled (GALAXY
// cohort: three peers on the mesh, L2 blank on all three).
func TestPollMatchesHandleKeyedPeersOnSlotPoolGoatnet(t *testing.T) {
	r := openReg(t)
	ctx := context.Background()
	if _, err := r.InitializeSlotPool(ctx, "salt-test", 8); err != nil {
		t.Fatalf("init pool: %v", err)
	}
	slot, err := r.AllocateSlot(ctx, "river-s")
	if err != nil {
		t.Fatalf("allocate: %v", err)
	}
	// The org row is keyed by the anchor, exactly as onboarding creates it.
	if err := r.CreateVendor(ctx, &registry.Vendor{Prefix: slot.SlotID, APIKeyHash: "h"}); err != nil {
		t.Fatalf("create vendor: %v", err)
	}

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_ = json.NewEncoder(w).Encode([]netbirdpoll.Peer{
			{ID: "p1", Name: "peer-river-s-galaxy-2026-08-11-rhs-g14"},
			{ID: "p2", Name: "peer-someone-else-galaxy-2026-08-11-box"},
		})
	}))
	defer srv.Close()

	c := netbirdpoll.New(srv.URL, "tk")
	if _, err := c.Poll(ctx, r); err != nil {
		t.Fatalf("poll: %v", err)
	}
	rows, err := r.ListProgress(ctx, true)
	if err != nil {
		t.Fatalf("list progress: %v", err)
	}
	var found bool
	for _, row := range rows {
		if row.Prefix != slot.SlotID {
			continue
		}
		found = true
		if row.PeerCount != 1 {
			t.Errorf("handle-keyed peer should attribute to the anchor row; peer_count=%d", row.PeerCount)
		}
		if row.L2At == nil {
			t.Errorf("L2 must be stamped once a peer is attributed; got nil")
		}
	}
	if !found {
		t.Fatalf("no progress row for anchor %s", slot.SlotID)
	}
}

// A handle whose org row is gone (slot still assigned, vendor deleted) must not
// resurrect a row: MarkL2 would insert progress against a nonexistent vendor.
func TestPollSkipsHandleWithNoOrgRow(t *testing.T) {
	r := openReg(t)
	ctx := context.Background()
	if _, err := r.InitializeSlotPool(ctx, "salt-test", 4); err != nil {
		t.Fatalf("init pool: %v", err)
	}
	if _, err := r.AllocateSlot(ctx, "orphan-handle"); err != nil {
		t.Fatalf("allocate: %v", err)
	}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_ = json.NewEncoder(w).Encode([]netbirdpoll.Peer{
			{ID: "p1", Name: "peer-orphan-handle-galaxy-2026-08-11-box"},
		})
	}))
	defer srv.Close()

	c := netbirdpoll.New(srv.URL, "tk")
	if _, err := c.Poll(ctx, r); err != nil {
		t.Fatalf("poll: %v", err)
	}
	rows, err := r.ListProgress(ctx, true)
	if err != nil {
		t.Fatalf("list progress: %v", err)
	}
	if len(rows) != 0 {
		t.Errorf("expected no progress rows for an orphaned handle; got %d", len(rows))
	}
}
