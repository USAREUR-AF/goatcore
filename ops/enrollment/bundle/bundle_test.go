package bundle

import (
	"bytes"
	"crypto/rand"
	"errors"
	"testing"
	"time"
)

func fixtureBundle(t *testing.T) *EnrollmentBundle {
	t.Helper()
	// PeerPubkey is a WireGuard pubkey (32 bytes); a random buffer is
	// sufficient for round-trip / sign / verify tests since the bundle
	// package doesn't validate it as a real Curve25519 derivation.
	pub := make([]byte, 32)
	if _, err := rand.Read(pub); err != nil {
		t.Fatalf("gen peer key: %v", err)
	}
	issued := time.Date(2026, 4, 20, 12, 0, 0, 0, time.UTC)
	nonce := make([]byte, 16)
	if _, err := rand.Read(nonce); err != nil {
		t.Fatalf("gen nonce: %v", err)
	}
	return &EnrollmentBundle{
		Version:            Version,
		DeviceID:           "orin-region-a-01.c2",
		PeerPubkey:         pub,
		ACLGroups:          []string{"c2", "engagement-authority"},
		Site:               "region-a-cp",
		KnownEndpoints: []KnownEndpoint{
			{Addr: "rla-relay.example.net:51820", Pubkey: pub, Kind: KindRelay},
			{Addr: "10.0.0.5:51820", Pubkey: pub, Kind: KindPeer},
		},
		IssuedAt:           issued,
		ActivationDeadline: issued.Add(14 * 24 * time.Hour),
		ExpiresAt:          issued.Add(90 * 24 * time.Hour),
		Nonce:              nonce,
		CAID:               "dev-goat-ca-2026-04",
	}
}

func TestBundleRoundtrip(t *testing.T) {
	b := fixtureBundle(t)
	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	got, err := Unmarshal(data)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got.DeviceID != b.DeviceID {
		t.Errorf("DeviceID: got %q want %q", got.DeviceID, b.DeviceID)
	}
	if !bytes.Equal(got.Nonce, b.Nonce) {
		t.Errorf("Nonce: got %x want %x", got.Nonce, b.Nonce)
	}
	if !got.IssuedAt.Equal(b.IssuedAt) {
		t.Errorf("IssuedAt: got %s want %s", got.IssuedAt, b.IssuedAt)
	}
	if !got.ActivationDeadline.Equal(b.ActivationDeadline) {
		t.Errorf("ActivationDeadline: got %s want %s", got.ActivationDeadline, b.ActivationDeadline)
	}
	if !got.ExpiresAt.Equal(b.ExpiresAt) {
		t.Errorf("ExpiresAt: got %s want %s", got.ExpiresAt, b.ExpiresAt)
	}
	if len(got.ACLGroups) != len(b.ACLGroups) {
		t.Errorf("ACLGroups length: got %d want %d", len(got.ACLGroups), len(b.ACLGroups))
	}
	if len(got.KnownEndpoints) != len(b.KnownEndpoints) {
		t.Errorf("KnownEndpoints length: got %d want %d", len(got.KnownEndpoints), len(b.KnownEndpoints))
	}
}

func TestBundleCanonicalDeterministic(t *testing.T) {
	b := fixtureBundle(t)
	first, err := b.Marshal()
	if err != nil {
		t.Fatalf("first marshal: %v", err)
	}
	for i := 0; i < 10; i++ {
		again, err := b.Marshal()
		if err != nil {
			t.Fatalf("marshal %d: %v", i, err)
		}
		if !bytes.Equal(first, again) {
			t.Fatalf("iteration %d produced different bytes", i)
		}
	}
}

func TestBundleSignVerify(t *testing.T) {
	b := fixtureBundle(t)
	caPub, sign := testCAKey(t)

	if err := b.Sign(sign); err != nil {
		t.Fatalf("sign: %v", err)
	}
	if len(b.Signature) == 0 || len(b.Signature) > 256 {
		t.Fatalf("signature wrong size: got %d want 1-256 (ECDSA ASN.1 DER)", len(b.Signature))
	}
	if err := b.Verify(caPub); err != nil {
		t.Errorf("verify should succeed: %v", err)
	}
}

