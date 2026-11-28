package server

import (
	"bytes"
	"context"
	"net/http"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/agent"
)

func admits(conf, slot string) bool { return strings.Contains(conf, `"`+slot+`.g1"`) }

// clock is a settable agent clock.
func (e *env) clock() func(time.Duration) {
	offset := time.Duration(0)
	e.agent.Now = func() time.Time { return time.Now().Add(offset) }
	return func(d time.Duration) { offset += d }
}

func TestFailedRemovalNeverRestoresTheSource(t *testing.T) {
	ctx := context.Background()
	carries := func(conf string) bool { return strings.Contains(conf, "carriage-src-") }
	// After each removal only b is left, and the router rejects any configuration that carries b
	// alone; the base configuration works.
	rejectB := func(conf string) bool { return carries(conf) && !admits(conf, srcA) }
	for name, remove := range map[string]func(t *testing.T, e *env) error{
		"new generation": func(_ *testing.T, e *env) error {
			e.publish(doc(2, source(srcB, "cr_b")))
			return e.agent.Pull(ctx)
		},
		"expiry": func(_ *testing.T, e *env) error {
			e.clock()(2 * time.Hour)
			return e.agent.Reconcile(ctx)
		},
		"revoke": func(t *testing.T, e *env) error {
			code, out := e.call(e.client("lead.g1"), "POST", "/v1/sources/"+srcA+"/revoke", map[string]any{"reason": "lost"})
			if code != http.StatusAccepted || !strings.Contains(out["warning"].(string), "suspended") {
				t.Fatalf("revoke: %d %v", code, out)
			}
			return nil
		},
	} {
		t.Run(name, func(t *testing.T) {
			e := newEnv(t)
			a := source(srcA, "cr_a")
			a["kind"], a["until"] = "named", time.Now().Add(time.Hour).UTC().Format(time.RFC3339)
			e.publish(doc(1, a, source(srcB, "cr_b")))
			if err := e.agent.Pull(ctx); err != nil {
				t.Fatal(err)
			}
			e.rt.fail = rejectB
			err := remove(t, e)
			if name != "revoke" && (err == nil || !strings.Contains(err.Error(), "suspended")) {
				t.Fatalf("removal: %v", err)
			}
			if e.liveConfig() != baseCfg || e.rt.stops != 0 {
				t.Fatalf("not on the base configuration (stopped %d times)", e.rt.stops)
			}
			lg, _ := os.ReadFile(e.cfg.Router.LastGood)
			if string(lg) != baseCfg {
				t.Fatal("the last known good configuration still admits a removed source")
			}
			st := e.agent.Status()
			if !st.Suspended || !strings.Contains(st.LastApplyError, "suspended") || *st.SourcesCarried != 0 {
				t.Fatalf("status: suspended %v, %d carried, %q", st.Suspended, *st.SourcesCarried, st.LastApplyError)
			}
			for _, s := range st.Sources {
				if s.Carried || s.Carriage != "refused" {
					t.Fatalf("status shows %s as carried", s.Slot)
				}
			}
			if name == "new generation" && st.Generation != 2 {
				t.Fatalf("fell back to generation %d", st.Generation)
			}

			// Not again in the same tick; then once per tick; then it works and b is carried.
			n := e.rt.restarts
			_ = e.agent.Reconcile(ctx)
			if e.rt.restarts != n {
				t.Fatal("retried at once")
			}
			old := e.agent.Now
			e.agent.Now = func() time.Time { return old().Add(40 * time.Second) }
			if err := e.agent.Reconcile(ctx); err == nil || e.rt.restarts == n || e.liveConfig() != baseCfg {
				t.Fatalf("no failing retry at the next tick: %v", err)
			}
			// A restart of the service keeps retrying.
			again, err := agent.New(e.cfg, e.rt)
			if err != nil {
				t.Fatal(err)
			}
			again.Now = e.agent.Now
			again.Log.SetOutput(new(bytes.Buffer))
			if !again.Status().Suspended {
				t.Fatal("suspension forgotten across a restart")
			}
			e.rt.fail = nil
			if err := again.Reconcile(ctx); err != nil {
				t.Fatal(err)
			}
			if admits(e.liveConfig(), srcA) || !admits(e.liveConfig(), srcB) {
				t.Fatal("after the retry: want b only")
			}
			if st := again.Status(); st.Suspended || st.LastApplyError != "" || *st.SourcesCarried != 1 {
				t.Fatalf("status after recovery: %+v", st)
			}
		})
	}
}

