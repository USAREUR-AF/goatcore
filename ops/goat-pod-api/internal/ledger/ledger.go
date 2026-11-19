// Package ledger keeps the pod's append-only record of local rule changes
// (pod-carriage-and-provenance.md §7.4). One JSON object per line, each signed by the pod's key
// and chained to the previous line by its SHA-256, so a removed or edited line is detected.
package ledger

import (
	"bufio"
	"bytes"
	"crypto"
	"crypto/sha256"
	"crypto/x509"
	"encoding/hex"
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

// Schema names a ledger line.
const Schema = "goat.pod.ledger/v1"

// SignaturePrefix is prepended to the canonical line before signing.
const SignaturePrefix = "goat-pod-ledger-v1\n"

// Actions a ledger line records.
const (
	ActionAllow      = "allow"
	ActionExtend     = "extend"
	ActionRevoke     = "revoke"
	ActionReactivate = "reactivate"
)

// Entry is one local change.
type Entry struct {
	Schema     string               `json:"schema"`
	PodSlot    string               `json:"pod_slot"`
	Seq        int64                `json:"seq"`
	TS         time.Time            `json:"ts"`
	Action     string               `json:"action"`
	RuleID     string               `json:"rule_id"`
	Source     string               `json:"source"`
	Principals []string             `json:"principals,omitempty"`
	Write      []string             `json:"write,omitempty"`
	Until      time.Time            `json:"until"`
	Reason     string               `json:"reason"`
	Authority  generation.Authority `json:"authority"`
	Generation int64                `json:"generation"`
	PrevSHA256 string               `json:"prev_sha256"`
	Signer     string               `json:"signer"`
	Sig        string               `json:"sig"`
}

// Ack records how far the portal has acknowledged the ledger, and the central generation the
// pod held when it did. A later generation is the portal's decision on the acknowledged lines.
type Ack struct {
	LastSeq    int64     `json:"last_seq"`
	At         time.Time `json:"at"`
	Generation int64     `json:"generation"`
}

// Ledger is the open ledger file.
type Ledger struct {
	mu       sync.Mutex
	path     string
	ackPath  string
	key      crypto.Signer
	signerCN string
	podSlot  string
	entries  []Entry
	lines    [][]byte
	ack      Ack
}

// Open reads and checks an existing ledger (or starts an empty one). cert is the pod's
// certificate; every line must verify against its public key and chain to the line before.
func Open(path string, key crypto.Signer, cert *x509.Certificate, podSlot string) (*Ledger, error) {
	l := &Ledger{path: path, ackPath: path + ".ack", key: key, signerCN: cert.Subject.CommonName, podSlot: podSlot}
	f, err := os.Open(path) // #nosec G304 -- path comes from the operator's configuration
	switch {
	case errors.Is(err, os.ErrNotExist):
	case err != nil:
		return nil, fmt.Errorf("open ledger: %w", err)
	default:
		defer func() { _ = f.Close() }()
		sc := bufio.NewScanner(f)
		sc.Buffer(make([]byte, 64*1024), 1024*1024)
		prev := ""
		for n := 1; sc.Scan(); n++ {
			line := append([]byte(nil), sc.Bytes()...)
			if len(bytes.TrimSpace(line)) == 0 {
				return nil, fmt.Errorf("ledger line %d is empty", n)
			}
			e, err := check(line, cert, podSlot, int64(n), prev)
			if err != nil {
				return nil, fmt.Errorf("ledger line %d: %w", n, err)
			}
			l.entries = append(l.entries, e)
			l.lines = append(l.lines, line)
			prev = lineHash(line)
		}
		if err := sc.Err(); err != nil {
			return nil, fmt.Errorf("read ledger: %w", err)
		}
	}
	if raw, err := os.ReadFile(l.ackPath); err == nil { // #nosec G304 -- beside the ledger
		if err := json.Unmarshal(raw, &l.ack); err != nil {
			return nil, fmt.Errorf("ledger acknowledgment %s: %w", l.ackPath, err)
		}
	}
	return l, nil
}

func lineHash(line []byte) string {
	h := sha256.Sum256(line)
	return hex.EncodeToString(h[:])
}

// check verifies one stored line.
func check(line []byte, cert *x509.Certificate, podSlot string, seq int64, prev string) (Entry, error) {
	var e Entry
	if err := json.Unmarshal(line, &e); err != nil {
		return e, err
	}
	body, sig, err := canon.WithoutKey(line, "sig")
	if err != nil {
		return e, err
	}
	if err := canon.Verify(cert.PublicKey, SignaturePrefix, body, sig); err != nil {
		return e, err
	}
	switch {
	case e.Schema != Schema:
		return e, fmt.Errorf("schema %q", e.Schema)
	case e.PodSlot != podSlot:
		return e, fmt.Errorf("written by pod %q", e.PodSlot)
	case e.Seq != seq:
		return e, fmt.Errorf("sequence %d where %d was expected", e.Seq, seq)
	case e.PrevSHA256 != prev:
		return e, errors.New("does not chain to the line before it")
	}
	return e, nil
}

// Verify checks a ledger read from elsewhere (for example by the portal or a test) against the
// pod certificate. It returns the entries.
func Verify(data []byte, cert *x509.Certificate, podSlot string) ([]Entry, error) {
	var out []Entry
	prev := ""
	for n, line := range bytes.Split(bytes.TrimRight(data, "\n"), []byte("\n")) {
		if len(line) == 0 {
			continue
		}
		e, err := check(line, cert, podSlot, int64(n+1), prev)
		if err != nil {
			return nil, fmt.Errorf("line %d: %w", n+1, err)
		}
		out = append(out, e)
		prev = lineHash(line)
	}
	return out, nil
}

// Append signs e, writes it as the next line, and returns the stored entry. Seq, PrevSHA256,
// Schema, PodSlot, Signer and Sig are set here.
func (l *Ledger) Append(e Entry) (Entry, error) {
	l.mu.Lock()
	defer l.mu.Unlock()
	e.Schema = Schema
	e.PodSlot = l.podSlot
	e.Seq = int64(len(l.entries)) + 1
	e.Signer = l.signerCN
	e.Sig = ""
	if len(l.lines) > 0 {
		e.PrevSHA256 = lineHash(l.lines[len(l.lines)-1])
	} else {
		e.PrevSHA256 = ""
	}
	if e.TS.IsZero() {
		e.TS = time.Now().UTC()
	}
	body, err := canon.Marshal(e)
	if err != nil {
		return e, err
	}
	// canon.Marshal includes "sig":""; sign the document without it, as Verify does.
	body, _, err = canon.WithoutKey(body, "sig")
	if err != nil {
		return e, err
	}
	sig, err := canon.Sign(l.key, SignaturePrefix, body)
	if err != nil {
		return e, err
	}
	e.Sig = sig
	line, err := canon.Marshal(e)
	if err != nil {
		return e, err
	}
	if err := os.MkdirAll(filepath.Dir(l.path), 0o700); err != nil {
		return e, err
	}
	f, err := os.OpenFile(l.path, os.O_WRONLY|os.O_CREATE|os.O_APPEND, 0o600) // #nosec G304 -- configured path
	if err != nil {
		return e, fmt.Errorf("open ledger for append: %w", err)
	}
	if _, err := f.Write(append(line, '\n')); err != nil {
		_ = f.Close()
		return e, fmt.Errorf("append ledger: %w", err)
	}
	if err := f.Sync(); err != nil {
		_ = f.Close()
		return e, fmt.Errorf("sync ledger: %w", err)
	}
	if err := f.Close(); err != nil {
		return e, err
	}
	l.entries = append(l.entries, e)
	l.lines = append(l.lines, line)
	return e, nil
}

// Entries returns a copy of every entry.
func (l *Ledger) Entries() []Entry {
	l.mu.Lock()
	defer l.mu.Unlock()
	return append([]Entry(nil), l.entries...)
}

// Unsent returns the raw lines the portal has not acknowledged, newline-terminated.
func (l *Ledger) Unsent() ([]byte, int) {
	l.mu.Lock()
	defer l.mu.Unlock()
	var buf bytes.Buffer
	n := 0
	for i, line := range l.lines {
		if int64(i+1) > l.ack.LastSeq {
			buf.Write(line)
			buf.WriteByte('\n')
			n++
		}
	}
	return buf.Bytes(), n
}

// Acked returns the current acknowledgment.
func (l *Ledger) Acked() Ack {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.ack
}

// SetAck records that the portal holds every line up to lastSeq while the pod held generation
// heldGeneration. A lower value is ignored.
func (l *Ledger) SetAck(lastSeq int64, at time.Time, heldGeneration int64) error {
	l.mu.Lock()
	defer l.mu.Unlock()
	if lastSeq <= l.ack.LastSeq {
		return nil
	}
	if lastSeq > int64(len(l.entries)) {
		return fmt.Errorf("portal acknowledged line %d, the ledger has %d", lastSeq, len(l.entries))
	}
	a := Ack{LastSeq: lastSeq, At: at.UTC(), Generation: heldGeneration}
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
