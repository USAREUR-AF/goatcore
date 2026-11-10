//go:build !nopiv

// yubikey_ca.go — production CA backend per ADR 0406.
//
// The CA private key lives on a YubiKey PIV slot 9c (Digital Signature)
// with touch-policy=always and pin-policy=once. Each Sign() requires PIN
// authentication and an LED-tap by the operator. The private key never
// leaves the secure element — bundle issuance pulls the CBOR payload to
// the YubiKey to be signed and pulls the signature back.
//
// Block 79: hard cutover from Ed25519 to ECDSA P-256.
// piv.AlgorithmEC256 has been in YubiKey PIV firmware for years
// (predates Ed25519 support); the change reverts to a more broadly
// supported algorithm choice while opening native trust-store import
// across macOS / iOS / Android / Windows / Linux. Block 22M's procurement
// notes about firmware 5.7+ no longer apply — any YubiKey 5 generation
// with PIV support works.
//
// Concurrency:
//   - YubiKey is a single PCSC handle; we serialize Sign() calls.
//   - Open()/Close() are paired; multiple goroutines must not share a
//     YubikeySigner without external sync.

package ca

import (
	"crypto"
	"crypto/ecdsa"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"crypto/x509/pkix"
	"errors"
	"fmt"
	"math/big"
	"strings"
	"sync"
	"time"

	"github.com/go-piv/piv-go/v2/piv"
)

// ErrYubikeyNotImplemented is the historical sentinel for the pre-Block-22K
// stub. Retained exported so callers that pinned to it during the
// procurement window keep compiling, but NewYubikeySigner now returns a
// real signer when hardware is present.
var ErrYubikeyNotImplemented = errors.New("yubikey CA backend not yet implemented; use AgeSigner in dev mode until hardware arrives")

// ErrNoYubikey is returned when no PCSC card matching the operator's
// selector is found at Open time. Distinct from ErrYubikeyNotImplemented
// so call sites can branch (the dev fallback is appropriate for the
// former; the latter means "the implementation is missing").
var ErrNoYubikey = errors.New("no YubiKey found via PCSC")

// ErrNoCertInSlot is returned when a slot has no provisioned certificate
// — operator must run YubikeyGenerate first.
var ErrNoCertInSlot = errors.New("no certificate in PIV slot; run ca-init --mode prod to provision")

// ErrSlotKeyNotECDSAP256 is returned when the provisioned key in the slot
// is not ECDSA P-256 (e.g. a legacy RSA / Ed25519 key from a previous
// purpose). Block 79 hard cutover requires P-256.
var ErrSlotKeyNotECDSAP256 = errors.New("PIV slot key is not ECDSA P-256; provision a new ECCP256 key per ADR 0406 + Block 79")

// YubikeySigner satisfies ca.Signer using a YubiKey-PIV-held ECDSA P-256 key.
type YubikeySigner struct {
	id        string
	yk        *piv.YubiKey
	priv      crypto.Signer // YubiKey-backed crypto.Signer; sign yields ECDSA ASN.1 DER bytes
	pub       *ecdsa.PublicKey
	closeOnce sync.Once
	mu        sync.Mutex // serializes Sign() against the YubiKey
}

// YubikeyOptions configures hardware selection + slot + auth.
type YubikeyOptions struct {
	// CardSelector is the PCSC reader-name substring. When empty, the
	// first connected YubiKey is used. Operators with multiple keys
	// inserted should pass a unique substring (e.g. "Yubico YubiKey OTP+FIDO+CCID 03").
	CardSelector string

	// Slot is the PIV slot to use. Default piv.SlotSignature (9c) per
	// ADR 0406. Operator-overridable for unusual deployments only.
	Slot piv.Slot

	// PIN is the PIV PIN. Pin-policy on slot 9c is "once" by default
	// in our provisioning flow; touch-policy is "always" so each Sign
	// still requires an LED tap independent of the PIN cache.
	PIN string

	// CAID is the identifier carried in the ca_id field of bundles
	// signed by this YubiKey. Should match the public-keys PEM filename.
	CAID string
}

