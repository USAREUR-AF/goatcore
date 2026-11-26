package main

import (
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/time-hygiene/internal/attest"
	"github.com/usareur-af/goatcore/ops/time-hygiene/internal/signer"
	"github.com/usareur-af/goatcore/ops/time-hygiene/internal/state"
)

// Socket readers decode {"ntp_class_str": ...} at the top level with no
// DisallowUnknownFields. Adding the attestation fields must keep ntp_class_str
// a TOP-LEVEL field, so this guards that contract for every existing reader.
func TestServedJSONKeepsNtpClassStrTopLevel(t *testing.T) {
	s := state.NtpState{
		HostID:         "h",
		NtpClass:       state.ClassHealthy,
		NtpClassStr:    "healthy",
		LastUpdatedUTC: time.Now().UTC(),
	}
	if err := attest.Sign(&s, nil, time.Minute); err != nil {
		t.Fatalf("Sign: %v", err)
	}
	b, err := json.Marshal(s)
	if err != nil {
		t.Fatalf("Marshal: %v", err)
	}
	var top struct {
		NtpClassStr string `json:"ntp_class_str"`
	}
	if err := json.Unmarshal(b, &top); err != nil {
		t.Fatalf("Unmarshal: %v", err)
	}
	if top.NtpClassStr != "healthy" {
		t.Errorf("ntp_class_str = %q (want \"healthy\"); attestation fields must not nest it\n  json=%s", top.NtpClassStr, b)
	}
}

// store must attest the verdict it swaps in: with a signer present, the stored
// state carries a valid signature, so the /health + Unix-socket handlers serve
// a signed verdict.
func TestStoreAttestsVerdict(t *testing.T) {
	sgn, _, err := signer.Generate(filepath.Join(t.TempDir(), "k.pem"))
	if err != nil {
		t.Fatalf("Generate: %v", err)
	}
	a := &agent{hostID: "h", signer: sgn, validity: time.Minute}
	a.store(state.NtpState{
		HostID:         "h",
		NtpClass:       state.ClassHealthy,
		NtpClassStr:    "healthy",
		LastUpdatedUTC: time.Now().UTC(),
	})

	got := a.currentState()
	if !got.Attested {
		t.Fatal("stored verdict not attested with a signer present")
	}
	ok, err := attest.Verify(got)
	if err != nil || !ok {
		t.Fatalf("stored verdict does not verify: ok=%v err=%v", ok, err)
	}
}

// Without a signer, store still completes and serves an honest unattested
// verdict (Attested=false) with the advisory validity window set.
func TestStoreUnattestedWithoutSigner(t *testing.T) {
	a := &agent{hostID: "h", validity: time.Minute}
	a.store(state.NtpState{
		HostID:         "h",
		NtpClass:       state.ClassHealthy,
		NtpClassStr:    "healthy",
		LastUpdatedUTC: time.Now().UTC(),
	})
	got := a.currentState()
	if got.Attested {
		t.Error("verdict marked attested without a signer")
	}
	if got.ValidUntil.IsZero() {
		t.Error("ValidUntil should be set even when unattested")
	}
}
