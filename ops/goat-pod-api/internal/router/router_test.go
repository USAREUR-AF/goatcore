package router

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// fake fails health for configurations containing "bad".
type fake struct {
	path     string
	restarts int
	stopped  bool
	halted   int
	stopErr  error
}

func (f *fake) Restart(context.Context) error { f.restarts++; return nil }
func (f *fake) Stop(context.Context) error    { f.halted++; return f.stopErr }
func (f *fake) Healthy(context.Context) error {
	raw, _ := os.ReadFile(f.path)
	if strings.Contains(string(raw), "bad") {
		if f.stopped {
			return ErrStopped
		}
		return errors.New("starting")
	}
	return nil
}

func setup(t *testing.T, stopped bool) (*Applier, *fake) {
	dir := t.TempDir()
	files := Files{Config: filepath.Join(dir, "config.json5"), LastGood: filepath.Join(dir, "config.last-known-good.json5"),
		AppliedSHA: filepath.Join(dir, ".config.applied.sha256")}
	f := &fake{path: files.Config, stopped: stopped}
	return &Applier{Files: files, Runtime: f, Timeout: 300 * time.Millisecond, Poll: 10 * time.Millisecond}, f
}

func TestApplyAndNoop(t *testing.T) {
	a, f := setup(t, false)
	ctx := context.Background()
	if res, err := a.Apply(ctx, []byte("one")); err != nil || !res.Changed {
		t.Fatalf("first apply: %+v %v", res, err)
	}
	if res, err := a.Apply(ctx, []byte("one")); err != nil || res.Changed {
		t.Fatalf("second apply restarted: %+v %v", res, err)
	}
	if f.restarts != 1 {
		t.Fatalf("restarts %d", f.restarts)
	}
	if _, err := a.Apply(ctx, []byte("two")); err != nil {
		t.Fatal(err)
	}
	lg, _ := os.ReadFile(a.Files.LastGood)
	if string(lg) != "one" {
		t.Fatalf("last good %q", lg)
	}
}

func TestRollback(t *testing.T) {
	for _, stopped := range []bool{false, true} {
		a, f := setup(t, stopped)
		ctx := context.Background()
		if _, err := a.Apply(ctx, []byte("good")); err != nil {
			t.Fatal(err)
		}
		res, err := a.Apply(ctx, []byte("bad"))
		if err == nil || !res.RolledBack {
			t.Fatalf("bad config not rolled back: %+v %v", res, err)
		}
		cur, _ := os.ReadFile(a.Files.Config)
		if string(cur) != "good" {
			t.Fatalf("live config after rollback %q", cur)
		}
		if f.restarts != 3 {
			t.Fatalf("restarts %d", f.restarts)
		}
		// The restored config counts as applied: applying it again restarts nothing.
		if res, err := a.Apply(ctx, []byte("good")); err != nil || res.Changed {
			t.Fatalf("restored config re-applied: %+v %v", res, err)
		}
	}
}
