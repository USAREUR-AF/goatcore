package main

import (
	"bytes"
	"crypto/ecdsa"
	"encoding/base64"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"filippo.io/age"
	"golang.org/x/crypto/curve25519"

	"github.com/usareur-af/goatcore/ops/enrollment/bundle"
	"github.com/usareur-af/goatcore/ops/enrollment/ca"
)

// fixedNow returns a deterministic timestamp so tests don't drift on
// activation_deadline / expires_at math.
func fixedNow() time.Time {
	return time.Date(2026, 5, 5, 12, 0, 0, 0, time.UTC)
}

// devCA builds a fresh age-encrypted CA key + the recipient identity
// needed to decrypt it. Returns the CA's age-encrypted private-key path
// (under tmpDir), the path to the age identity file, and the CA's
// ECDSA P-256 public key (for verifying signed bundles in the test).
// Block 79 hard cutover.
func devCA(t *testing.T) (caKeyPath, ageIdentityPath string, caPub *ecdsa.PublicKey) {
	t.Helper()
	tmpDir := t.TempDir()

	identity, err := age.GenerateX25519Identity()
	if err != nil {
		t.Fatalf("gen age identity: %v", err)
	}
	ageIdentityPath = filepath.Join(tmpDir, "keys.txt")
	if err := os.WriteFile(ageIdentityPath, []byte(identity.String()+"\n"), 0o600); err != nil {
		t.Fatalf("write age identity: %v", err)
	}

	caKeyPath = filepath.Join(tmpDir, "ca.key.age")
	pub, err := ca.GenerateAndWrite(caKeyPath, []age.Recipient{identity.Recipient()})
	if err != nil {
		t.Fatalf("ca generate: %v", err)
	}
	return caKeyPath, ageIdentityPath, pub
}

// fixtureRelays writes a relays.json under tmpDir and returns its path.
// Three relays mirroring the lab prod fleet — endpoint shapes match
// inventory/wg-cp0-prod.ini exactly so a relay-roster regression here
// catches schema drift early.
func fixtureRelays(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	type r struct {
		Name     string `json:"name"`
		Endpoint string `json:"endpoint"`
		Address  string `json:"address"`
		Pubkey   string `json:"pubkey"`
	}
	pubBytes := make([][]byte, 3)
	for i := range pubBytes {
		_, priv, err := generateWGKeypair()
		if err != nil {
			t.Fatalf("gen relay key %d: %v", i, err)
		}
		pub, err := curve25519.X25519(priv, curve25519.Basepoint)
		if err != nil {
			t.Fatalf("derive relay pub %d: %v", i, err)
		}
		pubBytes[i] = pub
	}
	roster := []r{
		{Name: "relay-rla", Endpoint: "203.0.113.1:51821", Address: "10.99.0.2", Pubkey: base64.StdEncoding.EncodeToString(pubBytes[0])},
		{Name: "relay-rlb", Endpoint: "203.0.113.23:51821", Address: "10.99.0.3", Pubkey: base64.StdEncoding.EncodeToString(pubBytes[1])},
		{Name: "relay-rlc", Endpoint: "203.0.113.10:51821", Address: "10.99.0.4", Pubkey: base64.StdEncoding.EncodeToString(pubBytes[2])},
	}
	data, err := json.MarshalIndent(roster, "", "  ")
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	path := filepath.Join(dir, "wg-cp0-relays.json")
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatalf("write relays: %v", err)
	}
	return path
}

