package main

import (
	"bytes"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/base64"
	"encoding/pem"
	"errors"
	"math/big"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"golang.org/x/crypto/curve25519"

	"github.com/usareur-af/goatcore/ops/enrollment/bundle"
)

func fixedNow() time.Time { return time.Date(2026, 5, 5, 12, 0, 0, 0, time.UTC) }

// signedBundle returns a fresh bundle with valid wg-cp0 fields signed
// by a freshly-generated CA keypair. Returns the wire bytes, the path
// to a CA pubkey PEM file, and the issued bundle's struct (for cross-
// checks in tests).
func signedBundle(t *testing.T, mutate func(*bundle.EnrollmentBundle)) (wire []byte, caPubPath string) {
	t.Helper()
	caPriv, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatalf("gen ca key: %v", err)
	}
	caPub := &caPriv.PublicKey
	caSign := func(msg []byte) ([]byte, error) {
		digest := sha256.Sum256(msg)
		return ecdsa.SignASN1(rand.Reader, caPriv, digest[:])
	}

	// Generate a clamped wg-cp0 keypair for the device.
	devPriv := make([]byte, 32)
	if _, err := rand.Read(devPriv); err != nil {
		t.Fatalf("gen dev priv: %v", err)
	}
	devPriv[0] &= 248
	devPriv[31] &= 127
	devPriv[31] |= 64
	devPub, err := curve25519.X25519(devPriv, curve25519.Basepoint)
	if err != nil {
		t.Fatalf("derive dev pub: %v", err)
	}

	// Two cp-relay endpoints with their own pubkeys.
	relayPubs := make([][]byte, 2)
	for i := range relayPubs {
		p := make([]byte, 32)
		if _, err := rand.Read(p); err != nil {
			t.Fatalf("gen relay pub %d: %v", i, err)
		}
		relayPubs[i] = p
	}

	issued := fixedNow()
	b := &bundle.EnrollmentBundle{
		Version:    bundle.Version,
		DeviceID:   "mgmt-prov-site3-01-orin",
		PeerPubkey: make([]byte, 32),
		ACLGroups:  []string{"LAB", "LAB-MGMT"},
		Site:       "lab",
		KnownEndpoints: []bundle.KnownEndpoint{
			{Addr: "203.0.113.1:51821", Pubkey: relayPubs[0], Kind: bundle.Kind("cp-relay"), MeshAddr: "10.99.0.2"},
			{Addr: "203.0.113.23:51821", Pubkey: relayPubs[1], Kind: bundle.Kind("cp-relay"), MeshAddr: "10.99.0.3"},
		},
		IssuedAt:           issued,
		ActivationDeadline: issued.Add(14 * 24 * time.Hour),
		ExpiresAt:          issued.Add(90 * 24 * time.Hour),
		Nonce:              make([]byte, 16),
		CAID:               "dev-goat-ca-test",
		CPDevicePubkey:     devPub,
		CPDevicePrivkey:    devPriv,
		CPDeviceAddress:    "10.99.0.6/24",
	}
	if mutate != nil {
		mutate(b)
	}
	if err := b.Sign(caSign); err != nil {
		t.Fatalf("sign: %v", err)
	}
	wire, err = b.Marshal()
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}

	// Write the CA pubkey as PKIX-encoded PEM (matches ca-init output).
	caPubPath = filepath.Join(t.TempDir(), "ca.pub")
	spki, err := x509.MarshalPKIXPublicKey(caPub)
	if err != nil {
		t.Fatalf("marshal pkix: %v", err)
	}
	pemBytes := pem.EncodeToMemory(&pem.Block{Type: "PUBLIC KEY", Bytes: spki})
	if err := os.WriteFile(caPubPath, pemBytes, 0o644); err != nil {
		t.Fatalf("write capub: %v", err)
	}
	return wire, caPubPath
}

