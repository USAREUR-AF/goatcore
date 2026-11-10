// audit_test.go — unit tests for the audit emitter + plan-summary
// helpers.

package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestFileAuditEmitter_AppendsJSONL(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "audit.jsonl")
	em := NewFileAuditEmitter(path)
	rec1 := AuditRecord{Timestamp: time.Now(), Operator: "test", Outcome: "ok"}
	rec2 := AuditRecord{Timestamp: time.Now(), Operator: "test", Outcome: "partial"}
	if err := em.Emit(rec1); err != nil {
		t.Fatalf("emit1: %v", err)
	}
	if err := em.Emit(rec2); err != nil {
		t.Fatalf("emit2: %v", err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read: %v", err)
	}
	lines := strings.Split(strings.TrimRight(string(data), "\n"), "\n")
	if len(lines) != 2 {
		t.Fatalf("lines = %d, want 2", len(lines))
	}
	for i, line := range lines {
		var rec AuditRecord
		if err := json.Unmarshal([]byte(line), &rec); err != nil {
			t.Errorf("line %d not JSON: %v", i, err)
		}
		if rec.Operator != "test" {
			t.Errorf("line %d operator = %q", i, rec.Operator)
		}
	}
}

func TestFileAuditEmitter_CreatesDir(t *testing.T) {
	dir := t.TempDir()
	nested := filepath.Join(dir, "a", "b", "c", "audit.jsonl")
	em := NewFileAuditEmitter(nested)
	if err := em.Emit(AuditRecord{Outcome: "ok"}); err != nil {
		t.Fatalf("emit: %v", err)
	}
	if _, err := os.Stat(nested); err != nil {
		t.Errorf("expected file at %s; got %v", nested, err)
	}
}

func TestNewFileAuditEmitter_DefaultPath(t *testing.T) {
	em := NewFileAuditEmitter("")
	if em.Path != DefaultAuditPath {
		t.Errorf("Path = %q, want %q", em.Path, DefaultAuditPath)
	}
}

func TestNoopAuditEmitter(t *testing.T) {
	em := NoopAuditEmitter{}
	if err := em.Emit(AuditRecord{}); err != nil {
		t.Errorf("Noop should never error; got %v", err)
	}
}

func TestRecordingAuditEmitter(t *testing.T) {
	em := &RecordingAuditEmitter{}
	if err := em.Emit(AuditRecord{Operator: "a"}); err != nil {
		t.Fatalf("emit: %v", err)
	}
	if err := em.Emit(AuditRecord{Operator: "b"}); err != nil {
		t.Fatalf("emit: %v", err)
	}
	if len(em.Records) != 2 {
		t.Fatalf("records = %d, want 2", len(em.Records))
	}
	if em.Records[0].Operator != "a" || em.Records[1].Operator != "b" {
		t.Errorf("records = %+v", em.Records)
	}
}

func TestSummarizePlan_NilPlan(t *testing.T) {
	s := SummarizePlan(nil)
	if s.Adds != 0 || s.Rotates != 0 || s.Removes != 0 || len(s.AffectedPeers) != 0 {
		t.Errorf("nil plan summary should be zero; got %+v", s)
	}
}

func TestSummarizePlan_OnboardShape(t *testing.T) {
	// Onboard-style intent: StatePlan has an Added row.
	plan := &ApplyPlan{
		StatePlan: &PeerSetDiff{
			Added: []PeerDelta{{Name: "lab-d"}},
		},
		LivePlan: &LiveStateDiff{
			PerRelay: []RelayLiveDiff{
				{Relay: "relay-rla", WouldAdd: []LivePeerOp{{PeerName: "lab-d", NewPubkey: "PK"}}},
			},
		},
	}
	s := SummarizePlan(plan)
	if s.Adds != 2 { // 1 state-side + 1 live-side
		t.Errorf("Adds = %d, want 2 (state + live)", s.Adds)
	}
	if len(s.AffectedPeers) != 1 || s.AffectedPeers[0] != "lab-d" {
		t.Errorf("AffectedPeers = %+v", s.AffectedPeers)
	}
	if len(s.AffectedRelays) != 1 || s.AffectedRelays[0] != "relay-rla" {
		t.Errorf("AffectedRelays = %+v", s.AffectedRelays)
	}
}

func TestSummarizePlan_ReconcileShape_LiveOnly(t *testing.T) {
	// Reconcile: StatePlan is empty; LivePlan carries all the change.
	plan := &ApplyPlan{
		StatePlan: &PeerSetDiff{},
		LivePlan: &LiveStateDiff{
			PerRelay: []RelayLiveDiff{
				{Relay: "relay-rla", WouldAdd: []LivePeerOp{{PeerName: "bravo"}}},
				{Relay: "relay-rlb", WouldAdd: []LivePeerOp{{PeerName: "bravo"}}},
			},
		},
	}
	s := SummarizePlan(plan)
	if s.Adds != 1 { // bravo, deduped across relays
		t.Errorf("Adds = %d, want 1 (deduped across relays)", s.Adds)
	}
	if len(s.AffectedRelays) != 2 {
		t.Errorf("AffectedRelays = %+v", s.AffectedRelays)
	}
}

func TestMakeCorrelationID_Stable(t *testing.T) {
	now := time.Date(2026, 5, 14, 22, 30, 0, 0, time.UTC)
	id := MakeCorrelationID(now, bytes.NewReader([]byte{1, 2, 3, 4}))
	want := "wg-cp0:20260514T223000Z:01020304"
	if id != want {
		t.Errorf("got %q, want %q", id, want)
	}
}

func TestMakeCorrelationID_RandFailure_Fallback(t *testing.T) {
	now := time.Date(2026, 5, 14, 22, 30, 0, 0, time.UTC)
	id := MakeCorrelationID(now, bytes.NewReader([]byte{})) // short
	if !strings.HasPrefix(id, "wg-cp0:20260514T223000Z:") {
		t.Errorf("got %q, want fallback suffix", id)
	}
}

func TestResolveOperator_FallbacksClean(t *testing.T) {
	// Ensure we get a non-empty string in all environments.
	got := ResolveOperator()
	if got == "" {
		t.Errorf("ResolveOperator returned empty string")
	}
}

func TestResolveTrack_EmptyWhenUnset(t *testing.T) {
	t.Setenv("WORK_TRACK", "")
	if got := ResolveTrack(); got != "" {
		t.Errorf("got %q, want empty when WORK_TRACK unset", got)
	}
	t.Setenv("WORK_TRACK", "test-track")
	if got := ResolveTrack(); got != "test-track" {
		t.Errorf("got %q, want test-track", got)
	}
}
