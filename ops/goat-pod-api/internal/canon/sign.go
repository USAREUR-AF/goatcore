package canon

import (
	"crypto"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/pem"
	"errors"
	"fmt"
	"os"
)

// p256 reports whether pub is an ECDSA P-256 key, the only key type goat carriage signs with.
func p256(pub crypto.PublicKey) (*ecdsa.PublicKey, bool) {
	k, ok := pub.(*ecdsa.PublicKey)
	return k, ok && k.Curve == elliptic.P256()
}

// Sign signs prefix||body with an ECDSA P-256 key: ASN.1 DER over the SHA-256 digest, standard
// base64.
func Sign(key crypto.Signer, prefix string, body []byte) (string, error) {
	if _, ok := p256(key.Public()); !ok {
		return "", errors.New("signing key must be ECDSA P-256")
	}
	digest := sha256.Sum256(append([]byte(prefix), body...))
	sig, err := key.Sign(rand.Reader, digest[:], crypto.SHA256)
	if err != nil {
		return "", fmt.Errorf("sign: %w", err)
	}
	return base64.StdEncoding.EncodeToString(sig), nil
}

// Verify checks a base64 signature made by Sign.
func Verify(pub crypto.PublicKey, prefix string, body []byte, sigB64 string) error {
	sig, err := base64.StdEncoding.DecodeString(sigB64)
	if err != nil {
		return errors.New("signature is not base64")
	}
	k, ok := p256(pub)
	if !ok {
		return errors.New("signer key must be ECDSA P-256")
	}
	digest := sha256.Sum256(append([]byte(prefix), body...))
	if !ecdsa.VerifyASN1(k, digest[:], sig) {
		return errors.New("signature does not verify")
	}
	return nil
}

// LoadCertificate reads the first PEM certificate in path.
func LoadCertificate(path string) (*x509.Certificate, error) {
	raw, err := os.ReadFile(path) // #nosec G304 -- path comes from the operator's configuration
	if err != nil {
		return nil, fmt.Errorf("read certificate %s: %w", path, err)
	}
	for {
		var block *pem.Block
		block, raw = pem.Decode(raw)
		if block == nil {
			return nil, fmt.Errorf("no PEM certificate in %s", path)
		}
		if block.Type == "CERTIFICATE" {
			cert, err := x509.ParseCertificate(block.Bytes)
			if err != nil {
				return nil, fmt.Errorf("parse certificate %s: %w", path, err)
			}
			return cert, nil
		}
	}
}

// LoadPrivateKey reads a PEM ECDSA P-256 private key (PKCS#8 or SEC 1).
func LoadPrivateKey(path string) (crypto.Signer, error) {
	raw, err := os.ReadFile(path) // #nosec G304 -- path comes from the operator's configuration
	if err != nil {
		return nil, fmt.Errorf("read key %s: %w", path, err)
	}
	for {
		var block *pem.Block
		block, raw = pem.Decode(raw)
		if block == nil {
			return nil, fmt.Errorf("no PEM private key in %s", path)
		}
		switch block.Type {
		case "PRIVATE KEY":
			k, err := x509.ParsePKCS8PrivateKey(block.Bytes)
			if err != nil {
				return nil, fmt.Errorf("parse key %s: %w", path, err)
			}
			s, ok := k.(*ecdsa.PrivateKey)
			if !ok || s.Curve != elliptic.P256() {
				return nil, fmt.Errorf("key in %s is not ECDSA P-256", path)
			}
			return s, nil
		case "EC PRIVATE KEY":
			k, err := x509.ParseECPrivateKey(block.Bytes)
			if err != nil {
				return nil, fmt.Errorf("parse key %s: %w", path, err)
			}
			if k.Curve != elliptic.P256() {
				return nil, fmt.Errorf("key in %s is not ECDSA P-256", path)
			}
			return k, nil
		}
	}
}