// NewYubikeySigner opens a YubiKey, authenticates with the supplied PIN,
// resolves the public key from the slot's certificate, and returns a
// Signer. The caller MUST call Close() when done — the YubiKey handle
// is exclusive across PCSC clients on the host.
//
// Returns ErrNoYubikey when no card matches the selector,
// ErrNoCertInSlot when the slot is empty (operator should run
// ca-init --mode prod first), ErrSlotKeyNotECDSAP256 when the slot
// holds the wrong algorithm.
func NewYubikeySigner(opts YubikeyOptions) (*YubikeySigner, error) {
	if opts.CAID == "" {
		return nil, errors.New("YubikeyOptions.CAID is required")
	}
	if opts.PIN == "" {
		return nil, errors.New("YubikeyOptions.PIN is required (operator should be PIN-prompted by the caller)")
	}
	slot := opts.Slot
	if slot == (piv.Slot{}) {
		slot = piv.SlotSignature
	}

	card, err := selectCard(opts.CardSelector)
	if err != nil {
		return nil, err
	}

	yk, err := piv.Open(card)
	if err != nil {
		return nil, fmt.Errorf("piv open %q: %w", card, err)
	}
	closeOnErr := func() { _ = yk.Close() } //nolint:errcheck // cleanup cannot replace the operation error

	cert, err := yk.Certificate(slot)
	if err != nil {
		closeOnErr()
		// piv-go returns wrapped errno-shaped errors here; the
		// "card returned 6a82" / equivalent maps to "object not found."
		return nil, fmt.Errorf("%w: slot %x: %v", ErrNoCertInSlot, slot.Key, err)
	}

	pub, ok := cert.PublicKey.(*ecdsa.PublicKey)
	if !ok {
		closeOnErr()
		return nil, fmt.Errorf("%w: got %T", ErrSlotKeyNotECDSAP256, cert.PublicKey)
	}
	if pub.Curve.Params().Name != "P-256" {
		closeOnErr()
		return nil, fmt.Errorf("%w: got curve %s", ErrSlotKeyNotECDSAP256, pub.Curve.Params().Name)
	}

	priv, err := yk.PrivateKey(slot, cert.PublicKey, piv.KeyAuth{PIN: opts.PIN})
	if err != nil {
		closeOnErr()
		return nil, fmt.Errorf("piv private key (slot %x, with PIN): %w", slot.Key, err)
	}
	signer, ok := priv.(crypto.Signer)
	if !ok {
		closeOnErr()
		return nil, fmt.Errorf("piv-go returned non-Signer for slot %x (got %T)", slot.Key, priv)
	}

	return &YubikeySigner{
		id:   opts.CAID,
		yk:   yk,
		priv: signer,
		pub:  pub,
	}, nil
}

// Sign hashes msg with SHA-256 then signs the digest with the YubiKey-held
// ECDSA P-256 key. Each call requires a touch tap on the YubiKey
// (touch-policy=always). PIN authentication is cached by the YubiKey
// under pin-policy=once for the duration of the open session.
//
// Returns ASN.1 DER ECDSA-Sig-Value bytes (variable length, ~70-72 typical).
func (s *YubikeySigner) Sign(msg []byte) ([]byte, error) {
	digest := sha256.Sum256(msg)
	return s.SignDigest(digest[:])
}

// SignDigest signs an already-computed 32-byte SHA-256 digest with the
// YubiKey-held ECDSA P-256 key. Used by the crypto.Signer adapter
// consumed by crypto/x509.CreateCertificate.
func (s *YubikeySigner) SignDigest(digest []byte) ([]byte, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.priv == nil {
		return nil, errors.New("yubikey signer closed")
	}
	if len(digest) != sha256.Size {
		return nil, fmt.Errorf("digest wrong size: got %d want %d", len(digest), sha256.Size)
	}
	sig, err := s.priv.Sign(rand.Reader, digest, crypto.SHA256)
	if err != nil {
		return nil, fmt.Errorf("yubikey sign (touch-policy=always; did you tap the LED within the timeout?): %w", err)
	}
	return sig, nil
}

// PublicKey returns the CA public key extracted from the slot certificate.
func (s *YubikeySigner) PublicKey() *ecdsa.PublicKey { return s.pub }

// ID returns the CA identifier set at construction time.
func (s *YubikeySigner) ID() string { return s.id }

// Close releases the YubiKey handle. Safe to call multiple times.
func (s *YubikeySigner) Close() error {
	var err error
	s.closeOnce.Do(func() {
		s.mu.Lock()
		defer s.mu.Unlock()
		if s.yk != nil {
			err = s.yk.Close()
			s.yk = nil
		}
		s.priv = nil
	})
	return err
}

// pivCards is overridable in tests. Production reads from piv.Cards()
// (the PCSC daemon); tests inject a fixed slice.
var pivCards = piv.Cards

// selectCard finds the first connected PCSC reader matching selector
// (substring match, case-insensitive). Empty selector returns the first
// card.
func selectCard(selector string) (string, error) {
	cards, err := pivCards()
	if err != nil {
		return "", fmt.Errorf("piv list cards: %w", err)
	}
	if len(cards) == 0 {
		return "", ErrNoYubikey
	}
	if selector == "" {
		return cards[0], nil
	}
	wanted := strings.ToLower(selector)
	for _, c := range cards {
		if strings.Contains(strings.ToLower(c), wanted) {
			return c, nil
		}
	}
	return "", fmt.Errorf("%w: selector %q matched none of %d cards", ErrNoYubikey, selector, len(cards))
}

// YubikeyGenerateOptions configures the one-time provisioning ceremony.
type YubikeyGenerateOptions struct {
	CardSelector  string
	Slot          piv.Slot
	ManagementKey []byte // PIV management key — defaults to piv.DefaultManagementKey when nil
	PIN           string
	CAID          string

	// CertSubject controls the self-signed cert's CN. Defaults to
	// YubikeyCertSubjectPrefix + CAID. The cert is needed because PIV slots
	// store certificates, not raw pubkeys; we read the pubkey back
	// from the cert at NewYubikeySigner time.
	CertSubject string

	// CertValidityYears bounds the self-signed cert that wraps the
	// pubkey on the YubiKey. Default 10y. The CA's effective lifetime
	// is also bounded by bundle TTLs (90d) + procurement-resilience
	// rotation per ADR 0406.
	CertValidityYears int
}

