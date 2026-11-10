package x509ca

import (
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha1" //nolint:gosec // SubjectKeyIdentifier per RFC 5280; not used for security.
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"errors"
	"fmt"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/ca"
)

// RootCertOptions configures GenerateRootCert. Zero values yield sensible
// defaults documented per field.
type RootCertOptions struct {
	// Organization defaults to DefaultOrganization.
	Organization string
	// CommonName defaults to DefaultRootCommonName.
	CommonName string
	// OrgUnit defaults to "offline-ca".
	OrgUnit string
	// Validity defaults to 10 * 365 * 24h (10 years).
	Validity time.Duration
}

// GenerateRootCert produces a self-signed X.509 v3 root cert from the
// existing offline CA Ed25519 key (held by signer). The signer's public
// key becomes the cert's SubjectPublicKeyInfo; the signer signs the TBS
// bytes.
//
// Output is a PEM-encoded cert ready for distribution as the bundle's
// tls_trust_anchor (DER-decoded form) and as the trust anchor mounted on
// service hosts that need to verify chain-to-root.
//
// The same root cert is used for the lifetime of the offline CA root key.
// Re-issued only on root key rotation (separate ceremony, ADR 0406).
func GenerateRootCert(signer ca.Signer, opts RootCertOptions) (certPEM []byte, parsed *x509.Certificate, err error) {
	if signer == nil {
		return nil, nil, errors.New("x509ca: nil ca signer")
	}
	if opts.Organization == "" {
		opts.Organization = DefaultOrganization
	}
	if opts.CommonName == "" {
		opts.CommonName = DefaultRootCommonName
	}
	if opts.OrgUnit == "" {
		opts.OrgUnit = "offline-ca"
	}
	if opts.Validity == 0 {
		opts.Validity = 10 * 365 * 24 * time.Hour
	}

	pub := signer.PublicKey()
	if pub == nil {
		return nil, nil, errors.New("ca signer returned nil public key")
	}
	if pub.Curve.Params().Name != "P-256" {
		return nil, nil, fmt.Errorf("ca signer curve is %s, want P-256 (Block 79)", pub.Curve.Params().Name)
	}

	serial, err := randomSerial()
	if err != nil {
		return nil, nil, fmt.Errorf("generate serial: %w", err)
	}

	now := time.Now().UTC()
	// SubjectKeyIdentifier per RFC 5280: SHA-1 of the uncompressed
	// SubjectPublicKey BIT STRING (X || Y prefixed by 0x04 for ECDSA).
	pubBytes := elliptic.Marshal(pub.Curve, pub.X, pub.Y)
	skid := sha1.Sum(pubBytes) //nolint:gosec // RFC 5280 SubjectKeyIdentifier.

	template := &x509.Certificate{
		SerialNumber: serial,
		Subject: pkix.Name{
			Organization:       []string{opts.Organization},
			OrganizationalUnit: []string{opts.OrgUnit},
			CommonName:         opts.CommonName,
		},
		NotBefore:             now.Add(-1 * time.Minute),
		NotAfter:              now.Add(opts.Validity),
		KeyUsage:              x509.KeyUsageCertSign | x509.KeyUsageCRLSign,
		BasicConstraintsValid: true,
		IsCA:                  true,
		MaxPathLen:            0, // single-tier per ADR 0407
		MaxPathLenZero:        true,
		SubjectKeyId:          skid[:],
	}

	// Self-signed: parent == template. crypto/x509 walks template==parent
	// and produces a v3 cert with Subject==Issuer.
	certDER, err := x509.CreateCertificate(rand.Reader, template, template, pub, adapt(signer))
	if err != nil {
		return nil, nil, fmt.Errorf("create root cert: %w", err)
	}

	parsed, err = x509.ParseCertificate(certDER)
	if err != nil {
		return nil, nil, fmt.Errorf("parse generated root cert: %w", err)
	}

	certPEM = pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: certDER})
	return certPEM, parsed, nil
}

// LoadRootCert reads a PEM-encoded root cert from bytes and returns the
// parsed structure. Used by IssueCert callers loading the root cert that
// GenerateRootCert produced earlier.
func LoadRootCert(pemBytes []byte) (*x509.Certificate, error) {
	block, _ := pem.Decode(pemBytes)
	if block == nil {
		return nil, errors.New("x509ca: no PEM block found")
	}
	if block.Type != "CERTIFICATE" {
		return nil, fmt.Errorf("x509ca: PEM block has wrong type %q (want CERTIFICATE)", block.Type)
	}
	cert, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		return nil, fmt.Errorf("parse root cert: %w", err)
	}
	if !cert.IsCA {
		return nil, errors.New("x509ca: cert is not a CA")
	}
	return cert, nil
}
