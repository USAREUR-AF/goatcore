package x509ca

import (
	"crypto"
	"crypto/ecdsa"
	"crypto/ed25519"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha1" //nolint:gosec // X.509 SubjectKeyIdentifier per RFC 5280; SHA-1 is the standard choice and is not used for security here.
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"errors"
	"fmt"
	"math/big"
	"net/url"
	"time"

	"github.com/usareur-af/goatcore/ops/enrollment/ca"
)

// LeafKeyType selects the leaf private-key algorithm.
type LeafKeyType int

const (
	// LeafKeyECDSAP256 is the default post-Block-79. Universally
	// supported across macOS / iOS / Android / Windows / Linux trust stores
	// and TLS stacks, no F-071 LibreSSL gotchas, no F-090 Keychain rejection.
	// Chain is full ECDSA P-256 from root to leaf.
	LeafKeyECDSAP256 LeafKeyType = 0
	// LeafKeyEd25519 is retained for callers that explicitly want the
	// small / fast / post-quantum-hybrid-friendly Ed25519 algorithm. NOT
	// the default — F-071 surfaces on macOS LibreSSL 3.3.6 + some macOS
	// browser TLS stacks (no Ed25519 in TLS 1.3 signature_algorithms,
	// causing SSL_ERROR_NO_CYPHER_OVERLAP). Use only when the leaf
	// consumer is known to be Ed25519-friendly.
	LeafKeyEd25519 LeafKeyType = 1
)

// Issued is the output of IssueCert. The leaf private key never crosses a
// process boundary inside the CA; the caller writes both PEMs under their
// own custody (per docs/design/internal-tls-ca.md §4.1).
type Issued struct {
	// CertPEM is the leaf X.509 certificate in PEM form.
	CertPEM []byte
	// KeyPEM is the leaf Ed25519 private key in PKCS8 PEM form.
	KeyPEM []byte
	// SerialNumber is the cert's RFC 5280 random 128-bit serial.
	SerialNumber *big.Int
	// NotAfter is the cert's expiry timestamp (UTC).
	NotAfter time.Time
}

