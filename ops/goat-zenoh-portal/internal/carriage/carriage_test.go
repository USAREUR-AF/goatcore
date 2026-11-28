package carriage

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/json"
	"encoding/pem"
	"math/big"
	"strings"
	"testing"
	"time"
)

// testCert makes a certificate for key, self-signed when parent is nil.
func testCert(t *testing.T, cn string, key *ecdsa.PrivateKey, parent *x509.Certificate, parentKey *ecdsa.PrivateKey, isCA bool) *x509.Certificate {
	t.Helper()
	tmpl := &x509.Certificate{
		SerialNumber:          big.NewInt(time.Now().UnixNano()),
		Subject:               pkix.Name{CommonName: cn},
		NotBefore:             time.Now().Add(-time.Hour),
		NotAfter:              time.Now().Add(time.Hour),
		BasicConstraintsValid: true,
		IsCA:                  isCA,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth},
		KeyUsage:              x509.KeyUsageDigitalSignature | x509.KeyUsageCertSign,
	}
	if parent == nil {
		parent, parentKey = tmpl, key
	}
	der, err := x509.CreateCertificate(rand.Reader, tmpl, parent, &key.PublicKey, parentKey)
	if err != nil {
		t.Fatal(err)
	}
	c, err := x509.ParseCertificate(der)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func newKey(t *testing.T) *ecdsa.PrivateKey {
	t.Helper()
	k, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	return k
}

func pemOf(t *testing.T, key *ecdsa.PrivateKey, cert *x509.Certificate) ([]byte, []byte) {
	t.Helper()
	der, err := x509.MarshalPKCS8PrivateKey(key)
	if err != nil {
		t.Fatal(err)
	}
	return pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: der}),
		pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: cert.Raw})
}

func sampleGeneration() Generation {
	until := "2027-02-17T00:00:00Z"
	return Generation{
		Schema: Schema, PodSlot: strings.Repeat("a", 32), Generation: 3, IssuedAt: "2026-11-19T10:00:00Z",
		LocalAuthority: []Authority{{Identity: "us-dod:1234567890", CertCN: "lead<&>"}},
		Read:           []string{"*/**"},
		Sources: []Source{
			{Slot: strings.Repeat("b", 32), RuleID: "cr_1", Kind: "org", Principals: []string{strings.Repeat("b", 32) + ".g2"}, Write: DefaultWrite(strings.Repeat("b", 32))},
			{Slot: strings.Repeat("c", 32), RuleID: "cr_2", Kind: "named", Until: &until, Principals: []string{strings.Repeat("d", 32)}, Write: DefaultWrite(strings.Repeat("c", 32))},
		},
	}
}

func TestCanonicalSortedCompactUnescaped(t *testing.T) {
	b, err := Canonical(map[string]any{"b": 1, "a": "<&>", "c": []int{2, 1}})
	if err != nil {
		t.Fatal(err)
	}
	if got, want := string(b), `{"a":"<&>","b":1,"c":[2,1]}`; got != want {
		t.Fatalf("canonical = %s, want %s", got, want)
	}
}

func TestGenerationSignVerifyAndTamper(t *testing.T) {
	key := newKey(t)
	cert := testCert(t, "portal-carriage-signer", key, nil, nil, false)
	kp, cp := pemOf(t, key, cert)
	signer, err := ParseSigner(kp, cp)
	if err != nil {
		t.Fatal(err)
	}
	raw, err := signer.Sign(sampleGeneration())
	if err != nil {
		t.Fatal(err)
	}
	g, err := VerifyGeneration(raw, cert)
	if err != nil {
		t.Fatalf("verify: %v", err)
	}
	if g.Signer != "portal-carriage-signer" || g.Generation != 3 || len(g.Sources) != 2 {
		t.Fatalf("decoded generation wrong: %+v", g)
	}
	// The org rule's until is an explicit null (§7.1).
	if !strings.Contains(string(raw), `"until":null`) {
		t.Fatalf("org rule until must serialize as null: %s", raw)
	}
	// Byte-level: what was signed is the canonical document without signature.
	var m map[string]any
	_ = json.Unmarshal(raw, &m)
	m["generation"] = 4
	tampered, _ := json.Marshal(m)
	if _, err := VerifyGeneration(tampered, cert); err == nil {
		t.Fatal("a changed generation number verified")
	}
	other := newKey(t)
	if _, err := VerifyGeneration(raw, testCert(t, "portal-carriage-signer", other, nil, nil, false)); err == nil {
		t.Fatal("verified against the wrong key")
	}
}

func TestParseSignerRefusesMismatchAndWrongCurve(t *testing.T) {
	key := newKey(t)
	cert := testCert(t, "s", key, nil, nil, false)
	_, cp := pemOf(t, key, cert)
	other := newKey(t)
	kp2, _ := pemOf(t, other, cert)
	if _, err := ParseSigner(kp2, cp); err == nil {
		t.Fatal("a key that does not match the certificate was accepted")
	}
	p384, _ := ecdsa.GenerateKey(elliptic.P384(), rand.Reader)
	der, _ := x509.MarshalPKCS8PrivateKey(p384)
	if _, err := ParseSigner(pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: der}), cp); err == nil {
		t.Fatal("a P-384 key was accepted")
	}
}

func TestContentHashIgnoresIssueFields(t *testing.T) {
	a := sampleGeneration()
	b := a
	b.Generation, b.IssuedAt, b.Signer, b.Signature = 99, "2030-01-01T00:00:00Z", "x", "y"
	ha, _ := ContentHash(a)
	hb, _ := ContentHash(b)
	if ha != hb {
		t.Fatal("content hash changed with generation/issued_at/signer/signature")
	}
	b.Read = []string{"other/**"}
	hc, _ := ContentHash(b)
	if hc == ha {
		t.Fatal("content hash did not change with the read set")
	}
}

func TestLedgerEntryVerify(t *testing.T) {
	caKey := newKey(t)
	ca := testCert(t, "goatnet-ca", caKey, nil, nil, true)
	podKey := newKey(t)
	pod := testCert(t, strings.Repeat("a", 32)+".g1", podKey, ca, caKey, false)
	roots := x509.NewCertPool()
	roots.AddCert(ca)
	if err := VerifyChain(pod, roots, time.Now()); err != nil {
		t.Fatalf("chain: %v", err)
	}
	if err := VerifyChain(testCert(t, "x", newKey(t), nil, nil, false), roots, time.Now()); err == nil {
		t.Fatal("a self-signed leaf chained")
	}
	e, err := SignLedgerEntry(podKey, LedgerEntry{
		V: 1, Seq: 1, TS: "2026-11-19T10:00:00Z", PodSlot: strings.Repeat("a", 32), Action: ActionAllow,
		RuleID: "lr_1", SourceSlot: strings.Repeat("b", 32), Until: "2026-11-20T10:00:00Z", Reason: "field radio",
		Authority: Authority{Identity: "us-dod:1234567890", CertCN: "lead"},
	})
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(e)
	got, canon, err := VerifyLedgerEntry(raw, pod)
	if err != nil {
		t.Fatalf("verify: %v", err)
	}
	if got.Seq != 1 || len(canon) == 0 {
		t.Fatalf("decoded entry wrong: %+v", got)
	}
	tampered := strings.Replace(string(raw), "field radio", "field radar", 1)
	if _, _, err := VerifyLedgerEntry([]byte(tampered), pod); err == nil {
		t.Fatal("a changed reason verified")
	}
	if _, _, err := VerifyLedgerEntry(raw, ca); err == nil {
		t.Fatal("verified against a certificate that is not the pod's")
	}
}