// TestRun_OperatorWorkflow is the load-bearing scenario: operator runs
// bundle-create with a real-shaped argument set against a dev-mode CA;
// the resulting bundle.cbor parses, verifies, contains all the fields
// the device-side render tool expects, and the wg-cp0 keypair is a
// valid (clamped privkey + matching pubkey) Curve25519 pair.
func TestRun_OperatorWorkflow(t *testing.T) {
	caKey, ageID, caPub := devCA(t)
	relaysPath := fixtureRelays(t)
	outPath := filepath.Join(t.TempDir(), "bundle.cbor")

	args := runArgs{
		Site:            "lab",
		DeviceID:        "mgmt-prov-site3-01-orin",
		WGCP0Address:    "10.99.0.6/24",
		RelaysPath:      relaysPath,
		ACLGroupsRaw:    "LAB,LAB-MGMT",
		CAID:            "dev-goat-ca-test",
		CAKeyPath:       caKey,
		AgeIdentityPath: ageID,
		OutPath:         outPath,
		TTLDays:         defaultTTLDays,
		ActivationDays:  defaultActivationDays,
		Now:             fixedNow(),
	}
	if err := run(args); err != nil {
		t.Fatalf("run: %v", err)
	}

	wire, err := os.ReadFile(outPath)
	if err != nil {
		t.Fatalf("read out: %v", err)
	}
	st, err := os.Stat(outPath)
	if err != nil {
		t.Fatalf("stat out: %v", err)
	}
	if st.Mode().Perm() != 0o600 {
		t.Errorf("out mode = %o; want 0600 (sensitive material)", st.Mode().Perm())
	}

	b, err := bundle.Unmarshal(wire)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if err := b.Verify(caPub); err != nil {
		t.Fatalf("verify: %v", err)
	}

	if b.DeviceID != "mgmt-prov-site3-01-orin" {
		t.Errorf("DeviceID = %q; want mgmt-prov-site3-01-orin", b.DeviceID)
	}
	if b.Site != "lab" {
		t.Errorf("Site = %q; want lab", b.Site)
	}
	if got := strings.Join(b.ACLGroups, ","); got != "LAB,LAB-MGMT" {
		t.Errorf("ACLGroups = %q; want LAB,LAB-MGMT", got)
	}
	if len(b.KnownEndpoints) != 3 {
		t.Errorf("KnownEndpoints count = %d; want 3", len(b.KnownEndpoints))
	}
	for i, ep := range b.KnownEndpoints {
		if ep.Kind != bundle.Kind("cp-relay") {
			t.Errorf("endpoint[%d].Kind = %q; want cp-relay", i, ep.Kind)
		}
	}

	// Bundle pairing invariant: both halves of the wg-cp0 keypair present.
	if err := b.CheckCPDeviceKeypair(); err != nil {
		t.Errorf("CheckCPDeviceKeypair: %v", err)
	}
	if len(b.CPDevicePubkey) != wgPubkeySize {
		t.Errorf("CPDevicePubkey size = %d; want %d", len(b.CPDevicePubkey), wgPubkeySize)
	}
	if len(b.CPDevicePrivkey) != wgPubkeySize {
		t.Errorf("CPDevicePrivkey size = %d; want %d", len(b.CPDevicePrivkey), wgPubkeySize)
	}

	// Pubkey matches privkey: derive pubkey from the privkey and check.
	derived, err := curve25519.X25519(b.CPDevicePrivkey, curve25519.Basepoint)
	if err != nil {
		t.Fatalf("derive: %v", err)
	}
	if !bytes.Equal(derived, b.CPDevicePubkey) {
		t.Errorf("CPDevicePubkey != derive(CPDevicePrivkey); keypair is broken")
	}

	// Privkey is clamped per RFC 7748 §5.
	if b.CPDevicePrivkey[0]&0b00000111 != 0 {
		t.Errorf("CPDevicePrivkey[0] = 0x%02x; low 3 bits should be cleared", b.CPDevicePrivkey[0])
	}
	if b.CPDevicePrivkey[31]&0b10000000 != 0 {
		t.Errorf("CPDevicePrivkey[31] = 0x%02x; high bit should be cleared", b.CPDevicePrivkey[31])
	}
	if b.CPDevicePrivkey[31]&0b01000000 == 0 {
		t.Errorf("CPDevicePrivkey[31] = 0x%02x; second-high bit should be set", b.CPDevicePrivkey[31])
	}

	// Activation + expiry are anchored to args.Now, NOT real wall-clock.
	wantActivation := fixedNow().Add(14 * 24 * time.Hour)
	if !b.ActivationDeadline.Equal(wantActivation) {
		t.Errorf("ActivationDeadline = %s; want %s", b.ActivationDeadline, wantActivation)
	}
	wantExpires := fixedNow().Add(90 * 24 * time.Hour)
	if !b.ExpiresAt.Equal(wantExpires) {
		t.Errorf("ExpiresAt = %s; want %s", b.ExpiresAt, wantExpires)
	}

	// CPDeviceAddress propagates verbatim from --wg-cp0-address.
	if b.CPDeviceAddress != "10.99.0.6/24" {
		t.Errorf("CPDeviceAddress = %q; want 10.99.0.6/24", b.CPDeviceAddress)
	}
	// Per-relay MeshAddr matches the address field in the relays JSON.
	wantMesh := []string{"10.99.0.2", "10.99.0.3", "10.99.0.4"}
	for i, want := range wantMesh {
		if b.KnownEndpoints[i].MeshAddr != want {
			t.Errorf("KnownEndpoints[%d].MeshAddr = %q; want %q", i, b.KnownEndpoints[i].MeshAddr, want)
		}
	}
}

