// Package generation verifies a signed carriage rule generation (goat.carriage.generation/v1,
// pod-carriage-and-provenance.md §7.1).
package generation

import (
	"bytes"
	"crypto"
	"crypto/x509"
	"encoding/json"
	"errors"
	"fmt"
	"regexp"
	"strings"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/canon"
)

// Schema is the only generation schema this build accepts.
const Schema = "goat.carriage.generation/v1"

// SignaturePrefix is prepended to the canonical document before signing.
const SignaturePrefix = "goat-carriage-v1\n"

// Authority is a person who may change this pod's rules while it is cut off.
type Authority struct {
	Identity string `json:"identity"`
	CertCN   string `json:"cert_cn"`
}

// Source is one carried source.
type Source struct {
	Slot       string     `json:"slot"`
	RuleID     string     `json:"rule_id"`
	Kind       string     `json:"kind"`
	Until      *time.Time `json:"until"`
	Principals []string   `json:"principals"`
	Write      []string   `json:"write"`
}

// Decision is the portal's decision on one local ledger entry.
type Decision struct {
	Seq         int64  `json:"seq"`
	LocalRuleID string `json:"local_rule_id"`
	State       string `json:"state"` // ratified | lapsed
}

// Generation is a verified rule generation.
type Generation struct {
	Schema         string      `json:"schema"`
	PodSlot        string      `json:"pod_slot"`
	Generation     int64       `json:"generation"`
	IssuedAt       time.Time   `json:"issued_at"`
	LocalAuthority []Authority `json:"local_authority"`
	Read           []string    `json:"read"`
	Sources        []Source    `json:"sources"`
	LocalDecisions []Decision  `json:"local_decisions"`
	Signer         string      `json:"signer"`
	Signature      string      `json:"signature"`
}

