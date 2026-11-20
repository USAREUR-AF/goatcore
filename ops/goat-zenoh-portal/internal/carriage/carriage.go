// Package carriage holds the signed documents of pod carriage (ADR 1251,
// docs/design/pod-carriage-and-provenance.md §7): the rule generation the
// portal renders for a pod (§7.1) and the local-authority ledger entries a pod
// reports back (§7.4). It is pure: no database, no HTTP. The registry stores
// rules; the API layer decides what a pod carries; this package canonicalizes,
// signs and verifies.
package carriage

import (
	"bytes"
	"crypto"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"errors"
	"fmt"
	"io"
	"os"
	"strings"
	"time"
)

// Schema is the generation document's schema name (§7.1).
const Schema = "goat.carriage.generation/v1"

// GenerationPrefix is prepended to the canonical generation before signing.
const GenerationPrefix = "goat-carriage-v1\n"

// LedgerPrefix is prepended to a canonical ledger entry before signing.
const LedgerPrefix = "goat-ledger-v1\n"

// TimeLayout is how every time in these documents is written: RFC 3339, UTC,
// whole seconds.
const TimeLayout = "2006-01-02T15:04:05Z"

// Authority is one person named in advance as a pod's local authority.
type Authority struct {
	Identity string `json:"identity"`
	CertCN   string `json:"cert_cn"`
}

// Source is one carried source in a generation.
type Source struct {
	Slot       string   `json:"slot"`
	RuleID     string   `json:"rule_id"`
	Kind       string   `json:"kind"`
	Until      *string  `json:"until"`
	Principals []string `json:"principals"`
	Write      []string `json:"write"`
}

// LocalDecision is the portal's answer to one line of the pod's ledger.
type LocalDecision struct {
	Seq         int64  `json:"seq"`
	LocalRuleID string `json:"local_rule_id"`
	State       string `json:"state"` // ratified | lapsed
}

// Generation is the goat.carriage.generation/v1 document.
type Generation struct {
	Schema         string      `json:"schema"`
	PodSlot        string      `json:"pod_slot"`
	Generation     int64       `json:"generation"`
	IssuedAt       string      `json:"issued_at"`
	LocalAuthority []Authority `json:"local_authority"`
	Read           []string    `json:"read"`
	Sources        []Source    `json:"sources"`
	// LocalDecisions lists every ledger line of this pod the portal has
	// decided, by seq. A ratified grant also appears in Sources as a central
	// named rule; the pod drops its local copy of each decided line.
	LocalDecisions []LocalDecision `json:"local_decisions"`
	Signer         string          `json:"signer"`
	Signature      string          `json:"signature,omitempty"`
}

// FormatTime writes t in TimeLayout.
func FormatTime(t time.Time) string { return t.UTC().Format(TimeLayout) }

// DefaultWrite is the write set for a carried source: its own namespace, its
// volatile namespace and its liveliness/advertisement keys (§7.1 example).
func DefaultWrite(slot string) []string {
	return []string{slot + "/**", "volatile/" + slot + "/**", slot + "/**/@adv/**"}
}

// Canonical returns the canonical JSON of v: object keys sorted, no
// insignificant whitespace, numbers as written, HTML characters not escaped,
// no trailing newline (§7.1). Every signer and verifier in goat uses this form.
func Canonical(v any) ([]byte, error) {
	raw, err := json.Marshal(v)
	if err != nil {
		return nil, fmt.Errorf("canonical: encode: %w", err)
	}
	generic, err := decodeGeneric(raw)
	if err != nil {
		return nil, err
	}
	return encodeGeneric(generic)
}

func decodeGeneric(raw []byte) (any, error) {
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	var v any
	if err := dec.Decode(&v); err != nil {
		return nil, fmt.Errorf("canonical: parse: %w", err)
	}
	if _, err := dec.Token(); !errors.Is(err, io.EOF) {
		return nil, errors.New("canonical: trailing data after the document")
	}
	return v, nil
}

func encodeGeneric(v any) ([]byte, error) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(v); err != nil {
		return nil, fmt.Errorf("canonical: encode: %w", err)
	}
	return bytes.TrimSuffix(buf.Bytes(), []byte("\n")), nil
}

// canonicalWithout returns the canonical form of the JSON object raw with key
// removed, and the string the key held.
func canonicalWithout(raw []byte, key string) ([]byte, string, error) {
	v, err := decodeGeneric(raw)
	if err != nil {
		return nil, "", err
	}
	obj, ok := v.(map[string]any)
	if !ok {
		return nil, "", errors.New("canonical: document is not a JSON object")
	}
	val, _ := obj[key].(string)
	delete(obj, key)
	body, err := encodeGeneric(obj)
	return body, val, err
}

