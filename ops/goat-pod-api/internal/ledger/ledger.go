// Package ledger keeps the pod's append-only record of local rule changes
// (pod-carriage-and-provenance.md §7.4).
//
// Each line of the file is {"entry": <wire entry>, "local": <pod-only detail>}. The wire entry is
// exactly what the portal receives (POST /api/carriage/pods/{pod_slot}/ledger) and is signed by
// the pod's key; "local" holds what only the pod needs to render its ACL (the principals and
// write set of a local allow, the end of a local revoke) and is not uploaded.
package ledger

import (
	"bufio"
	"bytes"
	"crypto"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/x509"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/canon"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/generation"
)

// Version is the wire entry version ("v").
const Version = 1

// SignaturePrefix is prepended to the canonical entry before signing.
const SignaturePrefix = "goat-ledger-v1\n"

// Actions a ledger entry records.
const (
	ActionAllow      = "allow"
	ActionExtend     = "extend"
	ActionRevoke     = "revoke"
	ActionReactivate = "reactivate"
)

// Entry is one local change as the portal receives it. Nothing else may be added: the portal
// refuses unknown fields.
type Entry struct {
	V          int                  `json:"v"`
	Seq        int64                `json:"seq"`
	TS         string               `json:"ts"`
	PodSlot    string               `json:"pod_slot"`
	Action     string               `json:"action"`
	RuleID     string               `json:"rule_id"`
	SourceSlot string               `json:"source_slot"`
	Until      string               `json:"until"` // RFC 3339 UTC; "" for revoke
	Reason     string               `json:"reason"`
	Authority  generation.Authority `json:"authority"`
	Sig        string               `json:"sig"`
}

// Local is the pod-only detail of an entry.
type Local struct {
	Principals []string `json:"principals,omitempty"`
	Write      []string `json:"write,omitempty"`
	// Generation is the central generation the pod held when the change was made.
	Generation int64 `json:"generation"`
}

// Record is one stored line.
type Record struct {
	Entry Entry `json:"entry"`
	Local Local `json:"local"`
}

// UntilTime parses Until (zero when empty).
func (e Entry) UntilTime() time.Time {
	t, err := time.Parse(time.RFC3339, e.Until)
	if err != nil {
		return time.Time{}
	}
	return t
}

// PortalEntry is one row of the portal's answer to an upload.
type PortalEntry struct {
	Seq         int64  `json:"seq"`
	LocalRuleID string `json:"local_rule_id"`
	State       string `json:"state"`
	Duplicate   bool   `json:"duplicate"`
	Note        string `json:"note"`
}

// Ack records how far the portal holds the ledger and what it said about each entry.
type Ack struct {
	LastSeq int64                 `json:"last_seq"`
	At      time.Time             `json:"at"`
	States  map[int64]PortalEntry `json:"states,omitempty"`
}

// Ledger is the open ledger file.
type Ledger struct {
	mu      sync.Mutex
	path    string
	ackPath string
	key     crypto.Signer
	cert    *x509.Certificate
	podSlot string
	records []Record
	ack     Ack
}

// Open reads and checks an existing ledger (or starts an empty one). Every entry must verify
// against the pod certificate and the sequence must run 1, 2, 3, ... without a gap.
func Open(path string, key crypto.Signer, cert *x509.Certificate, podSlot string) (*Ledger, error) {
	if k, ok := key.Public().(*ecdsa.PublicKey); !ok || k.Curve != elliptic.P256() {
		return nil, errors.New("the pod key must be ECDSA P-256 to sign ledger entries")
	}
	l := &Ledger{path: path, ackPath: path + ".ack", key: key, cert: cert, podSlot: podSlot}
	f, err := os.Open(path) // #nosec G304 -- path comes from the operator's configuration
	switch {
	case errors.Is(err, os.ErrNotExist):
	case err != nil:
		return nil, fmt.Errorf("open ledger: %w", err)
	default:
		defer func() { _ = f.Close() }()
		sc := bufio.NewScanner(f)
		sc.Buffer(make([]byte, 64*1024), 1024*1024)
		for n := 1; sc.Scan(); n++ {
			var r Record
			if err := json.Unmarshal(sc.Bytes(), &r); err != nil {
				return nil, fmt.Errorf("ledger line %d: %w", n, err)
			}
			if err := Check(r.Entry, cert, podSlot, int64(n)); err != nil {
				return nil, fmt.Errorf("ledger line %d: %w", n, err)
			}
			l.records = append(l.records, r)
		}
		if err := sc.Err(); err != nil {
			return nil, fmt.Errorf("read ledger: %w", err)
		}
	}
	if raw, err := os.ReadFile(l.ackPath); err == nil { // #nosec G304 -- beside the ledger
		if err := json.Unmarshal(raw, &l.ack); err != nil {
			return nil, fmt.Errorf("ledger acknowledgment %s: %w", l.ackPath, err)
		}
		if l.ack.LastSeq > int64(len(l.records)) {
			return nil, fmt.Errorf("the portal holds %d ledger entries but the file has %d: the ledger was truncated",
				l.ack.LastSeq, len(l.records))
		}
	}
	return l, nil
}

