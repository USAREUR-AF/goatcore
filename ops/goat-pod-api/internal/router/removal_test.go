package router

import (
	"context"
	"errors"
	"os"
	"strings"
	"testing"
)

func TestApplyRemoval(t *testing.T) {
	ctx := context.Background()
	read := func(p string) string { b, _ := os.ReadFile(p); return string(b) } // #nosec G304 -- test temp dir
	for _, c := range []struct {
		name, next, base, live string
		nilNext                bool
		stopErr                error
		halted                 int
		want                   string
	}{
		{name: "applied", next: "next", base: "base", live: "next"},
		{name: "rejected", next: "bad", base: "base", live: "base", want: "base configuration is running"},
		{name: "base rejected too", next: "bad1", base: "bad2", live: "bad2", halted: 1, want: "router is stopped"},
		{name: "base missing", next: "bad", live: "bad", halted: 1, want: "router is stopped"},
		{name: "cannot be stopped", next: "bad1", base: "bad2", live: "bad2", halted: 1, stopErr: errors.New("no docker"), want: "COULD NOT BE STOPPED"},
		{name: "nothing rendered", nilNext: true, base: "base", live: "base", want: "base configuration is running"},
	} {
		a, f := setup(t, true)
		// Twice, so that the last known good file holds a configuration that admits the source.
		for _, conf := range []string{"old 1 admits the source", "old 2 admits the source"} {
			if _, err := a.Apply(ctx, []byte(conf)); err != nil {
				t.Fatal(err)
			}
		}
		f.stopErr = c.stopErr
		var next, base []byte
		if !c.nilNext {
			next = []byte(c.next)
		}
		if c.base != "" {
			base = []byte(c.base)
		}
		err := a.ApplyRemoval(ctx, next, base)
		if (c.want == "") != (err == nil) || (err != nil && (!errors.Is(err, ErrSuspended) || !strings.Contains(err.Error(), c.want))) {
			t.Errorf("%s: %v", c.name, err)
		}
		if got := read(a.Files.Config); got != c.live || f.halted != c.halted {
			t.Errorf("%s: live %q, stopped %d times", c.name, got, f.halted)
		}
		if strings.Contains(read(a.Files.Config)+read(a.Files.LastGood), "old") {
			t.Errorf("%s: the old configuration can come back", c.name)
		}
	}
}