// TestRun_RejectsNonCIDRAddress catches a footgun where the operator
// passes "10.99.0.6" without a prefix. The device-side renderer needs
// a complete CIDR for the [Interface] Address line.
func TestRun_RejectsNonCIDRAddress(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	args := runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.10",
		RelaysPath: relaysPath, ACLGroupsRaw: "G",
		CAID: "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		OutPath: filepath.Join(t.TempDir(), "bundle.cbor"),
		TTLDays: 90, ActivationDays: 14, Now: fixedNow(),
	}
	err := run(args)
	if err == nil {
		t.Fatal("expected error for non-CIDR address")
	}
	if !strings.Contains(err.Error(), "CIDR") {
		t.Errorf("error %q does not mention 'CIDR'", err)
	}
}

// TestRun_RejectsTLSAnchorThatIsntPEM catches a footgun: operator passes
// a path to a binary file or a key file by mistake instead of a cert.
func TestRun_RejectsTLSAnchorThatIsntPEM(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	bogusAnchor := filepath.Join(t.TempDir(), "not-a-cert")
	if err := os.WriteFile(bogusAnchor, []byte("not pem at all"), 0o600); err != nil {
		t.Fatalf("write bogus: %v", err)
	}
	args := runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.10/24",
		RelaysPath: relaysPath, ACLGroupsRaw: "G",
		CAID: "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		TLSTrustAnchorPath: bogusAnchor,
		OutPath:            filepath.Join(t.TempDir(), "bundle.cbor"),
		TTLDays:            90, ActivationDays: 14, Now: fixedNow(),
	}
	err := run(args)
	if err == nil {
		t.Fatal("expected error for non-PEM trust anchor")
	}
	if !strings.Contains(err.Error(), "PEM") && !strings.Contains(err.Error(), "tls trust anchor") {
		t.Errorf("error doesn't mention PEM/tls trust anchor: %v", err)
	}
}

// TestRun_RejectsActivationLongerThanTTL catches a misuse: operator
// types --activation-days 90 --ttl-days 14 (or just forgets to bump
// --ttl-days when raising --activation-days).
func TestRun_RejectsActivationLongerThanTTL(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	args := runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.10/24",
		RelaysPath: relaysPath, ACLGroupsRaw: "G",
		CAID: "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		OutPath: filepath.Join(t.TempDir(), "bundle.cbor"),
		TTLDays: 14, ActivationDays: 90, Now: fixedNow(),
	}
	err := run(args)
	if err == nil {
		t.Fatal("expected error for activation > ttl")
	}
}

