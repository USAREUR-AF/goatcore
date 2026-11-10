package api

import (
	"strings"
	"testing"
)

// probe_test.go — interpreter-only tests. The bash script itself is
// exercised by the live-fire drill the operator runs; here we cover the
// per-layer Go interpretation of probe JSON pastes.

func TestInterpretProbe_AllLayersPass(t *testing.T) {
	res := probeResult{
		SchemaVersion: 1,
		Hostname:      "zenoh.example.net",
		Port:          7447,
		IntendedPrefix: "my-vendor",
		DNS: &probeLayerDNS{OK: true, ResolvedIPs: []string{"192.0.2.2"}},
		TCP: &probeLayerTCP{OK: true},
		TLS: &probeLayerTLS{OK: true, NegotiatedCipher: "TLS_AES_256_GCM_SHA384", NegotiatedProto: "TLSv1.3"},
		CertChain: &probeLayerChain{OK: true, ObservedFingerprintSHA256: "AABBCCDD"},
		URISAN: &probeLayerURISAN{OK: true, DeclaredPrefix: "my-vendor", CertURISANs: []string{"spiffe://goat.example/my-vendor"}},
	}
	findings := interpretProbe(res, "AABBCCDD")
	if got := overallStatus(findings); got != "pass" {
		t.Fatalf("expected pass, got %s; findings=%+v", got, findings)
	}
	for _, f := range findings {
		if f.Status != "pass" {
			t.Errorf("layer %s: expected pass, got %s (%s)", f.Layer, f.Status, f.Message)
		}
	}
}

func TestInterpretProbe_DNSFailsCascadesSkips(t *testing.T) {
	res := probeResult{
		DNS: &probeLayerDNS{OK: false, Error: "name not known"},
	}
	findings := interpretProbe(res, "")
	if got := overallStatus(findings); got != "fail" {
		t.Fatalf("expected fail, got %s", got)
	}
	// First finding is the DNS fail.
	if findings[0].Status != "fail" || findings[0].Layer != "1-dns" {
		t.Fatalf("expected first finding to be DNS fail, got %+v", findings[0])
	}
	// Subsequent layers must be skips.
	for _, f := range findings[1:] {
		if f.Status != "skip" {
			t.Errorf("after DNS fail, expected skip on %s, got %s", f.Layer, f.Status)
		}
	}
}

func TestInterpretProbe_TCPFailsCascadesSkips(t *testing.T) {
	res := probeResult{
		DNS: &probeLayerDNS{OK: true, ResolvedIPs: []string{"1.2.3.4"}},
		TCP: &probeLayerTCP{OK: false, Error: "connection timed out"},
	}
	findings := interpretProbe(res, "")
	if findings[0].Status != "pass" {
		t.Fatalf("DNS should pass; got %+v", findings[0])
	}
	if findings[1].Status != "fail" || findings[1].Layer != "2-tcp" {
		t.Fatalf("expected TCP fail at index 1; got %+v", findings[1])
	}
	// TLS/chain/URI-SAN should be skips.
	for _, f := range findings[2:] {
		if f.Status != "skip" {
			t.Errorf("after TCP fail, expected skip on %s, got %s", f.Layer, f.Status)
		}
	}
}

func TestInterpretProbe_FingerprintMismatch(t *testing.T) {
	res := probeResult{
		DNS: &probeLayerDNS{OK: true, ResolvedIPs: []string{"1.2.3.4"}},
		TCP: &probeLayerTCP{OK: true},
		TLS: &probeLayerTLS{OK: true},
		CertChain: &probeLayerChain{OK: true, ObservedFingerprintSHA256: "DEADBEEF"},
	}
	findings := interpretProbe(res, "FACEFEED")
	chain := findingByLayer(findings, "4-cert-chain")
	if chain == nil {
		t.Fatal("missing cert-chain finding")
	}
	if chain.Status != "fail" {
		t.Fatalf("expected fingerprint mismatch to fail layer 4, got %s", chain.Status)
	}
	if !strings.Contains(chain.Message, "DEADBEEF") {
		t.Errorf("expected observed fingerprint in message, got %s", chain.Message)
	}
}

func TestInterpretProbe_URISANMismatch(t *testing.T) {
	res := probeResult{
		DNS: &probeLayerDNS{OK: true, ResolvedIPs: []string{"1.2.3.4"}},
		TCP: &probeLayerTCP{OK: true},
		TLS: &probeLayerTLS{OK: true},
		CertChain: &probeLayerChain{OK: true, ObservedFingerprintSHA256: "AA"},
		URISAN: &probeLayerURISAN{OK: false, DeclaredPrefix: "my-vendor", CertURISANs: []string{"spiffe://goat.example/other-vendor"}, Error: "no URI-SAN contains the declared prefix my-vendor"},
	}
	findings := interpretProbe(res, "AA")
	uri := findingByLayer(findings, "5-uri-san")
	if uri == nil {
		t.Fatal("missing URI-SAN finding")
	}
	if uri.Status != "fail" {
		t.Fatalf("expected URI-SAN fail, got %s", uri.Status)
	}
	if !strings.Contains(uri.Message, "other-vendor") {
		t.Errorf("expected cert URI-SAN in message, got %s", uri.Message)
	}
}

func TestInterpretProbe_PartialWhenSkipPresent(t *testing.T) {
	res := probeResult{
		DNS: &probeLayerDNS{OK: true, ResolvedIPs: []string{"1.2.3.4"}},
		TCP: &probeLayerTCP{OK: true},
		TLS: &probeLayerTLS{OK: true},
		CertChain: &probeLayerChain{OK: true, ObservedFingerprintSHA256: "AA"},
		// URISAN absent — script ran without --cert.
	}
	findings := interpretProbe(res, "AA")
	if got := overallStatus(findings); got != "partial" {
		t.Fatalf("expected partial when URI-SAN absent, got %s", got)
	}
	uri := findingByLayer(findings, "5-uri-san")
	if uri == nil || uri.Status != "skip" {
		t.Fatalf("expected URI-SAN to be skip; got %+v", uri)
	}
}

// --- helpers ---

func findingByLayer(findings []probeFinding, layer string) *probeFinding {
	for i := range findings {
		if findings[i].Layer == layer {
			return &findings[i]
		}
	}
	return nil
}
