package main

import (
	"bytes"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/sha256"
	"crypto/rand"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/bundle"
)

func newFixture(t *testing.T) *bundle.EnrollmentBundle {
	t.Helper()
	// PeerPubkey is a WireGuard pubkey (32 bytes); a random buffer is
	// sufficient for tests.
	pub := make([]byte, 32)
	if _, err := rand.Read(pub); err != nil {
		t.Fatalf("gen peer key: %v", err)
	}
	probeKey := make([]byte, 32)
	probeKey[0] = 0xAA
	probeKey[31] = 0xBB
	nonce := make([]byte, 16)
	if _, err := rand.Read(nonce); err != nil {
		t.Fatalf("gen nonce: %v", err)
	}
	issued := time.Date(2026, 5, 4, 12, 0, 0, 0, time.UTC)
	return &bundle.EnrollmentBundle{
		Version:            bundle.Version,
		DeviceID:           "orin-region-a-01.c2",
		PeerPubkey:         pub,
		ACLGroups:          []string{"c2", "engagement-authority"},
		Site:               "lab",
		IssuedAt:           issued,
		ActivationDeadline: issued.Add(14 * 24 * time.Hour),
		ExpiresAt:          issued.Add(90 * 24 * time.Hour),
		Nonce:              nonce,
		CAID:               "dev-goat-ca-2026-04",
		SnitchProbeKey:     probeKey,
	}
}

func TestWriteField_SnitchProbeKeyHex(t *testing.T) {
	b := newFixture(t)
	var buf bytes.Buffer
	if err := writeField(&buf, b, "snitch_probe_key", encodingHex); err != nil {
		t.Fatalf("writeField: %v", err)
	}
	got := strings.TrimSpace(buf.String())
	if len(got) != 64 {
		t.Errorf("hex output length = %d; want 64 (32 bytes)", len(got))
	}
	if !strings.HasPrefix(got, "aa") || !strings.HasSuffix(got, "bb") {
		t.Errorf("hex output = %q; want aa...bb pattern", got)
	}
}

func TestWriteField_SnitchProbeKeyBase64(t *testing.T) {
	b := newFixture(t)
	var buf bytes.Buffer
	if err := writeField(&buf, b, "snitch_probe_key", encodingBase64); err != nil {
		t.Fatalf("writeField: %v", err)
	}
	got := strings.TrimSpace(buf.String())
	// 32 bytes base64 = 44 chars (with padding).
	if len(got) != 44 {
		t.Errorf("base64 output length = %d; want 44 (32 bytes std b64)", len(got))
	}
}

func TestWriteField_SnitchProbeKeyAbsent(t *testing.T) {
	b := newFixture(t)
	b.SnitchProbeKey = nil
	var buf bytes.Buffer
	err := writeField(&buf, b, "snitch_probe_key", encodingHex)
	if err == nil {
		t.Fatal("expected error for absent field")
	}
	if !strings.HasPrefix(err.Error(), "field ") {
		t.Errorf("expected error to start with 'field ', got: %v", err)
	}
}

func TestWriteField_StringFields(t *testing.T) {
	b := newFixture(t)
	var buf bytes.Buffer
	if err := writeField(&buf, b, "device_id", encodingNone); err != nil {
		t.Fatalf("writeField: %v", err)
	}
	got := strings.TrimSpace(buf.String())
	if got != "orin-region-a-01.c2" {
		t.Errorf("device_id = %q; want orin-region-a-01.c2", got)
	}
}

func TestWriteField_TimeFields(t *testing.T) {
	b := newFixture(t)
	var buf bytes.Buffer
	if err := writeField(&buf, b, "expires_at", encodingNone); err != nil {
		t.Fatalf("writeField: %v", err)
	}
	got := strings.TrimSpace(buf.String())
	// Issued + 90d =
	if !strings.HasPrefix(got, "2026-08-02") {
		t.Errorf("expires_at = %q; want 2026-08-02 prefix", got)
	}
}

func TestWriteField_ACLGroups(t *testing.T) {
	b := newFixture(t)
	var buf bytes.Buffer
	if err := writeField(&buf, b, "acl_groups", encodingNone); err != nil {
		t.Fatalf("writeField: %v", err)
	}
	got := strings.TrimSpace(buf.String())
	if got != "c2,engagement-authority" {
		t.Errorf("acl_groups = %q; want c2,engagement-authority", got)
	}
}

