package main

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/usareur-af/goatcore/ops/enrollment/bundle"
)

// fullGoatCLIArgs returns a runArgs with all six --goat-cli-* flags
// populated against tempdir files. Tests can mutate individual fields
// to exercise the atomicity + validation surface.
func fullGoatCLIArgs(t *testing.T) runArgs {
	t.Helper()
	dir := t.TempDir()
	tokenPath := filepath.Join(dir, "portal.token")
	certPath := filepath.Join(dir, "operator.cert.pem")
	keyPath := filepath.Join(dir, "operator.key.pem")
	caRootsPath := filepath.Join(dir, "ca-roots.pem")
	if err := os.WriteFile(tokenPath, []byte("opaque-portal-bearer-32chars-pad\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(certPath, []byte(certOnlyPEM), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(keyPath, []byte(keyOnlyPEM), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(caRootsPath, []byte(certOnlyPEM), 0o600); err != nil {
		t.Fatal(err)
	}
	return runArgs{
		GoatCLIPortalEndpoint:  "https://portal.nebula.netbird.nebula.example",
		GoatCLIPortalTokenPath: tokenPath,
		GoatCLIRouterEndpoint:  "tls/192.0.2.10:7447",
		GoatCLIMTLSCertPath:    certPath,
		GoatCLIMTLSKeyPath:     keyPath,
		GoatCLICARootsPath:     caRootsPath,
	}
}

// TestBuildGoatCLIExtras_NoFlags — returns the zero value so the CBOR
// omitempty drops the field on the wire. v0.2-and-earlier bundles
// round-trip byte-identically.
func TestBuildGoatCLIExtras_NoFlags(t *testing.T) {
	got, err := buildGoatCLIExtras(runArgs{})
	if err != nil {
		t.Fatalf("buildGoatCLIExtras: %v", err)
	}
	if got.PortalEndpoint != "" || got.RouterEndpoint != "" ||
		len(got.PortalToken) != 0 || len(got.MTLSCert) != 0 ||
		len(got.MTLSKey) != 0 || len(got.CARoots) != 0 {
		t.Errorf("expected zero value, got %+v", got)
	}
}

// TestBuildGoatCLIExtras_HappyPath_All — all six flags populate.
// Cert/key/ca-roots PEM contents flow through unchanged.
func TestBuildGoatCLIExtras_HappyPath_All(t *testing.T) {
	args := fullGoatCLIArgs(t)
	got, err := buildGoatCLIExtras(args)
	if err != nil {
		t.Fatalf("buildGoatCLIExtras: %v", err)
	}
	if got.PortalEndpoint != args.GoatCLIPortalEndpoint {
		t.Errorf("PortalEndpoint=%q want %q", got.PortalEndpoint, args.GoatCLIPortalEndpoint)
	}
	if got.RouterEndpoint != args.GoatCLIRouterEndpoint {
		t.Errorf("RouterEndpoint=%q want %q", got.RouterEndpoint, args.GoatCLIRouterEndpoint)
	}
	if !bytes.Equal(got.PortalToken, []byte("opaque-portal-bearer-32chars-pad")) {
		t.Errorf("PortalToken=%q (trailing newline should be stripped)", got.PortalToken)
	}
	if !strings.Contains(string(got.MTLSCert), "BEGIN CERTIFICATE") {
		t.Errorf("MTLSCert missing CERTIFICATE block")
	}
	if !strings.Contains(string(got.MTLSKey), "BEGIN PRIVATE KEY") {
		t.Errorf("MTLSKey missing PRIVATE KEY block")
	}
	if !strings.Contains(string(got.CARoots), "BEGIN CERTIFICATE") {
		t.Errorf("CARoots missing CERTIFICATE block")
	}
}

// TestBuildGoatCLIExtras_AtomicityEachFlagRequired — drop any one of
// the six and the call fails with that specific flag named. Closes
// the "half-populated bundle that fails at goat profile init" class.
func TestBuildGoatCLIExtras_AtomicityEachFlagRequired(t *testing.T) {
	for _, tc := range []struct {
		name    string
		drop    func(*runArgs)
		wantSub string
	}{
		{"portal_endpoint", func(a *runArgs) { a.GoatCLIPortalEndpoint = "" }, "--goat-cli-portal-endpoint is required"},
		{"portal_token", func(a *runArgs) { a.GoatCLIPortalTokenPath = "" }, "--goat-cli-portal-token is required"},
		{"router_endpoint", func(a *runArgs) { a.GoatCLIRouterEndpoint = "" }, "--goat-cli-router-endpoint is required"},
		{"mtls_cert", func(a *runArgs) { a.GoatCLIMTLSCertPath = "" }, "--goat-cli-mtls-cert is required"},
		{"mtls_key", func(a *runArgs) { a.GoatCLIMTLSKeyPath = "" }, "--goat-cli-mtls-key is required"},
		{"ca_roots", func(a *runArgs) { a.GoatCLICARootsPath = "" }, "--goat-cli-ca-roots is required"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			args := fullGoatCLIArgs(t)
			tc.drop(&args)
			_, err := buildGoatCLIExtras(args)
			if err == nil || !strings.Contains(err.Error(), tc.wantSub) {
				t.Errorf("expected %q, got %v", tc.wantSub, err)
			}
		})
	}
}

