package server

import (
	"context"
	"errors"
	"net/http"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/agent"
)

// row returns a source's row in /v1/status as the API serves it.
func (e *env) row(slot string) (map[string]any, map[string]any) {
	e.t.Helper()
	code, st := e.call(e.client("lead.g1"), http.MethodGet, "/v1/status", nil)
	if code != http.StatusOK {
		e.t.Fatalf("status: %d", code)
	}
	for _, s := range st["sources"].([]any) {
		if m := s.(map[string]any); m["slot"] == slot {
			return st, m
		}
	}
	return st, nil
}

// Status says what the running router admits. While a restart is under way the old process still
// admits a source the rules have just removed, and does not admit one they have just added.
func TestStatusFollowsTheRunningRouter(t *testing.T) {
	ctx := context.Background()
	e := newEnv(t)
	e.publish(doc(1, source(srcA, "cr_a")))
	if err := e.agent.Pull(ctx); err != nil {
		t.Fatal(err)
	}
	want := func(when, slot string, carried, pending bool, carriage, change string) {
		t.Helper()
		st, m := e.row(slot)
		if m == nil {
			t.Fatalf("%s: no row for the source", when)
		}
		got, _ := m["change"].(string)
		if m["carried"] != carried || m["carriage"] != carriage || got != change || st["change_pending"] != pending {
			t.Fatalf("%s: carried %v, carriage %v, change %q, pending %v", when, m["carried"], m["carriage"], got, st["change_pending"])
		}
	}
	want("settled", srcA, true, false, "ok", "")

	// Generation 2 removes a and adds b. Look while the restart is under way.
	looked := false
	e.rt.duringRestart = func() {
		looked = true
		want("restarting", srcA, true, true, "expiring", "removing")
		want("restarting", srcB, false, true, "refused", "adding")
		if st, _ := e.row(srcA); st["sources_carried"].(float64) != 1 || st["sources_wanted"].(float64) != 1 {
			t.Fatalf("restarting: %v carried, %v wanted", st["sources_carried"], st["sources_wanted"])
		}
	}
	e.publish(doc(2, source(srcB, "cr_b")))
	if err := e.agent.Pull(ctx); err != nil || !looked {
		t.Fatalf("pull: %v (looked %v)", err, looked)
	}
	e.rt.duringRestart = nil
	want("after", srcA, false, false, "refused", "")
	want("after", srcB, true, false, "ok", "")

	// A rule ends, and nothing has acted on it yet.
	b := source(srcB, "cr_b")
	b["kind"], b["until"] = "named", time.Now().Add(48*time.Hour).UTC().Format(time.RFC3339)
	e.publish(doc(3, b))
	if err := e.agent.Pull(ctx); err != nil {
		t.Fatal(err)
	}
	step := e.clock()
	step(72 * time.Hour)
	want("ended, router not changed", srcB, true, true, "expiring", "removing")
	if err := e.agent.Reconcile(ctx); err != nil {
		t.Fatal(err)
	}
	want("ended", srcB, false, false, "refused", "")

	// A certificate changes: still carried, by the old certificate.
	rotated := source(srcA, "cr_a2")
	rotated["principals"] = []any{srcA + ".g2"}
	e.publish(doc(4, source(srcA, "cr_a2")))
	if err := e.agent.Pull(ctx); err != nil {
		t.Fatal(err)
	}
	e.rt.duringRestart = func() { want("rotating", srcA, true, true, "ok", "changing") }
	e.publish(doc(5, rotated))
	if err := e.agent.Pull(ctx); err != nil {
		t.Fatal(err)
	}
	e.rt.duringRestart = nil
	want("rotated", srcA, true, false, "ok", "")

	// A new service process reads it from the files.
	again, err := agent.New(e.cfg, e.rt)
	if err != nil {
		t.Fatal(err)
	}
	if st := again.Status(); st.SourcesCarried == nil || *st.SourcesCarried != 1 || st.ChangePending {
		t.Fatalf("after a restart of the service: %+v", st)
	}

	// docker fails: what runs is not known.
	e.rt.restartErr = errors.New("no docker")
	e.publish(doc(6, rotated, source(srcB, "cr_b6")))
	if err := e.agent.Pull(ctx); err == nil {
		t.Fatal("pull without docker")
	}
	if st := e.agent.Status(); st.SourcesCarried != nil || !st.ChangePending {
		t.Fatalf("with docker failing: carried %v, pending %v", st.SourcesCarried, st.ChangePending)
	}
}

// A pure addition is pending too, and a slot the router admits though no rule names it is shown.
func TestStatusShowsAdditionsAndUnnamedSlots(t *testing.T) {
	ctx := context.Background()
	e := newEnv(t)
	e.publish(doc(1, source(srcA, "cr_a")))
	if err := e.agent.Pull(ctx); err != nil {
		t.Fatal(err)
	}
	looked := false
	e.rt.duringRestart = func() {
		looked = true
		if st := e.agent.Status(); !st.ChangePending || *st.SourcesCarried != 1 || st.SourcesWanted != 2 {
			t.Errorf("adding: pending %v, %d carried, %d wanted", st.ChangePending, *st.SourcesCarried, st.SourcesWanted)
		}
	}
	e.publish(doc(2, source(srcA, "cr_a"), source(srcB, "cr_b")))
	if err := e.agent.Pull(ctx); err != nil || !looked {
		t.Fatal(err, looked)
	}
	e.rt.duringRestart = nil

	// The router runs b's grants; the state directory has never heard of b.
	cfg := *e.cfg
	cfg.StateDir = t.TempDir()
	fresh, err := agent.New(&cfg, e.rt)
	if err != nil {
		t.Fatal(err)
	}
	st := fresh.Status()
	if len(st.Sources) != 2 || *st.SourcesCarried != 2 || st.SourcesWanted != 0 || !st.ChangePending {
		t.Fatalf("unnamed slots: %+v", st)
	}
	for _, s := range st.Sources {
		if !s.Carried || s.Change != "removing" || s.Carriage != "expiring" || s.Origin != "none" {
			t.Fatalf("unnamed slot: %+v", s)
		}
	}
}