// ContentHash is the digest of what a generation says, without the fields that
// change on every issue (generation, issued_at, signer, signature). The portal
// bumps a pod's generation exactly when this changes.
func ContentHash(g Generation) (string, error) {
	g.Generation, g.IssuedAt, g.Signer, g.Signature = 0, "", "", ""
	b, err := Canonical(g)
	if err != nil {
		return "", err
	}
	sum := sha256.Sum256(b)
	return hex.EncodeToString(sum[:]), nil
}

// Signer is the portal's carriage signing identity: an ECDSA P-256 key and the
// certificate whose CN the generation names as `signer`.
type Signer struct {
	Key  *ecdsa.PrivateKey
	Cert *x509.Certificate
}

// CN is the signer certificate's common name.
func (s *Signer) CN() string { return s.Cert.Subject.CommonName }

// LoadSigner reads the signer key and certificate from PEM files. The key must
// be ECDSA P-256 and must match the certificate.
func LoadSigner(keyPath, certPath string) (*Signer, error) {
	if strings.TrimSpace(keyPath) == "" || strings.TrimSpace(certPath) == "" {
		return nil, errors.New("carriage signer: both a key file and a certificate file are required")
	}
	keyPEM, err := os.ReadFile(keyPath) // #nosec G304 -- operator-configured path
	if err != nil {
		return nil, fmt.Errorf("carriage signer: read key: %w", err)
	}
	certPEM, err := os.ReadFile(certPath) // #nosec G304 -- operator-configured path
	if err != nil {
		return nil, fmt.Errorf("carriage signer: read certificate: %w", err)
	}
	return ParseSigner(keyPEM, certPEM)
}

// ParseSigner is LoadSigner over PEM bytes.
func ParseSigner(keyPEM, certPEM []byte) (*Signer, error) {
	key, err := parseECKey(keyPEM)
	if err != nil {
		return nil, fmt.Errorf("carriage signer: %w", err)
	}
	cert, err := ParseCertPEM(certPEM)
	if err != nil {
		return nil, fmt.Errorf("carriage signer: %w", err)
	}
	pub, ok := cert.PublicKey.(*ecdsa.PublicKey)
	if !ok || !pub.Equal(&key.PublicKey) {
		return nil, errors.New("carriage signer: the key does not match the certificate")
	}
	if cert.Subject.CommonName == "" {
		return nil, errors.New("carriage signer: the certificate has no common name")
	}
	return &Signer{Key: key, Cert: cert}, nil
}

func parseECKey(pemBytes []byte) (*ecdsa.PrivateKey, error) {
	for {
		var block *pem.Block
		block, pemBytes = pem.Decode(pemBytes)
		if block == nil {
			return nil, errors.New("no PEM private key")
		}
		var k any
		var err error
		switch block.Type {
		case "PRIVATE KEY":
			k, err = x509.ParsePKCS8PrivateKey(block.Bytes)
		case "EC PRIVATE KEY":
			k, err = x509.ParseECPrivateKey(block.Bytes)
		default:
			continue
		}
		if err != nil {
			return nil, fmt.Errorf("parse private key: %w", err)
		}
		ec, ok := k.(*ecdsa.PrivateKey)
		if !ok || ec.Curve != elliptic.P256() {
			return nil, errors.New("the signing key must be ECDSA P-256")
		}
		return ec, nil
	}
}

// ParseCertPEM returns the first certificate in pemBytes.
func ParseCertPEM(pemBytes []byte) (*x509.Certificate, error) {
	for {
		var block *pem.Block
		block, pemBytes = pem.Decode(pemBytes)
		if block == nil {
			return nil, errors.New("no PEM certificate")
		}
		if block.Type != "CERTIFICATE" {
			continue
		}
		c, err := x509.ParseCertificate(block.Bytes)
		if err != nil {
			return nil, fmt.Errorf("parse certificate: %w", err)
		}
		return c, nil
	}
}

func sign(key *ecdsa.PrivateKey, prefix string, body []byte) (string, error) {
	digest := sha256.Sum256(append([]byte(prefix), body...))
	sig, err := key.Sign(rand.Reader, digest[:], crypto.SHA256)
	if err != nil {
		return "", fmt.Errorf("sign: %w", err)
	}
	return base64.StdEncoding.EncodeToString(sig), nil
}

func verify(pub crypto.PublicKey, prefix string, body []byte, sigB64 string) error {
	ec, ok := pub.(*ecdsa.PublicKey)
	if !ok {
		return errors.New("the signer key is not ECDSA")
	}
	sig, err := base64.StdEncoding.DecodeString(sigB64)
	if err != nil {
		return errors.New("the signature is not base64")
	}
	digest := sha256.Sum256(append([]byte(prefix), body...))
	if !ecdsa.VerifyASN1(ec, digest[:], sig) {
		return errors.New("the signature does not verify")
	}
	return nil
}

// Sign fills g.Signer and g.Signature and returns the signed document bytes.
func (s *Signer) Sign(g Generation) ([]byte, error) {
	g.Signer = s.CN()
	g.Signature = ""
	body, err := Canonical(g)
	if err != nil {
		return nil, err
	}
	g.Signature, err = sign(s.Key, GenerationPrefix, body)
	if err != nil {
		return nil, err
	}
	return Canonical(g)
}