// TestBuildGoatCLIExtras_RejectsNonHTTPSPortal — http:// portal URL fails.
func TestBuildGoatCLIExtras_RejectsNonHTTPSPortal(t *testing.T) {
	args := fullGoatCLIArgs(t)
	args.GoatCLIPortalEndpoint = "http://portal.example/"
	_, err := buildGoatCLIExtras(args)
	if err == nil || !strings.Contains(err.Error(), "must be https") {
		t.Errorf("expected https-required error, got %v", err)
	}
}

// TestBuildGoatCLIExtras_RejectsBareIPRouter — Zenoh router endpoint
// without a transport prefix fails. Closes the "operator pasted just
// host:port" mistake before signing.
func TestBuildGoatCLIExtras_RejectsBareIPRouter(t *testing.T) {
	args := fullGoatCLIArgs(t)
	args.GoatCLIRouterEndpoint = "192.0.2.10:7447"
	_, err := buildGoatCLIExtras(args)
	if err == nil || !strings.Contains(err.Error(), "Zenoh transport prefix") {
		t.Errorf("expected transport-prefix error, got %v", err)
	}
}

// TestBuildGoatCLIExtras_AcceptsAllZenohTransports — every documented
// prefix is accepted.
func TestBuildGoatCLIExtras_AcceptsAllZenohTransports(t *testing.T) {
	for _, prefix := range []string{"tls/", "tcp/", "quic/", "udp/", "ws/"} {
		t.Run(prefix, func(t *testing.T) {
			args := fullGoatCLIArgs(t)
			args.GoatCLIRouterEndpoint = prefix + "host:7447"
			if _, err := buildGoatCLIExtras(args); err != nil {
				t.Errorf("prefix %q: %v", prefix, err)
			}
		})
	}
}