// signedBody is the canonical entry without "sig".
func signedBody(e Entry) ([]byte, error) {
	raw, err := json.Marshal(e)
	if err != nil {
		return nil, err
	}
	body, _, err := canon.WithoutKey(raw, "sig")
	return body, err
}

// Check verifies one entry's signature and fields. seq 0 skips the sequence check.
func Check(e Entry, cert *x509.Certificate, podSlot string, seq int64) error {
	body, err := signedBody(e)
	if err != nil {
		return err
	}
	if err := canon.Verify(cert.PublicKey, SignaturePrefix, body, e.Sig); err != nil {
		return err
	}
	switch {
	case e.V != Version:
		return fmt.Errorf("version %d", e.V)
	case e.PodSlot != podSlot:
		return fmt.Errorf("written by pod %q", e.PodSlot)
	case seq != 0 && e.Seq != seq:
		return fmt.Errorf("sequence %d where %d was expected", e.Seq, seq)
	}
	return nil
}

// Append signs e and stores it with its local detail. Seq, V, PodSlot and Sig are set here.
func (l *Ledger) Append(e Entry, local Local) (Record, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	e.V = Version
	e.PodSlot = l.podSlot
	e.Seq = int64(len(l.records)) + 1
	e.Sig = ""
	if e.TS == "" {
		e.TS = time.Now().UTC().Format(time.RFC3339)
	}
	body, err := signedBody(e)
	if err != nil {
		return Record{}, err
	}
	if e.Sig, err = canon.Sign(l.key, SignaturePrefix, body); err != nil {
		return Record{}, err
	}
	r := Record{Entry: e, Local: local}
	line, err := json.Marshal(r)
	if err != nil {
		return r, err
	}
	if err := os.MkdirAll(filepath.Dir(l.path), 0o700); err != nil {
		return r, err
	}
	f, err := os.OpenFile(l.path, os.O_WRONLY|os.O_CREATE|os.O_APPEND, 0o600) // #nosec G304 -- configured path
	if err != nil {
		return r, fmt.Errorf("open ledger for append: %w", err)
	}
	if _, err := f.Write(append(line, '\n')); err != nil {
		_ = f.Close()
		return r, fmt.Errorf("append ledger: %w", err)
	}
	if err := f.Sync(); err != nil {
		_ = f.Close()
		return r, fmt.Errorf("sync ledger: %w", err)
	}
	if err := f.Close(); err != nil {
		return r, err
	}
	l.records = append(l.records, r)
	return r, nil
}

// Records returns a copy of every record.
func (l *Ledger) Records() []Record {
	l.mu.Lock()
	defer l.mu.Unlock()
	return append([]Record(nil), l.records...)
}

// UploadBody returns the portal upload body for the entries after the acknowledged one:
// {"pod_cert": <PEM>, "entries": [...]}, and how many entries it holds.
func (l *Ledger) UploadBody(certPEM []byte) ([]byte, int, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	entries := []Entry{}
	for _, r := range l.records {
		if r.Entry.Seq > l.ack.LastSeq {
			entries = append(entries, r.Entry)
		}
	}
	if len(entries) == 0 {
		return nil, 0, nil
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	err := enc.Encode(struct {
		PodCert string  `json:"pod_cert"`
		Entries []Entry `json:"entries"`
	}{string(certPEM), entries})
	return buf.Bytes(), len(entries), err
}

// Unsent counts the entries the portal has not acknowledged.
func (l *Ledger) Unsent() int {
	l.mu.Lock()
	defer l.mu.Unlock()
	return len(l.records) - int(l.ack.LastSeq)
}

// Acked returns the current acknowledgment.
func (l *Ledger) Acked() Ack {
	l.mu.Lock()
	defer l.mu.Unlock()
	a := l.ack
	a.States = map[int64]PortalEntry{}
	for k, v := range l.ack.States {
		a.States[k] = v
	}
	return a
}

// SetAck records that the portal holds every entry up to lastSeq and what it said about the
// entries it answered for.
func (l *Ledger) SetAck(lastSeq int64, at time.Time, rows []PortalEntry) error {
	l.mu.Lock()
	defer l.mu.Unlock()
	if lastSeq > int64(len(l.records)) {
		return fmt.Errorf("portal acknowledged entry %d, the ledger has %d", lastSeq, len(l.records))
	}
	a := Ack{LastSeq: l.ack.LastSeq, At: at.UTC(), States: map[int64]PortalEntry{}}
	for k, v := range l.ack.States {
		a.States[k] = v
	}
	if lastSeq > a.LastSeq {
		a.LastSeq = lastSeq
	}
	for _, r := range rows {
		a.States[r.Seq] = r
	}
	raw, err := json.Marshal(a)
	if err != nil {
		return err
	}
	tmp := l.ackPath + ".tmp"
	if err := os.WriteFile(tmp, raw, 0o600); err != nil {
		return err
	}
	if err := os.Rename(tmp, l.ackPath); err != nil {
		return err
	}
	l.ack = a
	return nil
}