// VerifyGeneration checks a signed generation against the signer certificate
// and returns it decoded. It does not judge the contents.
func VerifyGeneration(raw []byte, signer *x509.Certificate) (*Generation, error) {
	body, sig, err := canonicalWithout(raw, "signature")
	if err != nil {
		return nil, err
	}
	if sig == "" {
		return nil, errors.New("generation carries no signature")
	}
	if err := verify(signer.PublicKey, GenerationPrefix, body, sig); err != nil {
		return nil, fmt.Errorf("generation: %w", err)
	}
	var g Generation
	if err := json.Unmarshal(raw, &g); err != nil {
		return nil, fmt.Errorf("generation: %w", err)
	}
	if g.Signer != signer.Subject.CommonName {
		return nil, fmt.Errorf("generation names signer %q, the certificate is %q", g.Signer, signer.Subject.CommonName)
	}
	return &g, nil
}

// Ledger actions a pod's local authority may take (§4, §7.4).
const (
	ActionAllow      = "allow"
	ActionExtend     = "extend"
	ActionRevoke     = "revoke"
	ActionReactivate = "reactivate"
)

// LedgerEntry is one line of a pod's local-authority ledger.
type LedgerEntry struct {
	V          int       `json:"v"`
	Seq        int64     `json:"seq"`
	TS         string    `json:"ts"`
	PodSlot    string    `json:"pod_slot"`
	Action     string    `json:"action"`
	RuleID     string    `json:"rule_id"`
	SourceSlot string    `json:"source_slot"`
	Until      string    `json:"until"`
	Reason     string    `json:"reason"`
	Authority  Authority `json:"authority"`
	Sig        string    `json:"sig,omitempty"`
}

// SignLedgerEntry fills e.Sig with the pod key's signature. Pods sign; the
// portal only verifies. Kept here so tests and fixtures sign the same way.
func SignLedgerEntry(key *ecdsa.PrivateKey, e LedgerEntry) (LedgerEntry, error) {
	e.Sig = ""
	body, err := Canonical(e)
	if err != nil {
		return e, err
	}
	e.Sig, err = sign(key, LedgerPrefix, body)
	return e, err
}

// VerifyLedgerEntry checks one raw ledger line against the pod's certificate
// and decodes it. It checks the shape a ratifier relies on; policy (who may,
// how long) is the caller's.
func VerifyLedgerEntry(raw []byte, pod *x509.Certificate) (*LedgerEntry, []byte, error) {
	body, sig, err := canonicalWithout(raw, "sig")
	if err != nil {
		return nil, nil, err
	}
	if sig == "" {
		return nil, nil, errors.New("ledger entry carries no signature")
	}
	if err := verify(pod.PublicKey, LedgerPrefix, body, sig); err != nil {
		return nil, nil, fmt.Errorf("ledger entry: %w", err)
	}
	var e LedgerEntry
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&e); err != nil {
		return nil, nil, fmt.Errorf("ledger entry: %w", err)
	}
	if e.V != 1 {
		return nil, nil, fmt.Errorf("ledger entry version %d is not 1", e.V)
	}
	if e.Seq < 1 {
		return nil, nil, errors.New("ledger entry seq must be 1 or more")
	}
	switch e.Action {
	case ActionAllow, ActionExtend, ActionRevoke, ActionReactivate:
	default:
		return nil, nil, fmt.Errorf("ledger entry action %q is unknown", e.Action)
	}
	if _, err := time.Parse(time.RFC3339, e.TS); err != nil {
		return nil, nil, fmt.Errorf("ledger entry ts: %w", err)
	}
	// A revoke ends carriage at once and may carry no end date; every other
	// action needs one.
	if e.Until != "" || e.Action != ActionRevoke {
		if _, err := time.Parse(time.RFC3339, e.Until); err != nil {
			return nil, nil, fmt.Errorf("ledger entry until: %w", err)
		}
	}
	if strings.TrimSpace(e.Reason) == "" {
		return nil, nil, errors.New("ledger entry has no reason")
	}
	if strings.TrimSpace(e.Authority.Identity) == "" {
		return nil, nil, errors.New("ledger entry names no authority")
	}
	canon, err := Canonical(e)
	if err != nil {
		return nil, nil, err
	}
	return &e, canon, nil
}

// VerifyChain checks that leaf chains to one of roots and is valid at now for
// client authentication.
func VerifyChain(leaf *x509.Certificate, roots *x509.CertPool, now time.Time) error {
	_, err := leaf.Verify(x509.VerifyOptions{
		Roots:       roots,
		CurrentTime: now,
		KeyUsages:   []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth},
	})
	if err != nil {
		return fmt.Errorf("certificate does not chain to the goatnet CA: %w", err)
	}
	return nil
}
