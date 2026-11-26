package main

import (
	"context"
	"encoding/json"
	"net"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/time-hygiene/internal/attest"
	"github.com/usareur-af/goatcore/ops/time-hygiene/internal/signer"
	"github.com/usareur-af/goatcore/ops/time-hygiene/internal/state"
)

// healthyAt is a healthy verdict last refreshed at t.
func healthyAt(t time.Time) state.NtpState {
	return state.NtpState{
		HostID:         "h",
		NtpClass:       state.ClassHealthy,
		NtpClassStr:    state.ClassHealthy.String(),
		SelectedSource: state.SourceNone,
		Stratum:        2,
		LastUpdatedUTC: t.UTC(),
	}
}

// readUnix returns the verdict a socket client reads from the agent.
func readUnix(t *testing.T, a *agent) state.NtpState {
	t.Helper()
	client, server := net.Pipe()
	done := make(chan struct{})
	go func() { a.handleUnix(server); close(done) }()
	var got state.NtpState
	if err := json.NewDecoder(client).Decode(&got); err != nil {
		t.Fatalf("decode: %v", err)
	}
	client.Close()
	<-done
	return got
}

// A verdict the poll loop stopped refreshing (it returned after a recovered
// panic, or chronyd hangs) must not keep answering healthy on the socket.
func TestSocketDoesNotServeAnExpiredHealthyVerdict(t *testing.T) {
	a := &agent{hostID: "h", validity: 5 * time.Minute, pollInterval: 10 * time.Second}
	a.store(healthyAt(time.Now().Add(-10 * time.Minute)))

	got := readUnix(t, a)
	if got.NtpClass == state.ClassHealthy || got.NtpClassStr == state.ClassHealthy.String() {
		t.Fatalf("expired verdict served as %q; want unknown", got.NtpClassStr)
	}
	if got.NtpClassStr != state.ClassUnknown.String() {
		t.Fatalf("expired verdict served as %q; want unknown", got.NtpClassStr)
	}
}

// Within its validity window but well past the poll cadence, the verdict is
// also stale: a long --attestation-validity must not stretch how long a dead
// poll loop goes unnoticed.
func TestSocketDoesNotServeAVerdictPastThePollCadence(t *testing.T) {
	a := &agent{hostID: "h", validity: 24 * time.Hour, pollInterval: 10 * time.Second}
	a.store(healthyAt(time.Now().Add(-10 * time.Minute)))

	if got := readUnix(t, a); got.NtpClassStr != state.ClassUnknown.String() {
		t.Fatalf("verdict 10m old at a 10s poll interval served as %q; want unknown", got.NtpClassStr)
	}
}

func TestSocketServesAFreshHealthyVerdict(t *testing.T) {
	a := &agent{hostID: "h", validity: 5 * time.Minute, pollInterval: 10 * time.Second}
	fresh := healthyAt(time.Now())
	a.store(fresh)

	got := readUnix(t, a)
	if got.NtpClassStr != state.ClassHealthy.String() {
		t.Fatalf("fresh verdict served as %q; want healthy", got.NtpClassStr)
	}
	if !got.LastUpdatedUTC.Equal(fresh.LastUpdatedUTC) {
		t.Fatalf("last_updated_utc = %v, want %v", got.LastUpdatedUTC, fresh.LastUpdatedUTC)
	}
}

func TestHealthEndpointReportsAStaleVerdictUnhealthy(t *testing.T) {
	a := &agent{hostID: "h", validity: 5 * time.Minute, pollInterval: 10 * time.Second}
	a.store(healthyAt(time.Now().Add(-10 * time.Minute)))

	rec := httptest.NewRecorder()
	a.healthHandler(rec, httptest.NewRequestWithContext(context.Background(), http.MethodGet, "/health", nil))
	if rec.Code != http.StatusServiceUnavailable {
		t.Fatalf("/health = %d for a stale verdict; want 503", rec.Code)
	}
}

// With a signer, the stale verdict served as unknown carries a valid signature
// over the unknown class.
func TestStaleVerdictIsReAttested(t *testing.T) {
	sgn, _, err := signer.Generate(filepath.Join(t.TempDir(), "k.pem"))
	if err != nil {
		t.Fatalf("Generate: %v", err)
	}
	a := &agent{hostID: "h", signer: sgn, validity: 5 * time.Minute, pollInterval: 10 * time.Second}
	a.store(healthyAt(time.Now().Add(-10 * time.Minute)))

	got := readUnix(t, a)
	if got.NtpClassStr != state.ClassUnknown.String() || !got.Attested {
		t.Fatalf("got class %q attested=%v; want a signed unknown", got.NtpClassStr, got.Attested)
	}
	if ok, err := attest.Verify(got); err != nil || !ok {
		t.Fatalf("stale verdict does not verify: ok=%v err=%v", ok, err)
	}
}
