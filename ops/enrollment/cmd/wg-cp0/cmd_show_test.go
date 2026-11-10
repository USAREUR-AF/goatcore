// cmd_show_test.go — tests for the show command's live-state path.

package main

import (
	"bytes"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/peerset"
)

func nowForTest() time.Time {
	return time.Date(2026, 5, 14, 22, 0, 0, 0, time.UTC)
}

func TestEmitPeer_TextWithLiveState(t *testing.T) {
	p := &peerset.Peer{Name: "alpha", WGCP0Address: "198.18.0.10", CurrentPubkey: "PK_ALPHA"}
	rows := []showLiveRow{
		{Relay: "relay-rla", Reachable: true, PubkeyPresent: true},
		{Relay: "relay-rlb", Reachable: false, Error: "dial timeout"},
		{Relay: "relay-rlc", Reachable: true, PubkeyPresent: false},
	}
	var buf bytes.Buffer
	code := emitPeer(p, "active", false, rows, &buf)
	if code != 0 {
		t.Fatalf("code = %d", code)
	}
	out := buf.String()
	for _, want := range []string{"relay-rla", "PRESENT", "UNREACHABLE", "dial timeout", "ABSENT"} {
		if !strings.Contains(out, want) {
			t.Errorf("missing %q in %q", want, out)
		}
	}
}

func TestEmitPeer_JSONWithLiveState(t *testing.T) {
	p := &peerset.Peer{Name: "alpha", WGCP0Address: "198.18.0.10", CurrentPubkey: "PK_ALPHA"}
	rows := []showLiveRow{
		{Relay: "relay-rla", Reachable: true, PubkeyPresent: true},
	}
	var buf bytes.Buffer
	code := emitPeer(p, "active", true, rows, &buf)
	if code != 0 {
		t.Fatalf("code = %d", code)
	}
	if !strings.Contains(buf.String(), "live_state") {
		t.Errorf("JSON output should contain live_state field: %q", buf.String())
	}
}

func TestEmitPeer_NoLiveState_TextStaysCompact(t *testing.T) {
	p := &peerset.Peer{Name: "alpha", WGCP0Address: "198.18.0.10", CurrentPubkey: "PK_ALPHA"}
	var buf bytes.Buffer
	code := emitPeer(p, "active", false, nil, &buf)
	if code != 0 {
		t.Fatalf("code = %d", code)
	}
	if strings.Contains(buf.String(), "live_state:") {
		t.Errorf("live_state header should not appear when no rows supplied")
	}
}

func TestRun_Show_IncludeLiveState_OfflineMode_OK(t *testing.T) {
	withOffline(t, func() {
		code, _, _ := runArgs(t, "show", "lab-a", "--path", fixturePath, "--include-live-state")
		// In offline mode, live state is empty; show still succeeds
		// and prints the peer row. Live-state section may be absent or
		// empty.
		if code != 0 {
			t.Errorf("code = %d, want 0", code)
		}
	})
}

func TestRun_Validate_IncludeLiveState_OK(t *testing.T) {
	withOffline(t, func() {
		code, stdout, _ := runArgs(t, "validate", "--path", fixturePath, "--include-live-state")
		if code != 0 {
			t.Errorf("code = %d, want 0", code)
		}
		if !strings.Contains(stdout, "PASS") {
			t.Errorf("PASS missing: %q", stdout)
		}
	})
}

func TestRun_List_IncludeLiveState_OfflineMode(t *testing.T) {
	withOffline(t, func() {
		code, stdout, _ := runArgs(t, "list", "--path", fixturePath, "--include-live-state")
		if code != 0 {
			t.Errorf("code = %d", code)
		}
		if !strings.Contains(stdout, "LAST_HANDSHAKE") {
			t.Errorf("LAST_HANDSHAKE column missing: %q", stdout)
		}
	})
}

func TestRun_List_IncludeLiveState_JSON(t *testing.T) {
	withOffline(t, func() {
		code, stdout, _ := runArgs(t, "list", "--path", fixturePath, "--include-live-state", "--json")
		if code != 0 {
			t.Errorf("code = %d", code)
		}
		if !strings.Contains(stdout, "peers") {
			t.Errorf("peers key missing: %q", stdout)
		}
	})
}