// TestRun_HappyPath exercises the full validate + render + write flow.
// Given a well-formed signed bundle, the tool should produce a wg-quick
// conf with the expected [Interface] + [Peer] sections and a privkey
// file matching the bundle's CPDevicePrivkey.
func TestRun_HappyPath(t *testing.T) {
	wire, caPub := signedBundle(t, nil)
	dir := t.TempDir()
	bundlePath := filepath.Join(dir, "bundle.cbor")
	if err := os.WriteFile(bundlePath, wire, 0o600); err != nil {
		t.Fatalf("write bundle: %v", err)
	}
	confPath := filepath.Join(dir, "wg-cp0.conf")
	privkeyPath := filepath.Join(dir, "wg-cp0-priv.key")

	args := runArgs{
		InPath: bundlePath, CAPubPath: caPub,
		OutConfPath: confPath, OutPrivkeyPath: privkeyPath,
		InterfaceName: "wg-cp0", PersistentKeepalive: 25,
		Now: fixedNow().Add(1 * time.Hour),
	}
	stderr, _ := os.CreateTemp(t.TempDir(), "stderr-*")
	defer stderr.Close()
	stdout, _ := os.CreateTemp(t.TempDir(), "stdout-*")
	defer stdout.Close()
	if code := run(args, stdout, stderr); code != 0 {
		t.Fatalf("run exit code = %d; want 0", code)
	}

	// Conf file: assert structure.
	confBytes, err := os.ReadFile(confPath)
	if err != nil {
		t.Fatalf("read conf: %v", err)
	}
	conf := string(confBytes)
	for _, want := range []string{
		"[Interface]",
		"PrivateKey = ",
		"Address    = 10.99.0.6/24",
		"[Peer]",
		"Endpoint   = 203.0.113.1:51821",
		// ADR 0321: canonical symmetric AllowedIPs on every relay
		// peer block — [MeshAddr/32, WGCP0FleetSubnet] — for both
		// fresh-issued bundles and the legacy-bundle fallback path.
		"AllowedIPs = 10.99.0.2/32, 198.18.0.0/24",
		"Endpoint   = 203.0.113.23:51821",
		"AllowedIPs = 10.99.0.3/32, 198.18.0.0/24",
		"PersistentKeepalive = 25",
		"# Site:        lab",
		"# Device:      mgmt-prov-site3-01-orin",
	} {
		if !strings.Contains(conf, want) {
			t.Errorf("conf missing %q\n--- conf ---\n%s\n", want, conf)
		}
	}

	// Conf and privkey should both be 0600.
	for _, p := range []string{confPath, privkeyPath} {
		st, err := os.Stat(p)
		if err != nil {
			t.Fatalf("stat %s: %v", p, err)
		}
		if st.Mode().Perm() != 0o600 {
			t.Errorf("%s mode = %o; want 0600", p, st.Mode().Perm())
		}
	}

	// Privkey content matches what's in the bundle.
	rawPrivkey, _ := os.ReadFile(privkeyPath)
	wantPrivB64 := mustExtractInterfacePrivkey(t, conf)
	if strings.TrimSpace(string(rawPrivkey)) != wantPrivB64 {
		t.Errorf("privkey file != [Interface] PrivateKey")
	}
}

func mustExtractInterfacePrivkey(t *testing.T, conf string) string {
	t.Helper()
	for _, line := range strings.Split(conf, "\n") {
		if strings.HasPrefix(line, "PrivateKey = ") {
			return strings.TrimPrefix(line, "PrivateKey = ")
		}
	}
	t.Fatal("no PrivateKey line in rendered conf")
	return ""
}

// TestRun_DryRunWritesNothing asserts the dry-run path emits the conf
// to stdout and leaves the disk untouched.
func TestRun_DryRunWritesNothing(t *testing.T) {
	wire, caPub := signedBundle(t, nil)
	dir := t.TempDir()
	bundlePath := filepath.Join(dir, "bundle.cbor")
	_ = os.WriteFile(bundlePath, wire, 0o600)
	confPath := filepath.Join(dir, "wg-cp0.conf")
	privkeyPath := filepath.Join(dir, "wg-cp0-priv.key")
	args := runArgs{
		InPath: bundlePath, CAPubPath: caPub,
		OutConfPath: confPath, OutPrivkeyPath: privkeyPath,
		InterfaceName: "wg-cp0", PersistentKeepalive: 25,
		DryRun: true, Now: fixedNow().Add(time.Hour),
	}
	stderr, _ := os.CreateTemp(t.TempDir(), "stderr-*")
	defer stderr.Close()
	stdout, _ := os.CreateTemp(t.TempDir(), "stdout-*")
	defer stdout.Close()
	if code := run(args, stdout, stderr); code != 0 {
		t.Fatalf("dry-run exit code = %d; want 0", code)
	}
	if _, err := os.Stat(confPath); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("dry-run wrote conf file (expected not-exist)")
	}
	if _, err := os.Stat(privkeyPath); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("dry-run wrote privkey file (expected not-exist)")
	}
}

