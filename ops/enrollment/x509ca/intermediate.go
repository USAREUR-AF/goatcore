package x509ca

import (
	"crypto"
	"crypto/ecdsa"
	"crypto/ed25519"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha1" //nolint:gosec // X.509 SubjectKeyIdentifier per RFC 5280; not used for security.
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"errors"
	"fmt"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/ca"
)

// IntermediateCertOptions configures GenerateIntermediate. Zero values
// yield sensible defaults documented per field.
type IntermediateCertOptions struct {
	// Organization defaults to DefaultOrganization.
	Organization string
	// CommonName defaults to DefaultIntermediateCommonName.
	CommonName string
	// OrgUnit defaults to "intermediate-ca".
	OrgUnit string
	// Validity defaults to 5 * 365 * 24h (5 years). Bounded by the parent
	// root's validity; callers should stay safely under it.
	Validity time.Duration
}

// GenerateIntermediate produces an ECDSA-P256 X.509 intermediate cert
// signed by the offline-CA root (held by parentSigner). Intent: Path B
// closure for F-090 — macOS Keychain Services cannot import the
// Ed25519 offline-CA root, so we sign an ECDSA-P256 intermediate from
// the Ed25519 root and operators install the INTERMEDIATE in their
// keychain (which macOS does accept). Leaves chain to intermediate;
// browsers validate without per-leaf trust hacks.
//
// The returned intermediate private key is in PKCS8 PEM form. Callers
// are expected to wrap it in age encryption before persisting (mirroring
// the dev-CA root's storage shape — see ca.GenerateAndWrite). The
// intermediate signs leaves; cert-sign loads it via --intermediate-cert
// + --intermediate-key when the operator opts in.
//
// Bundle signing remains Ed25519 (signed by parentSigner directly per
// the existing path); only X.509 cert chains route through the
// intermediate. The two workloads decouple cleanly.
func GenerateIntermediate(parentSigner ca.Signer, parentCert *x509.Certificate, opts IntermediateCertOptions) (certPEM, keyPEM []byte, err error) {
	if parentSigner == nil {
		return nil, nil, errors.New("x509ca: nil parent signer")
	}
	if parentCert == nil {
		return nil, nil, errors.New("x509ca: nil parent cert (run GenerateRootCert first)")
	}
	if opts.Organization == "" {
		opts.Organization = DefaultOrganization
	}
	if opts.CommonName == "" {
		opts.CommonName = DefaultIntermediateCommonName
	}
	if opts.OrgUnit == "" {
		opts.OrgUnit = "intermediate-ca"
	}
	if opts.Validity == 0 {
		opts.Validity = 5 * 365 * 24 * time.Hour
	}

	// Generate intermediate ECDSA-P256 keypair. macOS Keychain Services
	// imports ECDSA roots; that's the whole point of this tier.
	intKey, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, nil, fmt.Errorf("generate intermediate ecdsa-p256 keypair: %w", err)
	}
	intPub := &intKey.PublicKey

	serial, err := randomSerial()
	if err != nil {
		return nil, nil, fmt.Errorf("generate serial: %w", err)
	}

	now := time.Now().UTC()
	intSPKIBytes, err := x509.MarshalPKIXPublicKey(intPub)
	if err != nil {
		return nil, nil, fmt.Errorf("marshal intermediate pubkey: %w", err)
	}
	skid := sha1.Sum(intSPKIBytes) //nolint:gosec // RFC 5280 SubjectKeyIdentifier.

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
		MaxPathLen:            0,
		MaxPathLenZero:        true,
		SubjectKeyId:          skid[:],
		AuthorityKeyId:        parentCert.SubjectKeyId,
	}

	certDER, err := x509.CreateCertificate(rand.Reader, template, parentCert, intPub, adapt(parentSigner))
	if err != nil {
		return nil, nil, fmt.Errorf("create intermediate cert: %w", err)
	}

	certPEM = pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: certDER})

	keyDER, err := x509.MarshalPKCS8PrivateKey(intKey)
	if err != nil {
		return nil, nil, fmt.Errorf("marshal intermediate private key: %w", err)
	}
	keyPEM = pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: keyDER})

	return certPEM, keyPEM, nil
}

