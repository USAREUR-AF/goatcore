package server

import (
	"context"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/agent"
	"github.com/usareur-af/goatcore/ops/goat-pod-api/internal/generation"
)

// Each end date must take effect when it passes. The pull interval is 60 s here, so a change
// seen within two seconds did not come from the ticker.
func TestExpiryIsNotLeftToTheTicker(t *testing.T) {
	const grace = 2 * time.Second
	waitGone := func(e *env, slot string, until time.Time) {
		e.t.Helper()
		cn := `"cert_common_names":["` + slot + `.g1"]`
		time.Sleep(time.Until(until.Add(-300 * time.Millisecond)))
		if !strings.Contains(e.liveConfig(), cn) {
			e.t.Fatal("cut off before its end date")
		}
		for strings.Contains(e.liveConfig(), cn) {
			if time.Since(until) > grace {
				e.t.Fatalf("still carried %s after its end date", time.Since(until))
			}
			time.Sleep(20 * time.Millisecond)
		}
	}
	start := func(e *env) {
		ctx, cancel := context.WithCancel(context.Background())
		done := make(chan struct{})
		go func() { e.agent.Run(ctx); close(done) }()
		// The subtest's own cleanup: it must stop before the subtest's directory is removed.
		e.t.Cleanup(func() { cancel(); <-done })
	}
	waitFor := func(e *env, slot string) {
		e.t.Helper()
		for i := 0; !strings.Contains(e.liveConfig(), slot+".g1"); i++ {
			if i > 200 {
				e.t.Fatal("never carried")
			}
			time.Sleep(20 * time.Millisecond)
		}
	}
	soon := func() time.Time { return time.Now().Add(3 * time.Second).Truncate(time.Second).UTC() }

	t.Run("central rule with the portal unreachable", func(t *testing.T) {
		e := newEnv(t)
		until := soon()
		s := source(srcA, "cr_a")
		s["kind"], s["until"] = "named", until.Format(time.RFC3339)
		e.publish(doc(1, s))
		if err := e.agent.Pull(context.Background()); err != nil {
			t.Fatal(err)
		}
		e.portal.Close()
		start(e)
		waitGone(e, srcA, until)
		if st := e.agent.Status(); st.Generation != 1 || st.LastPullError == "" {
			t.Fatalf("expected generation 1 and a pull error: %+v", st)
		}
	})

	t.Run("central rule while a pull hangs", func(t *testing.T) {
		e := newEnv(t)
		until := soon()
		s := source(srcA, "cr_a")
		s["kind"], s["until"] = "named", until.Format(time.RFC3339)
		e.publish(doc(1, s))
		if err := e.agent.Pull(context.Background()); err != nil {
			t.Fatal(err)
		}
		// Dropped packets, not a refused connection: the portal accepts and never answers.
		release := make(chan struct{})
		t.Cleanup(func() { close(release) })
		e.portal.Config.Handler = http.HandlerFunc(func(_ http.ResponseWriter, r *http.Request) {
			select {
			case <-release:
			case <-r.Context().Done():
			}
		})
		start(e)
		waitGone(e, srcA, until)
	})

	t.Run("local rule made while waiting", func(t *testing.T) {
		e := newEnv(t)
		e.publish(doc(1, source(srcA, "cr_a")))
		start(e)
		waitFor(e, srcA)
		until := soon()
		if _, err := e.agent.Allow(context.Background(), generation.Authority{Identity: "test:lead", CertCN: "lead.g1"},
			agent.Change{Source: srcB + ".g1", Until: &until, Reason: "x"}); err != nil {
			t.Fatal(err)
		}
		waitGone(e, srcB, until)
		if !strings.Contains(e.liveConfig(), srcA+".g1") {
			t.Fatal("the org rule went with the local rule")
		}
	})
}