// TestRun_RejectsMissingRequiredFlags asserts every flag marked required
// in validateArgs() actually fails on omission.
func TestRun_RejectsMissingRequiredFlags(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	full := runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.10/24",
		RelaysPath: relaysPath, ACLGroupsRaw: "G",
		CAID: "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		OutPath: filepath.Join(t.TempDir(), "bundle.cbor"),
		TTLDays: 90, ActivationDays: 14, Now: fixedNow(),
	}
	for _, name := range []string{"Site", "DeviceID", "WGCP0Address", "RelaysPath", "ACLGroupsRaw", "CAID", "CAKeyPath", "AgeIdentityPath", "OutPath"} {
		t.Run(name, func(t *testing.T) {
			a := full
			switch name {
			case "Site":
				a.Site = ""
			case "DeviceID":
				a.DeviceID = ""
			case "WGCP0Address":
				a.WGCP0Address = ""
			case "RelaysPath":
				a.RelaysPath = ""
			case "ACLGroupsRaw":
				a.ACLGroupsRaw = ""
			case "CAID":
				a.CAID = ""
			case "CAKeyPath":
				a.CAKeyPath = ""
			case "AgeIdentityPath":
				a.AgeIdentityPath = ""
			case "OutPath":
				a.OutPath = ""
			}
			if err := run(a); err == nil {
				t.Errorf("expected error when %s empty", name)
			}
		})
	}
}

// TestLoadRelaysFile_RejectsBadInputs covers the relays-file validators
// (duplicate names, malformed pubkey, missing ports, bad IPs).
func TestLoadRelaysFile_RejectsBadInputs(t *testing.T) {
	for _, tc := range []struct {
		name    string
		content string
		wantSub string
	}{
		{"not json", "this is not json", "parse json"},
		{"missing port", `[{"name":"r","endpoint":"203.0.113.1","address":"10.99.0.2","pubkey":"AAA="}]`, "must include port"},
		{"bad ip", `[{"name":"r","endpoint":"203.0.113.1:51821","address":"not-an-ip","pubkey":"AAA="}]`, "not a valid IP"},
		{"bad pubkey", `[{"name":"r","endpoint":"203.0.113.1:51821","address":"10.99.0.2","pubkey":"!!!"}]`, "base64 decode"},
		{"short pubkey", `[{"name":"r","endpoint":"203.0.113.1:51821","address":"10.99.0.2","pubkey":"AAA="}]`, "wrong size"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			p := filepath.Join(t.TempDir(), "relays.json")
			if err := os.WriteFile(p, []byte(tc.content), 0o600); err != nil {
				t.Fatalf("write: %v", err)
			}
			_, err := loadRelaysFile(p)
			if err == nil {
				t.Fatal("expected error")
			}
			if !strings.Contains(err.Error(), tc.wantSub) {
				t.Errorf("error %q does not contain %q", err, tc.wantSub)
			}
		})
	}
}

// TestLoadRelaysFile_RejectsDuplicateNames asserts duplicate detection
// on top of validation. Twin-named relays in a single roster would
// produce ambiguous (endpoint, pubkey) bindings on the device side.
func TestLoadRelaysFile_RejectsDuplicateNames(t *testing.T) {
	pub := make([]byte, 32)
	pub[0] = 0x01
	pubB64 := base64.StdEncoding.EncodeToString(pub)
	body := []map[string]string{
		{"name": "relay-rla", "endpoint": "1.2.3.4:51821", "address": "10.99.0.2", "pubkey": pubB64},
		{"name": "relay-rla", "endpoint": "5.6.7.8:51821", "address": "10.99.0.3", "pubkey": pubB64},
	}
	data, _ := json.Marshal(body)
	p := filepath.Join(t.TempDir(), "relays.json")
	_ = os.WriteFile(p, data, 0o600)
	_, err := loadRelaysFile(p)
	if err == nil {
		t.Fatal("expected duplicate error")
	}
	if !strings.Contains(err.Error(), "duplicate") {
		t.Errorf("error %q does not mention 'duplicate'", err)
	}
}

// TestResolvePeerPubkey_PlaceholderFlag asserts the placeholder branch
// signals back to the caller (so the summary can warn).
func TestResolvePeerPubkey_PlaceholderFlag(t *testing.T) {
	pub, placeholder, err := resolvePeerPubkey("")
	if err != nil {
		t.Fatalf("resolvePeerPubkey: %v", err)
	}
	if !placeholder {
		t.Error("placeholder flag should be true when no flag passed")
	}
	if len(pub) != peerPubkeySize {
		t.Errorf("placeholder len = %d; want %d", len(pub), peerPubkeySize)
	}
}