func TestCompareDeclaredVsLive_Drift(t *testing.T) {
	ps := &peerset.PeerSet{
		SchemaVersion: peerset.SchemaVersion,
		Peers: []peerset.Peer{
			{Name: "alpha", WGCP0Address: "198.18.0.10", CurrentPubkey: "PK_ALPHA"},
			{Name: "bravo", WGCP0Address: "198.18.0.11", CurrentPubkey: "PK_BRAVO_NEW",
				PubkeyHistory: []peerset.PubkeyHistoryEntry{{Pubkey: "PK_BRAVO_OLD"}}},
		},
		Relays: []peerset.Relay{{Name: "relay-rla"}},
	}
	state := LiveState{PerRelay: map[string]*RelayLiveState{
		"relay-rla": {Reachable: true, Peers: map[string]LivePeer{
			"PK_BRAVO_OLD": {Pubkey: "PK_BRAVO_OLD"}, // rotated-away, still present
			"PK_ROGUE":     {Pubkey: "PK_ROGUE"},     // unknown
			// alpha is declared but absent
		}},
	}}
	rows := compareDeclaredVsLive(ps, state)
	// Expect 4 rows: alpha DECLARED-absent, bravo DECLARED-absent
	// (new pubkey not on relay), bravo ROTATED-AWAY (old pubkey
	// still present), and an UNKNOWN row for PK_ROGUE.
	if len(rows) != 4 {
		t.Fatalf("rows = %d (%v), want 4", len(rows), rows)
	}
	joined := strings.Join(rows, "\n")
	for _, want := range []string{"DECLARED", "ROTATED-AWAY", "UNKNOWN"} {
		if !strings.Contains(joined, want) {
			t.Errorf("missing %q in %v", want, rows)
		}
	}
}

func TestReadFreshestHandshakes_WithRelays(t *testing.T) {
	// Custom peerset with relays so readFreshestHandshakes enters
	// the per-relay loop. Offline mode means the live reader returns
	// an empty snapshot; freshest is therefore empty (every peer is
	// "absent on every reachable relay").
	ps := twoPeerSet()
	dir := t.TempDir()
	path := savePeerSetForTest(t, ps, dir)
	withOffline(t, func() {
		code, _, _ := runArgs(t, "list", "--path", path, "--include-live-state")
		if code != 0 {
			t.Errorf("code = %d", code)
		}
	})
}

func TestRun_List_JSONWithIncludeDeprecated(t *testing.T) {
	// Build a peerset with both active + deprecated peers and exercise
	// the JSON + --include-deprecated combination.
	ps := twoPeerSet()
	ps.DeprecatedPeers = []peerset.DeprecatedPeer{
		{Peer: peerset.Peer{Name: "old", WGCP0Address: "198.18.0.99", CurrentPubkey: "PK_OLD"}, RemovedAt: nowForTest(), RemovedReason: "decom"},
	}
	dir := t.TempDir()
	path := savePeerSetForTest(t, ps, dir)
	withOffline(t, func() {
		code, stdout, _ := runArgs(t, "list", "--path", path, "--json", "--include-deprecated")
		if code != 0 {
			t.Errorf("code = %d", code)
		}
		if !strings.Contains(stdout, "deprecated_peers") || !strings.Contains(stdout, "old") {
			t.Errorf("deprecated section missing: %q", stdout)
		}
	})
}

func TestRun_List_TextWithIncludeDeprecated(t *testing.T) {
	ps := twoPeerSet()
	ps.DeprecatedPeers = []peerset.DeprecatedPeer{
		{Peer: peerset.Peer{Name: "old", WGCP0Address: "198.18.0.99", CurrentPubkey: "PK_OLD"}, RemovedAt: nowForTest(), RemovedReason: "decom"},
	}
	dir := t.TempDir()
	path := savePeerSetForTest(t, ps, dir)
	withOffline(t, func() {
		code, stdout, _ := runArgs(t, "list", "--path", path, "--include-deprecated")
		if code != 0 {
			t.Errorf("code = %d", code)
		}
		if !strings.Contains(stdout, "DEPRECATED PEERS") {
			t.Errorf("DEPRECATED PEERS header missing: %q", stdout)
		}
	})
}

