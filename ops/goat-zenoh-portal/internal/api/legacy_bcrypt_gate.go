package api

import (
	"context"
	"crypto/sha256"
	"sync"
	"time"
)

// The legacy API-key fallback costs one bcrypt (~70 ms) per attempt, and an
// anonymous caller can aim it at any not-yet-backfilled prefix (the prefixes
// are public). The gate bounds that cost without locking the vendor's own key
// out (GHSA-fgm6):
//
//   - CPU bound. At most legacyBcryptConcurrency compares run at once,
//     process-wide, whatever the number of clients or prefixes. Admission
//     (the pending count) is checked and taken under one lock and the slot
//     is a channel send, so the bound is exact, not approximate.
//   - No per-prefix lockout. Failures are remembered per (prefix, stored
//     hash, token), not per prefix, so wrong tokens never refuse a different
//     token. The vendor's correct key is always compared when a slot frees.
//   - Repeats are free. An outcome is cached under that key, so a token that
//     has already been compared against this hash costs no further bcrypt,
//     and identical concurrent requests share one compare (singleflight).
//     The cache holds at most legacyBcryptCacheSize entries and evicts the
//     oldest, so a stream of novel tokens cannot grow it.
//   - Bounded starvation. A request that finds both slots busy waits up to
//     legacyBcryptSlotWait in a queue of at most legacyBcryptQueue, rather
//     than failing at once; the queue drains in about
//     (legacyBcryptQueue/legacyBcryptConcurrency) x 70 ms, well inside the
//     wait, so unrelated compares in flight do not turn a valid key into a
//     401. Only when the queue itself is full is a request refused, which
//     takes a sustained flood of novel tokens (about 28 per second plus a
//     full queue). A refusal is answered 503 auth_busy with Retry-After,
//     never 401, because the key was not checked. Under such a flood a
//     valid key competes for a place and needs retries; one success
//     backfills its digest, after which it never reaches this path again.
//     A token whose outcome is already cached is answered from the cache
//     even when the queue is full. Refusals are counted on /api/health as
//     api_key_bcrypt_refused.
//
// Including the stored hash in the key means a rotated row never reuses an
// outcome cached against its old hash.
const (
	legacyBcryptConcurrency = 2
	legacyBcryptQueue       = 8
	legacyBcryptSlotWait    = time.Second
	legacyBcryptCacheSize   = 4096
)

type legacyOutcome int

const (
	legacyMismatch legacyOutcome = iota
	legacyMatch
	legacyRefused
)

type legacyCall struct {
	done chan struct{}
	out  legacyOutcome
}

type legacyBcryptGate struct {
	slots    chan struct{}
	slotWait time.Duration

	mu       sync.Mutex
	pending  int // admitted compares, waiting for or holding a slot
	maxQueue int
	inflight map[[32]byte]*legacyCall
	cache    map[[32]byte]legacyOutcome
	ring     [][32]byte // insertion order of cache keys, for eviction
	next     int
}

func newLegacyBcryptGate() *legacyBcryptGate {
	return &legacyBcryptGate{
		slots:    make(chan struct{}, legacyBcryptConcurrency),
		slotWait: legacyBcryptSlotWait,
		maxQueue: legacyBcryptQueue,
		inflight: map[[32]byte]*legacyCall{},
		cache:    map[[32]byte]legacyOutcome{},
		ring:     make([][32]byte, legacyBcryptCacheSize),
	}
}

// legacyBcryptKey identifies one (row, stored hash, presented token) compare.
func legacyBcryptKey(prefix, storedHash, token string) [32]byte {
	h := sha256.New()
	h.Write([]byte(prefix))
	h.Write([]byte{0})
	h.Write([]byte(storedHash))
	h.Write([]byte{0})
	h.Write([]byte(token))
	var k [32]byte
	h.Sum(k[:0])
	return k
}

// do returns the outcome of compare for key, running compare at most once
// per key while its outcome is cached, and never more than
// legacyBcryptConcurrency at a time across all keys. legacyRefused means no
// compare ran for this request; the caller must treat it as a failed login.
func (g *legacyBcryptGate) do(ctx context.Context, key [32]byte, compare func() bool) legacyOutcome {
	g.mu.Lock()
	if out, ok := g.cache[key]; ok {
		g.mu.Unlock()
		return out
	}
	if c, ok := g.inflight[key]; ok {
		g.mu.Unlock()
		select {
		case <-c.done:
			return c.out
		case <-ctx.Done():
			return legacyRefused
		}
	}
	if g.pending >= cap(g.slots)+g.maxQueue {
		g.mu.Unlock()
		return legacyRefused
	}
	g.pending++
	c := &legacyCall{done: make(chan struct{})}
	g.inflight[key] = c
	g.mu.Unlock()

	c.out = legacyRefused
	timer := time.NewTimer(g.slotWait)
	select {
	case g.slots <- struct{}{}:
		timer.Stop()
		if compare() {
			c.out = legacyMatch
		} else {
			c.out = legacyMismatch
		}
		<-g.slots
	case <-timer.C:
	case <-ctx.Done():
		timer.Stop()
	}

	g.mu.Lock()
	g.pending--
	delete(g.inflight, key)
	if c.out != legacyRefused {
		g.storeLocked(key, c.out)
	}
	g.mu.Unlock()
	close(c.done)
	return c.out
}

// storeLocked caches out under key, evicting whatever key last held this ring
// position. Every cached key sits in at least one ring position (its latest),
// so the cache never holds more than len(g.ring) entries.
func (g *legacyBcryptGate) storeLocked(key [32]byte, out legacyOutcome) {
	delete(g.cache, g.ring[g.next])
	g.ring[g.next] = key
	g.next = (g.next + 1) % len(g.ring)
	g.cache[key] = out
}