// TestResolvePeerPubkey_AcceptsValidB64 asserts the operator-supplied
// path: a base64-encoded 32-byte key is decoded and returned as-is.
func TestResolvePeerPubkey_AcceptsValidB64(t *testing.T) {
	want := make([]byte, peerPubkeySize)
	want[0] = 0xab
	want[31] = 0xcd
	b64 := base64.StdEncoding.EncodeToString(want)
	got, placeholder, err := resolvePeerPubkey(b64)
	if err != nil {
		t.Fatalf("resolvePeerPubkey: %v", err)
	}
	if placeholder {
		t.Error("placeholder flag should be false when flag passed")
	}
	if !bytes.Equal(got, want) {
		t.Errorf("got %x want %x", got, want)
	}
}

// TestResolvePeerPubkey_RejectsWrongSize catches an operator typo
// (truncated or extended b64).
func TestResolvePeerPubkey_RejectsWrongSize(t *testing.T) {
	_, _, err := resolvePeerPubkey(base64.StdEncoding.EncodeToString([]byte{1, 2, 3, 4}))
	if err == nil {
		t.Fatal("expected error for too-short pubkey")
	}
	if !strings.Contains(err.Error(), "wrong size") {
		t.Errorf("error %q does not mention 'wrong size'", err)
	}
}

// TestGenerateWGKeypair_PubMatchesPriv guards against silent regression
// in clampPrivateKey or the curve25519 import — generating a thousand
// keypairs and checking pubkey-from-privkey derivation should always
// match the returned pubkey.
func TestGenerateWGKeypair_PubMatchesPriv(t *testing.T) {
	for i := 0; i < 100; i++ {
		pub, priv, err := generateWGKeypair()
		if err != nil {
			t.Fatalf("iter %d: %v", i, err)
		}
		derived, err := curve25519.X25519(priv, curve25519.Basepoint)
		if err != nil {
			t.Fatalf("iter %d derive: %v", i, err)
		}
		if !bytes.Equal(pub, derived) {
			t.Fatalf("iter %d: pub != derive(priv)", i)
		}
	}
}

// TestRun_TamperedBundleFailsVerify is the negative-control for
// TestRun_OperatorWorkflow — flips a byte after run() writes the
// bundle, asserts Unmarshal+Verify rejects.
func TestRun_TamperedBundleFailsVerify(t *testing.T) {
	caKey, ageID, caPub := devCA(t)
	relaysPath := fixtureRelays(t)
	outPath := filepath.Join(t.TempDir(), "bundle.cbor")
	args := runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.10/24",
		RelaysPath: relaysPath, ACLGroupsRaw: "G",
		CAID: "dev-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		OutPath: outPath, TTLDays: 90, ActivationDays: 14, Now: fixedNow(),
	}
	if err := run(args); err != nil {
		t.Fatalf("run: %v", err)
	}
	wire, _ := os.ReadFile(outPath)
	// Find the device_id string and flip a byte in it.
	idx := bytes.Index(wire, []byte("x"))
	if idx < 0 {
		t.Fatal("device_id payload not found in bundle bytes")
	}
	wire[idx] = 'y'
	b, err := bundle.Unmarshal(wire)
	if err != nil {
		// Some tampers break CBOR structurally; still a rejection.
		return
	}
	if err := b.Verify(caPub); err == nil {
		t.Error("verify should reject tampered bundle")
	} else if !errors.Is(err, errors.New("signature invalid")) {
		// Just assert *some* error, since the exact sentinel may differ.
		// (errors.Is on a fresh errors.New is always false; we don't care
		// about the specific shape, only that verification fails.)
		_ = err
	}
}