func TestBundleVerifyWrongKey(t *testing.T) {
	b := fixtureBundle(t)
	_, sign := testCAKey(t)

	if err := b.Sign(sign); err != nil {
		t.Fatalf("sign: %v", err)
	}
	otherPub, _ := testCAKey(t)
	if err := b.Verify(otherPub); err == nil {
		t.Error("verify should fail with different public key")
	}
}

func TestBundleVerifyTamperedPayload(t *testing.T) {
	b := fixtureBundle(t)
	caPub, sign := testCAKey(t)
	if err := b.Sign(sign); err != nil {
		t.Fatalf("sign: %v", err)
	}
	// Tamper: escalate ACL groups after signing.
	b.ACLGroups = append(b.ACLGroups, "admin")
	if err := b.Verify(caPub); err == nil {
		t.Error("verify should fail after payload tamper")
	}
}

func TestBundleVerifyTamperedSignature(t *testing.T) {
	b := fixtureBundle(t)
	caPub, sign := testCAKey(t)
	if err := b.Sign(sign); err != nil {
		t.Fatalf("sign: %v", err)
	}
	b.Signature[0] ^= 0x01
	if err := b.Verify(caPub); err == nil {
		t.Error("verify should fail after signature tamper")
	}
}

func TestBundleCheckExpiry(t *testing.T) {
	b := fixtureBundle(t)
	before := b.ExpiresAt.Add(-time.Hour)
	if err := b.CheckExpiry(before); err != nil {
		t.Errorf("expiry check before deadline should pass: %v", err)
	}
	after := b.ExpiresAt.Add(time.Hour)
	err := b.CheckExpiry(after)
	if err == nil {
		t.Fatal("expiry check after deadline should fail")
	}
	if !errors.Is(err, ErrExpired) {
		t.Errorf("expected ErrExpired, got %v", err)
	}
}

func TestBundleCheckActivationDeadline(t *testing.T) {
	b := fixtureBundle(t)
	before := b.ActivationDeadline.Add(-time.Hour)
	if err := b.CheckActivationDeadline(before); err != nil {
		t.Errorf("activation check before deadline should pass: %v", err)
	}
	after := b.ActivationDeadline.Add(time.Hour)
	err := b.CheckActivationDeadline(after)
	if err == nil {
		t.Fatal("activation check after deadline should fail")
	}
	if !errors.Is(err, ErrActivationDeadlinePassed) {
		t.Errorf("expected ErrActivationDeadlinePassed, got %v", err)
	}
}

// TestBundleSnitchProbeKey_Roundtrip asserts that the new optional
// snitch_probe_key field round-trips through CBOR + survives signing.
// Block 61 / docs/design/offline-enrollment.md §11.6a.
func TestBundleSnitchProbeKey_Roundtrip(t *testing.T) {
	b := fixtureBundle(t)
	probeKey := make([]byte, 32)
	if _, err := rand.Read(probeKey); err != nil {
		t.Fatalf("gen probe key: %v", err)
	}
	b.SnitchProbeKey = probeKey

	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	got, err := Unmarshal(data)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if !bytes.Equal(got.SnitchProbeKey, probeKey) {
		t.Errorf("SnitchProbeKey: got %x want %x", got.SnitchProbeKey, probeKey)
	}
}

// TestBundleSnitchProbeKey_OmitEmpty asserts that bundles without a
// snitch_probe_key (the legacy / non-snitch case) DO NOT carry the
// field on the wire — operators issuing pre-Block-61 bundles or
// non-snitch deployments must not waste bytes or surprise old
// validators.
func TestBundleSnitchProbeKey_OmitEmpty(t *testing.T) {
	b := fixtureBundle(t)
	b.SnitchProbeKey = nil // explicit
	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	// CBOR canonical encoding of "snitch_probe_key" key = 0x70 + 16 bytes
	// of UTF-8 "snitch_probe_key"; we just check absence of the literal.
	if bytes.Contains(data, []byte("snitch_probe_key")) {
		t.Error("nil SnitchProbeKey should not appear in canonical encoding")
	}
}