// TestBuildGoatCLIExtras_RejectsEmptyTokenFile — a token file with
// only whitespace is treated as empty.
func TestBuildGoatCLIExtras_RejectsEmptyTokenFile(t *testing.T) {
	args := fullGoatCLIArgs(t)
	if err := os.WriteFile(args.GoatCLIPortalTokenPath, []byte("   \n\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	_, err := buildGoatCLIExtras(args)
	if err == nil || !strings.Contains(err.Error(), "token file is empty") {
		t.Errorf("expected empty-token error, got %v", err)
	}
}

// TestBuildGoatCLIExtras_RejectsCertWithoutCertBlock — a PEM file that
// doesn't contain a CERTIFICATE block fails the smoke check.
func TestBuildGoatCLIExtras_RejectsCertWithoutCertBlock(t *testing.T) {
	args := fullGoatCLIArgs(t)
	if err := os.WriteFile(args.GoatCLIMTLSCertPath, []byte("not a pem file at all"), 0o600); err != nil {
		t.Fatal(err)
	}
	_, err := buildGoatCLIExtras(args)
	if err == nil || !strings.Contains(err.Error(), "no CERTIFICATE block") {
		t.Errorf("expected cert-block error, got %v", err)
	}
}

// TestBuildGoatCLIExtras_RejectsKeyWithoutKeyBlock — a PEM file that
// contains only a CERTIFICATE (not a key) fails.
func TestBuildGoatCLIExtras_RejectsKeyWithoutKeyBlock(t *testing.T) {
	args := fullGoatCLIArgs(t)
	if err := os.WriteFile(args.GoatCLIMTLSKeyPath, []byte(certOnlyPEM), 0o600); err != nil {
		t.Fatal(err)
	}
	_, err := buildGoatCLIExtras(args)
	if err == nil || !strings.Contains(err.Error(), "no PRIVATE KEY block") {
		t.Errorf("expected key-block error, got %v", err)
	}
}

// TestBuildGoatCLIExtras_AcceptsECKey — EC PRIVATE KEY block satisfies
// the key requirement (rustls handles SEC1 + PKCS#8 + PKCS#1).
func TestBuildGoatCLIExtras_AcceptsECKey(t *testing.T) {
	args := fullGoatCLIArgs(t)
	ecKeyPEM := "-----BEGIN EC PRIVATE KEY-----\nMHcCAQEEIK0HFAKE\n-----END EC PRIVATE KEY-----\n"
	if err := os.WriteFile(args.GoatCLIMTLSKeyPath, []byte(ecKeyPEM), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := buildGoatCLIExtras(args); err != nil {
		t.Errorf("EC PRIVATE KEY should be accepted, got %v", err)
	}
}

// TestBuildGoatCLIExtras_TokenFilePathNotFound — surface the underlying
// file-system error wrapped in a flag-named prefix.
func TestBuildGoatCLIExtras_TokenFilePathNotFound(t *testing.T) {
	args := fullGoatCLIArgs(t)
	args.GoatCLIPortalTokenPath = "/nonexistent/portal.token"
	_, err := buildGoatCLIExtras(args)
	if err == nil || !strings.Contains(err.Error(), "--goat-cli-portal-token") {
		t.Errorf("expected wrapped token-path error, got %v", err)
	}
}

// TestRun_GoatCLIExtras_EndToEnd — mint a real bundle with the
// --goat-cli-* flags populated, parse it back, verify the signature,
// and assert every extras sub-field round-trips through CBOR. The
// integrity property (CA signature covers the goat-cli endpoint set
// + cert + token together) is the motivation for Path B over Path A.
func TestRun_GoatCLIExtras_EndToEnd(t *testing.T) {
	caKey, ageID, caPub := devCA(t)
	relaysPath := fixtureRelays(t)
	gArgs := fullGoatCLIArgs(t)
	outPath := filepath.Join(t.TempDir(), "operator-bundle.cbor")

	args := runArgs{
		Site:                   "site-a",
		DeviceID:               "operator-example-user",
		WGCP0Address:           "10.99.0.42/24",
		RelaysPath:             relaysPath,
		ACLGroupsRaw:           "OPERATORS",
		CAID:                   "dev-nebula-ca-test",
		CAKeyPath:              caKey,
		AgeIdentityPath:        ageID,
		OutPath:                outPath,
		TTLDays:                defaultTTLDays,
		ActivationDays:         defaultActivationDays,
		GoatCLIPortalEndpoint:  gArgs.GoatCLIPortalEndpoint,
		GoatCLIPortalTokenPath: gArgs.GoatCLIPortalTokenPath,
		GoatCLIRouterEndpoint:  gArgs.GoatCLIRouterEndpoint,
		GoatCLIMTLSCertPath:    gArgs.GoatCLIMTLSCertPath,
		GoatCLIMTLSKeyPath:     gArgs.GoatCLIMTLSKeyPath,
		GoatCLICARootsPath:     gArgs.GoatCLICARootsPath,
		Now:                    fixedNow(),
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
	if !b.HasGoatCLIExtras() {
		t.Fatal("HasGoatCLIExtras() = false; want true")
	}

	x := b.GoatCLIExtras
	if x.PortalEndpoint != args.GoatCLIPortalEndpoint {
		t.Errorf("PortalEndpoint=%q want %q", x.PortalEndpoint, args.GoatCLIPortalEndpoint)
	}
	if x.RouterEndpoint != args.GoatCLIRouterEndpoint {
		t.Errorf("RouterEndpoint=%q want %q", x.RouterEndpoint, args.GoatCLIRouterEndpoint)
	}
	if !bytes.Equal(x.PortalToken, []byte("opaque-portal-bearer-32chars-pad")) {
		t.Errorf("PortalToken did not survive round-trip; got %q", x.PortalToken)
	}
	if !bytes.Contains(x.MTLSCert, []byte("BEGIN CERTIFICATE")) {
		t.Errorf("MTLSCert missing CERTIFICATE header")
	}
	if !bytes.Contains(x.MTLSKey, []byte("BEGIN PRIVATE KEY")) {
		t.Errorf("MTLSKey missing PRIVATE KEY header")
	}
	if !bytes.Contains(x.CARoots, []byte("BEGIN CERTIFICATE")) {
		t.Errorf("CARoots missing CERTIFICATE header")
	}
}

// TestRun_OperatorBundle_NoWGCP0 — operator-bundle mode: when
// --goat-cli-portal-endpoint is set without --wg-cp0-address /
// --wg-cp0-relays, the bundle mints successfully with empty
// KnownEndpoints + zero CPDevice* fields. The goat-cli extras are
// the load-bearing content. Operators run goat-cli + goat-client,
// not wg-cp0; minting them a wg-cp0 keypair was unnecessary
// dead-weight + a needless leak of placeholder privkey material.
func TestRun_OperatorBundle_NoWGCP0(t *testing.T) {
	caKey, ageID, caPub := devCA(t)
	gArgs := fullGoatCLIArgs(t)
	outPath := filepath.Join(t.TempDir(), "operator-bundle.cbor")

	args := runArgs{
		Site:                   "site-a",
		DeviceID:               "operator-example-user",
		ACLGroupsRaw:           "OPERATORS",
		CAID:                   "dev-nebula-ca-test",
		CAKeyPath:              caKey,
		AgeIdentityPath:        ageID,
		OutPath:                outPath,
		TTLDays:                defaultTTLDays,
		ActivationDays:         defaultActivationDays,
		GoatCLIPortalEndpoint:  gArgs.GoatCLIPortalEndpoint,
		GoatCLIPortalTokenPath: gArgs.GoatCLIPortalTokenPath,
		GoatCLIRouterEndpoint:  gArgs.GoatCLIRouterEndpoint,
		GoatCLIMTLSCertPath:    gArgs.GoatCLIMTLSCertPath,
		GoatCLIMTLSKeyPath:     gArgs.GoatCLIMTLSKeyPath,
		GoatCLICARootsPath:     gArgs.GoatCLICARootsPath,
		Now:                    fixedNow(),
		// NOTE: no WGCP0Address, no RelaysPath
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
	if len(b.KnownEndpoints) != 0 {
		t.Errorf("operator bundle should have empty KnownEndpoints, got %d", len(b.KnownEndpoints))
	}
	if len(b.CPDevicePubkey) != 0 || len(b.CPDevicePrivkey) != 0 {
		t.Errorf("operator bundle should have no wg-cp0 keypair, got pub=%d priv=%d",
			len(b.CPDevicePubkey), len(b.CPDevicePrivkey))
	}
	if b.CPDeviceAddress != "" {
		t.Errorf("operator bundle should have no wg-cp0 address, got %q", b.CPDeviceAddress)
	}
	if !b.HasGoatCLIExtras() {
		t.Error("operator bundle should carry full goat-cli extras")
	}
	// Pairing invariant still holds (both absent).
	if err := b.CheckCPDeviceKeypair(); err != nil {
		t.Errorf("CheckCPDeviceKeypair: %v", err)
	}
}

// TestRun_OperatorBundle_RejectsHalfWGCP0 — in operator-bundle mode,
// supplying only one of {--wg-cp0-address, --wg-cp0-relays} is a
// validation error. Either both or neither.
func TestRun_OperatorBundle_RejectsHalfWGCP0(t *testing.T) {
	caKey, ageID, _ := devCA(t)
	gArgs := fullGoatCLIArgs(t)
	args := runArgs{
		Site:                   "site-a",
		DeviceID:               "operator-example-user",
		WGCP0Address:           "10.99.0.42/24", // set
		ACLGroupsRaw:           "OPERATORS",
		CAID:                   "dev-nebula-ca-test",
		CAKeyPath:              caKey,
		AgeIdentityPath:        ageID,
		OutPath:                filepath.Join(t.TempDir(), "out.cbor"),
		TTLDays:                defaultTTLDays,
		ActivationDays:         defaultActivationDays,
		GoatCLIPortalEndpoint:  gArgs.GoatCLIPortalEndpoint,
		GoatCLIPortalTokenPath: gArgs.GoatCLIPortalTokenPath,
		GoatCLIRouterEndpoint:  gArgs.GoatCLIRouterEndpoint,
		GoatCLIMTLSCertPath:    gArgs.GoatCLIMTLSCertPath,
		GoatCLIMTLSKeyPath:     gArgs.GoatCLIMTLSKeyPath,
		GoatCLICARootsPath:     gArgs.GoatCLICARootsPath,
		Now:                    fixedNow(),
		// RelaysPath omitted
	}
	err := run(args)
	if err == nil || !strings.Contains(err.Error(), "must be set together") {
		t.Errorf("expected wg-cp0 atomicity error, got %v", err)
	}
}

// TestRun_GoatCLIExtras_TamperedAtRest — the bundle.cbor is the unit
// of trust. Mutating any goat-cli extras sub-field after signing
// invalidates the signature, so an attacker who swaps the portal
// endpoint in transit can't bypass verification.
func TestRun_GoatCLIExtras_TamperedAtRest(t *testing.T) {
	caKey, ageID, caPub := devCA(t)
	relaysPath := fixtureRelays(t)
	gArgs := fullGoatCLIArgs(t)
	outPath := filepath.Join(t.TempDir(), "operator-bundle.cbor")

	args := runArgs{
		Site:                   "site-a",
		DeviceID:               "operator-example-user",
		WGCP0Address:           "10.99.0.42/24",
		RelaysPath:             relaysPath,
		ACLGroupsRaw:           "OPERATORS",
		CAID:                   "dev-nebula-ca-test",
		CAKeyPath:              caKey,
		AgeIdentityPath:        ageID,
		OutPath:                outPath,
		TTLDays:                defaultTTLDays,
		ActivationDays:         defaultActivationDays,
		GoatCLIPortalEndpoint:  gArgs.GoatCLIPortalEndpoint,
		GoatCLIPortalTokenPath: gArgs.GoatCLIPortalTokenPath,
		GoatCLIRouterEndpoint:  gArgs.GoatCLIRouterEndpoint,
		GoatCLIMTLSCertPath:    gArgs.GoatCLIMTLSCertPath,
		GoatCLIMTLSKeyPath:     gArgs.GoatCLIMTLSKeyPath,
		GoatCLICARootsPath:     gArgs.GoatCLICARootsPath,
		Now:                    fixedNow(),
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
	// Tamper after parse.
	b.GoatCLIExtras.PortalEndpoint = "https://attacker.example/"
	if err := b.Verify(caPub); err == nil {
		t.Error("verify should fail after PortalEndpoint tamper")
	}
}