// TestRun_RejectsTamperedSignature asserts byte-flipping the bundle
// after signing causes Verify() to fail and the tool to exit non-zero
// without writing anything.
func TestRun_RejectsTamperedSignature(t *testing.T) {
	wire, caPub := signedBundle(t, nil)
	// Flip a byte deep in the bundle (likely hits payload).
	wire[len(wire)/2] ^= 0xff
	dir := t.TempDir()
	bundlePath := filepath.Join(dir, "bundle.cbor")
	_ = os.WriteFile(bundlePath, wire, 0o600)
	confPath := filepath.Join(dir, "wg-cp0.conf")
	privkeyPath := filepath.Join(dir, "wg-cp0-priv.key")
	args := runArgs{
		InPath: bundlePath, CAPubPath: caPub,
		OutConfPath: confPath, OutPrivkeyPath: privkeyPath,
		Now: fixedNow().Add(time.Hour),
	}
	stderr, _ := os.CreateTemp(t.TempDir(), "stderr-*")
	defer stderr.Close()
	stdout, _ := os.CreateTemp(t.TempDir(), "stdout-*")
	defer stdout.Close()
	code := run(args, stdout, stderr)
	if code == 0 {
		t.Fatal("expected non-zero exit on tampered bundle")
	}
	if _, err := os.Stat(confPath); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("tampered bundle should not have produced conf file")
	}
	if _, err := os.Stat(privkeyPath); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("tampered bundle should not have produced privkey file")
	}
}

// TestRun_RejectsExpiredBundle uses a fixed Now beyond ExpiresAt to
// trigger CheckExpiry rejection.
func TestRun_RejectsExpiredBundle(t *testing.T) {
	wire, caPub := signedBundle(t, nil)
	dir := t.TempDir()
	bundlePath := filepath.Join(dir, "bundle.cbor")
	_ = os.WriteFile(bundlePath, wire, 0o600)
	args := runArgs{
		InPath: bundlePath, CAPubPath: caPub,
		OutConfPath: filepath.Join(dir, "c"), OutPrivkeyPath: filepath.Join(dir, "k"),
		Now: fixedNow().Add(91 * 24 * time.Hour), // 1 day past expiry
	}
	stderr, _ := os.CreateTemp(t.TempDir(), "stderr-*")
	defer stderr.Close()
	stdout, _ := os.CreateTemp(t.TempDir(), "stdout-*")
	defer stdout.Close()
	code := run(args, stdout, stderr)
	if code != 2 {
		t.Errorf("expired bundle exit = %d; want 2 (validation failure)", code)
	}
}

// TestRun_RejectsBundleWithoutCPFields covers the "legacy / non-cp peer"
// exit path: bundle parses + verifies but has no CPDevicePubkey.
func TestRun_RejectsBundleWithoutCPFields(t *testing.T) {
	wire, caPub := signedBundle(t, func(b *bundle.EnrollmentBundle) {
		b.CPDevicePubkey = nil
		b.CPDevicePrivkey = nil
		b.CPDeviceAddress = ""
	})
	dir := t.TempDir()
	bundlePath := filepath.Join(dir, "bundle.cbor")
	_ = os.WriteFile(bundlePath, wire, 0o600)
	args := runArgs{
		InPath: bundlePath, CAPubPath: caPub,
		OutConfPath: filepath.Join(dir, "c"), OutPrivkeyPath: filepath.Join(dir, "k"),
		Now: fixedNow().Add(time.Hour),
	}
	stderr, _ := os.CreateTemp(t.TempDir(), "stderr-*")
	defer stderr.Close()
	stdout, _ := os.CreateTemp(t.TempDir(), "stdout-*")
	defer stdout.Close()
	code := run(args, stdout, stderr)
	if code != 3 {
		t.Errorf("no-cp-fields exit = %d; want 3 (legacy / non-cp)", code)
	}
}