var (
	slotRe = regexp.MustCompile(`^([0-9a-f]{32}|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$`)
	// A certificate common name rendered into a router ACL. Quotes, backslashes and control
	// characters never pass, so a CN cannot break out of the rendered string.
	cnRe = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._:@-]{0,127}$`)
	// A key expression chunk set: no quotes, backslashes, whitespace or control characters.
	keyRe = regexp.MustCompile(`^[A-Za-z0-9_.~*$@/:+=-]{1,512}$`)
)

// ValidSlot reports whether s is a slot id (32 lowercase hex, or the UUID form).
func ValidSlot(s string) bool { return slotRe.MatchString(s) }

// ValidCN reports whether s is a certificate common name the renderer accepts.
func ValidCN(s string) bool { return cnRe.MatchString(s) }

// ValidKeyExpr reports whether s is a key expression the renderer accepts.
func ValidKeyExpr(s string) bool { return keyRe.MatchString(s) && !strings.Contains(s, "//") }

// WithinSlot reports whether a write expression stays inside the source's own namespace:
// <slot>, <slot>/..., volatile/<slot>, volatile/<slot>/....
func WithinSlot(expr, slot string) bool {
	for _, root := range []string{slot, "volatile/" + slot} {
		if expr == root || strings.HasPrefix(expr, root+"/") {
			return true
		}
	}
	return false
}

// DefaultWrite is the write set for a source whose rule names none.
func DefaultWrite(slot string) []string {
	return []string{slot + "/**", "volatile/" + slot + "/**", slot + "/**/@adv/**"}
}

// Verify checks the signature on raw against signer, then the document's contents. podSlot is
// this pod's slot; a generation for another pod is refused.
func Verify(raw []byte, signer *x509.Certificate, podSlot string) (*Generation, error) {
	body, sig, err := canon.WithoutKey(raw, "signature")
	if err != nil {
		return nil, err
	}
	if sig == "" {
		return nil, errors.New("generation carries no signature")
	}
	if err := canon.Verify(signer.PublicKey, SignaturePrefix, body, sig); err != nil {
		return nil, fmt.Errorf("generation signature: %w", err)
	}
	var g Generation
	dec := json.NewDecoder(bytes.NewReader(raw))
	if err := dec.Decode(&g); err != nil {
		return nil, fmt.Errorf("generation contents: %w", err)
	}
	if g.Signer != signer.Subject.CommonName {
		return nil, fmt.Errorf("generation names signer %q, the configured signer certificate is %q",
			g.Signer, signer.Subject.CommonName)
	}
	if err := g.Validate(podSlot); err != nil {
		return nil, err
	}
	return &g, nil
}

// Validate checks the contents of a generation. It refuses anything the renderer could not
// carry safely, rather than dropping single entries.
func (g *Generation) Validate(podSlot string) error {
	if g.Schema != Schema {
		return fmt.Errorf("generation schema %q is not %q", g.Schema, Schema)
	}
	if g.PodSlot != podSlot {
		return fmt.Errorf("generation is for pod %q, this pod is %q", g.PodSlot, podSlot)
	}
	if g.Generation < 1 {
		return errors.New("generation number must be 1 or more")
	}
	for _, a := range g.LocalAuthority {
		if a.Identity == "" || !ValidCN(a.CertCN) {
			return fmt.Errorf("local authority entry %q/%q is malformed", a.Identity, a.CertCN)
		}
	}
	for _, r := range g.Read {
		if r == "**" {
			return errors.New("generation read set contains a bare ** (ADR 1123 forbids it)")
		}
		if !ValidKeyExpr(r) {
			return fmt.Errorf("read expression %q is malformed", r)
		}
	}
	for _, d := range g.LocalDecisions {
		if d.Seq < 1 || (d.State != "ratified" && d.State != "lapsed") {
			return fmt.Errorf("local decision %d/%q is malformed", d.Seq, d.State)
		}
	}
	seen := map[string]bool{}
	for _, s := range g.Sources {
		if err := ValidateSource(s); err != nil {
			return err
		}
		if seen[s.Slot] {
			return fmt.Errorf("source %s appears twice", s.Slot)
		}
		seen[s.Slot] = true
	}
	return nil
}

// ValidateSource checks one source entry.
func ValidateSource(s Source) error {
	if !ValidSlot(s.Slot) {
		return fmt.Errorf("source slot %q is malformed", s.Slot)
	}
	switch s.Kind {
	case "org":
	case "named", "temporary", "local":
		if s.Until == nil {
			return fmt.Errorf("source %s: a %s rule needs an end date", s.Slot, s.Kind)
		}
	default:
		return fmt.Errorf("source %s: unknown rule kind %q", s.Slot, s.Kind)
	}
	if len(s.Principals) == 0 {
		return fmt.Errorf("source %s names no principals", s.Slot)
	}
	for _, p := range s.Principals {
		if !ValidCN(p) {
			return fmt.Errorf("source %s: principal %q is malformed", s.Slot, p)
		}
	}
	if len(s.Write) == 0 {
		return fmt.Errorf("source %s names no write expressions", s.Slot)
	}
	for _, w := range s.Write {
		if !ValidKeyExpr(w) || !WithinSlot(w, s.Slot) {
			return fmt.Errorf("source %s: write expression %q is outside its own slot", s.Slot, w)
		}
	}
	return nil
}

// Sign signs an unsigned generation document (any JSON object) and returns the signed bytes.
// Used by tests and fixtures; the portal signs in production.
func Sign(doc map[string]any, key crypto.Signer) ([]byte, error) {
	delete(doc, "signature")
	body, err := canon.Marshal(doc)
	if err != nil {
		return nil, err
	}
	sig, err := canon.Sign(key, SignaturePrefix, body)
	if err != nil {
		return nil, err
	}
	v, err := canon.Decode(body)
	if err != nil {
		return nil, err
	}
	obj := v.(map[string]any)
	obj["signature"] = sig
	return canon.Encode(obj)
}