// TestBundleSnitchProbeKey_TamperDetected asserts that mutating the
// snitch_probe_key after signing invalidates the signature — the new
// field IS in the signed payload, not bolted on outside it.
func TestBundleSnitchProbeKey_TamperDetected(t *testing.T) {
	b := fixtureBundle(t)
	b.SnitchProbeKey = make([]byte, 32)
	b.SnitchProbeKey[0] = 0x42
	caPub, sign := testCAKey(t)
	if err := b.Sign(sign); err != nil {
		t.Fatalf("sign: %v", err)
	}
	// Tamper the probe key after signing.
	b.SnitchProbeKey[0] = 0x43
	if err := b.Verify(caPub); err == nil {
		t.Error("verify should fail after snitch_probe_key tamper")
	}
}

// TestBundleTLSTrustAnchor_Roundtrip mirrors the snitch_probe_key shape
// for the new tls_trust_anchor field added by Block 22J / ADR 0407.
// The DER-encoded root cert round-trips through canonical CBOR
// encode/decode without modification.
func TestBundleTLSTrustAnchor_Roundtrip(t *testing.T) {
	b := fixtureBundle(t)
	// Synthetic DER bytes — the bundle layer doesn't validate cert structure;
	// it just round-trips opaque bytes.
	der := make([]byte, 512)
	if _, err := rand.Read(der); err != nil {
		t.Fatalf("gen der bytes: %v", err)
	}
	b.TLSTrustAnchor = der

	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	got, err := Unmarshal(data)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if !bytes.Equal(got.TLSTrustAnchor, der) {
		t.Errorf("TLSTrustAnchor: got len=%d want len=%d", len(got.TLSTrustAnchor), len(der))
	}
}

// TestBundleTLSTrustAnchor_OmitEmpty asserts that bundles issued without
// an internal TLS CA (legacy dev/test, pre-Block-22J bundles) DO NOT
// carry the field on the wire.
func TestBundleTLSTrustAnchor_OmitEmpty(t *testing.T) {
	b := fixtureBundle(t)
	b.TLSTrustAnchor = nil
	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if bytes.Contains(data, []byte("tls_trust_anchor")) {
		t.Error("nil TLSTrustAnchor should not appear in canonical encoding")
	}
}

// TestBundleTLSTrustAnchor_TamperDetected asserts that mutating the
// trust anchor after signing invalidates the signature — the field is
// inside the signed payload, not bolted on outside it.
func TestBundleTLSTrustAnchor_TamperDetected(t *testing.T) {
	b := fixtureBundle(t)
	b.TLSTrustAnchor = []byte("forged-trust-anchor-bytes")
	caPub, sign := testCAKey(t)
	if err := b.Sign(sign); err != nil {
		t.Fatalf("sign: %v", err)
	}
	// Tamper.
	b.TLSTrustAnchor[0] = 0xff
	if err := b.Verify(caPub); err == nil {
		t.Error("verify should fail after tls_trust_anchor tamper")
	}
}

// TestBundleCPDeviceKeypair_Roundtrip mirrors the SnitchProbeKey shape for
// the new wg-cp0 keypair fields added by Block 22K. Both halves of the
// keypair round-trip through canonical CBOR encode/decode without
// modification. Block 22K / docs/design/offline-enrollment.md §11.2.
func TestBundleCPDeviceKeypair_Roundtrip(t *testing.T) {
	b := fixtureBundle(t)
	cpPub := make([]byte, 32)
	cpPriv := make([]byte, 32)
	if _, err := rand.Read(cpPub); err != nil {
		t.Fatalf("gen cp pub: %v", err)
	}
	if _, err := rand.Read(cpPriv); err != nil {
		t.Fatalf("gen cp priv: %v", err)
	}
	b.CPDevicePubkey = cpPub
	b.CPDevicePrivkey = cpPriv

	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	got, err := Unmarshal(data)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if !bytes.Equal(got.CPDevicePubkey, cpPub) {
		t.Errorf("CPDevicePubkey: got %x want %x", got.CPDevicePubkey, cpPub)
	}
	if !bytes.Equal(got.CPDevicePrivkey, cpPriv) {
		t.Errorf("CPDevicePrivkey: got %x want %x", got.CPDevicePrivkey, cpPriv)
	}
}

