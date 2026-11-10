package measure

import (
	"context"
	"encoding/json"
	"os"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/snitch/internal/types"
)

// stubStatus loads the fixture file and returns it as a StatusFunc.
func stubStatus(t *testing.T) StatusFunc {
	t.Helper()
	data, err := os.ReadFile("testdata/netbird-status.json")
	if err != nil {
		t.Fatalf("read fixture: %v", err)
	}
	var status types.NetbirdStatusJSON
	if err := json.Unmarshal(data, &status); err != nil {
		t.Fatalf("unmarshal fixture: %v", err)
	}
	return func(ctx context.Context) (*types.NetbirdStatusJSON, error) {
		return &status, nil
	}
}

// stubPing returns fixed latency for any IP.
func stubPing(latencyMs float64) PingFunc {
	return func(ctx context.Context, ip string, count int) (*PingResult, error) {
		return &PingResult{
			AvgMs:    latencyMs,
			P50Ms:    latencyMs,
			P95Ms:    latencyMs * 1.1,
			JitterMs: 0.5,
			LossPct:  0,
			Samples:  count,
		}, nil
	}
}

// stubNetwork returns a fixed network fingerprint.
func stubNetwork() NetworkFunc {
	return func(ctx context.Context) types.NetworkFingerprint {
		return types.NetworkFingerprint{
			Interface:     "wt0",
			InterfaceType: "tunnel",
			GatewayIP:     "192.168.1.1",
			PublicIP:      "198.51.100.7",
			ASN:           "AS14593",
			ASNOrg:        "SpaceX Services, Inc.",
		}
	}
}

func TestRunProbeWithFixture(t *testing.T) {
	ctx := context.Background()

	report, err := RunProbe(ctx,
		WithStatus(stubStatus(t)),
		WithPing(stubPing(42.0)),
		WithNetwork(stubNetwork()),
	)
	if err != nil {
		t.Fatalf("RunProbe: %v", err)
	}

	// Local peer identity
	if report.PeerIP != "100.127.0.1" {
		t.Errorf("PeerIP = %q, want 100.127.0.1", report.PeerIP)
	}
	if report.PeerName != "example.netbird.selfhosted" {
		t.Errorf("PeerName = %q, want example.netbird.selfhosted", report.PeerName)
	}

	// Should have measurements for all 27 peers
	if len(report.Peers) != 27 {
		t.Fatalf("got %d peers, want 27", len(report.Peers))
	}

	// Connected peers should have latency data from stub
	var connectedCount int
	for _, p := range report.Peers {
		if p.Connected {
			connectedCount++
			if p.LatencyMs != 42.0 {
				t.Errorf("peer %s latency = %.1f, want 42.0", p.PeerIP, p.LatencyMs)
			}
			if p.Samples != defaultPingCount {
				t.Errorf("peer %s samples = %d, want %d", p.PeerIP, p.Samples, defaultPingCount)
			}
		} else {
			if p.LatencyMs != 0 {
				t.Errorf("disconnected peer %s should have 0 latency, got %.1f", p.PeerIP, p.LatencyMs)
			}
		}
	}
	if connectedCount != 6 {
		t.Errorf("connected peers = %d, want 6", connectedCount)
	}

	// Network fingerprint from stub
	if report.Network.ASN != "AS14593" {
		t.Errorf("ASN = %q, want AS14593", report.Network.ASN)
	}

	// Direct vs relayed
	var directCount, relayedCount int
	for _, p := range report.Peers {
		if p.Connected && p.Direct {
			directCount++
		}
		if p.Connected && !p.Direct {
			relayedCount++
		}
	}
	if directCount != 5 {
		t.Errorf("direct connected = %d, want 5", directCount)
	}
	if relayedCount != 1 {
		t.Errorf("relayed connected = %d, want 1", relayedCount)
	}
}

// TestBoundedSubstepsRunBeforeThePeerSweep pins the ORDER, because the order
// is what makes the sub-steps reachable at all.
//
// They previously sat after the sweep and were gated on a budget the sweep had
// already spent: on a large mesh the sweep takes most of a 60s interval, so all
// three opted out on every cycle and the fleet collected no relay probe, no
// dialer counters and no MTU. Raising the budget does not fix it —
// the sweep is what consumes the interval.
//
// Asserting on source order is blunt, but the alternative is a timing test that
// passes on a fast host and hides the regression on the slow one that matters.
func TestBoundedSubstepsRunBeforeThePeerSweep(t *testing.T) {
	src, err := os.ReadFile("fingerprint.go")
	if err != nil {
		t.Fatalf("read: %v", err)
	}
	body := string(src)
	sweep := strings.Index(body, "// Ping each connected peer")
	if sweep < 0 {
		t.Fatal("peer sweep comment not found — did it move?")
	}
	for _, step := range []string{
		"// Per-relay probe",
		"// Race-dialer outcome counters",
		"// MTU probe",
	} {
		at := strings.Index(body, step)
		if at < 0 {
			t.Errorf("%s not found", step)
			continue
		}
		if at > sweep {
			t.Errorf("%s runs AFTER the peer sweep — it will be starved by the "+
				"budget the sweep spends, and will never report on a real fleet", step)
		}
	}
}