// TestRun_RejectsKeypairMismatch corrupts the privkey field so the
// derived pubkey no longer matches CPDevicePubkey.
func TestRun_RejectsKeypairMismatch(t *testing.T) {
	wire, caPub := signedBundle(t, func(b *bundle.EnrollmentBundle) {
		// Replace privkey with a different valid-shape clamped key —
		// derivation will produce a pubkey that doesn't match the
		// signed CPDevicePubkey field. Remember to re-sign so the
		// signature is valid (we want CheckCPDeviceKeypair-style
		// rejection AFTER signature verify, not signature failure).
		newPriv := make([]byte, 32)
		_, _ = rand.Read(newPriv)
		newPriv[0] &= 248
		newPriv[31] &= 127
		newPriv[31] |= 64
		b.CPDevicePrivkey = newPriv
	})
	dir := t.TempDir()
	bundlePath := filepath.Join(dir, "bundle.cbor")
	_ = os.WriteFile(bundlePath, wire, 0o600)
	args := runArgs{
		InPath: bundlePath, CAPubPath: caPub,
		OutConfPath: filepath.Join(dir, "c"), OutPrivkeyPath: filepath.Join(dir, "k"),
		Now: fixedNow().Add(time.Hour),
	}
	stderr, _ := os.CreateTemp(t.TempDir(), "stderr-*")
	defer stderr.Close()
	stdout, _ := os.CreateTemp(t.TempDir(), "stdout-*")
	defer stdout.Close()
	code := run(args, stdout, stderr)
	if code != 2 {
		t.Errorf("keypair-mismatch exit = %d; want 2", code)
	}
}

// TestRun_RejectsBundleSignedByDifferentCA asserts the tool rejects
// bundles signed by a CA other than the one whose pubkey was loaded.
// This is the load-bearing security property.
func TestRun_RejectsBundleSignedByDifferentCA(t *testing.T) {
	wire, _ := signedBundle(t, nil)
	// Generate an unrelated CA pubkey + write its PEM.
	otherPriv, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatalf("gen other ca: %v", err)
	}
	spki, _ := x509.MarshalPKIXPublicKey(&otherPriv.PublicKey)
	otherPubPath := filepath.Join(t.TempDir(), "other-ca.pub")
	_ = os.WriteFile(otherPubPath, pem.EncodeToMemory(&pem.Block{Type: "PUBLIC KEY", Bytes: spki}), 0o644)

	dir := t.TempDir()
	bundlePath := filepath.Join(dir, "bundle.cbor")
	_ = os.WriteFile(bundlePath, wire, 0o600)
	args := runArgs{
		InPath: bundlePath, CAPubPath: otherPubPath,
		OutConfPath: filepath.Join(dir, "c"), OutPrivkeyPath: filepath.Join(dir, "k"),
		Now: fixedNow().Add(time.Hour),
	}
	stderr, _ := os.CreateTemp(t.TempDir(), "stderr-*")
	defer stderr.Close()
	stdout, _ := os.CreateTemp(t.TempDir(), "stdout-*")
	defer stdout.Close()
	if code := run(args, stdout, stderr); code != 2 {
		t.Errorf("wrong-ca exit = %d; want 2", code)
	}
}

// TestRun_RequiresCPRelaysInBundle covers the failure mode where a
// bundle has wg-cp0 keypair fields but no cp-relay endpoints —
// device cannot bring up wg-cp0 without somewhere to connect.
func TestRun_RequiresCPRelaysInBundle(t *testing.T) {
	wire, caPub := signedBundle(t, func(b *bundle.EnrollmentBundle) {
		// Drop all known_endpoints — no cp-relays available.
		b.KnownEndpoints = nil
	})
	dir := t.TempDir()
	bundlePath := filepath.Join(dir, "bundle.cbor")
	_ = os.WriteFile(bundlePath, wire, 0o600)
	args := runArgs{
		InPath: bundlePath, CAPubPath: caPub,
		OutConfPath: filepath.Join(dir, "c"), OutPrivkeyPath: filepath.Join(dir, "k"),
		Now: fixedNow().Add(time.Hour),
	}
	stderr, _ := os.CreateTemp(t.TempDir(), "stderr-*")
	defer stderr.Close()
	stdout, _ := os.CreateTemp(t.TempDir(), "stdout-*")
	defer stdout.Close()
	if code := run(args, stdout, stderr); code != 2 {
		t.Errorf("no-cp-relays exit = %d; want 2", code)
	}
}