// TestBundleCPDeviceKeypair_OmitEmpty asserts that bundles issued without
// wg-cp0 in scope (legacy pre-Block-22 bundles, non-control-plane peers)
// DO NOT carry the fields on the wire.
func TestBundleCPDeviceKeypair_OmitEmpty(t *testing.T) {
	b := fixtureBundle(t)
	b.CPDevicePubkey = nil
	b.CPDevicePrivkey = nil
	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if bytes.Contains(data, []byte("cp_device_pubkey")) {
		t.Error("nil CPDevicePubkey should not appear in canonical encoding")
	}
	if bytes.Contains(data, []byte("cp_device_privkey")) {
		t.Error("nil CPDevicePrivkey should not appear in canonical encoding")
	}
}

// TestBundleCPDeviceKeypair_TamperDetected asserts that mutating either
// half of the keypair after signing invalidates the signature — the
// fields are inside the signed payload, not bolted on outside it.
func TestBundleCPDeviceKeypair_TamperDetected(t *testing.T) {
	for _, tc := range []struct {
		name   string
		tamper func(*EnrollmentBundle)
	}{
		{"pubkey", func(b *EnrollmentBundle) { b.CPDevicePubkey[0] ^= 0xff }},
		{"privkey", func(b *EnrollmentBundle) { b.CPDevicePrivkey[0] ^= 0xff }},
	} {
		t.Run(tc.name, func(t *testing.T) {
			b := fixtureBundle(t)
			b.CPDevicePubkey = make([]byte, 32)
			b.CPDevicePrivkey = make([]byte, 32)
			b.CPDevicePubkey[0] = 0x42
			b.CPDevicePrivkey[0] = 0x43
			caPub, sign := testCAKey(t)
			if err := b.Sign(sign); err != nil {
				t.Fatalf("sign: %v", err)
			}
			tc.tamper(b)
			if err := b.Verify(caPub); err == nil {
				t.Errorf("verify should fail after %s tamper", tc.name)
			}
		})
	}
}

// TestBundleCPDeviceAddress_Roundtrip covers the optional CIDR string
// field that carries the device's wg-cp0 interface address. Block 22K.
func TestBundleCPDeviceAddress_Roundtrip(t *testing.T) {
	b := fixtureBundle(t)
	b.CPDeviceAddress = "10.99.0.6/24"
	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	got, err := Unmarshal(data)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got.CPDeviceAddress != "10.99.0.6/24" {
		t.Errorf("CPDeviceAddress = %q; want 10.99.0.6/24", got.CPDeviceAddress)
	}
}

// TestBundleCPDeviceAddress_OmitEmpty asserts that bundles issued
// without wg-cp0 in scope don't carry the empty string on the wire.
func TestBundleCPDeviceAddress_OmitEmpty(t *testing.T) {
	b := fixtureBundle(t)
	b.CPDeviceAddress = ""
	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if bytes.Contains(data, []byte("cp_device_address")) {
		t.Error("empty CPDeviceAddress should not appear in canonical encoding")
	}
}

