package main

import (
	"encoding/hex"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"

	"github.com/usareur-af/goatcore/ops/enrollment/bundle"
)

// TestRun_Snitch_KeyGeneratedAndEmbedded asserts that --snitch=true
// causes a 32-byte snitch_probe_key to be generated, embedded in the
// bundle, and round-tripped through CBOR.
func TestRun_Snitch_KeyGeneratedAndEmbedded(t *testing.T) {
	caKey, ageID, caPub := devCA(t)
	relaysPath := fixtureRelays(t)
	outPath := filepath.Join(t.TempDir(), "bundle.cbor")

	args := runArgs{
		Site: "lab", DeviceID: "mgmt-test-01-laptop",
		WGCP0Address: "10.99.0.10/24",
		RelaysPath:   relaysPath, ACLGroupsRaw: "LAB",
		CAID: "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		OutPath: outPath,
		TTLDays: 90, ActivationDays: 14, Now: fixedNow(),
		Snitch: true,
	}
	if err := run(args); err != nil {
		t.Fatalf("run: %v", err)
	}

	wire, err := os.ReadFile(outPath)
	if err != nil {
		t.Fatalf("read out: %v", err)
	}
	b, err := bundle.Unmarshal(wire)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if err := b.Verify(caPub); err != nil {
		t.Fatalf("verify: %v", err)
	}
	if len(b.SnitchProbeKey) != snitchProbeKeyBytes {
		t.Errorf("SnitchProbeKey size = %d; want %d", len(b.SnitchProbeKey), snitchProbeKeyBytes)
	}
}

// TestRun_Snitch_DefaultOff asserts that without --snitch, the bundle
// has no snitch_probe_key (omitempty round-trip).
func TestRun_Snitch_DefaultOff(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	outPath := filepath.Join(t.TempDir(), "bundle.cbor")

	args := runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.10/24",
		RelaysPath: relaysPath, ACLGroupsRaw: "G",
		CAID: "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		OutPath: outPath,
		TTLDays: 90, ActivationDays: 14, Now: fixedNow(),
		// Snitch: false (zero value)
	}
	if err := run(args); err != nil {
		t.Fatalf("run: %v", err)
	}
	wire, err := os.ReadFile(outPath)
	if err != nil {
		t.Fatalf("read out: %v", err)
	}
	b, err := bundle.Unmarshal(wire)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if b.SnitchProbeKey != nil {
		t.Errorf("SnitchProbeKey = %x; want nil (omitempty)", b.SnitchProbeKey)
	}
}

// TestRun_Snitch_RegistersWithCollector asserts that --snitch + --snitch-collector
// causes a POST to /api/keys carrying the bearer string the device will
// later present, with role=probe and label=device-id.
func TestRun_Snitch_RegistersWithCollector(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	outPath := filepath.Join(t.TempDir(), "bundle.cbor")

	const adminKey = "snitch_admin_test_key_xyz"
	var (
		gotMethod, gotPath, gotAuth, gotCT string
		gotBody                            map[string]string
		hits                               atomic.Int32
	)

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits.Add(1)
		gotMethod = r.Method
		gotPath = r.URL.Path
		gotAuth = r.Header.Get("Authorization")
		gotCT = r.Header.Get("Content-Type")
		body, _ := io.ReadAll(r.Body)
		_ = json.Unmarshal(body, &gotBody)
		w.WriteHeader(http.StatusOK)
		if _, err := w.Write([]byte(`{"registered":true}`)); err != nil {
			t.Errorf("write response: %v", err)
		}
	}))
	defer srv.Close()

	args := runArgs{
		Site: "lab", DeviceID: "mgmt-test-01-laptop",
		WGCP0Address: "10.99.0.10/24",
		RelaysPath:   relaysPath, ACLGroupsRaw: "LAB",
		CAID: "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		OutPath: outPath,
		TTLDays: 90, ActivationDays: 14, Now: fixedNow(),
		Snitch:          true,
		SnitchCollector: srv.URL,
		SnitchAdminKey:  adminKey,
	}
	if err := run(args); err != nil {
		t.Fatalf("run: %v", err)
	}

	if hits.Load() != 1 {
		t.Errorf("collector hits = %d; want 1", hits.Load())
	}
	if gotMethod != http.MethodPost {
		t.Errorf("method = %q; want POST", gotMethod)
	}
	if gotPath != "/api/keys" {
		t.Errorf("path = %q; want /api/keys", gotPath)
	}
	if gotAuth != "Bearer "+adminKey {
		t.Errorf("Authorization = %q; want %q", gotAuth, "Bearer "+adminKey)
	}
	if !strings.HasPrefix(gotCT, "application/json") {
		t.Errorf("Content-Type = %q; want application/json", gotCT)
	}
	if gotBody["role"] != "probe" {
		t.Errorf("body.role = %q; want probe", gotBody["role"])
	}
	if gotBody["label"] != "mgmt-test-01-laptop" {
		t.Errorf("body.label = %q; want mgmt-test-01-laptop", gotBody["label"])
	}
	bearer := gotBody["plaintext_key"]
	if !strings.HasPrefix(bearer, "snitch_") {
		t.Errorf("body.plaintext_key = %q; want snitch_<hex> prefix", bearer)
	}
	hexPart := strings.TrimPrefix(bearer, "snitch_")
	if len(hexPart) != 2*snitchProbeKeyBytes {
		t.Errorf("hex part length = %d; want %d (32 bytes hex)", len(hexPart), 2*snitchProbeKeyBytes)
	}
	if _, err := hex.DecodeString(hexPart); err != nil {
		t.Errorf("hex part not decodable: %v", err)
	}

	// Bundle was written: registration ran successfully, write must have
	// followed.
	if _, err := os.Stat(outPath); err != nil {
		t.Errorf("bundle file not written after successful registration: %v", err)
	}
}

