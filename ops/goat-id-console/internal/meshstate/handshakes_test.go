package meshstate

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func writeSummary(t *testing.T, body string, age time.Duration) string {
	t.Helper()
	p := filepath.Join(t.TempDir(), "peer-handshakes.json")
	if err := os.WriteFile(p, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	mt := time.Now().Add(-age)
	if err := os.Chtimes(p, mt, mt); err != nil {
		t.Fatal(err)
	}
	return p
}

// Shaped on the zagros case of: registered with management, the
// management host never completed a handshake with it.
const summary = `{"peers":[
 {"ip":"100.127.0.64","fqdn":"srv.example","status":"Connecting","last_handshake":"0001-01-01T00:00:00Z"},
 {"ip":"100.127.0.42","fqdn":"laptop.example","status":"Connected","last_handshake":"2026-10-05T10:00:00Z"},
 {"ip":"100.127.0.43","fqdn":"idle.example","status":"Idle","last_handshake":"2026-10-04T10:00:00Z"}
]}`

func TestNeverHandshaked(t *testing.T) {
	cases := []struct {
		h    PeerHandshake
		want bool
	}{
		{PeerHandshake{Status: "Connecting", LastHandshake: "0001-01-01T00:00:00Z"}, true},
		{PeerHandshake{Status: "Connecting", LastHandshake: ""}, true},
		{PeerHandshake{Status: "Connected", LastHandshake: ""}, false},
		{PeerHandshake{Status: "Idle", LastHandshake: "2026-10-04T10:00:00Z"}, false},
		{PeerHandshake{Status: "Connecting", LastHandshake: "garbage"}, false},
	}
	for _, c := range cases {
		if got := c.h.NeverHandshaked(); got != c.want {
			t.Errorf("%+v: got %v want %v", c.h, got, c.want)
		}
	}
}

func TestReadHandshakesRefusesStale(t *testing.T) {
	p := writeSummary(t, summary, HandshakeStaleAfter+time.Minute)
	if _, err := ReadHandshakes(p, time.Now()); err == nil {
		t.Fatal("a stale summary must be refused")
	}
	if _, err := ReadHandshakes("", time.Now()); err == nil {
		t.Fatal("an unset path must be refused")
	}
}

func TestDevicesForFlagsControlOnly(t *testing.T) {
	hs, err := ReadHandshakes(writeSummary(t, summary, 0), time.Now())
	if err != nil {
		t.Fatal(err)
	}
	known := map[string]struct{}{"mz": {}, "al": {}}
	srv := Peer{Name: "peer-mz-zagros-2026-10-05", IP: "100.127.0.64", Connected: true, OS: "Oracle Linux Server 8.10"}
	got := DevicesFor([]Peer{srv}, "mz", known, hs)
	if !got.ControlOnly || !got.Peers[0].ControlOnly {
		t.Fatalf("control-only device not flagged: %+v", got)
	}

	// One working device clears the person-level flag.
	lap := Peer{Name: "peer-mz-zagros-laptop", IP: "100.127.0.42", Connected: true, OS: "Darwin"}
	got = DevicesFor([]Peer{srv, lap}, "mz", known, hs)
	if got.ControlOnly || got.Peers[1].ControlOnly {
		t.Fatalf("a working device must clear the flag: %+v", got)
	}

	// No row for the peer means unknown, never flagged; neither is a nil view.
	other := Peer{Name: "peer-al-zagros-x", IP: "100.127.0.60", Connected: true}
	if got := DevicesFor([]Peer{other}, "al", known, hs); got.ControlOnly {
		t.Fatal("a peer the host has no view of must not be flagged")
	}
	if got := DevicesFor([]Peer{srv}, "mz", known, nil); got.ControlOnly {
		t.Fatal("no handshake view must not flag")
	}
}