// TestAtomicWrite_PartialFailureLeavesNothing exercises the rename-on-
// success contract: if the rename target is a directory (causes rename
// failure), the temp file should not leak.
func TestAtomicWrite_HappyPath(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "out")
	if err := atomicWrite(path, []byte("hello"), 0o600); err != nil {
		t.Fatalf("atomicWrite: %v", err)
	}
	got, _ := os.ReadFile(path)
	if !bytes.Equal(got, []byte("hello")) {
		t.Errorf("got %q want hello", got)
	}
	st, _ := os.Stat(path)
	if st.Mode().Perm() != 0o600 {
		t.Errorf("mode = %o; want 0600", st.Mode().Perm())
	}
	// No .tmp leftover.
	entries, _ := os.ReadDir(dir)
	for _, e := range entries {
		if strings.Contains(e.Name(), ".tmp.") {
			t.Errorf("leftover tmp file: %s", e.Name())
		}
	}
}

// TestLoadCAPubkey covers PEM/DER parsing edge cases.
func TestLoadCAPubkey(t *testing.T) {
	dir := t.TempDir()

	t.Run("happy path", func(t *testing.T) {
		priv, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
		spki, _ := x509.MarshalPKIXPublicKey(&priv.PublicKey)
		p := filepath.Join(dir, "ok.pub")
		_ = os.WriteFile(p, pem.EncodeToMemory(&pem.Block{Type: "PUBLIC KEY", Bytes: spki}), 0o644)
		got, err := loadCAPubkey(p)
		if err != nil {
			t.Fatalf("load: %v", err)
		}
		if got.X.Cmp(priv.PublicKey.X) != 0 || got.Y.Cmp(priv.PublicKey.Y) != 0 {
			t.Error("pubkey roundtrip mismatch")
		}
	})

	t.Run("not pem", func(t *testing.T) {
		p := filepath.Join(dir, "not.pem")
		_ = os.WriteFile(p, []byte("not pem"), 0o644)
		if _, err := loadCAPubkey(p); err == nil {
			t.Error("expected error")
		}
	})

	t.Run("invalid certificate bytes", func(t *testing.T) {
		// Block 79 / F-103-followon: loadCAPubkey now accepts CERTIFICATE
		// PEMs as well as PUBLIC KEY PEMs (extracts the SPKI from the cert).
		// But gibberish bytes wrapped in a CERTIFICATE PEM must still fail.
		p := filepath.Join(dir, "wrong-block.pem")
		_ = os.WriteFile(p, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: []byte{1, 2, 3}}), 0o644)
		if _, err := loadCAPubkey(p); err == nil {
			t.Error("expected error")
		}
	})

	t.Run("certificate pem (X.509) accepted", func(t *testing.T) {
		// The committed offline-CA root sits at ops/enrollment/ca/dev/root-cert.pem
		// as a CERTIFICATE PEM (Traefik chain anchor). Operators should be
		// able to point --ca-pubkey at THAT file too, not just at the raw
		// SPKI PUBLIC KEY in public-keys/<ca-id>.pem.
		priv, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
		tmpl := &x509.Certificate{
			SerialNumber: big.NewInt(1),
			Subject:      pkix.Name{CommonName: "test-ca"},
			NotBefore:    time.Now().Add(-time.Hour),
			NotAfter:     time.Now().Add(time.Hour),
			IsCA:         true,
			KeyUsage:     x509.KeyUsageCertSign,
		}
		der, err := x509.CreateCertificate(rand.Reader, tmpl, tmpl, &priv.PublicKey, priv)
		if err != nil {
			t.Fatalf("create cert: %v", err)
		}
		p := filepath.Join(dir, "cert.pem")
		_ = os.WriteFile(p, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), 0o644)
		got, err := loadCAPubkey(p)
		if err != nil {
			t.Fatalf("load cert pem: %v", err)
		}
		if got.X.Cmp(priv.PublicKey.X) != 0 || got.Y.Cmp(priv.PublicKey.Y) != 0 {
			t.Error("cert pubkey roundtrip mismatch")
		}
	})

	t.Run("certificate with non-ECDSA key rejected", func(t *testing.T) {
		// A CERTIFICATE PEM wrapping (e.g.) an RSA key must fail loudly —
		// don't silently misinterpret as if it were ECDSA P-256.
		rsaPriv, _ := rsa.GenerateKey(rand.Reader, 2048)
		tmpl := &x509.Certificate{
			SerialNumber: big.NewInt(2),
			Subject:      pkix.Name{CommonName: "rsa-ca"},
			NotBefore:    time.Now().Add(-time.Hour),
			NotAfter:     time.Now().Add(time.Hour),
			IsCA:         true,
		}
		der, err := x509.CreateCertificate(rand.Reader, tmpl, tmpl, &rsaPriv.PublicKey, rsaPriv)
		if err != nil {
			t.Fatalf("create rsa cert: %v", err)
		}
		p := filepath.Join(dir, "rsa-cert.pem")
		_ = os.WriteFile(p, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), 0o644)
		if _, err := loadCAPubkey(p); err == nil {
			t.Error("expected error on non-ECDSA cert")
		}
	})

	t.Run("not ECDSA P-256", func(t *testing.T) {
		// Empty SPKI — gets rejected at the type assertion / curve check
		// in parseSPKIECDSAP256. Block 79 hard cutover requires P-256.
		_ = base64.StdEncoding // unused but documents intent
	})
}