// TestRun_Snitch_RegistrationFailureRollsBack asserts that if the
// collector rejects the registration, the bundle file is NOT written
// — the operator can re-run with the same args and get a clean retry
// (and bundle-extract won't find a stale file with a key the collector
// doesn't accept).
func TestRun_Snitch_RegistrationFailureRollsBack(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	outPath := filepath.Join(t.TempDir(), "bundle.cbor")

	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, "synthetic-failure", http.StatusInternalServerError)
	}))
	defer srv.Close()

	args := runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.10/24",
		RelaysPath: relaysPath, ACLGroupsRaw: "G",
		CAID: "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		OutPath: outPath,
		TTLDays: 90, ActivationDays: 14, Now: fixedNow(),
		Snitch: true, SnitchCollector: srv.URL, SnitchAdminKey: "any",
	}
	err := run(args)
	if err == nil {
		t.Fatal("expected error on collector 500")
	}
	if !strings.Contains(err.Error(), "register snitch") {
		t.Errorf("error = %v; want to mention 'register snitch'", err)
	}
	if _, statErr := os.Stat(outPath); statErr == nil {
		t.Error("bundle file written despite registration failure; rollback contract violated")
	}
}

// TestRun_Snitch_CollectorRequiresAdminKey asserts that --snitch-collector
// without an admin key (flag empty + env empty) is a validation error.
func TestRun_Snitch_CollectorRequiresAdminKey(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	t.Setenv("SNITCH_ADMIN_KEY", "") // make sure env doesn't sneak in

	args := runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.10/24",
		RelaysPath: relaysPath, ACLGroupsRaw: "G",
		CAID: "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		OutPath: filepath.Join(t.TempDir(), "bundle.cbor"),
		TTLDays: 90, ActivationDays: 14, Now: fixedNow(),
		Snitch: true, SnitchCollector: "http://example.invalid",
		// SnitchAdminKey: "" (zero value)
	}
	err := run(args)
	if err == nil {
		t.Fatal("expected error when --snitch-collector set with empty admin key")
	}
	if !strings.Contains(err.Error(), "admin key") {
		t.Errorf("error %q does not mention 'admin key'", err)
	}
}

// TestRun_Snitch_CollectorRequiresSnitchTrue asserts that
// --snitch-collector without --snitch=true is a validation error
// (would otherwise generate no key + try to register nothing).
func TestRun_Snitch_CollectorRequiresSnitchTrue(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)

	args := runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.10/24",
		RelaysPath: relaysPath, ACLGroupsRaw: "G",
		CAID: "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		OutPath: filepath.Join(t.TempDir(), "bundle.cbor"),
		TTLDays: 90, ActivationDays: 14, Now: fixedNow(),
		// Snitch: false
		SnitchCollector: "http://example.invalid", SnitchAdminKey: "any",
	}
	err := run(args)
	if err == nil {
		t.Fatal("expected error when --snitch-collector set without --snitch=true")
	}
	if !strings.Contains(err.Error(), "--snitch=true") {
		t.Errorf("error %q does not mention --snitch=true", err)
	}
}

// TestSnitchBearerString asserts the on-the-wire form matches the
// plaintext that auth.CreateKey hashes server-side (snitch_<hex>).
func TestSnitchBearerString(t *testing.T) {
	raw := []byte{
		0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
		0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
		0x10, 0x21, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87,
		0x98, 0xa9, 0xba, 0xcb, 0xdc, 0xed, 0xfe, 0x0f,
	}
	got := snitchBearerString(raw)
	want := "snitch_00112233445566778899aabbccddeeff10213243546576879" + "8a9bacbdcedfe0f"
	if got != want {
		t.Errorf("got  %q\nwant %q", got, want)
	}
}