func TestWriteField_HasInnerMesh(t *testing.T) {
	b := newFixture(t)
	var buf bytes.Buffer
	if err := writeField(&buf, b, "has_inner_mesh", encodingNone); err != nil {
		t.Fatalf("writeField: %v", err)
	}
	if got := strings.TrimSpace(buf.String()); got != "false" {
		t.Errorf("has_inner_mesh on wg-cp0-only bundle = %q; want false", got)
	}
	b.InnerMeshSetup = bundle.InnerMeshSetup{
		ManagementURL: "https://mgmt.example.net:443",
		SetupKey:      "AABBCCDD-0000-0000-0000-000000000000",
	}
	buf.Reset()
	if err := writeField(&buf, b, "has_inner_mesh", encodingNone); err != nil {
		t.Fatalf("writeField: %v", err)
	}
	if got := strings.TrimSpace(buf.String()); got != "true" {
		t.Errorf("has_inner_mesh on v0.2 bundle = %q; want true", got)
	}
	buf.Reset()
	if err := writeField(&buf, b, "inner_mesh_mgmt_url", encodingNone); err != nil {
		t.Fatalf("writeField inner_mesh_mgmt_url: %v", err)
	}
	if got := strings.TrimSpace(buf.String()); got != "https://mgmt.example.net:443" {
		t.Errorf("inner_mesh_mgmt_url = %q", got)
	}
	// The setup-key must never be extractable.
	buf.Reset()
	err := writeField(&buf, b, "inner_mesh_setup_key", encodingNone)
	if err == nil || !strings.Contains(err.Error(), "field unknown") {
		t.Errorf("inner_mesh_setup_key should be unknown; got err=%v out=%q", err, buf.String())
	}
}

func TestWriteField_Unknown(t *testing.T) {
	b := newFixture(t)
	var buf bytes.Buffer
	err := writeField(&buf, b, "no_such_field", encodingNone)
	if err == nil {
		t.Fatal("expected error for unknown field")
	}
	if !strings.Contains(err.Error(), "field unknown") {
		t.Errorf("error should say 'field unknown'; got: %v", err)
	}
}

func TestWriteField_KnownFieldsTableComplete(t *testing.T) {
	// Every name returned by knownFields() must be reachable via writeField.
	b := newFixture(t)
	for _, name := range knownFields() {
		var buf bytes.Buffer
		err := writeField(&buf, b, name, encodingNone)
		// An "unknown" error here means the table is out of sync with
		// the dispatch in writeField — a maintenance bug.
		if err != nil && strings.Contains(err.Error(), "field unknown") {
			t.Errorf("knownFields lists %q but writeField rejects it as unknown", name)
		}
	}
}

// TestRoundtrip_OperatorWorkflow is the load-bearing scenario test:
// the operator generates a probe key, embeds it in a bundle, signs +
// marshals, ships to the device. Device unmarshals + bundle-extract
// produces the same key bytes the operator started with. Any drift in
// CBOR canonical encoding, signing, or extraction shows up here.
func TestRoundtrip_OperatorWorkflow(t *testing.T) {
	b := newFixture(t)
	// Sign as the offline CA would.
	caPriv, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatalf("gen ca: %v", err)
	}
	caPub := &caPriv.PublicKey
	sign := func(msg []byte) ([]byte, error) {
		digest := sha256.Sum256(msg)
		return ecdsa.SignASN1(rand.Reader, caPriv, digest[:])
	}
	if err := b.Sign(sign); err != nil {
		t.Fatalf("sign: %v", err)
	}
	// Marshal to "wire" — the bytes that ship to the device.
	wire, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	// Device side: parse + verify + extract.
	parsed, err := bundle.Unmarshal(wire)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if err := parsed.Verify(caPub); err != nil {
		t.Fatalf("verify: %v", err)
	}
	var buf bytes.Buffer
	if err := writeField(&buf, parsed, "snitch_probe_key", encodingHex); err != nil {
		t.Fatalf("extract: %v", err)
	}
	got := strings.TrimSpace(buf.String())
	if len(got) != 64 {
		t.Errorf("operator-workflow extracted key length = %d; want 64", len(got))
	}
	if !strings.HasPrefix(got, "aa") || !strings.HasSuffix(got, "bb") {
		t.Errorf("extracted key = %q; lost the aa...bb signature bytes", got)
	}
}