// TestKnownEndpointMeshAddr_Roundtrip covers the optional MeshAddr
// field on cp-relay endpoints. Block 22K.
func TestKnownEndpointMeshAddr_Roundtrip(t *testing.T) {
	b := fixtureBundle(t)
	pub := make([]byte, 32)
	pub[0] = 0xa1
	b.KnownEndpoints = []KnownEndpoint{
		{Addr: "203.0.113.1:51821", Pubkey: pub, Kind: Kind("cp-relay"), MeshAddr: "10.99.0.2"},
		// Mixed: a non-cp-relay endpoint with no MeshAddr (omitempty).
		{Addr: "fra.example:51820", Pubkey: pub, Kind: KindRelay},
	}
	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	got, err := Unmarshal(data)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got.KnownEndpoints[0].MeshAddr != "10.99.0.2" {
		t.Errorf("KnownEndpoints[0].MeshAddr = %q; want 10.99.0.2", got.KnownEndpoints[0].MeshAddr)
	}
	if got.KnownEndpoints[1].MeshAddr != "" {
		t.Errorf("KnownEndpoints[1].MeshAddr = %q; want empty", got.KnownEndpoints[1].MeshAddr)
	}
}

// TestBundleCheckCPDeviceKeypair covers the pairing invariant: both
// fields present (wg-cp0 bundle) or both absent (legacy / non-cp peer).
// Exactly-one-of is rejected so the device-side render tool never sees
// a half-populated bundle.
func TestBundleCheckCPDeviceKeypair(t *testing.T) {
	for _, tc := range []struct {
		name    string
		pub     []byte
		priv    []byte
		wantErr bool
	}{
		{"both absent", nil, nil, false},
		{"both present", make([]byte, 32), make([]byte, 32), false},
		{"pub only", make([]byte, 32), nil, true},
		{"priv only", nil, make([]byte, 32), true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			b := fixtureBundle(t)
			b.CPDevicePubkey = tc.pub
			b.CPDevicePrivkey = tc.priv
			err := b.CheckCPDeviceKeypair()
			if tc.wantErr {
				if err == nil {
					t.Error("expected ErrCPDeviceKeypairUnpaired")
				}
				if !errors.Is(err, ErrCPDeviceKeypairUnpaired) {
					t.Errorf("wrong error type: %v", err)
				}
			} else if err != nil {
				t.Errorf("unexpected error: %v", err)
			}
		})
	}
}

// TestKnownEndpointAllowedIPs_Roundtrip — Block 22K.7. The optional
// AllowedIPs override on KnownEndpoint round-trips through canonical
// CBOR. Use case: route the wg-cp0 control-plane subnet through the
// first relay so client-to-client traffic transits via Linux IP
// forwarding on the relay.
func TestKnownEndpointAllowedIPs_Roundtrip(t *testing.T) {
	b := fixtureBundle(t)
	pub := b.KnownEndpoints[0].Pubkey
	b.KnownEndpoints = []KnownEndpoint{
		{
			Addr:       "rla-relay.example.net:51820",
			Pubkey:     pub,
			Kind:       Kind("cp-relay"),
			MeshAddr:   "10.99.0.2",
			AllowedIPs: []string{"10.99.0.2/32", "10.99.0.0/24"},
		},
		{
			Addr:     "rlb-relay.example.net:51820",
			Pubkey:   pub,
			Kind:     Kind("cp-relay"),
			MeshAddr: "10.99.0.3",
		},
	}
	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	got, err := Unmarshal(data)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if len(got.KnownEndpoints) != 2 {
		t.Fatalf("KnownEndpoints length: got %d want 2", len(got.KnownEndpoints))
	}
	if len(got.KnownEndpoints[0].AllowedIPs) != 2 {
		t.Fatalf("first endpoint AllowedIPs length: got %d want 2",
			len(got.KnownEndpoints[0].AllowedIPs))
	}
	want := []string{"10.99.0.2/32", "10.99.0.0/24"}
	for i, w := range want {
		if got.KnownEndpoints[0].AllowedIPs[i] != w {
			t.Errorf("AllowedIPs[%d]: got %q want %q", i,
				got.KnownEndpoints[0].AllowedIPs[i], w)
		}
	}
	if got.KnownEndpoints[1].AllowedIPs != nil {
		t.Errorf("second endpoint should have empty AllowedIPs (omit-empty), got %v",
			got.KnownEndpoints[1].AllowedIPs)
	}
}