func TestRun_Show_JSONDeprecated(t *testing.T) {
	ps := twoPeerSet()
	ps.DeprecatedPeers = []peerset.DeprecatedPeer{
		{Peer: peerset.Peer{Name: "old", WGCP0Address: "198.18.0.99", CurrentPubkey: "PK_OLD"}, RemovedAt: nowForTest(), RemovedReason: "decom"},
	}
	dir := t.TempDir()
	path := savePeerSetForTest(t, ps, dir)
	withOffline(t, func() {
		code, stdout, _ := runArgs(t, "show", "old", "--path", path, "--json")
		if code != 0 {
			t.Errorf("code = %d", code)
		}
		if !strings.Contains(stdout, "deprecated") {
			t.Errorf("status missing: %q", stdout)
		}
	})
}

func TestReadFreshestHandshakes_NilReader(t *testing.T) {
	got := readFreshestHandshakes(twoPeerSet(), nil)
	if got != nil {
		t.Errorf("nil reader should return nil; got %v", got)
	}
}

func TestReadFreshestHandshakes_ReadError(t *testing.T) {
	reader := &MockLiveStateReader{Err: errMockRead}
	got := readFreshestHandshakes(twoPeerSet(), reader)
	if got != nil {
		t.Errorf("read error should return nil; got %v", got)
	}
}

func TestReadFreshestHandshakes_PicksFreshestAcrossRelays(t *testing.T) {
	ps := twoPeerSet()
	oldTS := time.Date(2026, 5, 14, 22, 0, 0, 0, time.UTC)
	newTS := time.Date(2026, 5, 14, 22, 5, 0, 0, time.UTC)
	reader := &MockLiveStateReader{State: LiveState{PerRelay: map[string]*RelayLiveState{
		"relay-rla": {Reachable: true, Peers: map[string]LivePeer{
			"PK_ALPHA": {Pubkey: "PK_ALPHA", LastHandshake: oldTS},
		}},
		"relay-rlb": {Reachable: true, Peers: map[string]LivePeer{
			"PK_ALPHA": {Pubkey: "PK_ALPHA", LastHandshake: newTS}, // fresher
			"PK_BRAVO": {Pubkey: "PK_BRAVO", LastHandshake: oldTS},
		}},
	}}}
	got := readFreshestHandshakes(ps, reader)
	if got["PK_ALPHA"] != newTS {
		t.Errorf("PK_ALPHA = %v, want %v (fresher of two relays)", got["PK_ALPHA"], newTS)
	}
	if got["PK_BRAVO"] != oldTS {
		t.Errorf("PK_BRAVO = %v, want %v", got["PK_BRAVO"], oldTS)
	}
}

func TestReadPeerLiveState_NilReader(t *testing.T) {
	got := readPeerLiveState(twoPeerSet(), "PK_X", nil)
	if got != nil {
		t.Errorf("nil reader should return nil; got %v", got)
	}
}

func TestReadPeerLiveState_PerRelayBreakdown(t *testing.T) {
	ps := twoPeerSet()
	reader := &MockLiveStateReader{State: LiveState{PerRelay: map[string]*RelayLiveState{
		"relay-rla": {Reachable: true, Peers: map[string]LivePeer{"PK_X": {Pubkey: "PK_X", LastHandshake: time.Now()}}},
		"relay-rlb": {Reachable: true, Peers: map[string]LivePeer{}}, // peer absent
	}}}
	got := readPeerLiveState(ps, "PK_X", reader)
	if len(got) != 2 {
		t.Fatalf("rows = %d, want 2", len(got))
	}
	// Sorted by relay name: relay-rla first.
	if got[0].Relay != "relay-rla" || !got[0].PubkeyPresent {
		t.Errorf("got[0] = %+v", got[0])
	}
	if got[1].Relay != "relay-rlb" || got[1].PubkeyPresent {
		t.Errorf("got[1] = %+v", got[1])
	}
}

func TestRunLiveStateCrossCheck_NilReader(t *testing.T) {
	got := runLiveStateCrossCheck(twoPeerSet(), nil, &bytes.Buffer{})
	if got != nil {
		t.Errorf("nil reader should return nil; got %v", got)
	}
}

