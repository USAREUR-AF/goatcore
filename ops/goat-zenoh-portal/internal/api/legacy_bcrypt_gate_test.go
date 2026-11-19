package api

import (
	"context"
	"fmt"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func gateKey(token string) [32]byte { return legacyBcryptKey("acme", "$2a$hash", token) }

// waitPending polls until the gate has n admitted compares.
func waitPending(t *testing.T, g *legacyBcryptGate, n int) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		g.mu.Lock()
		p := g.pending
		g.mu.Unlock()
		if p == n {
			return
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatalf("gate never reached %d pending compares", n)
}

// holdSlots occupies every slot with a compare that blocks until the
// returned release func is called.
func holdSlots(t *testing.T, g *legacyBcryptGate) (release func(), done *sync.WaitGroup) {
	t.Helper()
	gate := make(chan struct{})
	done = &sync.WaitGroup{}
	for i := range cap(g.slots) {
		done.Add(1)
		go func() {
			defer done.Done()
			g.do(t.Context(), legacyBcryptKey(fmt.Sprintf("other-%d", i), "h", "t"), func() bool {
				<-gate
				return false
			})
		}()
	}
	waitPending(t, g, cap(g.slots))
	for len(g.slots) < cap(g.slots) {
		time.Sleep(time.Millisecond)
	}
	var once sync.Once
	return func() { once.Do(func() { close(gate) }) }, done
}

// A wrong token is compared once; repeating it costs nothing, and any number
// of wrong tokens never stop a different (correct) token being compared.
func TestLegacyBcryptGate_failuresDoNotLockOutOtherTokens(t *testing.T) {
	g := newLegacyBcryptGate()
	var compares int
	wrong := func() bool { compares++; return false }
	for i := range 100 {
		if out := g.do(t.Context(), gateKey(fmt.Sprintf("junk-%d", i)), wrong); out != legacyMismatch {
			t.Fatalf("junk %d: outcome %v; want mismatch", i, out)
		}
	}
	if compares != 100 {
		t.Fatalf("100 distinct wrong tokens cost %d compares; want 100", compares)
	}
	for range 5 {
		g.do(t.Context(), gateKey("junk-7"), wrong)
	}
	if compares != 100 {
		t.Fatalf("repeating a failed token cost %d more compare(s); want 0", compares-100)
	}
	if out := g.do(t.Context(), gateKey("right"), func() bool { return true }); out != legacyMatch {
		t.Fatalf("correct token after 100 failures: outcome %v; want match", out)
	}
}

// The cache key includes the stored hash, so a rotated row is compared afresh.
func TestLegacyBcryptGate_keyIncludesStoredHash(t *testing.T) {
	if legacyBcryptKey("acme", "old", "tok") == legacyBcryptKey("acme", "new", "tok") {
		t.Fatal("same key for different stored hashes")
	}
	if legacyBcryptKey("acme", "h", "tok") == legacyBcryptKey("acme2", "h", "tok") {
		t.Fatal("same key for different prefixes")
	}
}

// Finding 3: the in-flight bound is exact. With every slot held and the
// queue full, the next distinct token is refused at once, no more than
// legacyBcryptConcurrency compares ever run together, and every admitted
// request completes once the slots free.
func TestLegacyBcryptGate_boundIsExact(t *testing.T) {
	g := newLegacyBcryptGate()
	var running, maxRunning atomic.Int32
	gate := make(chan struct{})
	compare := func() bool {
		n := running.Add(1)
		for {
			m := maxRunning.Load()
			if n <= m || maxRunning.CompareAndSwap(m, n) {
				break
			}
		}
		<-gate
		running.Add(-1)
		return false
	}
	admitted := cap(g.slots) + g.maxQueue
	outs := make(chan legacyOutcome, admitted)
	for i := range admitted {
		go func() { outs <- g.do(t.Context(), gateKey(fmt.Sprintf("q-%d", i)), compare) }()
	}
	waitPending(t, g, admitted)

	var extra atomic.Int32
	start := time.Now()
	if out := g.do(t.Context(), gateKey("overflow"), func() bool { extra.Add(1); return true }); out != legacyRefused {
		t.Fatalf("request past the queue: outcome %v; want refused", out)
	}
	if extra.Load() != 0 {
		t.Fatal("a refused request ran a compare")
	}
	if d := time.Since(start); d > 100*time.Millisecond {
		t.Fatalf("refusal past a full queue took %v; want immediate", d)
	}

	close(gate)
	for range admitted {
		if out := <-outs; out != legacyMismatch {
			t.Fatalf("admitted request: outcome %v; want mismatch", out)
		}
	}
	if m := maxRunning.Load(); m > int32(cap(g.slots)) {
		t.Fatalf("%d compares ran at once; cap is %d", m, cap(g.slots))
	}
	if g.pending != 0 || len(g.inflight) != 0 {
		t.Fatalf("gate leaked state: pending=%d inflight=%d", g.pending, len(g.inflight))
	}
}

// Many concurrent distinct tokens, under -race: the cap holds and each
// request is either compared or refused, never both.
func TestLegacyBcryptGate_concurrentStress(t *testing.T) {
	g := newLegacyBcryptGate()
	var running, maxRunning, compares, refused atomic.Int32
	var wg sync.WaitGroup
	const n = 200
	for i := range n {
		wg.Add(1)
		go func() {
			defer wg.Done()
			out := g.do(t.Context(), gateKey(fmt.Sprintf("s-%d", i)), func() bool {
				c := running.Add(1)
				for {
					m := maxRunning.Load()
					if c <= m || maxRunning.CompareAndSwap(m, c) {
						break
					}
				}
				compares.Add(1)
				time.Sleep(2 * time.Millisecond)
				running.Add(-1)
				return false
			})
			if out == legacyRefused {
				refused.Add(1)
			}
		}()
	}
	wg.Wait()
	if m := maxRunning.Load(); m > int32(cap(g.slots)) {
		t.Fatalf("%d compares ran at once; cap is %d", m, cap(g.slots))
	}
	if compares.Load()+refused.Load() != n {
		t.Fatalf("compares %d + refused %d != %d requests", compares.Load(), refused.Load(), n)
	}
}

// Finding 2: unrelated compares holding every slot do not turn a valid key
// into a 401. It waits for a free slot and is compared.
func TestLegacyBcryptGate_validKeyWaitsOutUnrelatedCompares(t *testing.T) {
	g := newLegacyBcryptGate()
	release, done := holdSlots(t, g)
	defer done.Wait()
	defer release()

	got := make(chan legacyOutcome, 1)
	go func() { got <- g.do(t.Context(), gateKey("right"), func() bool { return true }) }()
	time.Sleep(50 * time.Millisecond)
	select {
	case out := <-got:
		t.Fatalf("valid key returned %v while every slot was held; want it to wait", out)
	default:
	}
	release()
	select {
	case out := <-got:
		if out != legacyMatch {
			t.Fatalf("valid key after slots freed: outcome %v; want match", out)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("valid key never got a slot")
	}
}

// The residual starvation bound: if the slots stay busy for the whole wait,
// the request is refused after slotWait, not held indefinitely.
func TestLegacyBcryptGate_waitIsBounded(t *testing.T) {
	g := newLegacyBcryptGate()
	g.slotWait = 50 * time.Millisecond
	release, done := holdSlots(t, g)
	defer done.Wait()
	defer release()

	start := time.Now()
	out := g.do(t.Context(), gateKey("right"), func() bool { return true })
	d := time.Since(start)
	if out != legacyRefused {
		t.Fatalf("outcome %v with every slot held past the wait; want refused", out)
	}
	if d < g.slotWait || d > g.slotWait+time.Second {
		t.Fatalf("refusal after %v; want about %v", d, g.slotWait)
	}
	// A refusal is not cached: the same key is compared once a slot frees.
	release()
	done.Wait()
	if out := g.do(t.Context(), gateKey("right"), func() bool { return true }); out != legacyMatch {
		t.Fatalf("after the flood: outcome %v; want match", out)
	}
}

// A cancelled request leaves the queue and is not cached.
func TestLegacyBcryptGate_contextCancelLeavesQueue(t *testing.T) {
	g := newLegacyBcryptGate()
	release, done := holdSlots(t, g)
	defer done.Wait()
	defer release()

	ctx, cancel := context.WithCancel(t.Context())
	got := make(chan legacyOutcome, 1)
	go func() { got <- g.do(ctx, gateKey("gone"), func() bool { return true }) }()
	waitPending(t, g, cap(g.slots)+1)
	cancel()
	if out := <-got; out != legacyRefused {
		t.Fatalf("cancelled request: outcome %v; want refused", out)
	}
	waitPending(t, g, cap(g.slots))
	g.mu.Lock()
	_, cached := g.cache[gateKey("gone")]
	g.mu.Unlock()
	if cached {
		t.Fatal("a cancelled request cached an outcome")
	}
}

// Identical concurrent requests share one compare.
func TestLegacyBcryptGate_singleflight(t *testing.T) {
	g := newLegacyBcryptGate()
	var compares atomic.Int32
	gate := make(chan struct{})
	var wg sync.WaitGroup
	outs := make([]legacyOutcome, 5)
	for i := range outs {
		wg.Add(1)
		go func() {
			defer wg.Done()
			outs[i] = g.do(t.Context(), gateKey("same"), func() bool {
				compares.Add(1)
				<-gate
				return true
			})
		}()
	}
	waitPending(t, g, 1)
	time.Sleep(20 * time.Millisecond) // let the others join the in-flight call
	close(gate)
	wg.Wait()
	if c := compares.Load(); c != 1 {
		t.Fatalf("5 identical concurrent requests cost %d compares; want 1", c)
	}
	for i, out := range outs {
		if out != legacyMatch {
			t.Fatalf("request %d: outcome %v; want match", i, out)
		}
	}
}

// A stream of novel tokens cannot grow the outcome cache past its size, even
// when evicted keys are stored again.
func TestLegacyBcryptGate_cacheIsBounded(t *testing.T) {
	g := newLegacyBcryptGate()
	g.ring = make([][32]byte, 4)
	for round := range 3 {
		for i := range 50 {
			g.do(t.Context(), gateKey(fmt.Sprintf("n-%d", i%10)), func() bool { return false })
			if len(g.cache) > len(g.ring) {
				t.Fatalf("round %d token %d: cache holds %d entries; cap %d", round, i, len(g.cache), len(g.ring))
			}
		}
	}
	if len(g.cache) == 0 {
		t.Fatal("cache is empty; outcomes are not being kept")
	}
	if got := newLegacyBcryptGate(); len(got.ring) != legacyBcryptCacheSize {
		t.Fatalf("default cache size %d; want %d", len(got.ring), legacyBcryptCacheSize)
	}
}
