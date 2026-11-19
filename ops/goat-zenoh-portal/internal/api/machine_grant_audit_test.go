package api_test

import (
	"bytes"
	"encoding/json"
	"log/slog"
	"net/http"
	"strings"
	"sync"
	"testing"
)

// Every machine-grant change (quota, affiliation, and the ADR 1113 delegated
// network-admin entitlement) leaves an append-only audit row naming the
// operator. Before, the only trace was the grant row's granted_by column,
// which the next change overwrites, so who granted or withdrew network-admin
// was lost as soon as anyone touched the grant again.
func TestMachineGrant_EveryChangeIsAudited(t *testing.T) {
	ts, reg := newMachineServer(t)
	c := &http.Client{}
	steps := []map[string]any{
		{"custodian": "morgan", "quota": 5, "affiliation": "usareur-af", "site": "zagros"},
		{"custodian": "morgan", "quota": 0, "network_admin": true},
		{"custodian": "morgan", "quota": 3, "network_admin": false},
	}
	for i, body := range steps {
		r := doJSON(t, c, "POST", ts.URL+"/api/machines/grant", "dev-operator-key", body)
		if r.StatusCode != http.StatusOK {
			t.Fatalf("step %d: %d %s", i, r.StatusCode, mustReadBody(t, r))
		}
		if b := mustReadJSON(t, r); b["audit"] != "recorded" {
			t.Errorf("step %d: response audit = %v, want recorded", i, b["audit"])
		}
	}
	events, err := reg.AuditEvents(t.Context(), "machine.grant", 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) != len(steps) {
		t.Fatalf("machine.grant audit rows = %d, want %d: %+v", len(events), len(steps), events)
	}
	// Newest first: withdraw, grant, seed.
	wantNA := []any{false, true, nil}
	for i, e := range events {
		if e.Actor == "" || e.Target != "morgan" {
			t.Errorf("row %d: actor=%q target=%q, want an operator actor and target morgan", i, e.Actor, e.Target)
		}
		var d map[string]any
		if err := json.Unmarshal([]byte(e.Details), &d); err != nil {
			t.Fatalf("row %d details %q: %v", i, e.Details, err)
		}
		if d["network_admin"] != wantNA[i] {
			t.Errorf("row %d: network_admin = %v, want %v (details %s)", i, d["network_admin"], wantNA[i], e.Details)
		}
		if _, ok := d["quota"]; !ok {
			t.Errorf("row %d: details %s lack the resulting quota", i, e.Details)
		}
	}
}

// A failed audit write is logged at ERROR rather than discarded, and the 200
// says audit: failed, so an unrecorded network-admin change is visible to the
// caller and to whoever watches the logs.
func TestMachineGrant_AuditWriteFailureIsLogged(t *testing.T) {
	ts, reg := newMachineServer(t)
	if _, err := reg.DB().ExecContext(t.Context(), `ALTER TABLE audit_events RENAME TO audit_events_unavailable`); err != nil {
		t.Fatal(err)
	}
	var buf bytes.Buffer
	prev := slog.Default()
	slog.SetDefault(slog.New(slog.NewTextHandler(&syncWriter{w: &buf}, &slog.HandlerOptions{Level: slog.LevelError})))
	t.Cleanup(func() { slog.SetDefault(prev) })

	r := doJSON(t, &http.Client{}, "POST", ts.URL+"/api/machines/grant", "dev-operator-key",
		map[string]any{"custodian": "morgan", "quota": 5, "affiliation": "usareur-af", "site": "zagros"})
	if r.StatusCode != http.StatusOK {
		t.Fatalf("grant: %d %s", r.StatusCode, mustReadBody(t, r))
	}
	if b := mustReadJSON(t, r); b["audit"] != "failed" {
		t.Errorf("response audit = %v, want failed: the grant landed but its record did not", b["audit"])
	}
	logged := buf.String()
	if !strings.Contains(logged, "level=ERROR") || !strings.Contains(logged, "machine.grant audit record NOT written") || !strings.Contains(logged, "custodian=morgan") {
		t.Fatalf("log = %q, want an ERROR naming the unrecorded machine.grant", logged)
	}
}

type syncWriter struct {
	mu sync.Mutex
	w  *bytes.Buffer
}

func (s *syncWriter) Write(p []byte) (int, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.w.Write(p)
}