// YubikeyGenerate provisions a fresh ECDSA P-256 keypair on the YubiKey,
// installs a self-signed cert wrapping the pubkey in the slot, and
// returns the public key for distribution. The private key never
// leaves the secure element — even this function only sees the pubkey.
//
// One-time per-YubiKey ceremony. Idempotent in the strict sense
// (re-running overwrites the slot), but the operator should treat each
// run as a fresh CA root. Existing bundles signed by the prior root
// remain verifiable only against the prior pubkey PEM.
//
// Returns ErrNoYubikey when no card matches.
func YubikeyGenerate(opts YubikeyGenerateOptions) (*ecdsa.PublicKey, *x509.Certificate, error) {
	if opts.CAID == "" {
		return nil, nil, errors.New("CAID is required")
	}
	if opts.PIN == "" {
		return nil, nil, errors.New("PIN is required")
	}
	slot := opts.Slot
	if slot == (piv.Slot{}) {
		slot = piv.SlotSignature
	}
	mgmtKey := opts.ManagementKey
	if len(mgmtKey) == 0 {
		mgmtKey = piv.DefaultManagementKey
	}
	subject := opts.CertSubject
	if subject == "" {
		subject = YubikeyCertSubjectPrefix + opts.CAID
	}
	years := opts.CertValidityYears
	if years <= 0 {
		years = 10
	}

	card, err := selectCard(opts.CardSelector)
	if err != nil {
		return nil, nil, err
	}
	yk, err := piv.Open(card)
	if err != nil {
		return nil, nil, fmt.Errorf("piv open %q: %w", card, err)
	}
	defer yk.Close() //nolint:errcheck // operation result is complete before device cleanup

	keyTmpl := piv.Key{
		Algorithm:   piv.AlgorithmEC256,
		PINPolicy:   piv.PINPolicyOnce,
		TouchPolicy: piv.TouchPolicyAlways,
	}
	pubAny, err := yk.GenerateKey(mgmtKey, slot, keyTmpl)
	if err != nil {
		return nil, nil, fmt.Errorf("piv generate key (ECDSA P-256): %w", err)
	}
	pub, ok := pubAny.(*ecdsa.PublicKey)
	if !ok {
		return nil, nil, fmt.Errorf("generated key is not ECDSA: got %T", pubAny)
	}
	if pub.Curve.Params().Name != "P-256" {
		return nil, nil, fmt.Errorf("generated key curve is %s, want P-256", pub.Curve.Params().Name)
	}

	// Build a self-signed cert wrapping the pubkey so NewYubikeySigner
	// can read it back. Sign with the just-generated YubiKey-held
	// private key (so we never see the privkey ourselves).
	priv, err := yk.PrivateKey(slot, pub, piv.KeyAuth{PIN: opts.PIN})
	if err != nil {
		return nil, nil, fmt.Errorf("piv private key for self-sign: %w", err)
	}
	signer, ok := priv.(crypto.Signer)
	if !ok {
		return nil, nil, fmt.Errorf("piv private key is not a crypto.Signer (got %T)", priv)
	}

	now := time.Now().UTC()
	tmpl := &x509.Certificate{
		SerialNumber:          mustSerialNumber(),
		Subject:               pkixName(subject),
		Issuer:                pkixName(subject),
		NotBefore:             now,
		NotAfter:              now.AddDate(years, 0, 0),
		KeyUsage:              x509.KeyUsageDigitalSignature | x509.KeyUsageCertSign,
		BasicConstraintsValid: true,
		IsCA:                  true,
	}
	der, err := x509.CreateCertificate(rand.Reader, tmpl, tmpl, pub, signer)
	if err != nil {
		return nil, nil, fmt.Errorf("self-sign root cert: %w", err)
	}
	cert, err := x509.ParseCertificate(der)
	if err != nil {
		return nil, nil, fmt.Errorf("parse self-signed cert: %w", err)
	}

	if err := yk.SetCertificate(mgmtKey, slot, cert); err != nil {
		return nil, nil, fmt.Errorf("piv set cert: %w", err)
	}
	return pub, cert, nil
}

// pkixName builds a one-attribute PKIX Name with the given CommonName.
// Helper for the YubiKey self-signed cert that wraps the pubkey in
// the slot.
func pkixName(cn string) pkix.Name {
	return pkix.Name{CommonName: cn}
}

// mustSerialNumber returns a fresh 128-bit serial. crypto/rand failure
// would mean the host is unrecoverably broken; panic is appropriate
// (this is operator-fired CA-provisioning code, not request-path code).
func mustSerialNumber() *big.Int {
	max := new(big.Int).Lsh(big.NewInt(1), 128)
	n, err := rand.Int(rand.Reader, max)
	if err != nil {
		panic(fmt.Sprintf("rand.Int: %v", err))
	}
	return n
}