// TestKnownEndpointAllowedIPs_OmitEmpty asserts that a KnownEndpoint
// with no AllowedIPs override does not carry the field on the wire.
// Backward-compat: pre-Block-22K.7 bundles + non-route-subnet
// deployments stay byte-identical.
func TestKnownEndpointAllowedIPs_OmitEmpty(t *testing.T) {
	b := fixtureBundle(t)
	// fixtureBundle's endpoints don't set AllowedIPs.
	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if bytes.Contains(data, []byte("allowed_ips")) {
		t.Error("nil AllowedIPs should not appear in canonical encoding")
	}
}

// TestKnownEndpointAllowedIPs_TamperDetected — the new field is part
// of the signed payload, not bolted on outside it. Mutating it after
// signing must invalidate the signature.
func TestKnownEndpointAllowedIPs_TamperDetected(t *testing.T) {
	b := fixtureBundle(t)
	pub := b.KnownEndpoints[0].Pubkey
	b.KnownEndpoints = []KnownEndpoint{
		{
			Addr:       "rla-relay.example.net:51820",
			Pubkey:     pub,
			Kind:       Kind("cp-relay"),
			MeshAddr:   "10.99.0.2",
			AllowedIPs: []string{"10.99.0.2/32", "10.99.0.0/24"},
		},
	}
	caPub, sign := testCAKey(t)
	if err := b.Sign(sign); err != nil {
		t.Fatalf("sign: %v", err)
	}
	// Tamper AllowedIPs after signing.
	b.KnownEndpoints[0].AllowedIPs = append(b.KnownEndpoints[0].AllowedIPs, "0.0.0.0/0")
	if err := b.Verify(caPub); err == nil {
		t.Error("verify should fail after AllowedIPs tamper")
	}
}

// fixtureGoatCLIExtras returns a non-empty GoatCLIExtras struct with
// deterministic byte content so round-trip + tamper tests can assert
// equality without needing reflect.DeepEqual on every sub-field.
func fixtureGoatCLIExtras() GoatCLIExtras {
	return GoatCLIExtras{
		PortalEndpoint: "https://portal.nebula.netbird.nebula.example",
		PortalToken:    []byte("opaque-bearer-token-32-bytes-pad"),
		RouterEndpoint: "tls/192.0.2.10:7447",
		MTLSCert:       []byte("-----BEGIN CERTIFICATE-----\nMIIB...AB\n-----END CERTIFICATE-----\n"),
		MTLSKey:        []byte("-----BEGIN PRIVATE KEY-----\nMIIE...AB\n-----END PRIVATE KEY-----\n"),
		CARoots:        []byte("-----BEGIN CERTIFICATE-----\nMIIC...AB\n-----END CERTIFICATE-----\n"),
	}
}

// TestBundleGoatCLIExtras_Roundtrip — the GoatCLIExtras nested field
// round-trips through canonical CBOR encode/decode without loss.
// Block 84-ish / ADR 1019 / docs/design/goat-cli.md.
func TestBundleGoatCLIExtras_Roundtrip(t *testing.T) {
	b := fixtureBundle(t)
	b.GoatCLIExtras = fixtureGoatCLIExtras()

	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	got, err := Unmarshal(data)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got.GoatCLIExtras.PortalEndpoint != b.GoatCLIExtras.PortalEndpoint {
		t.Errorf("PortalEndpoint: got %q want %q",
			got.GoatCLIExtras.PortalEndpoint, b.GoatCLIExtras.PortalEndpoint)
	}
	if got.GoatCLIExtras.RouterEndpoint != b.GoatCLIExtras.RouterEndpoint {
		t.Errorf("RouterEndpoint: got %q want %q",
			got.GoatCLIExtras.RouterEndpoint, b.GoatCLIExtras.RouterEndpoint)
	}
	if !bytes.Equal(got.GoatCLIExtras.PortalToken, b.GoatCLIExtras.PortalToken) {
		t.Errorf("PortalToken: got %x want %x",
			got.GoatCLIExtras.PortalToken, b.GoatCLIExtras.PortalToken)
	}
	if !bytes.Equal(got.GoatCLIExtras.MTLSCert, b.GoatCLIExtras.MTLSCert) {
		t.Errorf("MTLSCert mismatch (len got=%d want=%d)",
			len(got.GoatCLIExtras.MTLSCert), len(b.GoatCLIExtras.MTLSCert))
	}
	if !bytes.Equal(got.GoatCLIExtras.MTLSKey, b.GoatCLIExtras.MTLSKey) {
		t.Errorf("MTLSKey mismatch (len got=%d want=%d)",
			len(got.GoatCLIExtras.MTLSKey), len(b.GoatCLIExtras.MTLSKey))
	}
	if !bytes.Equal(got.GoatCLIExtras.CARoots, b.GoatCLIExtras.CARoots) {
		t.Errorf("CARoots mismatch (len got=%d want=%d)",
			len(got.GoatCLIExtras.CARoots), len(b.GoatCLIExtras.CARoots))
	}
}

