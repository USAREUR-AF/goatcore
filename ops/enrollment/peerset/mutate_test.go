package peerset

import (
	"errors"
	"testing"
	"time"
)

func samplePeer(name, pubkey, addr string) Peer {
	return Peer{Name: name, CurrentPubkey: pubkey, WGCP0Address: addr, Role: "mgmt-host", Site: "lab"}
}

func TestUpsert_InsertsNewPeerAndStampsMetadata(t *testing.T) {
	t.Cleanup(fixedClock(time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC)))
	ps := emptyV2()
	if err := ps.Upsert(samplePeer("lab-a", "PUB1", "198.18.0.1")); err != nil {
		t.Fatalf("Upsert: %v", err)
	}
	got, ok := ps.Lookup("lab-a")
	if !ok || got.CurrentPubkey != "PUB1" {
		t.Fatalf("did not insert: %+v", ps.Peers)
	}
	if got.Metadata.CreatedAt.IsZero() || got.Metadata.LastUpdatedAt.IsZero() {
		t.Fatalf("metadata not stamped: %+v", got.Metadata)
	}
	if ps.Metadata.LastModifiedVia == "" {
		t.Fatalf("envelope metadata not stamped")
	}
}

func TestUpsert_ReplacesByName(t *testing.T) {
	t.Cleanup(fixedClock(time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC)))
	ps := emptyV2()
	_ = ps.Upsert(samplePeer("lab-a", "PUB1", "198.18.0.1"))
	if err := ps.Upsert(samplePeer("lab-a", "PUB2", "198.18.0.1")); err != nil {
		t.Fatalf("re-upsert: %v", err)
	}
	if len(ps.Peers) != 1 {
		t.Fatalf("count = %d, want 1", len(ps.Peers))
	}
	if ps.Peers[0].CurrentPubkey != "PUB2" {
		t.Fatalf("pubkey not replaced; have %s", ps.Peers[0].CurrentPubkey)
	}
}

func TestUpsert_EmptyNameReturnsErrDuplicateName(t *testing.T) {
	ps := emptyV2()
	err := ps.Upsert(Peer{Name: "", CurrentPubkey: "P", WGCP0Address: "A"})
	if !errors.Is(err, ErrDuplicateName) {
		t.Fatalf("err = %v, want ErrDuplicateName", err)
	}
}

func TestUpsert_MissingRequiredFieldsReturnsSchemaInvalid(t *testing.T) {
	ps := emptyV2()
	cases := []Peer{
		{Name: "n", CurrentPubkey: "", WGCP0Address: "A"},
		{Name: "n", CurrentPubkey: "P", WGCP0Address: ""},
	}
	for i, p := range cases {
		err := ps.Upsert(p)
		if !errors.Is(err, ErrSchemaInvalid) {
			t.Fatalf("case %d err = %v, want ErrSchemaInvalid", i, err)
		}
	}
}

func TestUpsert_RejectsPubkeyCollisionWithOtherPeer(t *testing.T) {
	ps := emptyV2()
	_ = ps.Upsert(samplePeer("a", "SHARED", "198.18.0.1"))
	err := ps.Upsert(samplePeer("b", "SHARED", "198.18.0.2"))
	if !errors.Is(err, ErrDuplicatePubkey) {
		t.Fatalf("err = %v, want ErrDuplicatePubkey", err)
	}
}

func TestUpsert_RejectsAddressCollisionWithOtherPeer(t *testing.T) {
	ps := emptyV2()
	_ = ps.Upsert(samplePeer("a", "PUBA", "198.18.0.1"))
	err := ps.Upsert(samplePeer("b", "PUBB", "198.18.0.1"))
	if !errors.Is(err, ErrDuplicateAddress) {
		t.Fatalf("err = %v, want ErrDuplicateAddress", err)
	}
}

func TestRotatePubkey_HistoryAppendedAndCurrentReplaced(t *testing.T) {
	t.Cleanup(fixedClock(time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC)))
	ps := emptyV2()
	_ = ps.Upsert(samplePeer("a", "OLD", "198.18.0.1"))
	if err := ps.RotatePubkey("a", "NEW", "compromise drill"); err != nil {
		t.Fatalf("Rotate: %v", err)
	}
	p, _ := ps.Lookup("a")
	if p.CurrentPubkey != "NEW" {
		t.Fatalf("current = %s, want NEW", p.CurrentPubkey)
	}
	if len(p.PubkeyHistory) != 1 || p.PubkeyHistory[0].Pubkey != "OLD" {
		t.Fatalf("history = %+v, want [OLD]", p.PubkeyHistory)
	}
	if p.PubkeyHistory[0].Reason != "compromise drill" {
		t.Fatalf("reason = %q, want 'compromise drill'", p.PubkeyHistory[0].Reason)
	}
}

func TestRotatePubkey_PeerNotFound(t *testing.T) {
	ps := emptyV2()
	err := ps.RotatePubkey("ghost", "NEW", "drill")
	if !errors.Is(err, ErrPeerNotFound) {
		t.Fatalf("err = %v, want ErrPeerNotFound", err)
	}
}