// IssueCertViaIntermediate produces a leaf X.509 cert chained through
// an ECDSA-P256 intermediate. The intermediate signs the leaf using its
// own *ecdsa.PrivateKey (which already implements crypto.Signer);
// crypto/x509.CreateCertificate accepts any crypto.Signer as parent so
// this path needs no adapter shim.
//
// Use IssueCert (root-signed) when an operator-facing macOS browser
// will NOT see the cert (e.g. mesh-internal services accessed via
// programmatic clients). Use this function when chrome/safari on
// macOS WILL see the cert and per-leaf trust hacks are not desired.
func IssueCertViaIntermediate(template *LeafTemplate, intKey *ecdsa.PrivateKey, intCert *x509.Certificate) (*Issued, error) {
	if err := template.validate(); err != nil {
		return nil, err
	}
	if intKey == nil {
		return nil, errors.New("x509ca: nil intermediate key")
	}
	if intCert == nil {
		return nil, errors.New("x509ca: nil intermediate cert")
	}
	tpl := template.withDefaults()

	var (
		leafPub  crypto.PublicKey
		leafPriv crypto.PrivateKey
	)
	switch tpl.KeyType {
	case LeafKeyECDSAP256:
		k, gerr := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
		if gerr != nil {
			return nil, fmt.Errorf("generate ecdsa-p256 leaf keypair: %w", gerr)
		}
		leafPub = &k.PublicKey
		leafPriv = k
	default: // LeafKeyEd25519
		pub, priv, gerr := ed25519.GenerateKey(rand.Reader)
		if gerr != nil {
			return nil, fmt.Errorf("generate ed25519 leaf keypair: %w", gerr)
		}
		leafPub = pub
		leafPriv = priv
	}

	dnsNames, ipAddrs, err := classifySANs(tpl.SANs)
	if err != nil {
		return nil, fmt.Errorf("classify SANs: %w", err)
	}

	uriSANs, err := parseURISANs(tpl.URISANs)
	if err != nil {
		return nil, fmt.Errorf("parse URI SANs: %w", err)
	}

	serial, err := randomSerial()
	if err != nil {
		return nil, fmt.Errorf("generate serial: %w", err)
	}

	now := time.Now().UTC()
	leafSPKIBytes, err := x509.MarshalPKIXPublicKey(leafPub)
	if err != nil {
		return nil, fmt.Errorf("marshal leaf pubkey: %w", err)
	}
	leafSPKI := sha1.Sum(leafSPKIBytes) //nolint:gosec // RFC 5280 SubjectKeyIdentifier.

	leafTemplate := &x509.Certificate{
		SerialNumber: serial,
		Subject: pkix.Name{
			Organization:       []string{tpl.Organization},
			OrganizationalUnit: []string{tpl.OrgUnit},
			CommonName:         tpl.CommonName,
		},
		NotBefore:             now.Add(-1 * time.Minute),
		NotAfter:              now.Add(tpl.Validity),
		KeyUsage:              x509.KeyUsageDigitalSignature | x509.KeyUsageKeyEncipherment,
		ExtKeyUsage:           tpl.EKU,
		BasicConstraintsValid: true,
		IsCA:                  false,
		DNSNames:              dnsNames,
		IPAddresses:           ipAddrs,
		URIs:                  uriSANs,
		SubjectKeyId:          leafSPKI[:],
		AuthorityKeyId:        intCert.SubjectKeyId,
	}

	// intKey already implements crypto.Signer (ECDSA *ecdsa.PrivateKey
	// satisfies the interface). No adapter shim needed — unlike the
	// root path which wraps the Ed25519-only ca.Signer interface.
	certDER, err := x509.CreateCertificate(rand.Reader, leafTemplate, intCert, leafPub, intKey)
	if err != nil {
		return nil, fmt.Errorf("create leaf cert: %w", err)
	}

	certPEM := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: certDER})

	keyDER, err := x509.MarshalPKCS8PrivateKey(leafPriv)
	if err != nil {
		return nil, fmt.Errorf("marshal leaf private key: %w", err)
	}
	leafKeyPEM := pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: keyDER})

	return &Issued{
		CertPEM:      certPEM,
		KeyPEM:       leafKeyPEM,
		SerialNumber: serial,
		NotAfter:     leafTemplate.NotAfter,
	}, nil
}