// TestBundleGoatCLIExtras_OmitEmpty asserts that bundles issued
// without goat-cli operator material (device-enrollment, relay
// bundles, pre-Block-84 bundles in the wild) DO NOT carry the
// "goat_cli_extras" map on the wire. Wire-compat property: a v0.2
// device-enrollment bundle round-trips byte-identically through
// parser code that knows about the new field.
func TestBundleGoatCLIExtras_OmitEmpty(t *testing.T) {
	b := fixtureBundle(t)
	// Explicit zero value — the omitempty on the parent field tag
	// must drop the whole nested map.
	b.GoatCLIExtras = GoatCLIExtras{}
	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if bytes.Contains(data, []byte("goat_cli_extras")) {
		t.Error("zero-value GoatCLIExtras should not appear in canonical encoding")
	}
	if bytes.Contains(data, []byte("portal_endpoint")) {
		t.Error("zero-value GoatCLIExtras leaked sub-field key portal_endpoint")
	}
	if bytes.Contains(data, []byte("router_endpoint")) {
		t.Error("zero-value GoatCLIExtras leaked sub-field key router_endpoint")
	}
}

// TestBundleGoatCLIExtras_TamperDetected — the new field is part of
// the signed payload, not bolted on outside it. Mutating any
// sub-field after signing must invalidate the signature.
func TestBundleGoatCLIExtras_TamperDetected(t *testing.T) {
	for _, tc := range []struct {
		name   string
		tamper func(*EnrollmentBundle)
	}{
		{"portal_endpoint", func(b *EnrollmentBundle) {
			b.GoatCLIExtras.PortalEndpoint = "https://attacker.example/"
		}},
		{"router_endpoint", func(b *EnrollmentBundle) {
			b.GoatCLIExtras.RouterEndpoint = "tls/10.0.0.99:7447"
		}},
		{"portal_token", func(b *EnrollmentBundle) {
			b.GoatCLIExtras.PortalToken[0] ^= 0xff
		}},
		{"mtls_cert", func(b *EnrollmentBundle) {
			b.GoatCLIExtras.MTLSCert[0] ^= 0xff
		}},
		{"mtls_key", func(b *EnrollmentBundle) {
			b.GoatCLIExtras.MTLSKey[0] ^= 0xff
		}},
		{"ca_roots", func(b *EnrollmentBundle) {
			b.GoatCLIExtras.CARoots[0] ^= 0xff
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			b := fixtureBundle(t)
			b.GoatCLIExtras = fixtureGoatCLIExtras()
			caPub, sign := testCAKey(t)
			if err := b.Sign(sign); err != nil {
				t.Fatalf("sign: %v", err)
			}
			tc.tamper(b)
			if err := b.Verify(caPub); err == nil {
				t.Errorf("verify should fail after %s tamper", tc.name)
			}
		})
	}
}

