package canon

import (
	"crypto"
	"crypto/ecdsa"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/pem"
	"errors"
	"fmt"
	"os"
)

// Sign signs prefix||body. ECDSA keys sign the SHA-256 digest (ASN.1 DER signature);
// Ed25519 keys sign the message itself. The result is standard base64.
func Sign(key crypto.Signer, prefix string, body []byte) (string, error) {
	msg := append([]byte(prefix), body...)
	var sig []byte
	var err error
	switch key.Public().(type) {
	case *ecdsa.PublicKey:
		digest := sha256.Sum256(msg)
		sig, err = key.Sign(rand.Reader, digest[:], crypto.SHA256)
	case ed25519.PublicKey:
		sig, err = key.Sign(rand.Reader, msg, crypto.Hash(0))
	default:
		return "", errors.New("signing key must be ECDSA or Ed25519")
	}
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
	msg := append([]byte(prefix), body...)
	switch k := pub.(type) {
	case *ecdsa.PublicKey:
		digest := sha256.Sum256(msg)
		if !ecdsa.VerifyASN1(k, digest[:], sig) {
			return errors.New("signature does not verify")
		}
	case ed25519.PublicKey:
		if !ed25519.Verify(k, msg, sig) {
			return errors.New("signature does not verify")
		}
	default:
		return errors.New("signer key must be ECDSA or Ed25519")
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

// LoadPrivateKey reads a PEM private key (PKCS#8, SEC 1 EC, or Ed25519 PKCS#8).
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
			s, ok := k.(crypto.Signer)
			if !ok {
				return nil, fmt.Errorf("key in %s cannot sign", path)
			}
			return s, nil
		case "EC PRIVATE KEY":
			k, err := x509.ParseECPrivateKey(block.Bytes)
			if err != nil {
				return nil, fmt.Errorf("parse key %s: %w", path, err)
			}
			return k, nil
		}
	}
}