func TestRotatePubkey_RejectsCollisionWithOtherPeer(t *testing.T) {
	ps := emptyV2()
	_ = ps.Upsert(samplePeer("a", "PUBA", "198.18.0.1"))
	_ = ps.Upsert(samplePeer("b", "PUBB", "198.18.0.2"))
	err := ps.RotatePubkey("a", "PUBB", "drill")
	if !errors.Is(err, ErrDuplicatePubkey) {
		t.Fatalf("err = %v, want ErrDuplicatePubkey", err)
	}
}

func TestRotatePubkey_RejectsSameAsCurrent(t *testing.T) {
	ps := emptyV2()
	_ = ps.Upsert(samplePeer("a", "PUB", "198.18.0.1"))
	err := ps.RotatePubkey("a", "PUB", "no-op drill")
	if !errors.Is(err, ErrDuplicatePubkey) {
		t.Fatalf("err = %v, want ErrDuplicatePubkey", err)
	}
}

func TestRemove_MovesPeerToDeprecatedAndAllowsResurrect(t *testing.T) {
	t.Cleanup(fixedClock(time.Date(2026, 5, 14, 10, 0, 0, 0, time.UTC)))
	ps := emptyV2()
	_ = ps.Upsert(samplePeer("a", "PUB", "198.18.0.1"))
	if err := ps.Remove("a", "lab tear-down"); err != nil {
		t.Fatalf("Remove: %v", err)
	}
	if _, ok := ps.Lookup("a"); ok {
		t.Fatal("peer still active after Remove")
	}
	dep, ok := ps.LookupDeprecated("a")
	if !ok {
		t.Fatal("peer not in deprecated_peers after Remove")
	}
	if dep.RemovedReason != "lab tear-down" || dep.RemovedAt.IsZero() {
		t.Fatalf("removed metadata wrong: %+v", dep)
	}

	if err := ps.Resurrect("a"); err != nil {
		t.Fatalf("Resurrect: %v", err)
	}
	if _, ok := ps.Lookup("a"); !ok {
		t.Fatal("peer not restored")
	}
	if _, ok := ps.LookupDeprecated("a"); ok {
		t.Fatal("deprecated row not cleared")
	}
}

func TestRemove_PeerNotFound(t *testing.T) {
	ps := emptyV2()
	err := ps.Remove("ghost", "drill")
	if !errors.Is(err, ErrPeerNotFound) {
		t.Fatalf("err = %v, want ErrPeerNotFound", err)
	}
}

func TestResurrect_PeerNotFound(t *testing.T) {
	ps := emptyV2()
	err := ps.Resurrect("ghost")
	if !errors.Is(err, ErrPeerNotFound) {
		t.Fatalf("err = %v, want ErrPeerNotFound", err)
	}
}

func TestResurrect_RejectsCollisionWithActivePeer(t *testing.T) {
	ps := emptyV2()
	_ = ps.Upsert(samplePeer("a", "PUBA", "198.18.0.1"))
	_ = ps.Remove("a", "test")
	// Operator added a different peer with the same name in between.
	_ = ps.Upsert(samplePeer("a", "PUBB", "198.18.0.2"))
	err := ps.Resurrect("a")
	if !errors.Is(err, ErrDuplicateName) {
		t.Fatalf("err = %v, want ErrDuplicateName", err)
	}
}

func TestResurrect_RejectsPubkeyCollisionWithActivePeer(t *testing.T) {
	ps := emptyV2()
	_ = ps.Upsert(samplePeer("a", "PUBA", "198.18.0.1"))
	_ = ps.Remove("a", "test")
	// A different active peer now holds the same pubkey (somehow);
	// resurrect must fail loud rather than silently re-add a colliding row.
	_ = ps.Upsert(samplePeer("b", "PUBA", "198.18.0.2"))
	err := ps.Resurrect("a")
	if !errors.Is(err, ErrDuplicatePubkey) {
		t.Fatalf("err = %v, want ErrDuplicatePubkey", err)
	}
}

func TestResurrect_RejectsAddressCollisionWithActivePeer(t *testing.T) {
	ps := emptyV2()
	_ = ps.Upsert(samplePeer("a", "PUBA", "198.18.0.1"))
	_ = ps.Remove("a", "test")
	_ = ps.Upsert(samplePeer("b", "PUBB", "198.18.0.1"))
	err := ps.Resurrect("a")
	if !errors.Is(err, ErrDuplicateAddress) {
		t.Fatalf("err = %v, want ErrDuplicateAddress", err)
	}
}

func TestLookupByPubkey(t *testing.T) {
	ps := emptyV2()
	_ = ps.Upsert(samplePeer("a", "PUB1", "198.18.0.1"))
	got, ok := ps.LookupByPubkey("PUB1")
	if !ok || got.Name != "a" {
		t.Fatalf("got %+v, ok=%v", got, ok)
	}
	if _, ok := ps.LookupByPubkey("MISS"); ok {
		t.Fatal("phantom lookup succeeded")
	}
	if _, ok := ps.LookupByPubkey(""); ok {
		t.Fatal("empty pubkey matched")
	}
}

func TestLookupRelay(t *testing.T) {
	ps := emptyV2()
	ps.Relays = append(ps.Relays, Relay{Name: "relay-rla", Endpoint: "relay-rla.example:51821"})
	r, ok := ps.Relay("relay-rla")
	if !ok || r.Endpoint == "" {
		t.Fatalf("Relay lookup failed: %+v ok=%v", r, ok)
	}
	if _, ok := ps.Relay("absent"); ok {
		t.Fatal("phantom relay")
	}
}