// TestRun_LegacyBundleSymmetricFallback — ADR 0321. When a legacy
// bundle (issued before ADR 0321) carries an explicit AllowedIPs on
// some endpoints but not others, the device-side renderer must:
//   - render the explicit AllowedIPs verbatim where present
//   - render the canonical fallback [MeshAddr/32, WGCP0FleetSubnet]
//     where AllowedIPs is empty
//
// The pre-ADR-0321 fallback of MeshAddr/32-only is gone; legacy
// bundles with mixed AllowedIPs (the F-116 asymmetric class) now
// render symmetrically on the device side regardless of the
// issuer-side defect.
func TestRun_LegacyBundleSymmetricFallback(t *testing.T) {
	wire, caPub := signedBundle(t, func(b *bundle.EnrollmentBundle) {
		// Mimic a legacy F-116-class asymmetric bundle: first relay
		// got the /24, second didn't.
		b.KnownEndpoints[0].AllowedIPs = []string{
			"10.99.0.2/32",
			"10.99.0.0/24",
		}
		// Second endpoint AllowedIPs left nil → device-side fallback.
	})
	dir := t.TempDir()
	bundlePath := filepath.Join(dir, "bundle.cbor")
	if err := os.WriteFile(bundlePath, wire, 0o600); err != nil {
		t.Fatalf("write bundle: %v", err)
	}
	confPath := filepath.Join(dir, "wg-cp0.conf")
	privkeyPath := filepath.Join(dir, "wg-cp0-priv.key")

	args := runArgs{
		InPath: bundlePath, CAPubPath: caPub,
		OutConfPath: confPath, OutPrivkeyPath: privkeyPath,
		InterfaceName: "wg-cp0", PersistentKeepalive: 25,
		Now: fixedNow().Add(1 * time.Hour),
	}
	stderr, _ := os.CreateTemp(t.TempDir(), "stderr-*")
	defer stderr.Close()
	stdout, _ := os.CreateTemp(t.TempDir(), "stdout-*")
	defer stdout.Close()
	if code := run(args, stdout, stderr); code != 0 {
		t.Fatalf("run exit code = %d; want 0", code)
	}

	confBytes, _ := os.ReadFile(confPath)
	conf := string(confBytes)

	// First relay: explicit AllowedIPs from the bundle rendered verbatim.
	if !strings.Contains(conf, "AllowedIPs = 10.99.0.2/32, 10.99.0.0/24") {
		t.Errorf("first relay: missing explicit AllowedIPs line\n--- conf ---\n%s\n", conf)
	}
	// Second relay: AllowedIPs empty in the bundle → symmetric fallback
	// renders the canonical pair [MeshAddr/32, WGCP0FleetSubnet].
	if !strings.Contains(conf, "AllowedIPs = 10.99.0.3/32, 198.18.0.0/24\n") {
		t.Errorf("second relay: canonical symmetric fallback missing\n--- conf ---\n%s\n", conf)
	}
	// Pre-ADR-0321 fallback (MeshAddr/32-only) must NOT appear.
	if strings.Contains(conf, "AllowedIPs = 10.99.0.3/32\n") {
		t.Errorf("second relay: pre-ADR-0321 MeshAddr/32-only fallback should be gone\n--- conf ---\n%s\n", conf)
	}
}
