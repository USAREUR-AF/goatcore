package ledger

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/generation"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/testpki"
)

const pod = "0123456789abcdef0123456789abcdef"

func TestAppendReopenVerify(t *testing.T) {
	dir := t.TempDir()
	ca := testpki.NewCA(t, "ca")
	pk := ca.Leaf(t, pod)
	path := filepath.Join(dir, "ledger.jsonl")
	l, err := Open(path, pk.Key, pk.Cert, pod)
	if err != nil {
		t.Fatal(err)
	}
	auth := generation.Authority{Identity: "us-dod:1234567890", CertCN: "lead.g1"}
	for i := 0; i < 3; i++ {
		if _, err := l.Append(Entry{Action: ActionAllow, RuleID: "lr_1", Source: "s", Until: time.Now().Add(time.Hour),
			Reason: "exercise <a&b>", Authority: auth}); err != nil {
			t.Fatal(err)
		}
	}
	l2, err := Open(path, pk.Key, pk.Cert, pod)
	if err != nil {
		t.Fatalf("reopen: %v", err)
	}
	if got := len(l2.Entries()); got != 3 {
		t.Fatalf("reopened %d entries", got)
	}
	raw, _ := os.ReadFile(path)
	if _, err := Verify(raw, pk.Cert, pod); err != nil {
		t.Fatal(err)
	}

	// Unsent / ack.
	if _, n := l2.Unsent(); n != 3 {
		t.Fatalf("unsent %d", n)
	}
	if err := l2.SetAck(2, time.Now(), 1); err != nil {
		t.Fatal(err)
	}
	if err := l2.SetAck(9, time.Now(), 1); err == nil {
		t.Fatal("ack beyond the ledger accepted")
	}
	body, n := l2.Unsent()
	if n != 1 || !bytes.Contains(body, []byte(`"seq":3`)) {
		t.Fatalf("unsent after ack: %d %s", n, body)
	}
	l3, _ := Open(path, pk.Key, pk.Cert, pod)
	if l3.Acked().LastSeq != 2 {
		t.Fatal("ack not persisted")
	}
}

func TestTamperDetected(t *testing.T) {
	dir := t.TempDir()
	ca := testpki.NewCA(t, "ca")
	pk := ca.Leaf(t, pod)
	path := filepath.Join(dir, "ledger.jsonl")
	l, _ := Open(path, pk.Key, pk.Cert, pod)
	for _, r := range []string{"one", "two", "three"} {
		if _, err := l.Append(Entry{Action: ActionAllow, RuleID: "lr_" + r, Source: "s", Until: time.Now(), Reason: r}); err != nil {
			t.Fatal(err)
		}
	}
	raw, _ := os.ReadFile(path)
	lines := strings.Split(strings.TrimSpace(string(raw)), "\n")

	edited := strings.Replace(string(raw), `"reason":"two"`, `"reason":"TWO"`, 1)
	dropped := lines[0] + "\n" + lines[2] + "\n"
	for name, data := range map[string]string{"edited": edited, "dropped": dropped} {
		if err := os.WriteFile(path, []byte(data), 0o600); err != nil {
			t.Fatal(err)
		}
		if _, err := Open(path, pk.Key, pk.Cert, pod); err == nil {
			t.Errorf("%s ledger opened", name)
		}
	}
	other := ca.Leaf(t, pod)
	if _, err := Verify(raw, other.Cert, pod); err == nil {
		t.Error("ledger verified against another key")
	}
}