// IssueCert produces a leaf X.509 certificate signed by the offline CA
// root. The leaf's Ed25519 keypair is generated inside this function so
// the leaf private key never crosses a process boundary into the CA.
//
// signer signs with the offline CA root key (ADR 0406 — age-encrypted
// file in dev mode, YubiKey PIV in production when implemented).
//
// rootCert is the CA's self-signed root cert produced by GenerateRootCert.
// Its SubjectKeyId becomes the leaf's AuthorityKeyId, anchoring the chain.
//
// Returns an Issued bundle of cert+key PEMs ready for delivery to the
// service host.
func IssueCert(template *LeafTemplate, signer ca.Signer, rootCert *x509.Certificate) (*Issued, error) {
	if err := template.validate(); err != nil {
		return nil, err
	}
	if signer == nil {
		return nil, errors.New("x509ca: nil ca signer")
	}
	if rootCert == nil {
		return nil, errors.New("x509ca: nil root cert (run GenerateRootCert first)")
	}
	tpl := template.withDefaults()

	var (
		leafPub  crypto.PublicKey
		leafPriv crypto.PrivateKey
		err      error
	)
	switch tpl.KeyType {
	case LeafKeyEd25519:
		pub, priv, gerr := ed25519.GenerateKey(rand.Reader)
		if gerr != nil {
			return nil, fmt.Errorf("generate ed25519 leaf keypair: %w", gerr)
		}
		leafPub = pub
		leafPriv = priv
	default: // LeafKeyECDSAP256 (post-Block-79 default)
		k, gerr := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
		if gerr != nil {
			return nil, fmt.Errorf("generate ecdsa-p256 leaf keypair: %w", gerr)
		}
		leafPub = &k.PublicKey
		leafPriv = k
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
	notBefore := now.Add(-1 * time.Minute) // small skew tolerance
	notAfter := now.Add(tpl.Validity)

	// RFC 5280 SubjectKeyIdentifier — SHA-1 over the SubjectPublicKeyInfo's
	// BIT STRING content. We marshal the SPKI and re-parse the BIT STRING
	// content the simple way: derive from the marshalled SubjectPublicKeyInfo's
	// trailing bytes. For both Ed25519 and ECDSA P-256, sha1 of the raw
	// public-key bytes is acceptable per RFC 5280's "common method".
	leafSPKIBytes, err := publicKeyBytes(leafPub)
	if err != nil {
		return nil, fmt.Errorf("derive leaf SubjectKeyId source: %w", err)
	}
	leafSPKI := sha1.Sum(leafSPKIBytes) //nolint:gosec // RFC 5280 SubjectKeyIdentifier per X509KeyId formula.

	leafTemplate := &x509.Certificate{
		SerialNumber: serial,
		Subject: pkix.Name{
			Organization:       []string{tpl.Organization},
			OrganizationalUnit: []string{tpl.OrgUnit},
			CommonName:         tpl.CommonName,
		},
		NotBefore:             notBefore,
		NotAfter:              notAfter,
		KeyUsage:              x509.KeyUsageDigitalSignature | x509.KeyUsageKeyEncipherment,
		ExtKeyUsage:           tpl.EKU,
		BasicConstraintsValid: true,
		IsCA:                  false,
		DNSNames:              dnsNames,
		IPAddresses:           ipAddrs,
		URIs:                  uriSANs,
		SubjectKeyId:          leafSPKI[:],
		AuthorityKeyId:        rootCert.SubjectKeyId,
		// SignatureAlgorithm is auto-derived as PureEd25519 from the signer's
		// public key type (ed25519.PublicKey).
	}

	certDER, err := x509.CreateCertificate(rand.Reader, leafTemplate, rootCert, leafPub, adapt(signer))
	if err != nil {
		return nil, fmt.Errorf("create certificate: %w", err)
	}

	certPEM := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: certDER})

	keyDER, err := x509.MarshalPKCS8PrivateKey(leafPriv)
	if err != nil {
		return nil, fmt.Errorf("marshal leaf private key: %w", err)
	}
	keyPEM := pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: keyDER})

	return &Issued{
		CertPEM:      certPEM,
		KeyPEM:       keyPEM,
		SerialNumber: serial,
		NotAfter:     notAfter,
	}, nil
}

// publicKeyBytes returns a representation of pub suitable as input to the
// SHA-1 used in the SubjectKeyIdentifier formula. Marshals the full SPKI
// (SubjectPublicKeyInfo) — works uniformly for Ed25519 and ECDSA P-256.
func publicKeyBytes(pub crypto.PublicKey) ([]byte, error) {
	return x509.MarshalPKIXPublicKey(pub)
}

// parseURISANs parses each entry as a URI per net/url.Parse and returns
// the parsed *url.URL slice ready for assignment to x509.Certificate.URIs.
// Nil / empty input returns nil. Entries must be absolute URIs (have a
// non-empty scheme) so the resulting cert SAN is unambiguous per RFC 5280.
func parseURISANs(raw []string) ([]*url.URL, error) {
	if len(raw) == 0 {
		return nil, nil
	}
	out := make([]*url.URL, 0, len(raw))
	for i, s := range raw {
		if s == "" {
			return nil, fmt.Errorf("URI-SAN[%d] is empty", i)
		}
		u, err := url.Parse(s)
		if err != nil {
			return nil, fmt.Errorf("URI-SAN[%d] %q: %w", i, s, err)
		}
		if u.Scheme == "" {
			return nil, fmt.Errorf("URI-SAN[%d] %q: missing scheme (must be an absolute URI, e.g. urn:goat-galaxy:vendor=example-org)", i, s)
		}
		out = append(out, u)
	}
	return out, nil
}

// randomSerial returns a uniformly-random positive 128-bit integer suitable
// for use as an X.509 certificate serial number per RFC 5280 §4.1.2.2.
func randomSerial() (*big.Int, error) {
	limit := new(big.Int).Lsh(big.NewInt(1), 128)
	n, err := rand.Int(rand.Reader, limit)
	if err != nil {
		return nil, err
	}
	if n.Sign() == 0 {
		return big.NewInt(1), nil
	}
	return n, nil
}