func TestRunLiveStateCrossCheck_ReadError(t *testing.T) {
	var buf bytes.Buffer
	reader := &MockLiveStateReader{Err: errMockRead}
	got := runLiveStateCrossCheck(twoPeerSet(), reader, &buf)
	if got != nil {
		t.Errorf("read error should return nil; got %v", got)
	}
	if !strings.Contains(buf.String(), "live-state read error") {
		t.Errorf("expected error log; got %q", buf.String())
	}
}

func TestRunLiveStateCrossCheck_WithDrift(t *testing.T) {
	ps := &peerset.PeerSet{
		SchemaVersion: peerset.SchemaVersion,
		Peers: []peerset.Peer{
			{Name: "alpha", WGCP0Address: "198.18.0.10", CurrentPubkey: "PK_ALPHA"},
		},
		Relays: []peerset.Relay{{Name: "relay-rla"}},
	}
	reader := &MockLiveStateReader{State: LiveState{PerRelay: map[string]*RelayLiveState{
		"relay-rla": {Reachable: true, Peers: map[string]LivePeer{"PK_ROGUE": {Pubkey: "PK_ROGUE"}}},
	}}}
	rows := runLiveStateCrossCheck(ps, reader, &bytes.Buffer{})
	if len(rows) != 2 { // one DECLARED-absent + one UNKNOWN
		t.Errorf("rows = %d (%v), want 2", len(rows), rows)
	}
}

var errMockRead = readerErr("simulated read error")

type readerErr string

func (e readerErr) Error() string { return string(e) }

func TestReadPeerLiveState_WithRelays_OfflineModeEmpty(t *testing.T) {
	ps := twoPeerSet()
	dir := t.TempDir()
	path := savePeerSetForTest(t, ps, dir)
	withOffline(t, func() {
		code, stdout, _ := runArgs(t, "show", "alpha", "--path", path, "--include-live-state")
		if code != 0 {
			t.Errorf("code = %d", code)
		}
		// In offline mode the live snapshot is empty; readPeerLiveState
		// returns rows tagged "not in live-state snapshot".
		if !strings.Contains(stdout, "live_state:") {
			t.Errorf("live_state header missing: %q", stdout)
		}
	})
}

func TestRun_Show_Deprecated(t *testing.T) {
	// Build a fixture with a deprecated peer so the show command's
	// deprecated branch is exercised.
	ps := twoPeerSet()
	dir := t.TempDir()
	path := savePeerSetForTest(t, ps, dir)
	withOffline(t, func() {
		// First remove a peer so it lands in deprecated_peers[].
		// We do that by editing the file directly — the cmd-level
		// remove subcommand is owned by Session B.
		ps2 := twoPeerSet()
		ps2.DeprecatedPeers = []peerset.DeprecatedPeer{
			{Peer: ps2.Peers[0], RemovedAt: nowForTest(), RemovedReason: "test"},
		}
		ps2.Peers = ps2.Peers[1:]
		if err := ps2.Save(path); err != nil {
			t.Fatalf("save: %v", err)
		}
		code, stdout, _ := runArgs(t, "show", "alpha", "--path", path)
		if code != 0 {
			t.Fatalf("code = %d", code)
		}
		if !strings.Contains(stdout, "deprecated") {
			t.Errorf("deprecated status missing: %q", stdout)
		}
	})
	_ = path
}

func TestCompareDeclaredVsLive_NoDrift(t *testing.T) {
	ps := &peerset.PeerSet{
		SchemaVersion: peerset.SchemaVersion,
		Peers: []peerset.Peer{
			{Name: "alpha", WGCP0Address: "198.18.0.10", CurrentPubkey: "PK_ALPHA"},
		},
		Relays: []peerset.Relay{{Name: "relay-rla"}},
	}
	state := LiveState{PerRelay: map[string]*RelayLiveState{
		"relay-rla": {Reachable: true, Peers: map[string]LivePeer{"PK_ALPHA": {Pubkey: "PK_ALPHA"}}},
	}}
	rows := compareDeclaredVsLive(ps, state)
	if len(rows) != 0 {
		t.Errorf("drift rows = %d, want 0", len(rows))
	}
}
