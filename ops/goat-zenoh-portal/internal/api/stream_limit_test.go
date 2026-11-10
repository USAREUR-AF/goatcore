package api

import (
	"context"
	"net/http"
	"net/http/httptest"
	"sync"
	"testing"

	"github.com/usareur-af/goatcore/ops/goat-zenoh-portal/internal/config"
)

func TestStreamSlots_capPerVendorAndRelease(t *testing.T) {
	var ss streamSlots
	acme := &Principal{Vendor: "acme"}
	var releases []func()
	for i := 0; i < 3; i++ {
		rel, ok := ss.acquire(acme, 3)
		if !ok {
			t.Fatalf("stream %d refused under the cap", i+1)
		}
		releases = append(releases, rel)
	}
	if _, ok := ss.acquire(acme, 3); ok {
		t.Fatal("a 4th stream was admitted past a cap of 3")
	}
	// Another key has its own budget.
	if rel, ok := ss.acquire(&Principal{Vendor: "other"}, 3); !ok {
		t.Fatal("one key's streams exhausted another key's budget")
	} else {
		rel()
	}
	releases[0]()
	releases[0]() // release is idempotent
	if got := ss.count("acme"); got != 2 {
		t.Fatalf("after one release (called twice): %d open, want 2", got)
	}
	if _, ok := ss.acquire(acme, 3); !ok {
		t.Fatal("a freed slot was not reusable")
	}
}

func TestStreamSlots_operatorsAndDisabledCapAreUnbounded(t *testing.T) {
	var ss streamSlots
	op := &Principal{Operator: true, Actor: "operator"}
	for i := 0; i < 50; i++ {
		if _, ok := ss.acquire(op, 1); !ok {
			t.Fatal("an operator was capped")
		}
	}
	for i := 0; i < 50; i++ {
		if _, ok := ss.acquire(&Principal{Vendor: "acme"}, 0); !ok {
			t.Fatal("cap 0 must disable the limit")
		}
	}
	if ss.count("acme") != 0 {
		t.Fatal("a disabled cap must not count")
	}
}

func TestStreamSlots_concurrentAcquireNeverExceedsCap(t *testing.T) {
	var ss streamSlots
	acme := &Principal{Vendor: "acme"}
	var mu sync.Mutex
	admitted := 0
	var wg sync.WaitGroup
	for i := 0; i < 64; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if _, ok := ss.acquire(acme, 5); ok {
				mu.Lock()
				admitted++
				mu.Unlock()
			}
		}()
	}
	wg.Wait()
	if admitted != 5 {
		t.Fatalf("admitted %d concurrent streams, want exactly 5", admitted)
	}
}

func TestHandleStream_refusesPastTheCapWith409(t *testing.T) {
	s := &Server{cfg: &config.Config{MaxStreamsPerPrincipal: 2}}
	acme := &Principal{Vendor: "acme"}
	for i := 0; i < 2; i++ {
		if _, ok := s.streams.acquire(acme, 2); !ok {
			t.Fatal("setup: could not fill the budget")
		}
	}
	r := httptest.NewRequest("GET", "/api/topics/stream?key=acme/sensors/temp", nil)
	r = r.WithContext(context.WithValue(r.Context(), principalKey, acme))
	w := httptest.NewRecorder()
	s.handleStream(w, r)
	if w.Code != http.StatusConflict {
		t.Fatalf("status %d, want 409: %s", w.Code, w.Body.String())
	}
	if ct := w.Header().Get("Content-Type"); ct != problemContentType {
		t.Fatalf("Content-Type %q, want a problem body", ct)
	}
	if s.streams.count("acme") != 2 {
		t.Fatal("a refused stream changed the count")
	}
}