// TestRun_MeshRouteSubnet — ADR 0321 (wg-cp0 perimeter, not ACL).
// Operator passes `--mesh-route-subnet=10.99.0.0/24`; bundle-create must
// populate AllowedIPs on EVERY KnownEndpoint as
// [MeshAddr/32, WGCP0FleetSubnet, 10.99.0.0/24] — canonical pair first,
// operator extras additive. Pre-ADR-0321 (Block 22K.7 / F-114 era) the
// operator flag was the only path to a non-MeshAddr/32 AllowedIPs and
// gated the catchall entirely; that produced the asymmetric-/24
// failure mode F-116 closes.
func TestRun_MeshRouteSubnet(t *testing.T) {
	caKey, ageID, caPub := devCA(t)
	relaysPath := fixtureRelays(t)
	outPath := filepath.Join(t.TempDir(), "bundle.cbor")

	args := runArgs{
		Site:               "lab",
		DeviceID:           "mgmt-prov-site3-01-orin",
		WGCP0Address:       "10.99.0.6/24",
		RelaysPath:         relaysPath,
		ACLGroupsRaw:       "LAB",
		CAID:               "dev-goat-ca-test",
		CAKeyPath:          caKey,
		AgeIdentityPath:    ageID,
		MeshRouteSubnetRaw: "10.99.0.0/24",
		OutPath:            outPath,
		TTLDays:            defaultTTLDays,
		ActivationDays:     defaultActivationDays,
		Now:                fixedNow(),
	}
	if err := run(args); err != nil {
		t.Fatalf("run: %v", err)
	}
	wire, _ := os.ReadFile(outPath)
	b, err := bundle.Unmarshal(wire)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if err := b.Verify(caPub); err != nil {
		t.Fatalf("verify: %v", err)
	}

	// Every endpoint: AllowedIPs = [MeshAddr/32, WGCP0FleetSubnet, 10.99.0.0/24].
	if len(b.KnownEndpoints) == 0 {
		t.Fatal("expected at least one KnownEndpoint")
	}
	for i, ep := range b.KnownEndpoints {
		want := []string{ep.MeshAddr + "/32", bundle.WGCP0FleetSubnet, "10.99.0.0/24"}
		if len(ep.AllowedIPs) != len(want) {
			t.Fatalf("endpoint[%d] AllowedIPs len = %d; want %d (got %v)", i, len(ep.AllowedIPs), len(want), ep.AllowedIPs)
		}
		for j, w := range want {
			if ep.AllowedIPs[j] != w {
				t.Errorf("endpoint[%d] AllowedIPs[%d] = %q; want %q", i, j, ep.AllowedIPs[j], w)
			}
		}
	}
}

// TestRun_FirstRelayRouteSubnet_BackCompat — the deprecated alias still
// accepts input and produces the ADR-0321 canonical-plus-extras shape.
// Existing operator scripts passing the old flag continue to mint
// working bundles. The pre-F-114 first-relay-only shape is unreachable
// and the pre-ADR-0321 catchall-gated shape is also unreachable.
func TestRun_FirstRelayRouteSubnet_BackCompat(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	outPath := filepath.Join(t.TempDir(), "bundle.cbor")

	args := runArgs{
		Site:                     "lab",
		DeviceID:                 "mgmt-prov-site3-01-orin",
		WGCP0Address:             "10.99.0.6/24",
		RelaysPath:               relaysPath,
		ACLGroupsRaw:             "LAB",
		CAID:                     "dev-goat-ca-test",
		CAKeyPath:                caKey,
		AgeIdentityPath:          ageID,
		FirstRelayRouteSubnetRaw: "10.99.0.0/24",
		OutPath:                  outPath,
		TTLDays:                  defaultTTLDays,
		ActivationDays:           defaultActivationDays,
		Now:                      fixedNow(),
	}
	if err := run(args); err != nil {
		t.Fatalf("run: %v", err)
	}
	wire, _ := os.ReadFile(outPath)
	b, _ := bundle.Unmarshal(wire)

	// Every endpoint carries the canonical pair + the extra from the
	// deprecated flag, in that order.
	for i, ep := range b.KnownEndpoints {
		want := []string{ep.MeshAddr + "/32", bundle.WGCP0FleetSubnet, "10.99.0.0/24"}
		if len(ep.AllowedIPs) != len(want) {
			t.Fatalf("endpoint[%d] AllowedIPs len = %d; want %d (got %v)", i, len(ep.AllowedIPs), len(want), ep.AllowedIPs)
		}
		for j, w := range want {
			if ep.AllowedIPs[j] != w {
				t.Errorf("endpoint[%d] AllowedIPs[%d] = %q; want %q", i, j, ep.AllowedIPs[j], w)
			}
		}
	}
}

