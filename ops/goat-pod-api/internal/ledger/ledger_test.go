package ledger

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/canon"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/generation"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/testpki"
)

const (
	pod = "0123456789abcdef0123456789abcdef"
	src = "fedcba9876543210fedcba9876543210"
)

var auth = generation.Authority{Identity: "us-dod:1234567890", CertCN: "lead.g1"}

func allow(rule string) Entry {
	return Entry{Action: ActionAllow, RuleID: rule, SourceSlot: src,
		Until: time.Now().Add(time.Hour).UTC().Format(time.RFC3339), Reason: "exercise <a&b>", Authority: auth}
}

func TestAppendReopenUpload(t *testing.T) {
	dir := t.TempDir()
	ca := testpki.NewCA(t, "ca")
	pk := ca.Leaf(t, pod)
	path := filepath.Join(dir, "ledger.jsonl")
	l, err := Open(path, pk.Key, pk.Cert, pod)
	if err != nil {
		t.Fatal(err)
	}
	for _, id := range []string{"lr_1", "lr_2", "lr_3"} {
		if _, err := l.Append(allow(id), Local{Principals: []string{src + ".g1"}}); err != nil {
			t.Fatal(err)
		}
	}
	l2, err := Open(path, pk.Key, pk.Cert, pod)
	if err != nil {
		t.Fatalf("reopen: %v", err)
	}
	if got := len(l2.Records()); got != 3 || l2.Records()[0].Local.Principals[0] != src+".g1" {
		t.Fatalf("reopened %d records", got)
	}

	body, n, err := l2.UploadBody(pk.CertPEM)
	if err != nil || n != 3 {
		t.Fatalf("upload body: %d %v", n, err)
	}
	// The wire entries carry exactly the agreed fields, and each verifies as the portal checks it:
	// "goat-ledger-v1\n" + canonical(entry without sig), ECDSA P-256.
	var up struct {
		PodCert string                       `json:"pod_cert"`
		Entries []map[string]json.RawMessage `json:"entries"`
	}
	if err := json.Unmarshal(body, &up); err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(up.PodCert, "BEGIN CERTIFICATE") {
		t.Fatal("pod_cert missing")
	}
	want := "action authority pod_slot reason rule_id seq sig source_slot ts until v"
	for _, e := range up.Entries {
		var keys []string
		for k := range e {
			keys = append(keys, k)
		}
		if got := strings.Join(sorted(keys), " "); got != want {
			t.Fatalf("wire fields %q, want %q", got, want)
		}
		raw, _ := json.Marshal(e)
		bodyNoSig, sig, err := canon.WithoutKey(raw, "sig")
		if err != nil {
			t.Fatal(err)
		}
		if err := canon.Verify(pk.Cert.PublicKey, "goat-ledger-v1\n", bodyNoSig, sig); err != nil {
			t.Fatalf("portal-side verification: %v", err)
		}
	}

	if err := l2.SetAck(2, time.Now(), []PortalEntry{{Seq: 2, LocalRuleID: "lr_2", State: "lapsed"}}); err != nil {
		t.Fatal(err)
	}
	if err := l2.SetAck(9, time.Now(), nil); err == nil {
		t.Fatal("ack beyond the ledger accepted")
	}
	body, n, _ = l2.UploadBody(pk.CertPEM)
	if n != 1 || !bytes.Contains(body, []byte(`"seq":3`)) {
		t.Fatalf("upload after ack: %d %s", n, body)
	}
	l3, _ := Open(path, pk.Key, pk.Cert, pod)
	if l3.Acked().LastSeq != 2 || l3.Acked().States[2].State != "lapsed" {
		t.Fatal("ack not persisted")
	}
}

func sorted(s []string) []string {
	for i := 1; i < len(s); i++ {
		for j := i; j > 0 && s[j] < s[j-1]; j-- {
			s[j], s[j-1] = s[j-1], s[j]
		}
	}
	return s
}

func TestTamperDetected(t *testing.T) {
	dir := t.TempDir()
	ca := testpki.NewCA(t, "ca")
	pk := ca.Leaf(t, pod)
	path := filepath.Join(dir, "ledger.jsonl")
	l, _ := Open(path, pk.Key, pk.Cert, pod)
	for _, r := range []string{"one", "two", "three"} {
		e := allow("lr_" + r)
		e.Reason = r
		if _, err := l.Append(e, Local{}); err != nil {
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
	// Truncation below what the portal holds is detected.
	if err := os.WriteFile(path, []byte(lines[0]+"\n"), 0o600); err != nil { // #nosec G703 -- test temp dir
		t.Fatal(err)
	}
	if err := os.WriteFile(path+".ack", []byte(`{"last_seq":3}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := Open(path, pk.Key, pk.Cert, pod); err == nil || !strings.Contains(err.Error(), "truncated") {
		t.Errorf("truncated ledger: %v", err)
	}
}