func TestFailedRemovalWithFailingBaseStopsTheRouter(t *testing.T) {
	ctx := context.Background()
	e := newEnv(t)
	e.publish(doc(1, source(srcA, "cr_a")))
	if err := e.agent.Pull(ctx); err != nil {
		t.Fatal(err)
	}
	e.rt.fail = func(string) bool { return true }
	e.publish(doc(2))
	if err := e.agent.Pull(ctx); err == nil || !strings.Contains(err.Error(), "router is stopped") {
		t.Fatalf("pull: %v", err)
	}
	if e.rt.stops != 1 || admits(e.liveConfig(), srcA) {
		t.Fatalf("stopped %d times, source admitted %v", e.rt.stops, admits(e.liveConfig(), srcA))
	}
	if st := e.agent.Status(); st.SourcesCarried == nil || *st.SourcesCarried != 0 || !st.Suspended {
		t.Fatalf("a stopped router carries nothing: %+v", st)
	}
	// The next tick tries again and stops it again; then the router recovers.
	e.clock()(40 * time.Second)
	if err := e.agent.Reconcile(ctx); err == nil || e.rt.stops != 2 {
		t.Fatalf("retry: %v, stopped %d times", err, e.rt.stops)
	}
	e.rt.fail = nil
	e.clock()(5 * time.Minute)
	if err := e.agent.Reconcile(ctx); err != nil || e.agent.Status().Suspended {
		t.Fatalf("recovery: %v", err)
	}
}

// With the rules impossible to render, a certificate the live configuration still admits but the
// rules no longer carry is removed all the same.
func TestUnrenderableRemovalFallsToBase(t *testing.T) {
	ctx := context.Background()
	e := newEnv(t)
	e.publish(doc(1, source(srcA, "cr_a")))
	if err := e.agent.Pull(ctx); err != nil {
		t.Fatal(err)
	}
	b := source(srcB, "cr_b")
	b["principals"] = []any{"fabric-router"} // the fabric router's own certificate: refused
	e.publish(doc(2, b))
	if err := e.agent.Pull(ctx); err == nil || e.liveConfig() != baseCfg {
		t.Fatalf("pull: %v; source still admitted: %v", err, admits(e.liveConfig(), srcA))
	}
}

// A caller that goes away cannot leave a removal half done.
func TestRemovalOutlivesItsCaller(t *testing.T) {
	e := newEnv(t)
	e.publish(doc(1, source(srcA, "cr_a")))
	if err := e.agent.Pull(context.Background()); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	who, err := e.agent.Authorize("lead.g1")
	if err != nil {
		t.Fatal(err)
	}
	e.rt.needCtx = true
	if _, err := e.agent.Revoke(ctx, who, srcA, agent.Change{Reason: "x"}); err != nil || admits(e.liveConfig(), srcA) || e.agent.Status().LastApplyError != "" {
		t.Fatalf("revoke: %v %q", err, e.agent.Status().LastApplyError)
	}
}

// An addition keeps the rollback it had: previous configuration and generation, not retried.
func TestFailedAdditionStillRollsBack(t *testing.T) {
	ctx := context.Background()
	e := newEnv(t)
	e.publish(doc(1, source(srcB, "cr_b")))
	if err := e.agent.Pull(ctx); err != nil {
		t.Fatal(err)
	}
	before := e.liveConfig()
	e.rt.fail = func(conf string) bool { return admits(conf, srcA) }
	e.publish(doc(2, source(srcA, "cr_a"), source(srcB, "cr_b")))
	if err := e.agent.Pull(ctx); err == nil {
		t.Fatal("rejected addition adopted")
	}
	n := e.rt.restarts
	e.clock()(10 * time.Minute)
	_ = e.agent.Pull(ctx)
	_ = e.agent.Reconcile(ctx)
	st := e.agent.Status()
	if e.liveConfig() != before || st.Generation != 1 || st.Suspended || *st.SourcesCarried != 1 || e.rt.restarts != n {
		t.Fatalf("generation %d, suspended %v, %d carried, restarts %d to %d", st.Generation, st.Suspended, *st.SourcesCarried, n, e.rt.restarts)
	}
}
