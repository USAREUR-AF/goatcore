package measure

import (
	"encoding/json"
	"os"
	"testing"

	"github.com/usareur-af/goatcore/ops/snitch/internal/types"
)

func TestParseNetbirdStatus(t *testing.T) {
	data, err := os.ReadFile("testdata/netbird-status.json")
	if err != nil {
		t.Fatalf("read fixture: %v", err)
	}

	var status types.NetbirdStatusJSON
	if err := json.Unmarshal(data, &status); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}

	// Local peer
	name, ip := LocalPeerFromStatus(&status)
	if ip != "100.127.0.1" {
		t.Errorf("local IP = %q, want 100.127.0.1", ip)
	}
	if name != "example.netbird.selfhosted" {
		t.Errorf("local name = %q, want example.netbird.selfhosted", name)
	}

	// Peers
	if status.Peers.Total != 27 {
		t.Errorf("peers.total = %d, want 27", status.Peers.Total)
	}
	if status.Peers.Connected != 6 {
		t.Errorf("peers.connected = %d, want 6", status.Peers.Connected)
	}

	peers := PeersFromStatus(&status)
	if len(peers) != 27 {
		t.Fatalf("PeersFromStatus returned %d peers, want 27", len(peers))
	}

	// Check a known connected P2P peer
	var found bool
	for _, p := range peers {
		if p.IP == "100.127.0.15" { // peer-05
			found = true
			if !p.Connected {
				t.Error("peer-05 should be connected")
			}
			if !p.Direct {
				t.Error("peer-05 should be direct (P2P)")
			}
			if p.ICELocal != "srflx" {
				t.Errorf("peer-05 ICE local = %q, want srflx", p.ICELocal)
			}
		}
	}
	if !found {
		t.Error("peer-05 (100.127.0.15) not found in peers")
	}

	// Check a relayed peer
	for _, p := range peers {
		if p.IP == "100.127.0.27" { // peer-17
			if !p.Connected {
				t.Error("dhcp peer should be connected")
			}
			if p.Direct {
				t.Error("dhcp peer should be relayed, not direct")
			}
		}
	}

	// Management / Signal
	if !status.Management.Connected {
		t.Error("management should be connected")
	}
	if !status.Signal.Connected {
		t.Error("signal should be connected")
	}

	// Relays
	if status.Relays.Total != 4 {
		t.Errorf("relays.total = %d, want 4", status.Relays.Total)
	}
	if status.Relays.Available != 4 {
		t.Errorf("relays.available = %d, want 4", status.Relays.Available)
	}
}