// TestRun_RejectsInvalidMeshRouteSubnet — operator typos (e.g.
// "10.99.0/24" missing an octet) should fail-fast at the issuance
// step rather than producing a bundle that wg-cp0-bundle-apply will
// reject downstream on the device.
func TestRun_RejectsInvalidMeshRouteSubnet(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	cases := []struct {
		name string
		raw  string
	}{
		{"missing prefix", "10.99.0.0"},
		{"junk", "not-a-cidr"},
		{"empty after split", ",,,"},
		{"truncated", "10.99.0/24"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			args := runArgs{
				Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.6/24",
				RelaysPath: relaysPath, ACLGroupsRaw: "LAB",
				CAID: "dev-goat-ca-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
				MeshRouteSubnetRaw: tc.raw,
				OutPath:                  filepath.Join(t.TempDir(), "out.cbor"),
				TTLDays:                  defaultTTLDays, ActivationDays: defaultActivationDays,
				Now: fixedNow(),
			}
			if err := run(args); err == nil {
				t.Errorf("expected error for raw=%q", tc.raw)
			}
		})
	}
}

// TestRun_NoMeshRouteSubnet — empty flag → canonical pair only.
// ADR 0321: every bundle's relay peer block carries the canonical
// AllowedIPs = [MeshAddr/32, WGCP0FleetSubnet] regardless of whether
// the operator passes --mesh-route-subnet. Pre-ADR-0321 the empty flag
// produced no AllowedIPs and the device-side fallback emitted
// MeshAddr/32-only, which was the asymmetric default F-116 closes.
func TestRun_NoMeshRouteSubnet(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	outPath := filepath.Join(t.TempDir(), "bundle.cbor")
	args := runArgs{
		Site: "lab", DeviceID: "x", WGCP0Address: "10.99.0.6/24",
		RelaysPath: relaysPath, ACLGroupsRaw: "LAB",
		CAID: "dev-goat-ca-test", CAKeyPath: caKey, AgeIdentityPath: ageID,
		// MeshRouteSubnetRaw not set
		OutPath: outPath,
		TTLDays: defaultTTLDays, ActivationDays: defaultActivationDays,
		Now: fixedNow(),
	}
	if err := run(args); err != nil {
		t.Fatalf("run: %v", err)
	}
	wire, _ := os.ReadFile(outPath)
	b, _ := bundle.Unmarshal(wire)
	for i, ep := range b.KnownEndpoints {
		want := []string{ep.MeshAddr + "/32", bundle.WGCP0FleetSubnet}
		if len(ep.AllowedIPs) != len(want) {
			t.Fatalf("endpoint[%d] AllowedIPs len = %d; want %d (got %v)", i, len(ep.AllowedIPs), len(want), ep.AllowedIPs)
		}
		for j, w := range want {
			if ep.AllowedIPs[j] != w {
				t.Errorf("endpoint[%d] AllowedIPs[%d] = %q; want %q", i, j, ep.AllowedIPs[j], w)
			}
		}
	}
}