// TestBundleHasGoatCLIExtras covers the eligibility helper.
// Full population → true; any required sub-field missing → false.
func TestBundleHasGoatCLIExtras(t *testing.T) {
	full := fixtureGoatCLIExtras()
	for _, tc := range []struct {
		name string
		mut  func(*GoatCLIExtras)
		want bool
	}{
		{"full population", func(*GoatCLIExtras) {}, true},
		{"missing portal endpoint", func(x *GoatCLIExtras) { x.PortalEndpoint = "" }, false},
		{"missing router endpoint", func(x *GoatCLIExtras) { x.RouterEndpoint = "" }, false},
		{"missing portal token", func(x *GoatCLIExtras) { x.PortalToken = nil }, false},
		{"missing mtls cert", func(x *GoatCLIExtras) { x.MTLSCert = nil }, false},
		{"missing mtls key", func(x *GoatCLIExtras) { x.MTLSKey = nil }, false},
		{"missing ca roots", func(x *GoatCLIExtras) { x.CARoots = nil }, false},
		{"zero value", func(x *GoatCLIExtras) { *x = GoatCLIExtras{} }, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			b := fixtureBundle(t)
			b.GoatCLIExtras = full
			tc.mut(&b.GoatCLIExtras)
			if got := b.HasGoatCLIExtras(); got != tc.want {
				t.Errorf("HasGoatCLIExtras() = %v; want %v", got, tc.want)
			}
		})
	}
}

func TestSignableExcludesSignature(t *testing.T) {
	b := fixtureBundle(t)
	// Payload with no signature must equal payload after signature is set.
	before, err := b.Signable()
	if err != nil {
		t.Fatalf("signable before: %v", err)
	}
	_, sign := testCAKey(t)

	if err := b.Sign(sign); err != nil {
		t.Fatalf("sign: %v", err)
	}
	after, err := b.Signable()
	if err != nil {
		t.Fatalf("signable after: %v", err)
	}
	if !bytes.Equal(before, after) {
		t.Error("Signable payload differs before/after signing — Signature field is leaking into signed bytes")
	}
}

// --- machine-mesh-alignment D6a: registry-provenance fields --------------

// TestBundleRegistryProvenance_Roundtrip covers the additive
// enrollment_anchor + node_class fields stamped by bundle-create
// --from-machine: the joined peer ties back to its portal custody edge
// without a portal round-trip.
func TestBundleRegistryProvenance_Roundtrip(t *testing.T) {
	b := fixtureBundle(t)
	b.EnrollmentAnchor = "slot-abc123"
	b.NodeClass = "KID"
	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	got, err := Unmarshal(data)
	if err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if got.EnrollmentAnchor != "slot-abc123" {
		t.Errorf("EnrollmentAnchor = %q; want slot-abc123", got.EnrollmentAnchor)
	}
	if got.NodeClass != "KID" {
		t.Errorf("NodeClass = %q; want KID", got.NodeClass)
	}
}

// TestBundleRegistryProvenance_OmitEmpty asserts bundles issued the
// pre-D6a way (hand-supplied --acl-groups) carry neither field on the
// wire — old bundles and new non-registry bundles stay byte-compatible.
func TestBundleRegistryProvenance_OmitEmpty(t *testing.T) {
	b := fixtureBundle(t)
	data, err := b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if bytes.Contains(data, []byte("enrollment_anchor")) {
		t.Error("empty EnrollmentAnchor should not appear in canonical encoding")
	}
	if bytes.Contains(data, []byte("node_class")) {
		t.Error("empty NodeClass should not appear in canonical encoding")
	}
}

// TestBundleRegistryProvenance_TamperDetected: the fields are inside the
// CA signature — swapping the anchor after signing must fail Verify.
func TestBundleRegistryProvenance_TamperDetected(t *testing.T) {
	b := fixtureBundle(t)
	b.EnrollmentAnchor = "slot-abc123"
	b.NodeClass = "KID"
	pub, sign := testCAKey(t)
	if err := b.Sign(sign); err != nil {
		t.Fatalf("sign: %v", err)
	}
	if err := b.Verify(pub); err != nil {
		t.Fatalf("verify untampered: %v", err)
	}
	b.EnrollmentAnchor = "slot-swapped"
	if err := b.Verify(pub); err == nil {
		t.Error("verify passed with a tampered enrollment_anchor")
	}
}