// TestRun_BundleSymmetryInvariant — ADR 1017 first instance applied at
// unit-test scope. Asserts that for any combination of relay count,
// operator flags, and bundle invocation, every relay endpoint's
// AllowedIPs starts with [MeshAddr/32, WGCP0FleetSubnet]. This is the
// load-bearing structural invariant from ADR 0321 §"Decision" §1: the
// substrate cannot drift back to the asymmetric-/24 failure mode
// without tripping this test.
func TestRun_BundleSymmetryInvariant(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	relaysPath := fixtureRelays(t)
	cases := []struct {
		name string
		args runArgs
	}{
		{"no flags", runArgs{}},
		{"mesh-route-subnet", runArgs{MeshRouteSubnetRaw: "10.99.0.0/24"}},
		{"first-relay-route-subnet (deprecated)", runArgs{FirstRelayRouteSubnetRaw: "10.99.0.0/24"}},
		{"multiple extras", runArgs{MeshRouteSubnetRaw: "10.99.0.0/24,192.168.0.0/16"}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			outPath := filepath.Join(t.TempDir(), "bundle.cbor")
			a := tc.args
			a.Site = "lab"
			a.DeviceID = "test-device"
			a.WGCP0Address = "10.99.0.6/24"
			a.RelaysPath = relaysPath
			a.ACLGroupsRaw = "LAB"
			a.CAID = "dev-goat-ca-test"
			a.CAKeyPath = caKey
			a.AgeIdentityPath = ageID
			a.OutPath = outPath
			a.TTLDays = defaultTTLDays
			a.ActivationDays = defaultActivationDays
			a.Now = fixedNow()
			if err := run(a); err != nil {
				t.Fatalf("run: %v", err)
			}
			wire, _ := os.ReadFile(outPath)
			b, _ := bundle.Unmarshal(wire)
			if len(b.KnownEndpoints) < 2 {
				t.Fatalf("fixture must have >= 2 relays to assert symmetry; got %d", len(b.KnownEndpoints))
			}
			for i, ep := range b.KnownEndpoints {
				if len(ep.AllowedIPs) < 2 {
					t.Fatalf("endpoint[%d] AllowedIPs len = %d; want at least 2 (canonical pair)", i, len(ep.AllowedIPs))
				}
				if got, want := ep.AllowedIPs[0], ep.MeshAddr+"/32"; got != want {
					t.Errorf("endpoint[%d] AllowedIPs[0] = %q; want %q (MeshAddr/32)", i, got, want)
				}
				if got, want := ep.AllowedIPs[1], bundle.WGCP0FleetSubnet; got != want {
					t.Errorf("endpoint[%d] AllowedIPs[1] = %q; want %q (WGCP0FleetSubnet)", i, got, want)
				}
			}
		})
	}
}

// TestParseMeshRouteSubnets unit-tests the helper directly.
func TestParseMeshRouteSubnets(t *testing.T) {
	t.Run("empty returns nil", func(t *testing.T) {
		got, err := parseMeshRouteSubnets("")
		if err != nil {
			t.Errorf("err = %v; want nil", err)
		}
		if got != nil {
			t.Errorf("got = %v; want nil", got)
		}
	})
	t.Run("whitespace-only returns nil", func(t *testing.T) {
		got, err := parseMeshRouteSubnets("   \t  ")
		if err != nil {
			t.Errorf("err = %v; want nil", err)
		}
		if got != nil {
			t.Errorf("got = %v; want nil", got)
		}
	})
	t.Run("single CIDR", func(t *testing.T) {
		got, err := parseMeshRouteSubnets("10.99.0.0/24")
		if err != nil {
			t.Fatalf("err = %v", err)
		}
		if len(got) != 1 || got[0] != "10.99.0.0/24" {
			t.Errorf("got = %v; want [10.99.0.0/24]", got)
		}
	})
	t.Run("multiple comma-separated", func(t *testing.T) {
		got, err := parseMeshRouteSubnets("10.99.0.0/24, 192.168.0.0/16")
		if err != nil {
			t.Fatalf("err = %v", err)
		}
		want := []string{"10.99.0.0/24", "192.168.0.0/16"}
		if len(got) != len(want) {
			t.Fatalf("got = %v; want %v", got, want)
		}
		for i := range want {
			if got[i] != want[i] {
				t.Errorf("[%d] got %q want %q", i, got[i], want[i])
			}
		}
	})
	t.Run("canonicalizes input", func(t *testing.T) {
		// 10.99.0.5/24 has host bits set; net.ParseCIDR canonicalizes
		// the network portion to 10.99.0.0/24. Issuer + validator
		// agreement requires we use the canonical form.
		got, err := parseMeshRouteSubnets("10.99.0.5/24")
		if err != nil {
			t.Fatalf("err = %v", err)
		}
		if len(got) != 1 || got[0] != "10.99.0.0/24" {
			t.Errorf("got = %v; want [10.99.0.0/24] (canonical form)", got)
		}
	})
	t.Run("rejects junk", func(t *testing.T) {
		if _, err := parseMeshRouteSubnets("not-a-cidr"); err == nil {
			t.Error("expected error for junk input")
		}
	})
}
